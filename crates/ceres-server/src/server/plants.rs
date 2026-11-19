//! Plants other than farmland crops: what grows by itself (random ticks) and what bone meal does.
//! - sugar cane and cactus grow up to 3 blocks (age 0-15 per block), bamboo up to 12-16;
//! - melon and pumpkin stems grow, then put a fruit next to them and bend towards it;
//! - sweet berry bushes, cocoa, nether wart, kelp and torchflower/pitcher crops grow stage by stage;
//! - bone meal: stems and berries and cocoa jump ahead, sugar cane shoots up, grass blocks sprout
//!   grass and flowers around, short grass and ferns double, tall flowers drop a copy.

use super::*;
use ceres_codec::nbt::Tag;

const LEVEL_EVENT_BONE_MEAL: i32 = 2005; // ParticleCropGrowth

fn state(gd: &GameData, id: i32, key: &str) -> Option<i64> {
    gd.block_states.get(&id)?.states.get(key)?.as_i64()
}

const TALL_FLOWERS: [&str; 4] = ["minecraft:sunflower", "minecraft:lilac", "minecraft:rose_bush", "minecraft:peony"];

impl Server {
    fn block(&mut self, world: usize, pos: [i32; 3]) -> i32 {
        self.worlds.worlds[world].get_block(pos, &self.gd)
    }

    fn name_at(&mut self, world: usize, pos: [i32; 3]) -> String {
        let id = self.block(world, pos);
        self.gd.block_name(id).to_string()
    }

    fn set_state(&mut self, world: usize, pos: [i32; 3], block: i32, key: &str, val: i64) -> bool {
        let tag = match self.gd.block_states.get(&block).and_then(|s| s.states.get(key)) {
            Some(Tag::Byte(_)) => Tag::Byte(val as i8),
            _ => Tag::Int(val as i32),
        };
        match self.gd.with_states(block, &[(key, tag)]) {
            Some(id) => self.change_block(world, pos, id),
            None => false,
        }
    }

    /// Height of the same plant below `pos` (counting `pos`).
    fn stack_height(&mut self, world: usize, pos: [i32; 3], name: &str) -> i32 {
        let mut h = 1;
        while h < 32 && self.name_at(world, [pos[0], pos[1] - h, pos[2]]) == name {
            h += 1;
        }
        h
    }

    /// Grows a column plant (sugar cane, cactus, bamboo, kelp) by one block if there is room.
    fn grow_up(&mut self, world: usize, pos: [i32; 3], block: i32, max: i32, into_water: bool) -> bool {
        let name = self.gd.block_name(block).to_string();
        let above = [pos[0], pos[1] + 1, pos[2]];
        let room = self.name_at(world, above);
        let ok = if into_water { room == "minecraft:water" || room == "minecraft:flowing_water" } else { room == "minecraft:air" };
        if !ok || self.stack_height(world, pos, &name) >= max {
            return false;
        }
        // the new top starts young
        let new = match state(&self.gd, block, "age") {
            Some(_) => self.gd.with_states(block, &[("age", Tag::Int(0))]).unwrap_or(block),
            None => block,
        };
        let new = match state(&self.gd, block, "kelp_age") {
            Some(a) => self.gd.with_states(block, &[("kelp_age", Tag::Int((a + 1).min(25) as i32))]).unwrap_or(new),
            None => new,
        };
        self.change_block(world, above, new)
    }

    /// Random tick of a plant this module knows. True when the block was one.
    pub(crate) fn plant_random_tick(&mut self, world: usize, pos: [i32; 3], block: i32, name: &str) -> bool {
        let roll = |n: u32| rand::random::<u32>() % n == 0;
        if name.ends_with("_sapling") {
            return self.sapling_tick(world, pos, block, name);
        }
        // living coral dies without water next to it (a fan or plant needs it in its own block)
        if name.contains("coral") && !name.starts_with("minecraft:dead_") {
            let near = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]].iter().any(|d| self.is_water(world, [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]]));
            // coral plants and fans are usually waterlogged: their water is on the second layer
            let logged = self.worlds.worlds[world].get_block_layer(pos, 1, &self.gd).is_some_and(|id| self.gd.block_name(id).contains("water"));
            let wet = near || logged;
            if !wet {
                let dead = format!("minecraft:dead_{}", name.trim_start_matches("minecraft:"));
                if let Some(id) = self.gd.with_name(block, &dead) {
                    self.change_block(world, pos, id);
                }
            }
            return true;
        }
        if name == "minecraft:mangrove_propagule" {
            if roll(7) {
                self.grow_propagule(world, pos, block);
            }
            return true;
        }
        match name {
            "minecraft:reeds" | "minecraft:cactus" => {
                let age = state(&self.gd, block, "age").unwrap_or(0);
                if age < 15 {
                    self.set_state(world, pos, block, "age", age + 1);
                } else if self.grow_up(world, pos, block, 3, false) {
                    self.set_state(world, pos, block, "age", 0);
                }
            }
            "minecraft:bamboo" => {
                if roll(3) {
                    let max = 12 + (pos[0].wrapping_mul(31) ^ pos[2].wrapping_mul(17)).rem_euclid(5);
                    self.grow_up(world, pos, block, max, false);
                }
            }
            "minecraft:cave_vines" | "minecraft:cave_vines_head_with_berries" | "minecraft:cave_vines_body_with_berries" => {
                if roll(10) {
                    self.grow_cave_vine(world, pos, block, name);
                }
            }
            "minecraft:brown_mushroom" | "minecraft:red_mushroom" => {
                if roll(25) {
                    self.spread_mushroom(world, pos, block, name);
                }
            }
            "minecraft:kelp" => {
                if roll(7) && state(&self.gd, block, "kelp_age").unwrap_or(0) < 25 {
                    self.grow_up(world, pos, block, 26, true);
                }
            }
            "minecraft:melon_stem" | "minecraft:pumpkin_stem" => {
                if !roll(2) {
                    return true;
                }
                let g = state(&self.gd, block, "growth").unwrap_or(0);
                if g < 7 {
                    self.set_state(world, pos, block, "growth", g + 1);
                } else {
                    self.stem_fruit(world, pos, block, name);
                }
            }
            "minecraft:sweet_berry_bush" => {
                let g = state(&self.gd, block, "growth").unwrap_or(0);
                if g < 3 && roll(5) {
                    self.set_state(world, pos, block, "growth", g + 1);
                }
            }
            "minecraft:cocoa" | "minecraft:nether_wart" => {
                let (max, chance) = if name == "minecraft:cocoa" { (2, 2) } else { (3, 10) };
                let a = state(&self.gd, block, "age").unwrap_or(0);
                if a < max && roll(chance) {
                    self.set_state(world, pos, block, "age", a + 1);
                }
            }
            "minecraft:torchflower_crop" | "minecraft:pitcher_crop" => {
                let g = state(&self.gd, block, "growth").unwrap_or(0);
                let max = if name == "minecraft:torchflower_crop" { 1 } else { 4 };
                if roll(3) {
                    if g < max {
                        self.set_state(world, pos, block, "growth", g + 1);
                    } else if name == "minecraft:torchflower_crop" {
                        if let Some(id) = self.gd.default_block("minecraft:torchflower") {
                            self.change_block(world, pos, id);
                        }
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// A ripe stem puts its fruit on a free side with soil under it and turns to face it.
    fn stem_fruit(&mut self, world: usize, pos: [i32; 3], stem: i32, name: &str) {
        let fruit = if name == "minecraft:melon_stem" { "minecraft:melon_block" } else { "minecraft:pumpkin" };
        // facing_direction: 2 north, 3 south, 4 west, 5 east
        let sides = [([0, 0, -1], 2), ([0, 0, 1], 3), ([-1, 0, 0], 4), ([1, 0, 0], 5)];
        let pick = sides[rand::random::<u32>() as usize % 4];
        let target = [pos[0] + pick.0[0], pos[1], pos[2] + pick.0[2]];
        if self.name_at(world, target) != "minecraft:air" {
            return;
        }
        let soil = self.name_at(world, [target[0], target[1] - 1, target[2]]);
        if !matches!(soil.as_str(), "minecraft:dirt" | "minecraft:grass_block" | "minecraft:farmland" | "minecraft:podzol" | "minecraft:coarse_dirt" | "minecraft:mycelium" | "minecraft:moss_block" | "minecraft:rooted_dirt" | "minecraft:mud") {
            return;
        }
        if let Some(id) = self.gd.default_block(fruit) {
            self.change_block(world, target, id);
            self.set_state(world, pos, stem, "facing_direction", pick.1);
        }
    }

    /// Bone meal on a plant this module knows. True when it did something (the item is used).
    pub(crate) fn bone_meal_plant(&mut self, world: usize, pos: [i32; 3]) -> bool {
        let block = self.block(world, pos);
        let name = self.gd.block_name(block).to_string();
        let done = match name.as_str() {
            "minecraft:melon_stem" | "minecraft:pumpkin_stem" => {
                let g = state(&self.gd, block, "growth").unwrap_or(0);
                if g >= 7 {
                    false
                } else {
                    let next = (g + 2 + (rand::random::<u32>() % 4) as i64).min(7);
                    self.set_state(world, pos, block, "growth", next)
                }
            }
            "minecraft:sweet_berry_bush" => {
                let g = state(&self.gd, block, "growth").unwrap_or(0);
                g < 3 && self.set_state(world, pos, block, "growth", g + 1)
            }
            "minecraft:cocoa" => {
                let a = state(&self.gd, block, "age").unwrap_or(0);
                a < 2 && self.set_state(world, pos, block, "age", a + 1)
            }
            "minecraft:torchflower_crop" | "minecraft:pitcher_crop" => {
                let g = state(&self.gd, block, "growth").unwrap_or(0);
                let max = if name == "minecraft:torchflower_crop" { 1 } else { 4 };
                g < max && self.set_state(world, pos, block, "growth", g + 1)
            }
            "minecraft:reeds" | "minecraft:bamboo" => {
                // Bedrock: bone meal grows the whole column at once
                let mut top = pos;
                while self.name_at(world, [top[0], top[1] + 1, top[2]]) == name {
                    top[1] += 1;
                }
                let max = if name == "minecraft:reeds" { 3 } else { 16 };
                let mut grew = false;
                for _ in 0..max {
                    let id = self.block(world, top);
                    if !self.grow_up(world, top, id, max, false) {
                        break;
                    }
                    top[1] += 1;
                    grew = true;
                }
                grew
            }
            "minecraft:kelp" => {
                let id = self.block(world, pos);
                self.grow_up(world, pos, id, 26, true)
            }
            "minecraft:short_grass" | "minecraft:fern" => {
                let tall = if name == "minecraft:short_grass" { "minecraft:tall_grass" } else { "minecraft:large_fern" };
                let above = [pos[0], pos[1] + 1, pos[2]];
                if self.name_at(world, above) != "minecraft:air" {
                    false
                } else if let Some(low) = self.gd.default_block(tall) {
                    let high = self.gd.with_states(low, &[("upper_block_bit", Tag::Byte(1))]).unwrap_or(low);
                    self.change_block(world, pos, low);
                    self.change_block(world, above, high);
                    true
                } else {
                    false
                }
            }
            n if TALL_FLOWERS.contains(&n) => {
                if let Some(it) = self.simple_item(n, 1) {
                    self.drop_at(world, [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5], it);
                }
                true
            }
            "minecraft:grass_block" => self.bone_meal_grass(world, pos),
            "minecraft:brown_mushroom" | "minecraft:red_mushroom" => {
                // used even when the mushroom does not grow this time
                if rand::random::<f32>() < 0.4 {
                    self.grow_huge_mushroom(world, pos, name == "minecraft:red_mushroom");
                }
                true
            }
            "minecraft:moss_block" => self.bone_meal_moss(world, pos),
            "minecraft:dirt_with_roots" => {
                // hanging roots grow under rooted dirt
                let below = [pos[0], pos[1] - 1, pos[2]];
                match (self.name_at(world, below) == "minecraft:air", self.gd.default_block("minecraft:hanging_roots")) {
                    (true, Some(roots)) => self.change_block(world, below, roots),
                    _ => false,
                }
            }
            "minecraft:cave_vines" => self.bone_meal_cave_vine(world, pos, &name),
            "minecraft:glow_lichen" => self.bone_meal_lichen(world, pos),
            "minecraft:small_dripleaf_block" | "minecraft:big_dripleaf" | "minecraft:sea_pickle" => self.bone_meal_dripleaf_pickle(world, pos, &name),
            n if self.is_water(world, [pos[0], pos[1] + 1, pos[2]]) || n == "minecraft:seagrass" => {
                let n = n.to_string();
                self.bone_meal_underwater(world, pos, &n)
            }
            "minecraft:mangrove_propagule" => {
                let block = self.block(world, pos);
                self.grow_propagule(world, pos, block)
            }
            "minecraft:azalea" | "minecraft:flowering_azalea" => {
                if rand::random::<f32>() < 0.45 {
                    self.grow_azalea(world, pos);
                }
                true
            }
            n if n.ends_with("_sapling") && !n.contains("bamboo") => {
                // used even when the tree does not come this time (vanilla: 45 %)
                if rand::random::<f32>() < 0.45 {
                    let n = n.to_string();
                    self.grow_tree(world, pos, &n);
                }
                true
            }
            _ => false,
        };
        if done {
            let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
            self.broadcast(&protocol::level_event(LEVEL_EVENT_BONE_MEAL, center, 0), |_, q| q.world == world);
        }
        done
    }

    /// Grass and flowers sprout on the grass blocks around.
    fn bone_meal_grass(&mut self, world: usize, pos: [i32; 3]) -> bool {
        let grass = self.gd.default_block("minecraft:short_grass");
        let flowers: Vec<i32> = ["minecraft:dandelion", "minecraft:poppy"].iter().filter_map(|n| self.gd.default_block(n)).collect();
        let mut any = false;
        for _ in 0..48 {
            let b = [pos[0] + (rand::random::<u32>() % 7) as i32 - 3, pos[1] + (rand::random::<u32>() % 3) as i32 - 1, pos[2] + (rand::random::<u32>() % 7) as i32 - 3];
            let above = [b[0], b[1] + 1, b[2]];
            if self.name_at(world, b) != "minecraft:grass_block" || self.name_at(world, above) != "minecraft:air" {
                continue;
            }
            if rand::random::<u32>() % 3 != 0 {
                continue;
            }
            let plant = if rand::random::<u32>() % 8 == 0 && !flowers.is_empty() { Some(flowers[rand::random::<u32>() as usize % flowers.len()]) } else { grass };
            if let Some(id) = plant {
                any |= self.change_block(world, above, id);
            }
        }
        any
    }
}

impl Server {
    /// Planting the seeds and fruits that are not farmland crops of farming.rs: melon and
    /// pumpkin seeds, torchflower seeds and pitcher pods on farmland, nether wart on soul sand,
    /// sweet berries on dirt, cocoa beans on the side of a jungle log. True when it was one.
    pub(crate) fn plant_item(&mut self, sid: SessionId, clicked: [i32; 3], face: u8) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm) = (p.world, p.gamemode);
        let item = self.gd.items_by_id.get(&p.inventory.held_item().id).map(|i| i.name.clone()).unwrap_or_default();
        let ground = self.name_at(world, clicked);
        let Some(target) = blocks::side(clicked, face) else { return false };
        let (plant, ok, states): (&str, bool, Vec<(&str, Tag)>) = match item.as_str() {
            "minecraft:melon_seeds" => ("minecraft:melon_stem", face == 1 && ground == "minecraft:farmland", vec![]),
            "minecraft:pumpkin_seeds" => ("minecraft:pumpkin_stem", face == 1 && ground == "minecraft:farmland", vec![]),
            "minecraft:torchflower_seeds" => ("minecraft:torchflower_crop", face == 1 && ground == "minecraft:farmland", vec![]),
            "minecraft:pitcher_pod" => ("minecraft:pitcher_crop", face == 1 && ground == "minecraft:farmland", vec![]),
            "minecraft:nether_wart" => ("minecraft:nether_wart", face == 1 && ground == "minecraft:soul_sand", vec![]),
            "minecraft:sweet_berries" => (
                "minecraft:sweet_berry_bush",
                face == 1 && matches!(ground.as_str(), "minecraft:grass_block" | "minecraft:dirt" | "minecraft:podzol" | "minecraft:coarse_dirt" | "minecraft:farmland" | "minecraft:moss_block"),
                vec![],
            ),
            "minecraft:glow_berries" => {
                let cid = self.worlds.worlds[world].get_block(clicked, &self.gd);
                let hold = self.gd.is_solid(cid) || ground.starts_with("minecraft:cave_vines");
                ("minecraft:cave_vines", face == 0 && hold, vec![])
            }
            "minecraft:cocoa_beans" => {
                let jungle = matches!(ground.as_str(), "minecraft:jungle_log" | "minecraft:jungle_wood" | "minecraft:stripped_jungle_log" | "minecraft:stripped_jungle_wood");
                // the bean faces the log: dragonfly's horizontalDirection of the opposite face
                let dir = match face {
                    2 => 0,
                    3 => 2,
                    4 => 3,
                    5 => 1,
                    _ => -1,
                };
                ("minecraft:cocoa", jungle && dir >= 0, vec![("direction", Tag::Int(dir))])
            }
            _ => return false,
        };
        if !ok || self.name_at(world, target) != "minecraft:air" {
            // sweet berries in the air are still food: let eating handle the click
            return item != "minecraft:sweet_berries" && item != "minecraft:glow_berries" && ok;
        }
        let Some(mut id) = self.gd.default_block(plant) else { return true };
        if !states.is_empty() {
            id = self.gd.with_states(id, &states).unwrap_or(id);
        }
        self.change_block(world, target, id);
        let at = [target[0] as f32 + 0.5, target[1] as f32 + 0.5, target[2] as f32 + 0.5];
        self.broadcast(&protocol::level_sound_event("place", at, id), |_, q| q.world == world);
        self.swing_arm(sid);
        if gm != 1 {
            self.consume_held(sid, gm);
        }
        true
    }
}

impl Server {
    /// A mushroom given bone meal (40 %) becomes a huge one: brown with a flat 7x7 cap, red
    /// with a 5x5 dome. Caps use huge_mushroom_bits 14 (cap on every side), stems 15.
    fn grow_huge_mushroom(&mut self, world: usize, pos: [i32; 3], red: bool) -> bool {
        let cap_name = if red { "minecraft:red_mushroom_block" } else { "minecraft:brown_mushroom_block" };
        let cap = self.gd.default_block(cap_name).and_then(|b| self.gd.with_states(b, &[("huge_mushroom_bits", Tag::Int(14))]));
        let stem = self.gd.default_block("minecraft:mushroom_stem").and_then(|b| self.gd.with_states(b, &[("huge_mushroom_bits", Tag::Int(15))]));
        let (Some(cap), Some(stem)) = (cap, stem) else { return false };
        let height = 5 + (rand::random::<u32>() % 3) as i32;
        let below = self.name_at(world, [pos[0], pos[1] - 1, pos[2]]);
        if !matches!(below.as_str(), "minecraft:dirt" | "minecraft:grass_block" | "minecraft:mycelium" | "minecraft:podzol" | "minecraft:dirt_with_roots") {
            return false;
        }
        for y in 1..=height + 1 {
            for dx in -3..=3 {
                for dz in -3..=3 {
                    let n = self.name_at(world, [pos[0] + dx, pos[1] + y, pos[2] + dz]);
                    if n != "minecraft:air" && !n.ends_with("_leaves") {
                        return false;
                    }
                }
            }
        }
        for y in 0..height {
            self.change_block(world, [pos[0], pos[1] + y, pos[2]], stem);
        }
        let top = pos[1] + height;
        if red {
            for y in top - 3..top {
                for (dx, dz) in [(-2, -1), (-2, 0), (-2, 1), (2, -1), (2, 0), (2, 1), (-1, -2), (0, -2), (1, -2), (-1, 2), (0, 2), (1, 2)] {
                    self.change_block(world, [pos[0] + dx, y, pos[2] + dz], cap);
                }
            }
            for dx in -1..=1 {
                for dz in -1..=1 {
                    self.change_block(world, [pos[0] + dx, top, pos[2] + dz], cap);
                }
            }
        } else {
            for dx in -3..=3i32 {
                for dz in -3..=3i32 {
                    if dx.abs() == 3 && dz.abs() == 3 {
                        continue;
                    }
                    self.change_block(world, [pos[0] + dx, top, pos[2] + dz], cap);
                }
            }
        }
        true
    }

    /// Bone meal on a moss block: moss creeps over the stone and dirt around and grows moss
    /// carpets, grass and azaleas on top.
    fn bone_meal_moss(&mut self, world: usize, pos: [i32; 3]) -> bool {
        let moss = self.gd.default_block("minecraft:moss_block");
        let tops: Vec<(i32, u32)> = [("minecraft:moss_carpet", 40), ("minecraft:short_grass", 30), ("minecraft:tall_grass", 0), ("minecraft:azalea", 7), ("minecraft:flowering_azalea", 3)]
            .iter()
            .filter_map(|(n, w)| self.gd.default_block(n).map(|id| (id, *w)))
            .filter(|(_, w)| *w > 0)
            .collect();
        let Some(moss) = moss else { return false };
        let mut any = false;
        for dx in -3..=3i32 {
            for dz in -3..=3i32 {
                if dx.abs() == 3 && dz.abs() == 3 && rand::random::<bool>() {
                    continue;
                }
                for dy in (-1..=1).rev() {
                    let p = [pos[0] + dx, pos[1] + dy, pos[2] + dz];
                    let n = self.name_at(world, p);
                    let soft = matches!(n.as_str(), "minecraft:stone" | "minecraft:dirt" | "minecraft:grass_block" | "minecraft:deepslate" | "minecraft:tuff" | "minecraft:granite" | "minecraft:diorite" | "minecraft:andesite" | "minecraft:podzol" | "minecraft:mycelium" | "minecraft:moss_block");
                    if !soft || self.name_at(world, [p[0], p[1] + 1, p[2]]) != "minecraft:air" {
                        continue;
                    }
                    if n != "minecraft:moss_block" {
                        any |= self.change_block(world, p, moss);
                    }
                    if rand::random::<u32>() % 4 == 0 && !tops.is_empty() {
                        let total: u32 = tops.iter().map(|t| t.1).sum();
                        let mut pick = rand::random::<u32>() % total;
                        for (id, w) in &tops {
                            if pick < *w {
                                any |= self.change_block(world, [p[0], p[1] + 1, p[2]], *id);
                                break;
                            }
                            pick -= w;
                        }
                    }
                    break;
                }
            }
        }
        any
    }
}

impl Server {
    fn is_water(&mut self, world: usize, p: [i32; 3]) -> bool {
        let n = self.name_at(world, p);
        n == "minecraft:water" || n == "minecraft:flowing_water"
    }

    /// Bone meal under water: seagrass turns tall; on the sea floor seagrass sprouts around.
    fn bone_meal_underwater(&mut self, world: usize, pos: [i32; 3], name: &str) -> bool {
        let Some(grass) = self.gd.default_block("minecraft:seagrass") else { return false };
        let typed = |s: &Server, t: &str| s.gd.with_states(grass, &[("sea_grass_type", Tag::String(t.into()))]);
        if name == "minecraft:seagrass" {
            let above = [pos[0], pos[1] + 1, pos[2]];
            let here = self.block(world, pos);
            let single = self.gd.block_states.get(&here).and_then(|s| s.states.get("sea_grass_type")).and_then(|t| t.as_str()).map(String::from) == Some("default".into());
            if !single || !self.is_water(world, above) {
                return false;
            }
            let (Some(bot), Some(top)) = (typed(self, "double_bot"), typed(self, "double_top")) else { return false };
            self.change_block(world, pos, bot);
            self.change_block(world, above, top);
            return true;
        }
        if !matches!(name, "minecraft:sand" | "minecraft:gravel" | "minecraft:dirt" | "minecraft:clay" | "minecraft:red_sand" | "minecraft:mud") || !self.is_water(world, [pos[0], pos[1] + 1, pos[2]]) {
            return false;
        }
        let mut any = false;
        for _ in 0..24 {
            let p = [pos[0] + (rand::random::<u32>() % 7) as i32 - 3, pos[1] + (rand::random::<u32>() % 3) as i32 - 1, pos[2] + (rand::random::<u32>() % 7) as i32 - 3];
            let above = [p[0], p[1] + 1, p[2]];
            let ground = self.name_at(world, p);
            let pid = self.block(world, p);
            if self.gd.is_solid(pid) && !ground.contains("water") && self.is_water(world, above) && rand::random::<u32>() % 2 == 0 {
                any |= self.change_block(world, above, grass);
            }
        }
        any
    }

    /// Bone meal on a cave vine without berries makes glow berries grow on it.
    fn bone_meal_cave_vine(&mut self, world: usize, pos: [i32; 3], name: &str) -> bool {
        if name != "minecraft:cave_vines" {
            return false;
        }
        let block = self.block(world, pos);
        let Some(berries) = self.gd.with_name(block, "minecraft:cave_vines_head_with_berries") else { return false };
        self.change_block(world, pos, berries)
    }

    /// Glow berries are picked from a cave vine by using it.
    pub(crate) fn pick_glow_berries(&mut self, sid: SessionId, pos: [i32; 3]) -> bool {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return false };
        let block = self.block(world, pos);
        let name = self.gd.block_name(block).to_string();
        if name == "minecraft:sweet_berry_bush" {
            // a bush with berries (stage 2 or 3) gives 1-2 or 2-3 and goes back to stage 1
            let g = state(&self.gd, block, "growth").unwrap_or(0);
            let bone_meal = self.players.get(&sid).and_then(|p| self.gd.items_by_id.get(&p.inventory.held_item().id)).is_some_and(|i| i.name == "minecraft:bone_meal");
            if g < 2 || (bone_meal && g < 3) {
                return false; // bone meal makes it grow first
            }
            let count = if g == 3 { 2 } else { 1 } + (rand::random::<u32>() % 2) as u16;
            self.set_state(world, pos, block, "growth", 1);
            if let Some(it) = self.simple_item("minecraft:sweet_berries", count) {
                self.drop_at(world, [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5], it);
            }
            let at = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
            self.broadcast(&protocol::level_sound_event("block.sweet_berry_bush.pick", at, -1), |_, q| q.world == world);
            self.swing_arm(sid);
            return true;
        }
        if !name.starts_with("minecraft:cave_vines_") || !name.ends_with("_with_berries") {
            return false;
        }
        let Some(plain) = self.gd.with_name(block, "minecraft:cave_vines") else { return false };
        self.change_block(world, pos, plain);
        if let Some(it) = self.simple_item("minecraft:glow_berries", 1) {
            self.drop_at(world, [pos[0] as f32 + 0.5, pos[1] as f32 + 0.3, pos[2] as f32 + 0.5], it);
        }
        let at = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        self.broadcast(&protocol::level_sound_event("pick_berries.cave_vines", at, -1), |_, q| q.world == world);
        self.swing_arm(sid);
        true
    }
}

impl Server {
    /// Bone meal: a small dripleaf becomes a big one (2 blocks: stem and leaf), a big dripleaf
    /// grows one block taller; a sea pickle on a coral block in water fills up to four.
    fn bone_meal_dripleaf_pickle(&mut self, world: usize, pos: [i32; 3], name: &str) -> bool {
        let block = self.block(world, pos);
        match name {
            "minecraft:small_dripleaf_block" => {
                let upper = state(&self.gd, block, "upper_block_bit").unwrap_or(0) != 0;
                let base = if upper { [pos[0], pos[1] - 1, pos[2]] } else { pos };
                let Some(big) = self.gd.default_block("minecraft:big_dripleaf") else { return false };
                let stem = self.gd.with_states(big, &[("big_dripleaf_head", Tag::Byte(0))]).unwrap_or(big);
                let head = self.gd.with_states(big, &[("big_dripleaf_head", Tag::Byte(1))]).unwrap_or(big);
                self.change_block(world, base, stem);
                self.change_block(world, [base[0], base[1] + 1, base[2]], head)
            }
            "minecraft:big_dripleaf" => {
                // walk up to the leaf and add one stem under it
                let mut top = pos;
                while self.name_at(world, [top[0], top[1] + 1, top[2]]) == "minecraft:big_dripleaf" {
                    top[1] += 1;
                }
                let above = [top[0], top[1] + 1, top[2]];
                if self.name_at(world, above) != "minecraft:air" {
                    return false;
                }
                let head = self.block(world, top);
                let stem = self.gd.with_states(head, &[("big_dripleaf_head", Tag::Byte(0))]).unwrap_or(head);
                self.change_block(world, top, stem);
                self.change_block(world, above, head)
            }
            "minecraft:sea_pickle" => {
                let below = self.name_at(world, [pos[0], pos[1] - 1, pos[2]]);
                let wet = self.is_water(world, [pos[0], pos[1] + 1, pos[2]]) || state(&self.gd, block, "dead_bit") == Some(0);
                if !below.contains("coral") || !wet {
                    return false;
                }
                self.set_state(world, pos, block, "cluster_count", 3)
            }
            _ => false,
        }
    }
}

impl Server {
    /// Bone meal on glow lichen: a new patch appears on a free block next to it, stuck to one
    /// of that block's solid sides (multi_face_direction_bits: down 1, up 2, south 4, west 8,
    /// north 16, east 32).
    fn bone_meal_lichen(&mut self, world: usize, pos: [i32; 3]) -> bool {
        let Some(lichen) = self.gd.default_block("minecraft:glow_lichen") else { return false };
        let faces: [([i32; 3], i32); 6] = [([0, -1, 0], 1), ([0, 1, 0], 2), ([0, 0, 1], 4), ([-1, 0, 0], 8), ([0, 0, -1], 16), ([1, 0, 0], 32)];
        let mut spots = Vec::new();
        for (d, _) in faces {
            let at = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
            if self.name_at(world, at) != "minecraft:air" {
                continue;
            }
            let mut bits = 0;
            for (s, bit) in faces {
                let next = [at[0] + s[0], at[1] + s[1], at[2] + s[2]];
                let id = self.block(world, next);
                if self.gd.is_solid(id) {
                    bits = bit;
                    if rand::random::<bool>() {
                        break;
                    }
                }
            }
            if bits != 0 {
                spots.push((at, bits));
            }
        }
        if spots.is_empty() {
            return false;
        }
        let (at, bits) = spots[rand::random::<u32>() as usize % spots.len()];
        let Some(id) = self.gd.with_states(lichen, &[("multi_face_direction_bits", Tag::Int(bits))]) else { return false };
        self.change_block(world, at, id)
    }
}

impl Server {
    /// The tip of cave vines grows one block down (age below 25); the new tip has berries 11 %
    /// of the time (minecraft.wiki "Glow Berries", growth).
    fn grow_cave_vine(&mut self, world: usize, pos: [i32; 3], block: i32, name: &str) {
        let below = [pos[0], pos[1] - 1, pos[2]];
        if pos[1] - 1 < ceres_world::MIN_Y || self.name_at(world, below) != "minecraft:air" {
            return;
        }
        let age = state(&self.gd, block, "growing_plant_age").unwrap_or(0);
        if age >= 25 {
            return;
        }
        let tip = if rand::random::<u32>() % 100 < 11 { "minecraft:cave_vines_head_with_berries" } else { "minecraft:cave_vines" };
        let Some(new) = self.gd.with_name(block, tip).and_then(|b| self.gd.with_states(b, &[("growing_plant_age", Tag::Int(age as i32 + 1))])) else { return };
        // the old tip with berries becomes a body with berries
        if name == "minecraft:cave_vines_head_with_berries" {
            if let Some(body) = self.gd.with_name(block, "minecraft:cave_vines_body_with_berries") {
                self.change_block(world, pos, body);
            }
        }
        self.change_block(world, below, new);
    }

    /// A mushroom spreads to a random spot near it while fewer than 5 of its kind are around
    /// (9x3x9), where a mushroom could stand.
    fn spread_mushroom(&mut self, world: usize, pos: [i32; 3], block: i32, name: &str) {
        let mut near = 0;
        for dx in -4..=4 {
            for dy in -1..=1 {
                for dz in -4..=4 {
                    if self.name_at(world, [pos[0] + dx, pos[1] + dy, pos[2] + dz]) == name {
                        near += 1;
                        if near >= 5 {
                            return;
                        }
                    }
                }
            }
        }
        let r = |n: u32| (rand::random::<u32>() % n) as i32;
        let to = [pos[0] + r(3) - 1, pos[1] + r(2) - r(2), pos[2] + r(3) - 1];
        if self.worlds.worlds[world].is_loaded(to) && self.name_at(world, to) == "minecraft:air" && self.can_stay(world, to, block, None, false) {
            self.change_block(world, to, block);
        }
    }
}
