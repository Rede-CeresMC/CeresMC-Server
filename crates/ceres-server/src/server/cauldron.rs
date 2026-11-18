//! Cauldrons: buckets fill and empty them (water or lava), glass bottles take water two levels
//! at a time, water bottles put it back; standing in a water cauldron puts out fire.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::ItemDesc;

const FULL: i64 = 6;
const FILL_WATER: i32 = 3506;
const TAKE_WATER: i32 = 3507;
const FILL_LAVA: i32 = 3512;
const TAKE_LAVA: i32 = 3513;

impl Server {
    fn cauldron_state(&self, id: i32) -> Option<(String, i64)> {
        let st = &self.gd.block_states.get(&id)?.states;
        Some((st.get("cauldron_liquid")?.as_str()?.to_string(), st.get("fill_level")?.as_i64()?))
    }

    fn set_cauldron(&mut self, world: usize, pos: [i32; 3], liquid: &str, level: i64) {
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        let updates = [("cauldron_liquid", Tag::String(liquid.into())), ("fill_level", Tag::Int(level as i32))];
        if let Some(id) = self.gd.with_states(block, &updates) {
            self.change_block(world, pos, id);
        }
    }

    fn give_back(&mut self, sid: SessionId, name: &str, meta: u32) {
        let Some(p) = self.players.get(&sid) else { return };
        if p.gamemode == 1 {
            return; // creative keeps its item
        }
        let Some(info) = self.gd.items_by_name.get(name) else { return };
        let item = ItemDesc {
            id: info.id,
            count: 1,
            meta,
            stack_id: 0,
            block_runtime_id: 0,
            extra: ceres_world::extra_data(None),
        };
        let p = self.players.get_mut(&sid).unwrap();
        let slot = (p.inventory.held_slot as usize).min(8);
        if p.inventory.main[slot].count <= 1 {
            p.inventory.set_main(slot, item);
        } else {
            let mut rest = p.inventory.main[slot].clone();
            rest.count -= 1;
            p.inventory.set_main(slot, rest);
            let (_, left) = p.inventory.add_item(item.clone(), &self.gd);
            if left > 0 {
                self.drop_from_player(sid, item);
            }
        }
        self.sync_full_inventory(sid);
        self.broadcast_equipment(sid);
    }

    /// Using an item on a cauldron. Returns true when the cauldron used it.
    pub(crate) fn use_cauldron(&mut self, sid: SessionId, pos: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let world = p.world;
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        if self.gd.block_name(block) != "minecraft:cauldron" {
            return false;
        }
        let Some((liquid, level)) = self.cauldron_state(block) else { return false };
        let held = p.inventory.held_item().clone();
        let Some(item) = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()) else { return false };
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        let event = |ev: i32| protocol::level_event(ev, center, 0);
        let empty = level == 0;
        match item.as_str() {
            "minecraft:water_bucket" if empty || liquid == "water" => {
                self.set_cauldron(world, pos, "water", FULL);
                self.broadcast(&event(FILL_WATER), |_, q| q.world == world);
                self.give_back(sid, "minecraft:bucket", 0);
            }
            "minecraft:lava_bucket" if empty => {
                self.set_cauldron(world, pos, "lava", FULL);
                self.broadcast(&event(FILL_LAVA), |_, q| q.world == world);
                self.give_back(sid, "minecraft:bucket", 0);
            }
            "minecraft:bucket" if level == FULL => {
                let (back, ev) = if liquid == "lava" { ("minecraft:lava_bucket", TAKE_LAVA) } else { ("minecraft:water_bucket", TAKE_WATER) };
                self.set_cauldron(world, pos, "water", 0);
                self.broadcast(&event(ev), |_, q| q.world == world);
                self.give_back(sid, back, 0);
            }
            "minecraft:glass_bottle" if liquid == "water" && level >= 2 => {
                self.set_cauldron(world, pos, "water", level - 2);
                self.broadcast(&event(TAKE_WATER), |_, q| q.world == world);
                self.give_back(sid, "minecraft:potion", 0); // water bottle
            }
            "minecraft:potion" if held.meta == 0 && (empty || liquid == "water") && level < FULL => {
                self.set_cauldron(world, pos, "water", (level + 2).min(FULL));
                self.broadcast(&event(FILL_WATER), |_, q| q.world == world);
                self.give_back(sid, "minecraft:glass_bottle", 0);
            }
            _ => return false,
        }
        self.swing_arm(sid);
        true
    }

    /// True if `block` is a cauldron holding water (puts out burning players).
    pub(crate) fn is_water_cauldron(&self, block: i32) -> bool {
        self.gd.block_name(block) == "minecraft:cauldron"
            && self.cauldron_state(block).is_some_and(|(l, level)| l == "water" && level > 0)
    }
}
