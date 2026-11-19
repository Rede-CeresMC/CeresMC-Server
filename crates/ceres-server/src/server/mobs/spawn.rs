//! Natural spawning from the spawn rules: a few attempts per second around each player, 24 to
//! 44 blocks away, on the surface, underground or under water; each rule's conditions (biome
//! filter, light, difficulty, height, ground block, density, distance) and weight decide what
//! comes, in herds, with permuted types and spawn events. Light comes from an estimate (sky by
//! time of day and light-giving blocks nearby) since the server has no light engine.

use super::filters::{self, Cx};
use super::json::*;
use super::nav::{SOLID, WATER, LAVA};
use super::*;
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Spot {
    Surface,
    Underground,
    Underwater,
    Lava,
}

/// Mobs of a population allowed near each player.
fn cap(population: &str) -> usize {
    match population {
        "monster" => 24,
        "animal" => 10,
        "water_animal" => 6,
        "ambient" => 3,
        "cat" => 2,
        _ => 2,
    }
}

impl Server {
    fn population_of(&self, id: &str) -> &str {
        self.mobsys.reg.spawn_rules.iter().find(|r| r.id == id).map(|r| r.population.as_str()).unwrap_or("")
    }

    pub(crate) fn natural_spawning(&mut self) {
        if self.ticks % 10 != 0 || self.mobsys.reg.spawn_rules.is_empty() {
            return;
        }
        let players: Vec<(usize, [f32; 3])> = self
            .players
            .values()
            .filter(|p| p.state == State::Spawned && !p.dead && p.gamemode != 6)
            .map(|p| (p.world, [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]))
            .collect();
        let pops = ["monster", "animal", "water_animal", "ambient", "monster", "cat"];
        for (k, (world, feet)) in players.into_iter().enumerate() {
            if self.worlds.worlds[world].void_world {
                continue;
            }
            let pop = pops[((self.ticks / 10) as usize + k) % pops.len()];
            let allowed = if pop == "monster" { self.cfg.spawn_monsters && self.cfg.difficulty > 0 } else { self.cfg.spawn_animals };
            if !allowed {
                continue;
            }
            if pop != "monster" && rand::random::<u32>() % 8 != 0 {
                continue; // animals come back slowly
            }
            let near = self
                .mobs
                .iter()
                .filter(|m| m.alive() && m.world == world && filters::dist(m.pos, feet) < 64.0)
                .filter(|m| self.population_of(&m.def.id) == pop)
                .count();
            if near >= cap(pop) {
                continue;
            }
            self.spawn_attempt(world, feet, pop);
        }
    }

    fn spawn_attempt(&mut self, world: usize, feet: [f32; 3], pop: &str) {
        let a = rand::random::<f32>() * std::f32::consts::TAU;
        let d = 24.0 + rand::random::<f32>() * 20.0;
        let (x, z) = ((feet[0] + a.cos() * d).floor() as i32, (feet[2] + a.sin() * d).floor() as i32);
        if !self.worlds.worlds[world].is_loaded([x, feet[1] as i32, z]) {
            return;
        }
        // standing spots in this column near the player's height
        let mut spots: Vec<([i32; 3], Spot)> = Vec::new();
        let top = (feet[1] as i32 + 24).min(ceres_world::MAX_Y - 2);
        let bottom = (feet[1] as i32 - 24).max(ceres_world::MIN_Y + 1);
        let mut sky = true;
        let mut y = top;
        while y > bottom {
            let c = self.class_at(world, [x, y, z]);
            let below = self.class_at(world, [x, y - 1, z]);
            let above = self.class_at(world, [x, y + 1, z]);
            if c & WATER != 0 && below & WATER != 0 {
                spots.push(([x, y, z], Spot::Underwater));
            } else if c & LAVA != 0 && above & (SOLID | LAVA) == 0 {
                spots.push(([x, y + 1, z], Spot::Lava));
            } else if c & (SOLID | WATER | LAVA) == 0 && above & SOLID == 0 && below & SOLID != 0 {
                spots.push(([x, y, z], if sky { Spot::Surface } else { Spot::Underground }));
            }
            if c & (SOLID | WATER) != 0 {
                sky = false;
            }
            y -= 1;
        }
        if spots.is_empty() {
            return;
        }
        let (spot, kind) = spots[rand::random::<u32>() as usize % spots.len()];
        let pos = [spot[0] as f32 + 0.5, spot[1] as f32, spot[2] as f32 + 0.5];
        let light = self.light_at(world, spot);
        let below_id = self.worlds.worlds[world].get_block([spot[0], spot[1] - 1, spot[2]], &self.gd);
        let below_name = self.gd.block_name(below_id).to_string();
        let mut probe = Mob { world, pos, ..Default::default() };
        probe.env = self.env_at(world, pos, 1.8);
        probe.env.light = light;
        let dist_to_player = filters::dist(pos, feet);
        let reg = self.mobsys.reg.clone();
        let mut choices: Vec<(usize, usize, f32)> = Vec::new();
        for (ri, rule) in reg.spawn_rules.iter().enumerate() {
            if rule.population != pop {
                continue;
            }
            if !reg.entity(&rule.id).is_some_and(|d| d.spawnable || d.summonable) {
                continue;
            }
            for (ci, cond) in rule.conditions.iter().enumerate() {
                if self.condition_ok(&cond.c, &probe, kind, light, &below_name, spot[1], dist_to_player, &rule.id, feet) {
                    choices.push((ri, ci, cond.weight.max(0.0)));
                }
            }
        }
        let total: f32 = choices.iter().map(|c| c.2).sum();
        if total <= 0.0 {
            return;
        }
        let mut pick = rand::random::<f32>() * total;
        let mut chosen = choices[0];
        for c in &choices {
            if pick < c.2 {
                chosen = *c;
                break;
            }
            pick -= c.2;
        }
        let rule = &reg.spawn_rules[chosen.0];
        let c = &rule.conditions[chosen.1].c;
        let herd = c.get("minecraft:herd").cloned().unwrap_or(Value::Null);
        let herd = match herd {
            Value::Array(a) => a.first().cloned().unwrap_or(Value::Null),
            h => h,
        };
        let size = rand_i(num(&herd, "min_size", 1.0) as i32, num(&herd, "max_size", 1.0) as i32).clamp(1, 8);
        let herd_event = string(&herd, "event").map(String::from);
        let skip = num(&herd, "event_skip_count", 0.0) as i32;
        let spawn_event = c.get("minecraft:spawn_event").and_then(|e| string(e, "event")).map(String::from);
        for i in 0..size {
            let mut id = rule.id.clone();
            if let Some(Value::Array(perms)) = c.get("minecraft:permute_type") {
                let tot: f32 = perms.iter().map(|p| num(p, "weight", 1.0)).sum();
                let mut r = rand::random::<f32>() * tot;
                for p in perms {
                    let w = num(p, "weight", 1.0);
                    if r < w {
                        if let Some(t) = string(p, "entity_type") {
                            id = t.split('<').next().unwrap_or(t).to_string();
                        }
                        break;
                    }
                    r -= w;
                }
            }
            let at = if i == 0 { Some(pos) } else { self.herd_spot(world, spot, kind) };
            let Some(at) = at else { continue };
            let event = if i >= skip { herd_event.clone().or(spawn_event.clone()) } else { spawn_event.clone() };
            // the spawn event replaces entity_spawned only when the entity has it; run both otherwise
            let has = |s: &Server, e: &str| s.mobsys.reg.entity(&id).is_some_and(|d| d.events.contains_key(e));
            match event {
                Some(e) if has(self, &e) => {
                    if let Some(eid) = self.spawn_entity(world, &id, at, None) {
                        let e2 = if e.contains(':') || has(self, &e) { e } else { format!("minecraft:{e}") };
                        self.with_mob(eid, |s, m| s.mob_event(m, &e2, None));
                    }
                }
                _ => {
                    self.spawn_entity(world, &id, at, None);
                }
            }
        }
    }

    fn herd_spot(&mut self, world: usize, around: [i32; 3], kind: Spot) -> Option<[f32; 3]> {
        for _ in 0..6 {
            let b = [around[0] + rand_i(-2, 2), around[1] + rand_i(-1, 1), around[2] + rand_i(-2, 2)];
            let c = self.class_at(world, b);
            let ok = match kind {
                Spot::Underwater => c & WATER != 0,
                Spot::Lava => self.class_at(world, [b[0], b[1] - 1, b[2]]) & LAVA != 0 && c & SOLID == 0,
                _ => c & (SOLID | WATER | LAVA) == 0 && self.class_at(world, [b[0], b[1] + 1, b[2]]) & SOLID == 0 && self.class_at(world, [b[0], b[1] - 1, b[2]]) & SOLID != 0,
            };
            if ok {
                return Some([b[0] as f32 + 0.5, b[1] as f32, b[2] as f32 + 0.5]);
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn condition_ok(&self, c: &serde_json::Map<String, Value>, probe: &Mob, kind: Spot, light: u8, below: &str, y: i32, dist: f32, id: &str, feet: [f32; 3]) -> bool {
        let where_ok = match kind {
            Spot::Surface => c.contains_key("minecraft:spawns_on_surface"),
            Spot::Underground => c.contains_key("minecraft:spawns_underground"),
            Spot::Underwater => c.contains_key("minecraft:spawns_underwater"),
            Spot::Lava => c.contains_key("minecraft:spawns_lava"),
        };
        if !where_ok {
            return false;
        }
        if ["minecraft:delay_filter", "minecraft:mob_event_filter", "minecraft:player_in_village_filter"].iter().any(|k| c.contains_key(*k)) {
            return false; // patrols, wandering traders and village cats come from events this server lacks
        }
        if let Some(b) = c.get("minecraft:brightness_filter") {
            let mut l = light as f32;
            if boolean(b, "adjust_for_weather", false) && self.raining(probe.world) && probe.env.sky {
                l = (l - 3.0).max(0.0);
            }
            if l < num(b, "min", 0.0) || l > num(b, "max", 15.0) {
                return false;
            }
        }
        if let Some(d) = c.get("minecraft:difficulty_filter") {
            let lv = |s: Option<&str>, def: i32| match s {
                Some("peaceful") => 0,
                Some("easy") => 1,
                Some("normal") => 2,
                Some("hard") => 3,
                _ => def,
            };
            let (lo, hi) = (lv(string(d, "min"), 0), lv(string(d, "max"), 3));
            if self.cfg.difficulty < lo || self.cfg.difficulty > hi {
                return false;
            }
        }
        if let Some(h) = c.get("minecraft:height_filter") {
            if (y as f32) < num(h, "min", -64.0) || (y as f32) > num(h, "max", 320.0) {
                return false;
            }
        }
        if let Some(d) = c.get("minecraft:distance_filter") {
            if dist < num(d, "min", 0.0) || dist > num(d, "max", 1000.0) {
                return false;
            }
        }
        if let Some(f) = c.get("minecraft:spawns_on_block_filter") {
            let names = item_list(Some(f));
            if !names.is_empty() && !names.iter().any(|n| n == below) {
                return false;
            }
        }
        if let Some(f) = c.get("minecraft:spawns_on_block_prevented_filter") {
            if item_list(Some(f)).iter().any(|n| n == below) {
                return false;
            }
        }
        if let Some(dl) = c.get("minecraft:density_limit") {
            let key = if kind == Spot::Surface { "surface" } else { "underground" };
            if let Some(limit) = dl.get(key).and_then(|v| as_num(Some(v))) {
                let have = self.mobs.iter().filter(|m| m.alive() && m.world == probe.world && m.def.id == id && filters::dist(m.pos, feet) < 96.0).count();
                if have as f32 >= limit {
                    return false;
                }
            }
        }
        if let Some(bf) = c.get("minecraft:biome_filter") {
            if !filters::test(Some(bf), &Cx::new(self, probe)) {
                return false;
            }
        }
        true
    }
}
