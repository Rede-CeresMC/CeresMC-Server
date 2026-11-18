//! Breaking, placing, orienting, interacting with blocks and editing signs.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::{ItemDesc, UseItem, block_action, level_event, use_item_action, window};

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
const EYE_HEIGHT: f32 = 1.62;

fn default_sign_nbt(pos: [i32; 3]) -> Tag {
    let text_side = || {
        Tag::Compound(vec![
            ("Text".into(), Tag::String(String::new())),
            ("SignTextColor".into(), Tag::Int(-16777216)),
            ("IgnoreLighting".into(), Tag::Byte(0)),
            ("PersistFormatting".into(), Tag::Byte(1)),
            ("HideGlowOutline".into(), Tag::Byte(0)),
            ("TextOwner".into(), Tag::String(String::new())),
        ])
    };
    Tag::Compound(vec![
        ("id".into(), Tag::String("Sign".into())),
        ("x".into(), Tag::Int(pos[0])),
        ("y".into(), Tag::Int(pos[1])),
        ("z".into(), Tag::Int(pos[2])),
        ("isMovable".into(), Tag::Byte(1)),
        ("FrontText".into(), text_side()),
        ("BackText".into(), text_side()),
        ("IsWaxed".into(), Tag::Byte(0)),
        ("LockedForEditingBy".into(), Tag::Long(-1)),
    ])
}

impl Server {
    fn in_reach(p: &Player, pos: [i32; 3]) -> bool {
        let d = |i: usize| p.pos[i] - (pos[i] as f32 + 0.5);
        d(0) * d(0) + d(1) * d(1) + d(2) * d(2) <= REACH * REACH
    }

    /// True if a full block at `pos` intersects any living non-spectator player in `world`.
    fn collides_with_player(&self, world: usize, pos: [i32; 3]) -> bool {
        let (bx0, bx1) = (pos[0] as f32, pos[0] as f32 + 1.0);
        let (by0, by1) = (pos[1] as f32, pos[1] as f32 + 1.0);
        let (bz0, bz1) = (pos[2] as f32, pos[2] as f32 + 1.0);
        for p in self.players.values() {
            if p.state != State::Spawned || p.world != world || p.dead || p.gamemode == 6 {
                continue;
            }
            let feet_y = p.pos[1] - EYE_HEIGHT;
            let (px0, px1) = (p.pos[0] - 0.29, p.pos[0] + 0.29);
            let (py0, py1) = (feet_y + 0.01, feet_y + p.actor_state.hitbox_height() - 0.01);
            let (pz0, pz1) = (p.pos[2] - 0.29, p.pos[2] + 0.29);
            if bx0 < px1 && bx1 > px0 && by0 < py1 && by1 > py0 && bz0 < pz1 && bz1 > pz0 {
                return true;
            }
        }
        false
    }

    /// Tells one player what is really at `pos` (undoes a client prediction we rejected).
    fn resync(&mut self, sid: SessionId, pos: [i32; 3]) {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return };
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        let be = self.worlds.worlds[world].get_block_entity(pos, &self.gd);
        if let Some(p) = self.players.get_mut(&sid) {
            p.out.push(protocol::update_block(pos, id));
            if let Some(tag) = be {
                p.out.push(protocol::block_actor_data(pos, &tag));
            }
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
        let (world, gm, dead) = (p.world, p.gamemode, p.dead);
        if dead || gm == 2 || gm == 6 || !Self::in_reach(p, pos) {
            self.resync(sid, pos);
            return;
        }
        let old = self.worlds.worlds[world].get_block(pos, &self.gd);
        let air = self.gd.air_id;
        if old == air {
            return;
        }
        // Bedrock is unbreakable in survival
        if gm == 0 && self.gd.block_name(old) == "minecraft:bedrock" {
            self.resync(sid, pos);
            return;
        }
        let contents = if containers::container_kind(self.gd.block_name(old)).is_some() {
            self.take_container_items(world, pos)
        } else {
            Vec::new()
        };
        if !self.change_block(world, pos, air) {
            self.resync(sid, pos);
            return;
        }
        self.swing_arm(sid);
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        // a broken container spills what it held, in every game mode (like vanilla)
        for item in contents {
            let rx = (rand::random::<f32>() - 0.5) * 0.2;
            let rz = (rand::random::<f32>() - 0.5) * 0.2;
            self.spawn_item(world, center, [rx, 0.2, rz], item, 10);
        }
        let destroy_fx = protocol::level_event(level_event::PARTICLE_DESTROY, center, old);
        self.broadcast(&destroy_fx, |o, q| o != sid && q.world == world);

        // Survival block drops
        if gm == 0 {
            if let Some(ce) = self.gd.block_to_item.get(&old).cloned() {
                let count = if self.gd.block_name(old).contains("_double_slab") { 2 } else { 1 };
                let drop = ItemDesc {
                    id: ce.id,
                    count,
                    meta: ce.meta,
                    stack_id: 0,
                    block_runtime_id: ce.block_runtime_id,
                    extra: ce.extra,
                };
                let rx = (rand::random::<f32>() - 0.5) * 0.15;
                let rz = (rand::random::<f32>() - 0.5) * 0.15;
                self.spawn_item(world, center, [rx, 0.15, rz], drop, 10);
            }
        }
    }

    pub(crate) fn place_block(&mut self, sid: SessionId, u: UseItem) {
        let clicked = u.block_pos;
        let face = u.face;
        let Some(p) = self.players.get(&sid) else { return };
        if p.dead {
            return;
        }
        let (world, gm, sneaking, pitch, yaw) = (p.world, p.gamemode, p.actor_state.sneaking, p.rot[0], p.rot[1]);
        if gm == 2 || gm == 6 || !Self::in_reach(p, clicked) {
            self.resync(sid, clicked);
            if let Some(t) = side(clicked, face) {
                self.resync(sid, t);
            }
            return;
        }

        // Sync hotbar selection if the client sent a valid hotbar slot
        if (0..9).contains(&u.hotbar_slot) {
            if let Some(p) = self.players.get_mut(&sid) {
                if p.inventory.held_slot != u.hotbar_slot as u8 {
                    p.inventory.held_slot = u.hotbar_slot as u8;
                    self.broadcast_equipment(sid);
                }
            }
        }

        let clicked_id = self.worlds.worlds[world].get_block(clicked, &self.gd);

        // 1. Containers (chest, barrel, shulker box) and interactive blocks (doors, trapdoors,
        //    fence gates, levers) when not sneaking
        if !sneaking && self.open_container(sid, clicked) {
            self.swing_arm(sid);
            if let Some(t) = side(clicked, face) {
                self.resync(sid, t);
            }
            return;
        }
        if !sneaking {
            if let Some(toggled) = self.gd.toggle_interactive(clicked_id) {
                self.change_block(world, clicked, toggled);
                let center = [clicked[0] as f32 + 0.5, clicked[1] as f32 + 0.5, clicked[2] as f32 + 0.5];
                let ev = if self.gd.block_name(clicked_id).ends_with("lever") {
                    level_event::SOUND_CLICK
                } else {
                    level_event::SOUND_DOOR
                };
                self.broadcast(&protocol::level_event(ev, center, 0), |_, q| q.world == world);
                self.swing_arm(sid);
                if let Some(t) = side(clicked, face) {
                    self.resync(sid, t);
                }
                return;
            }
        }

        // 2. Determine which block the player is placing
        let (base_block, survival_slot): (i32, Option<u8>) = if gm == 1 {
            let mut b = u.item.block_runtime_id;
            if b == 0 && u.item.id != 0 {
                if let Some(info) = self.gd.items_by_id.get(&u.item.id) {
                    b = info.block_runtime_id;
                    if b == 0 && info.name.ends_with("_sign") {
                        let standing = info.name.replace("_sign", "_standing_sign");
                        b = self.gd.default_block(&standing).unwrap_or(0);
                    }
                }
            }
            (b, None)
        } else {
            let p = self.players.get(&sid).unwrap();
            let slot = p.inventory.held_slot;
            let held = p.inventory.held_item();
            if held.is_empty() {
                if let Some(t) = side(clicked, face) {
                    self.resync(sid, t);
                }
                self.resync(sid, clicked);
                return;
            }
            let mut b = held.block_runtime_id;
            if b == 0 {
                if let Some(info) = self.gd.items_by_id.get(&held.id) {
                    b = info.block_runtime_id;
                    if b == 0 && info.name.ends_with("_sign") {
                        let standing = info.name.replace("_sign", "_standing_sign");
                        b = self.gd.default_block(&standing).unwrap_or(0);
                    }
                }
            }
            (b, Some(slot))
        };

        if base_block == 0 || !self.gd.state_nbt.contains_key(&base_block) {
            if let Some(t) = side(clicked, face) {
                self.resync(sid, t);
            }
            self.resync(sid, clicked);
            return;
        }

        // 3. Double slab combination on the clicked slab (inner face) or target slab
        let clicked_half = self
            .gd
            .block_states
            .get(&clicked_id)
            .and_then(|s| s.states.get("minecraft:vertical_half"))
            .and_then(|t| t.as_str());
        let inner_slab_click = matches!((clicked_half, face), (Some("bottom"), 1) | (Some("top"), 0));
        if inner_slab_click {
            if let Some(double_id) = self.gd.try_merge_double_slab(clicked_id, base_block) {
                if !self.collides_with_player(world, clicked) && self.change_block(world, clicked, double_id) {
                    self.finish_placement(sid, world, clicked, double_id, survival_slot);
                    return;
                }
            }
        }

        // 4. Replaceable clicked block vs adjacent target block
        let (target, place_face) = if self.gd.is_replaceable(clicked_id) {
            (clicked, 1)
        } else {
            let Some(adj) = side(clicked, face) else { return };
            (adj, face)
        };

        let Some(p) = self.players.get(&sid) else { return };
        if !Self::in_reach(p, target) {
            self.resync(sid, target);
            self.resync(sid, clicked);
            return;
        }

        let target_existing = self.worlds.worlds[world].get_block(target, &self.gd);
        let final_block = if let Some(double_id) = self.gd.try_merge_double_slab(target_existing, base_block) {
            double_id
        } else if self.gd.is_replaceable(target_existing) {
            self.gd.orient_placement(base_block, place_face, u.click_pos, yaw, pitch)
        } else {
            self.resync(sid, target);
            self.resync(sid, clicked);
            return;
        };

        if self.gd.is_solid(final_block) && self.collides_with_player(world, target) {
            self.resync(sid, target);
            self.resync(sid, clicked);
            return;
        }

        if !self.change_block(world, target, final_block) {
            self.resync(sid, target);
            return;
        }
        self.finish_placement(sid, world, target, final_block, survival_slot);
    }

    fn finish_placement(
        &mut self,
        sid: SessionId,
        world: usize,
        pos: [i32; 3],
        placed_id: i32,
        survival_slot: Option<u8>,
    ) {
        self.swing_arm(sid);
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        let snd = protocol::level_sound_event("place", center, placed_id);
        self.broadcast(&snd, |o, q| o != sid && q.world == world);

        if let Some(slot) = survival_slot {
            if let Some(p) = self.players.get_mut(&sid) {
                if let Some(rem) = p.inventory.consume_slot(slot, 1) {
                    p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &rem));
                }
            }
            self.broadcast_equipment(sid);
        }

        // If a sign was placed, create its BlockEntity NBT and open the sign editor on the placer!
        if self.gd.is_sign(placed_id) {
            let nbt = default_sign_nbt(pos);
            self.worlds.worlds[world].set_block_entity(pos, nbt.clone(), &self.gd);
            let be_pk = protocol::block_actor_data(pos, &nbt);
            self.broadcast(&be_pk, |_, q| q.world == world);
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(protocol::open_sign(pos, true));
                p.editing_sign = Some((pos, Instant::now()));
            }
        }
    }

    /// Client edited a sign (BlockActorDataPacket 0x38).
    pub(crate) fn on_block_actor_data(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let (pos, incoming) = protocol::read_block_actor_data(r)?;
        let Some(p) = self.players.get_mut(&sid) else { return Ok(()) };
        // only the sign whose editor we opened for this player, for a limited time
        let allowed = matches!(p.editing_sign, Some((spos, at)) if spos == pos && at.elapsed().as_secs() < 120);
        p.editing_sign = None;
        if !allowed || p.dead || p.gamemode == 2 || p.gamemode == 6 || !Self::in_reach(p, pos) {
            return Ok(());
        }
        let world = p.world;
        let bid = self.worlds.worlds[world].get_block(pos, &self.gd);
        if !self.gd.is_sign(bid) {
            return Ok(());
        }
        let mut sign =
            self.worlds.worlds[world].get_block_entity(pos, &self.gd).unwrap_or_else(|| default_sign_nbt(pos));
        if sign.get("IsWaxed").and_then(|t| t.as_i64()).unwrap_or(0) != 0 {
            return Ok(());
        }
        for side_key in ["FrontText", "BackText"] {
            if let Some(text) = incoming.get(side_key).and_then(|t| t.get("Text")).and_then(|t| t.as_str()) {
                let clean: String = text.chars().filter(|c| *c == '\n' || !c.is_control()).take(256).collect();
                let mut side_compound = sign.get(side_key).cloned().unwrap_or_else(Tag::empty_compound);
                side_compound.set("Text", Tag::String(clean));
                sign.set(side_key, side_compound);
            }
        }
        self.worlds.worlds[world].set_block_entity(pos, sign.clone(), &self.gd);
        let pk = protocol::block_actor_data(pos, &sign);
        self.broadcast(&pk, |_, q| q.world == world);
        Ok(())
    }

    pub(crate) fn on_use_item(&mut self, sid: SessionId, u: UseItem) {
        match u.action {
            use_item_action::BREAK_BLOCK => self.break_block(sid, u.block_pos),
            use_item_action::CLICK_BLOCK => self.place_block(sid, u),
            _ => {}
        }
    }

    pub(crate) fn on_block_action(&mut self, sid: SessionId, action: i32, pos: [i32; 3]) {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return };
        let fpos = [pos[0] as f32, pos[1] as f32, pos[2] as f32];
        match action {
            block_action::START_BREAK | block_action::CONTINUE_DESTROY_BLOCK => {
                self.swing_arm(sid);
                let pk = protocol::level_event(level_event::BLOCK_START_BREAK, fpos, 65535 / 20);
                self.broadcast(&pk, |o, q| o != sid && q.world == world);
            }
            block_action::ABORT_BREAK | block_action::STOP_BREAK => {
                let pk = protocol::level_event(level_event::BLOCK_STOP_BREAK, fpos, 0);
                self.broadcast(&pk, |o, q| o != sid && q.world == world);
            }
            block_action::CREATIVE_PLAYER_DESTROY_BLOCK | block_action::PREDICT_DESTROY_BLOCK => {
                let stop = protocol::level_event(level_event::BLOCK_STOP_BREAK, fpos, 0);
                self.broadcast(&stop, |o, q| o != sid && q.world == world);
                self.break_block(sid, pos);
            }
            _ => {}
        }
    }

    /// Writes changed chunks (and level.dat) of every world to disk, plus player states.
    pub(crate) fn save_worlds(&mut self) {
        self.save_all_players();
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
