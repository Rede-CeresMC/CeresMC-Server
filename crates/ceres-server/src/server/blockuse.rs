//! Using items on decorative blocks:
//! - candles and sea pickles stack up to 4 in one block (the same item used on it);
//! - flint and steel or a fire charge lights candles, candle cakes and campfires; a shovel
//!   puts a campfire out; an empty hand puts lit candles out;
//! - a candle on an uneaten cake makes a candle cake of its colour;
//! - a cake is eaten a slice at a time (2 hunger, 0.4 saturation, 7 slices).

use super::*;
use ceres_codec::nbt::Tag;

fn state(gd: &GameData, id: i32, key: &str) -> Option<i64> {
    gd.block_states.get(&id)?.states.get(key)?.as_i64()
}

impl Server {
    fn set_block_state(&mut self, world: usize, pos: [i32; 3], block: i32, key: &str, val: i64) -> bool {
        let tag = match self.gd.block_states.get(&block).and_then(|s| s.states.get(key)) {
            Some(Tag::Byte(_)) => Tag::Byte(val as i8),
            _ => Tag::Int(val as i32),
        };
        match self.gd.with_states(block, &[(key, tag)]) {
            Some(id) => self.change_block(world, pos, id),
            None => false,
        }
    }

    fn block_sound(&mut self, world: usize, pos: [i32; 3], sound: &str, data: i32) {
        let at = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        self.broadcast(&protocol::level_sound_event(sound, at, data), |_, q| q.world == world);
    }

    /// True when the click was handled here.
    pub(crate) fn use_on_decoration(&mut self, sid: SessionId, clicked: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm, hunger) = (p.world, p.gamemode, p.hunger);
        let held = p.inventory.held_item().clone();
        let item = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()).unwrap_or_default();
        let block = self.worlds.worlds[world].get_block(clicked, &self.gd);
        let name = self.gd.block_name(block).to_string();
        let lighter = item == "minecraft:flint_and_steel" || item == "minecraft:fire_charge";
        let candle = name == "minecraft:candle" || name.ends_with("_candle");
        if self.pick_glow_berries(sid, clicked) {
            return true;
        }
        // stacking
        if (candle || name == "minecraft:sea_pickle") && item == name {
            let key = if candle { "candles" } else { "cluster_count" };
            let n = state(&self.gd, block, key).unwrap_or(0);
            if n < 3 && self.set_block_state(world, clicked, block, key, n + 1) {
                self.block_sound(world, clicked, "place", block);
                self.swing_arm(sid);
                if gm != 1 {
                    self.consume_held(sid, gm);
                }
            }
            return true;
        }
        // lighting
        if lighter && (candle || name.ends_with("candle_cake")) && state(&self.gd, block, "lit") == Some(0) {
            self.set_block_state(world, clicked, block, "lit", 1);
            self.block_sound(world, clicked, "ignite", -1);
            self.swing_arm(sid);
            if gm != 1 {
                if item == "minecraft:fire_charge" { self.consume_held(sid, gm) } else { self.wear_held(sid, 1) }
            }
            return true;
        }
        if name == "minecraft:campfire" || name == "minecraft:soul_campfire" {
            let out = state(&self.gd, block, "extinguished").unwrap_or(0) != 0;
            if lighter && out {
                self.set_block_state(world, clicked, block, "extinguished", 0);
                self.block_sound(world, clicked, "ignite", -1);
                self.swing_arm(sid);
                if gm != 1 {
                    if item == "minecraft:fire_charge" { self.consume_held(sid, gm) } else { self.wear_held(sid, 1) }
                }
                return true;
            }
            let shovel = self.gd.tool_info(held.id).is_some_and(|t| t.tool == ceres_world::Tool::Shovel);
            if shovel && !out {
                self.set_block_state(world, clicked, block, "extinguished", 1);
                self.block_sound(world, clicked, "extinguish.fire", -1);
                self.swing_arm(sid);
                if gm != 1 {
                    self.wear_held(sid, 1);
                }
                return true;
            }
        }
        // an empty hand puts lit candles out
        if held.is_empty() && (candle || name.ends_with("candle_cake")) && state(&self.gd, block, "lit") == Some(1) {
            self.set_block_state(world, clicked, block, "lit", 0);
            self.block_sound(world, clicked, "extinguish.candle", -1);
            return true;
        }
        if name == "minecraft:cake" {
            let bites = state(&self.gd, block, "bite_counter").unwrap_or(0);
            // a candle on a whole cake
            if (item == "minecraft:candle" || item.ends_with("_candle")) && bites == 0 {
                let cake = format!("{item}_cake");
                if let Some(id) = self.gd.default_block(&cake) {
                    self.change_block(world, clicked, id);
                    self.block_sound(world, clicked, "cake.add_candle", -1);
                    self.swing_arm(sid);
                    if gm != 1 {
                        self.consume_held(sid, gm);
                    }
                }
                return true;
            }
            if hunger >= 20.0 && gm != 1 {
                return true;
            }
            if let Some(p) = self.players.get_mut(&sid) {
                p.hunger = (p.hunger + 2.0).min(20.0);
                p.saturation = (p.saturation + 0.4).min(p.hunger);
                let pk = p.vitals_packet();
                p.out.push(pk);
            }
            if bites >= 6 {
                let air = self.gd.air_id;
                self.change_block(world, clicked, air);
            } else {
                self.set_block_state(world, clicked, block, "bite_counter", bites + 1);
            }
            self.block_sound(world, clicked, "burp", -1);
            return true;
        }
        false
    }
}
