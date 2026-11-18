//! Beds: clicking one sets the respawn point; at night (or in a thunderstorm) the player lies down,
//! and when everyone in the world has slept for 5 seconds the night is skipped.

use super::*;
use ceres_codec::nbt::Tag;

const NIGHT_START: i64 = 12542;
const NIGHT_END: i64 = 23459;
const DAY: i64 = 24000;
/// ticks everyone must sleep before the night is skipped
const SLEEP_TICKS: u64 = 100;
const ANIMATE_STOP_SLEEP: u8 = 3;
pub(crate) const PLAYER_ACTION: u32 = 0x24;
const ACTION_STOP_SLEEPING: i32 = 6;

impl Server {
    fn set_bed_occupied(&mut self, world: usize, pos: [i32; 3], occupied: bool) {
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        if self.gd.block_name(id) != "minecraft:bed" {
            return;
        }
        for p in std::iter::once(pos).chain(twopart::partner(&self.gd, pos, id)) {
            let b = self.worlds.worlds[world].get_block(p, &self.gd);
            if let Some(new) = self.gd.with_states(b, &[("occupied_bit", Tag::Byte(occupied as i8))]) {
                // keep the colour block entity
                let tile = self.worlds.worlds[world].get_block_entity(p, &self.gd);
                self.change_block(world, p, new);
                if let Some(t) = tile {
                    self.worlds.worlds[world].set_block_entity(p, t, &self.gd);
                }
            }
        }
    }

    /// Right click on a bed. Returns true when the block is a bed.
    pub(crate) fn use_bed(&mut self, sid: SessionId, pos: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let world = p.world;
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        if self.gd.block_name(id) != "minecraft:bed" {
            return false;
        }
        if p.actor_state.sleeping.is_some() || p.dead {
            return true;
        }
        let occupied = self.gd.block_states.get(&id).and_then(|s| s.states.get("occupied_bit")).and_then(|t| t.as_i64())
            == Some(1);
        // the respawn point moves to the bed even during the day (Bedrock)
        if let Some(p) = self.players.get_mut(&sid) {
            p.spawn_point = Some((world, [pos[0], pos[1] + 1, pos[2]]));
            p.out.push(protocol::system_message("§7Ponto de renascimento definido."));
        }
        let tod = self.worlds.worlds[world].time.rem_euclid(DAY);
        let thunder = self.worlds.worlds[world].weather == ceres_world::Weather::Thunder;
        if !(NIGHT_START..NIGHT_END).contains(&tod) && !thunder {
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(protocol::system_message("§cVocê só pode dormir à noite ou durante tempestades."));
            }
            return true;
        }
        // a bed can stay marked occupied if its sleeper vanished; only a real sleeper blocks it
        let really_occupied = occupied
            && self.players.values().any(|q| q.sleep_world == world && q.actor_state.sleeping.is_some_and(|b| {
                b == pos || twopart::partner(&self.gd, pos, id) == Some(b)
            }));
        if really_occupied {
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(protocol::system_message("§cEsta cama está ocupada."));
            }
            return true;
        }
        let ticks = self.ticks;
        if let Some(p) = self.players.get_mut(&sid) {
            p.actor_state.sleeping = Some(pos);
            p.sleep_since = ticks;
            p.sleep_world = world;
        }
        self.set_bed_occupied(world, pos, true);
        self.sync_actor_state(sid);
        true
    }

    pub(crate) fn stop_sleeping(&mut self, sid: SessionId) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(bed) = p.actor_state.sleeping.take() else { return };
        let (world, eid) = (p.sleep_world, p.entity_id);
        p.out.push(protocol::animate(eid, ANIMATE_STOP_SLEEP));
        self.set_bed_occupied(world, bed, false);
        self.sync_actor_state(sid);
    }

    /// PlayerAction: the client only tells us when it leaves the bed.
    pub(crate) fn on_player_action(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        r.uvarlong()?; // runtime id
        if r.varint()? == ACTION_STOP_SLEEPING {
            self.stop_sleeping(sid);
        }
        Ok(())
    }

    /// Once a second: skip the night when everybody in a world has been sleeping long enough.
    pub(crate) fn check_sleep(&mut self) {
        if self.ticks % 20 != 0 {
            return;
        }
        for world in 0..self.worlds.worlds.len() {
            let in_world: Vec<(SessionId, bool)> = self
                .players
                .iter()
                .filter(|(_, p)| p.world == world && p.state == State::Spawned && p.gamemode != 6)
                .map(|(s, p)| (*s, p.actor_state.sleeping.is_some() && self.ticks - p.sleep_since >= SLEEP_TICKS))
                .collect();
            if in_world.is_empty() || !in_world.iter().all(|(_, asleep)| *asleep) {
                continue;
            }
            let w = &mut self.worlds.worlds[world];
            let tod = w.time.rem_euclid(DAY);
            w.time += DAY - tod;
            let was_raining = w.weather != ceres_world::Weather::Clear;
            w.weather = ceres_world::Weather::Clear;
            let time = w.time;
            self.broadcast(&protocol::set_time(time as i32), |_, q| q.world == world);
            if was_raining {
                use ceres_protocol::level_event;
                self.broadcast(&protocol::level_event(level_event::STOP_RAIN, [0.0; 3], 0), |_, q| q.world == world);
                self.broadcast(&protocol::level_event(level_event::STOP_THUNDER, [0.0; 3], 0), |_, q| q.world == world);
            }
            for (sid, _) in in_world {
                self.stop_sleeping(sid);
            }
        }
    }
}
