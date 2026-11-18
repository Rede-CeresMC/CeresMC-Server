//! Breaking and placing blocks.

use super::*;
use ceres_protocol::{UseItem, block_action, use_item_action};

/// Block position next to `pos` on `face` (0 down, 1 up, 2 north, 3 south, 4 west, 5 east).
fn side(pos: [i32; 3], face: u8) -> Option<[i32; 3]> {
    let [x, y, z] = pos;
    Some(match face {
        0 => [x, y - 1, z],
        1 => [x, y + 1, z],
        2 => [x, y, z - 1],
        3 => [x, y, z + 1],
        4 => [x - 1, y, z],
        5 => [x + 1, y, z],
        _ => return None,
    })
}

/// Players may only touch blocks within this distance of their eyes.
const REACH: f32 = 12.0;

impl Server {
    fn in_reach(p: &Player, pos: [i32; 3]) -> bool {
        let d = |i: usize| p.pos[i] - (pos[i] as f32 + 0.5);
        d(0) * d(0) + d(1) * d(1) + d(2) * d(2) <= REACH * REACH
    }

    /// Tells one player what is really at `pos` (undoes a client prediction we rejected).
    fn resync(&mut self, sid: SessionId, pos: [i32; 3]) {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return };
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        if let Some(p) = self.players.get_mut(&sid) {
            p.out.push(protocol::update_block(pos, id));
        }
    }

    /// Changes a block and shows it to everyone in that world.
    fn change_block(&mut self, world: usize, pos: [i32; 3], id: i32) -> bool {
        if !self.worlds.worlds[world].set_block(pos, id, &self.gd) {
            return false;
        }
        let pk = protocol::update_block(pos, id);
        self.broadcast(&pk, |_, p| p.world == world);
        true
    }

    pub(crate) fn break_block(&mut self, sid: SessionId, pos: [i32; 3]) {
        let Some(p) = self.players.get(&sid) else { return };
        let (world, adventure) = (p.world, p.gamemode == 2 || p.gamemode == 6);
        if adventure || !Self::in_reach(p, pos) {
            self.resync(sid, pos);
            return;
        }
        let air = self.gd.air_id;
        if self.worlds.worlds[world].get_block(pos, &self.gd) == air {
            return;
        }
        if !self.change_block(world, pos, air) {
            self.resync(sid, pos);
        }
    }

    pub(crate) fn place_block(&mut self, sid: SessionId, clicked: [i32; 3], face: u8, block: i32) {
        let Some(p) = self.players.get(&sid) else { return };
        let world = p.world;
        let Some(target) = side(clicked, face) else { return };
        let adventure = p.gamemode == 2 || p.gamemode == 6;
        let free = self.worlds.worlds[world].get_block(target, &self.gd) == self.gd.air_id;
        // TODO: collision with entities, replaceable blocks (grass, water), block orientation
        if adventure || block == 0 || !free || !Self::in_reach(p, target) || !self.gd.state_nbt.contains_key(&block) {
            self.resync(sid, target);
            self.resync(sid, clicked);
            return;
        }
        if !self.change_block(world, target, block) {
            self.resync(sid, target);
        }
    }

    pub(crate) fn on_use_item(&mut self, sid: SessionId, u: UseItem) {
        match u.action {
            use_item_action::BREAK_BLOCK => self.break_block(sid, u.block_pos),
            use_item_action::CLICK_BLOCK => self.place_block(sid, u.block_pos, u.face, u.item.block_runtime_id),
            _ => {}
        }
    }

    pub(crate) fn on_block_action(&mut self, sid: SessionId, action: i32, pos: [i32; 3]) {
        match action {
            block_action::CREATIVE_PLAYER_DESTROY_BLOCK | block_action::PREDICT_DESTROY_BLOCK => {
                self.break_block(sid, pos)
            }
            _ => {}
        }
    }

    /// Writes changed chunks of every world to disk.
    pub(crate) fn save_worlds(&mut self) {
        for w in self.worlds.worlds.iter_mut() {
            match w.save(&self.gd) {
                Ok(0) => {}
                Ok(n) => {
                    if ceres_net::debug_enabled() {
                        println!("[World {}] saved {n} chunk(s)", w.folder);
                    }
                }
                Err(e) => eprintln!("[World {}] save failed: {e}", w.folder),
            }
        }
    }
}
