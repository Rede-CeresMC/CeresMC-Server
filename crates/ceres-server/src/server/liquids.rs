//! Water and lava: buckets, flowing and drying up, and the blocks made where they meet.
//! Vanilla rules (simplified): liquid falls first, then spreads sideways one level per block
//! (water 7 blocks every 5 ticks, lava 3 blocks every 30 ticks in the overworld); flowing liquid
//! with no source dries up; two water sources make a third. `liquid_depth`: 0 source, 1-7 level,
//! 8+ falling.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::{ItemDesc, window};

const WATER_DELAY: u64 = 5;
const LAVA_DELAY: u64 = 30;
/// liquid updates handled per tick, so a big flood cannot stall the server
const MAX_UPDATES_PER_TICK: usize = 512;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Liquid {
    Water,
    Lava,
}

impl Liquid {
    fn names(self) -> (&'static str, &'static str) {
        match self {
            Liquid::Water => ("minecraft:water", "minecraft:flowing_water"),
            Liquid::Lava => ("minecraft:lava", "minecraft:flowing_lava"),
        }
    }
    fn delay(self) -> u64 {
        match self {
            Liquid::Water => WATER_DELAY,
            Liquid::Lava => LAVA_DELAY,
        }
    }
    /// levels lost per block sideways
    fn step(self) -> i64 {
        match self {
            Liquid::Water => 1,
            Liquid::Lava => 2,
        }
    }
}

const SIDES: [[i32; 3]; 4] = [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]];

fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

impl Server {
    /// (liquid, depth) of a block, if it is water or lava.
    pub(crate) fn liquid_at(&mut self, world: usize, pos: [i32; 3]) -> Option<(Liquid, i64)> {
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        if let Some(l) = self.liquid_of(id) {
            return Some(l);
        }
        // a waterlogged block is a water source that never gets replaced (it is not replaceable)
        let logged = self.worlds.worlds[world].get_block_layer(pos, 1, &self.gd).is_some_and(|w| self.gd.block_name(w) == "minecraft:water");
        logged.then_some((Liquid::Water, 0))
    }

    fn liquid_of(&self, id: i32) -> Option<(Liquid, i64)> {
        let kind = match self.gd.block_name(id) {
            "minecraft:water" | "minecraft:flowing_water" => Liquid::Water,
            "minecraft:lava" | "minecraft:flowing_lava" => Liquid::Lava,
            _ => return None,
        };
        let depth = self.gd.block_states.get(&id)?.states.get("liquid_depth")?.as_i64()?;
        Some((kind, depth))
    }

    fn liquid_block(&self, kind: Liquid, depth: i64) -> Option<i32> {
        let (still, flowing) = kind.names();
        let name = if depth == 0 { still } else { flowing };
        let base = self.gd.default_block(name)?;
        self.gd.with_states(base, &[("liquid_depth", Tag::Int(depth as i32))])
    }

    /// A block changed: it and its neighbours may need to flow again.
    pub(crate) fn queue_liquid(&mut self, world: usize, pos: [i32; 3]) {
        for p in std::iter::once(pos).chain(SIDES.iter().map(|s| add(pos, *s))).chain([[pos[0], pos[1] + 1, pos[2]], [pos[0], pos[1] - 1, pos[2]]]) {
            if let Some((kind, _)) = self.liquid_at(world, p) {
                let due = self.ticks + kind.delay();
                self.liquid_queue.entry((world, p)).or_insert(due);
            }
        }
    }

    pub(crate) fn set_liquid(&mut self, world: usize, pos: [i32; 3], kind: Liquid, depth: i64) {
        if let Some(id) = self.liquid_block(kind, depth) {
            self.change_block(world, pos, id);
        }
    }

    /// Can liquid flow into this block (air, plants, other flowing liquid)?
    fn can_flow_into(&mut self, world: usize, pos: [i32; 3]) -> bool {
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        self.gd.is_replaceable(id) && self.liquid_of(id).is_none_or(|(_, d)| d != 0)
    }

    /// Lava meeting water hardens; returns true when the liquid at `pos` turned into a block.
    fn harden(&mut self, world: usize, pos: [i32; 3], kind: Liquid, depth: i64) -> bool {
        if kind != Liquid::Lava {
            return false;
        }
        let touches_water = SIDES
            .iter()
            .map(|s| add(pos, *s))
            .chain([[pos[0], pos[1] + 1, pos[2]]])
            .any(|p| self.liquid_at(world, p).is_some_and(|(k, _)| k == Liquid::Water));
        if !touches_water {
            return false;
        }
        let name = if depth == 0 { "minecraft:obsidian" } else { "minecraft:cobblestone" };
        if let Some(id) = self.gd.default_block(name) {
            self.change_block(world, pos, id);
            let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
            self.broadcast(&protocol::level_sound_event("fizz", center, -1), |_, q| q.world == world);
        }
        true
    }

    fn flow(&mut self, world: usize, pos: [i32; 3]) {
        let Some((kind, depth)) = self.liquid_at(world, pos) else { return };
        if self.harden(world, pos, kind, depth) {
            return;
        }
        let above = [pos[0], pos[1] + 1, pos[2]];
        let below = [pos[0], pos[1] - 1, pos[2]];

        // 1. flowing liquid needs something feeding it, or it dries up
        if depth != 0 {
            let fed_from_above = self.liquid_at(world, above).is_some_and(|(k, _)| k == kind);
            let level = depth & 7;
            let mut best: Option<i64> = None; // lowest level among sideways neighbours
            let mut sources = 0;
            for s in SIDES {
                if let Some((k, d)) = self.liquid_at(world, add(pos, s)) {
                    if k == kind {
                        if d == 0 {
                            sources += 1;
                        }
                        let l = if d >= 8 { 0 } else { d };
                        best = Some(best.map_or(l, |b| b.min(l)));
                    }
                }
            }
            // two water sources beside a block on solid ground (or a source) make a new source
            let below_id = self.worlds.worlds[world].get_block(below, &self.gd);
            let held_up = self.gd.is_solid(below_id) || self.liquid_of(below_id).is_some_and(|(k, d)| k == kind && d == 0);
            if kind == Liquid::Water && sources >= 2 && held_up {
                self.set_liquid(world, pos, kind, 0);
                return;
            }
            let wanted = if fed_from_above {
                8
            } else {
                match best {
                    Some(b) if b + kind.step() <= 7 => b + kind.step(),
                    _ => -1,
                }
            };
            if wanted < 0 {
                let air = self.gd.air_id;
                self.change_block(world, pos, air);
                return;
            }
            let current = if depth >= 8 { 8 } else { level };
            if wanted != current {
                self.set_liquid(world, pos, kind, wanted);
                return;
            }
        }

        // 2. fall down first
        if below[1] >= ceres_world::MIN_Y {
            if let Some((k, _)) = self.liquid_at(world, below) {
                if k != kind && kind == Liquid::Water {
                    // water falling onto lava turns it to stone
                    if let Some(id) = self.gd.default_block("minecraft:stone") {
                        self.change_block(world, below, id);
                    }
                    return;
                }
            }
            if self.can_flow_into(world, below) {
                if self.liquid_at(world, below).is_none_or(|(_, d)| d < 8) {
                    self.set_liquid(world, below, kind, 8);
                }
                // a falling column does not spread sideways, a source still does
                if depth != 0 {
                    return;
                }
            }
        }

        // 3. spread sideways
        let level = if depth >= 8 { 0 } else { depth };
        let next = level + kind.step();
        if next > 7 {
            return;
        }
        for s in SIDES {
            let side = add(pos, s);
            if !self.can_flow_into(world, side) {
                continue;
            }
            match self.liquid_at(world, side) {
                Some((k, d)) if k == kind && (d >= 8 || d <= next) => continue,
                Some((k, _)) if k != kind => {
                    // flowing water reaching lava (or the other way round)
                    let name = if kind == Liquid::Lava { "minecraft:cobblestone" } else { "minecraft:stone" };
                    if let Some(id) = self.gd.default_block(name) {
                        self.change_block(world, side, id);
                    }
                }
                _ => self.set_liquid(world, side, kind, next),
            }
        }
    }

    /// Runs the liquid updates that are due.
    pub(crate) fn tick_liquids(&mut self) {
        if self.liquid_queue.is_empty() {
            return;
        }
        let now = self.ticks;
        let mut due: Vec<(usize, [i32; 3])> =
            self.liquid_queue.iter().filter(|(_, t)| **t <= now).map(|(k, _)| *k).take(MAX_UPDATES_PER_TICK).collect();
        due.sort_by_key(|(_, p)| -p[1]); // top to bottom
        for key in &due {
            self.liquid_queue.remove(key);
        }
        for (world, pos) in due {
            self.flow(world, pos);
        }
    }

    /// Bucket use on a block: empty a water/lava bucket, or fill an empty one.
    /// Returns false when the held item is not a bucket.
    pub(crate) fn use_bucket(&mut self, sid: SessionId, clicked: [i32; 3], target: Option<[i32; 3]>) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm) = (p.world, p.gamemode);
        let slot = (p.inventory.held_slot as usize).min(8);
        let held = p.inventory.main[slot].clone();
        let name = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()).unwrap_or_default();
        let (fill, empty_into) = match name.as_str() {
            "minecraft:bucket" => (true, None),
            "minecraft:water_bucket" => (false, Some(Liquid::Water)),
            "minecraft:lava_bucket" => (false, Some(Liquid::Lava)),
            _ => return false,
        };
        // a block that holds water: the bucket puts water in it or takes it out
        let clicked_id = self.worlds.worlds[world].get_block(clicked, &self.gd);
        let holds = blocks::waterloggable(self.gd.block_name(clicked_id)) && self.players.get(&sid).is_some_and(|p| Self::in_reach(p, clicked));
        let logged = holds && self.water_layer(world, clicked);
        let new_item_name = if holds && fill && logged {
            self.set_water_layer(world, clicked, false);
            self.liquid_sound(world, clicked, "bucket.fill.water");
            "minecraft:water_bucket"
        } else if holds && !fill && empty_into == Some(Liquid::Water) && !logged {
            self.set_water_layer(world, clicked, true);
            self.liquid_sound(world, clicked, "bucket.empty.water");
            "minecraft:bucket"
        } else if fill {
            // pick up a source: the clicked block itself or the one in front of the clicked face
            let reachable: Vec<[i32; 3]> = [Some(clicked), target]
                .into_iter()
                .flatten()
                .filter(|pos| self.players.get(&sid).is_some_and(|p| Self::in_reach(p, *pos)))
                .collect();
            let source = reachable
                .into_iter()
                .find_map(|p| self.liquid_at(world, p).filter(|(_, d)| *d == 0).map(|(k, _)| (p, k)));
            let Some((pos, kind)) = source else {
                self.resync_held(sid);
                return true;
            };
            let air = self.gd.air_id;
            let main = self.worlds.worlds[world].get_block(pos, &self.gd);
            if self.liquid_of(main).is_some() {
                self.change_block(world, pos, air);
            } else {
                self.set_water_layer(world, pos, false); // a waterlogged block keeps standing
            }
            let sound = if kind == Liquid::Water { "bucket.fill.water" } else { "bucket.fill.lava" };
            self.liquid_sound(world, pos, sound);
            if kind == Liquid::Water { "minecraft:water_bucket" } else { "minecraft:lava_bucket" }
        } else {
            let kind = empty_into.unwrap();
            let clicked_id = self.worlds.worlds[world].get_block(clicked, &self.gd);
            let pos = if self.gd.is_replaceable(clicked_id) { Some(clicked) } else { target };
            let in_reach = |pos: &[i32; 3]| self.players.get(&sid).is_some_and(|p| Self::in_reach(p, *pos));
            let pos = pos.filter(in_reach);
            let Some(pos) = pos.filter(|p| {
                let id = self.worlds.worlds[world].get_block(*p, &self.gd);
                self.gd.is_replaceable(id)
            }) else {
                self.resync_held(sid);
                return true;
            };
            self.set_liquid(world, pos, kind, 0);
            let sound = if kind == Liquid::Water { "bucket.empty.water" } else { "bucket.empty.lava" };
            self.liquid_sound(world, pos, sound);
            "minecraft:bucket"
        };
        self.swing_arm(sid);
        if gm == 1 {
            return true; // creative keeps its bucket
        }
        let Some(info) = self.gd.items_by_name.get(new_item_name) else { return true };
        let new_item = ItemDesc {
            id: info.id,
            count: 1,
            meta: 0,
            stack_id: 0,
            block_runtime_id: 0,
            extra: ceres_world::extra_data(None),
        };
        let Some(p) = self.players.get_mut(&sid) else { return true };
        if held.count > 1 {
            // a stack of empty buckets: one is used, the filled one goes elsewhere
            let mut rest = held.clone();
            rest.count -= 1;
            p.inventory.set_main(slot, rest);
            let (_, left) = p.inventory.add_item(new_item.clone(), &self.gd);
            self.sync_full_inventory(sid);
            if left > 0 {
                self.drop_from_player(sid, new_item);
            }
        } else {
            p.inventory.set_main(slot, new_item);
            p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
        }
        self.broadcast_equipment(sid);
        true
    }

    pub(crate) fn liquid_sound(&mut self, world: usize, pos: [i32; 3], sound: &str) {
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        self.broadcast(&protocol::level_sound_event(sound, center, -1), |_, q| q.world == world);
    }

    pub(crate) fn resync_held(&mut self, sid: SessionId) {
        if let Some(p) = self.players.get_mut(&sid) {
            let slot = (p.inventory.held_slot as usize).min(8);
            p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
        }
    }
}
