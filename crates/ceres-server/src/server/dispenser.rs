//! Dispensers and droppers (as PowerNukkitX's BlockDispenser/BlockDropper and its dispense
//! behaviours): 9-slot containers that act once each time power reaches them (4 ticks later),
//! with an item from a random filled slot, out of the face they look at:
//! - a dropper puts it into the container in front, or throws it out;
//! - a dispenser shoots arrows and throwables, places and picks up water and lava, lights TNT
//!   in front of it, hatches spawn eggs, uses bone meal and flint and steel; anything else it
//!   throws out like a dropper.
//!
//! Click 1000 and smoke 2000 when it works, click 1001 when it is empty.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::ItemDesc;

pub(crate) const SLOTS: usize = 9;
const SOUND_DISPENSE: i32 = 1000;
const SOUND_FAIL: i32 = 1001;
const PARTICLE_SMOKE: i32 = 2000;

/// down, up, north, south, west, east (the `facing_direction` values)
const DIRS: [[i32; 3]; 6] = [[0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]];

impl Server {
    pub(crate) fn dispense(&mut self, world: usize, pos: [i32; 3]) {
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        let name = self.gd.block_name(block).to_string();
        let dropper = name == "minecraft:dropper";
        let f = self.gd.block_states.get(&block).and_then(|s| s.states.get("facing_direction")).and_then(|t| t.as_i64()).unwrap_or(0).clamp(0, 5) as usize;
        let d = DIRS[f];
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        let Some((ui, tile, mut items)) = self.stored_items(world, pos) else { return };
        let filled: Vec<usize> = (0..items.len()).filter(|i| !items[*i].is_empty()).collect();
        if filled.is_empty() {
            self.broadcast(&protocol::level_event(SOUND_FAIL, center, 0), |_, q| q.world == world);
            return;
        }
        let slot = filled[rand::random::<u32>() as usize % filled.len()];
        let item = items[slot].clone();
        let front = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
        let out = [center[0] + d[0] as f32 * 0.7, center[1] + d[1] as f32 * 0.7 - if f > 1 { 0.15 } else { 0.0 }, center[2] + d[2] as f32 * 0.7];
        let dir = [d[0] as f32, d[1] as f32 + if f > 1 { 0.1 } else { 0.0 }, d[2] as f32];
        let iname = self.gd.items_by_id.get(&item.id).map(|i| i.name.clone()).unwrap_or_default();
        // what the slot holds afterwards: None = one used up
        let mut replaced: Option<ItemDesc> = None;
        let mut done = true;
        // what it does to the block in front goes through the plugins like a player's edit
        // (protected areas): a dropper filling a container, a dispenser placing liquid, fire,
        // TNT or a mob, taking liquid, using bone meal
        let event = if dropper {
            self.stored_items(world, front).is_some().then_some("block_interact")
        } else {
            match iname.as_str() {
                "minecraft:water_bucket" | "minecraft:lava_bucket" | "minecraft:tnt" | "minecraft:flint_and_steel" => Some("block_place"),
                n if n.ends_with("_spawn_egg") => Some("block_place"),
                "minecraft:bucket" => Some("block_break"),
                "minecraft:bone_meal" => Some("block_interact"),
                _ => None,
            }
        };
        if let Some(ev) = event {
            let here = self.worlds.worlds[world].get_block(front, &self.gd);
            let mut json = self.block_event_json(ev, super::projectiles::NO_OWNER, world, front, here);
            json["source"] = serde_json::Value::String("dispenser".into());
            if !self.plugins_allow(json) {
                self.broadcast(&protocol::level_event(SOUND_FAIL, center, 0), |_, q| q.world == world);
                return;
            }
        }
        if dropper {
            if !self.drop_into_container(world, front, &item) {
                self.throw_out(world, out, dir, &item);
            }
        } else {
            match iname.as_str() {
                "minecraft:water_bucket" | "minecraft:lava_bucket" => {
                    let liquid = if iname.starts_with("minecraft:water") { "minecraft:water" } else { "minecraft:lava" };
                    let here = self.worlds.worlds[world].get_block(front, &self.gd);
                    match self.gd.default_block(liquid).filter(|_| self.gd.is_replaceable(here)) {
                        Some(id) => {
                            self.change_block(world, front, id);
                            replaced = self.simple_item("minecraft:bucket", 1);
                        }
                        None => done = false,
                    }
                }
                "minecraft:bucket" => {
                    let here = self.worlds.worlds[world].get_block(front, &self.gd);
                    let hname = self.gd.block_name(here).to_string();
                    let depth = self.gd.block_states.get(&here).and_then(|s| s.states.get("liquid_depth")).and_then(|t| t.as_i64()).unwrap_or(1);
                    let full = match hname.as_str() {
                        "minecraft:water" | "minecraft:flowing_water" if depth == 0 => Some("minecraft:water_bucket"),
                        "minecraft:lava" | "minecraft:flowing_lava" if depth == 0 => Some("minecraft:lava_bucket"),
                        _ => None,
                    };
                    match full {
                        Some(b) => {
                            let air = self.gd.air_id;
                            self.change_block(world, front, air);
                            // the bucket is filled; with more than one, the full one goes in another slot or out
                            if item.count == 1 {
                                replaced = self.simple_item(b, 1);
                            } else if let Some(fb) = self.simple_item(b, 1) {
                                let mut rest = items.clone();
                                if !self.insert_into(&mut rest, &fb) {
                                    self.throw_out(world, out, dir, &fb);
                                } else {
                                    items = rest;
                                }
                            }
                        }
                        None => done = false,
                    }
                }
                "minecraft:tnt" => {
                    let at = [front[0] as f32 + 0.5, front[1] as f32, front[2] as f32 + 0.5];
                    self.spawn_entity(world, "minecraft:tnt", at, None);
                    self.broadcast(&protocol::level_sound_event("fuse", at, -1), |_, q| q.world == world);
                }
                "minecraft:bone_meal" => done = self.bone_meal_plant(world, front),
                "minecraft:flint_and_steel" => {
                    let here = self.worlds.worlds[world].get_block(front, &self.gd);
                    if self.gd.block_name(here) == "minecraft:tnt" {
                        self.prime_tnt(world, front);
                    } else if here == self.gd.air_id {
                        match self.gd.default_block("minecraft:fire") {
                            Some(fire) => {
                                self.change_block(world, front, fire);
                            }
                            None => done = false,
                        }
                    } else {
                        done = false;
                    }
                    // the lighter wears instead of being used up
                    if done {
                        let mut worn = item.clone();
                        if durability::wear_item(&mut worn, 1, &self.gd) {
                            replaced = Some(ItemDesc::default());
                        } else {
                            replaced = Some(worn);
                        }
                    }
                }
                n if n.ends_with("_spawn_egg") => {
                    let mob = n.trim_end_matches("_spawn_egg");
                    let at = [front[0] as f32 + 0.5, front[1] as f32, front[2] as f32 + 0.5];
                    done = self.spawn_entity(world, mob, at, None).is_some();
                }
                n => {
                    if !self.dispense_projectile(world, out, dir, n, item.meta) {
                        self.throw_out(world, out, dir, &item);
                    }
                }
            }
        }
        if !done {
            self.broadcast(&protocol::level_event(SOUND_FAIL, center, 0), |_, q| q.world == world);
            return;
        }
        match replaced {
            Some(r) => items[slot] = r,
            None => {
                items[slot].count -= 1;
                if items[slot].count == 0 {
                    items[slot] = ItemDesc::default();
                }
            }
        }
        self.store_items(world, pos, ui, tile, items);
        self.broadcast(&protocol::level_event(SOUND_DISPENSE, center, 0), |_, q| q.world == world);
        self.broadcast(&protocol::level_event(PARTICLE_SMOKE, out, f as i32), |_, q| q.world == world);
    }

    /// One of `item` into the container at `pos`; false when there is none or it is full.
    fn drop_into_container(&mut self, world: usize, pos: [i32; 3], item: &ItemDesc) -> bool {
        if !self.worlds.worlds[world].is_loaded(pos) {
            return false;
        }
        let Some((ui, tile, mut items)) = self.stored_items(world, pos) else { return false };
        if !self.insert_into(&mut items, item) {
            return false;
        }
        self.store_items(world, pos, ui, tile, items);
        true
    }

    fn insert_into(&self, items: &mut [ItemDesc], item: &ItemDesc) -> bool {
        let n = items.len();
        self.insert_one(items, 0..n, item)
    }

    /// One of `item` flies out of the face.
    fn throw_out(&mut self, world: usize, at: [f32; 3], dir: [f32; 3], item: &ItemDesc) {
        let mut one = item.clone();
        one.count = 1;
        one.stack_id = 0;
        let jitter = || (rand::random::<f32>() - 0.5) * 0.04;
        let speed = 0.25 + rand::random::<f32>() * 0.05;
        let motion = [dir[0] * speed + jitter(), dir[1] * speed + 0.1, dir[2] * speed + jitter()];
        self.spawn_item(world, at, motion, one, 10);
    }
}

/// Containers the redstone code wakes: a dispenser or dropper gets power.
pub(crate) fn is_dispenser(name: &str) -> bool {
    matches!(name, "minecraft:dispenser" | "minecraft:dropper")
}

/// The `triggered_bit` set or cleared.
pub(crate) fn with_triggered(gd: &GameData, id: i32, on: bool) -> Option<i32> {
    gd.with_states(id, &[("triggered_bit", Tag::Byte(on as i8))])
}
