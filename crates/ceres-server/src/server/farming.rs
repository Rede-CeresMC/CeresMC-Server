//! Tools used on blocks (hoe tills, shovel makes paths, axe strips logs, flint and steel lights
//! fire), farming (seeds on farmland, growth by random ticks, bone meal, harvest drops) and the
//! random block ticks near players (crops grow, fire burns out).

use super::*;
use ceres_codec::nbt::Tag;

/// Random block updates per column per tick around each player, over the 32 blocks around their
/// feet: about vanilla's randomTickSpeed of 3 per 16x16x16 section.
const RANDOM_TICKS_PER_COLUMN: u32 = 6;
const RANDOM_TICK_RADIUS: i32 = 4;

/// Crop block planted by a seed item.
fn crop_of(item: &str) -> Option<&'static str> {
    Some(match item {
        "minecraft:wheat_seeds" => "minecraft:wheat",
        "minecraft:carrot" => "minecraft:carrots",
        "minecraft:potato" => "minecraft:potatoes",
        "minecraft:beetroot_seeds" => "minecraft:beetroot",
        _ => return None,
    })
}

pub(crate) fn is_crop(name: &str) -> bool {
    matches!(name, "minecraft:wheat" | "minecraft:carrots" | "minecraft:potatoes" | "minecraft:beetroot")
}

/// What a crop drops at a growth stage.
pub(crate) fn crop_drops(name: &str, growth: i64) -> Vec<(&'static str, u32)> {
    let ripe = growth >= 7;
    let roll = |min: u32, max: u32| min + rand::random::<u32>() % (max - min + 1);
    match name {
        "minecraft:wheat" if ripe => vec![("minecraft:wheat", 1), ("minecraft:wheat_seeds", roll(0, 3))],
        "minecraft:wheat" => vec![("minecraft:wheat_seeds", 1)],
        "minecraft:carrots" => vec![("minecraft:carrot", if ripe { roll(1, 4) } else { 1 })],
        "minecraft:potatoes" if ripe => {
            let mut v = vec![("minecraft:potato", roll(1, 4))];
            if rand::random::<u32>() % 50 == 0 {
                v.push(("minecraft:poisonous_potato", 1));
            }
            v
        }
        "minecraft:potatoes" => vec![("minecraft:potato", 1)],
        "minecraft:beetroot" if ripe => vec![("minecraft:beetroot", 1), ("minecraft:beetroot_seeds", roll(0, 3))],
        "minecraft:beetroot" => vec![("minecraft:beetroot_seeds", 1)],
        _ => vec![],
    }
}

fn growth(gd: &GameData, id: i32) -> Option<i64> {
    gd.block_states.get(&id)?.states.get("growth")?.as_i64()
}

impl Server {
    /// A click on a block with a tool or seed. Returns true when it was handled.
    pub(crate) fn use_item_on_block(&mut self, sid: SessionId, clicked: [i32; 3], face: u8) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm) = (p.world, p.gamemode);
        let held = p.inventory.held_item().clone();
        let Some(item) = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()) else { return false };
        let block = self.worlds.worlds[world].get_block(clicked, &self.gd);
        let block_name = self.gd.block_name(block).to_string();
        let above = [clicked[0], clicked[1] + 1, clicked[2]];
        let air_above = {
            let a = self.worlds.worlds[world].get_block(above, &self.gd);
            a == self.gd.air_id
        };
        let tool = self.gd.tool_info(held.id).map(|t| t.tool);

        // hoe: dirt and grass become farmland (coarse dirt becomes dirt)
        if tool == Some(ceres_world::Tool::Hoe) && face != 0 && air_above {
            let result = match block_name.as_str() {
                "minecraft:grass_block" | "minecraft:dirt" | "minecraft:grass_path" => "minecraft:farmland",
                "minecraft:coarse_dirt" => "minecraft:dirt",
                _ => "",
            };
            if let Some(id) = (!result.is_empty()).then(|| self.gd.default_block(result)).flatten() {
                self.change_block(world, clicked, id);
                self.tool_used(sid, clicked, "use");
                return true;
            }
        }
        // shovel: grass and dirt become a path
        if tool == Some(ceres_world::Tool::Shovel) && face != 0 && air_above {
            let path = matches!(
                block_name.as_str(),
                "minecraft:grass_block" | "minecraft:dirt" | "minecraft:coarse_dirt" | "minecraft:podzol" | "minecraft:mycelium"
            );
            if let Some(id) = path.then(|| self.gd.default_block("minecraft:grass_path")).flatten() {
                self.change_block(world, clicked, id);
                self.tool_used(sid, clicked, "use");
                return true;
            }
        }
        // axe: logs and wood lose their bark (keeping their axis)
        if tool == Some(ceres_world::Tool::Axe) && !block_name.starts_with("minecraft:stripped_") {
            let short = block_name.strip_prefix("minecraft:").unwrap_or(&block_name);
            if ["_log", "_wood", "_stem", "_hyphae"].iter().any(|s| short.ends_with(s)) && short != "petrified_oak_wood" {
                if let Some(id) = self.gd.with_name(block, &format!("minecraft:stripped_{short}")) {
                    self.change_block(world, clicked, id);
                    self.tool_used(sid, clicked, "use");
                    return true;
                }
            }
        }
        // flint and steel: fire on the clicked face
        if item == "minecraft:flint_and_steel" {
            let Some(target) = blocks::side(clicked, face) else { return true };
            let here = self.worlds.worlds[world].get_block(target, &self.gd);
            if here == self.gd.air_id {
                if let Some(fire) = self.gd.default_block("minecraft:fire") {
                    self.change_block(world, target, fire);
                    self.tool_used(sid, target, "ignite");
                }
            }
            return true;
        }
        // seeds on farmland
        if let Some(crop) = crop_of(&item) {
            if block_name == "minecraft:farmland" && face == 1 && air_above {
                if let Some(id) = self.gd.default_block(crop) {
                    self.change_block(world, above, id);
                    self.consume_held(sid, gm);
                    self.swing_arm(sid);
                }
                return true;
            }
            if !matches!(item.as_str(), "minecraft:carrot" | "minecraft:potato") {
                return true; // seeds are not blocks
            }
        }
        // bone meal: crops grow 2-5 stages
        if item == "minecraft:bone_meal" {
            if let Some(g) = growth(&self.gd, block).filter(|g| *g < 7 && is_crop(&block_name)) {
                let next = (g + 2 + (rand::random::<u32>() % 4) as i64).min(7);
                if let Some(id) = self.gd.with_states(block, &[("growth", Tag::Int(next as i32))]) {
                    self.change_block(world, clicked, id);
                    let center = [clicked[0] as f32 + 0.5, clicked[1] as f32 + 0.5, clicked[2] as f32 + 0.5];
                    self.broadcast(&protocol::level_event(LEVEL_EVENT_BONE_MEAL, center, 0), |_, q| q.world == world);
                    self.consume_held(sid, gm);
                    self.swing_arm(sid);
                }
                return true;
            }
            if self.bone_meal_plant(world, clicked) {
                self.consume_held(sid, gm);
                self.swing_arm(sid);
                return true;
            }
        }
        false
    }

    pub(crate) fn tool_used(&mut self, sid: SessionId, pos: [i32; 3], sound: &str) {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return };
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        // hoe, shovel and axe: "item.use.on" with the resulting block, like vanilla
        let (name, data) = if sound == "use" {
            ("item.use.on", self.worlds.worlds[world].get_block(pos, &self.gd))
        } else {
            (sound, -1)
        };
        self.broadcast(&protocol::level_sound_event(name, center, data), |_, q| q.world == world);
        self.swing_arm(sid);
        self.wear_held(sid, 1);
    }

    /// Uses up one of the held items (survival).
    pub(crate) fn consume_held(&mut self, sid: SessionId, gamemode: i32) {
        if gamemode == 1 {
            return;
        }
        let Some(p) = self.players.get_mut(&sid) else { return };
        let slot = (p.inventory.held_slot as usize).min(8);
        let mut item = p.inventory.main[slot].clone();
        item.count = item.count.saturating_sub(1);
        p.inventory.set_main(slot, if item.count == 0 { Default::default() } else { item });
        p.out.push(protocol::inventory_slot(
            ceres_protocol::window::INVENTORY as u32,
            slot as u32,
            &p.inventory.main[slot],
        ));
        self.broadcast_equipment(sid);
    }

    /// Random block ticks around every player: crops grow, fire burns out, farmland without a crop
    /// slowly turns back into dirt is left out (needs water checks).
    pub(crate) fn random_ticks(&mut self) {
        let centers: Vec<(usize, [i32; 3])> = self
            .players
            .values()
            .filter(|p| p.state == State::Spawned)
            .filter(|p| p.pos[0].abs() < 30_000_000.0 && p.pos[2].abs() < 30_000_000.0)
            .map(|p| {
                let y = (p.pos[1].floor() as i32).clamp(ceres_world::MIN_Y, ceres_world::MAX_Y);
                (p.world, [p.pos[0].floor() as i32, y, p.pos[2].floor() as i32])
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        for (world, c) in centers {
            let (cx, cz) = (c[0] >> 4, c[2] >> 4);
            for dx in -RANDOM_TICK_RADIUS..=RANDOM_TICK_RADIUS {
                for dz in -RANDOM_TICK_RADIUS..=RANDOM_TICK_RADIUS {
                    if !seen.insert((world, cx + dx, cz + dz)) {
                        continue; // a column near two players ticks once
                    }
                    for _ in 0..RANDOM_TICKS_PER_COLUMN {
                        let r = rand::random::<u32>();
                        let pos = [
                            ((cx + dx) << 4) + (r & 15) as i32,
                            c[1] - 16 + ((r >> 4) & 31) as i32,
                            ((cz + dz) << 4) + ((r >> 9) & 15) as i32,
                        ];
                        self.random_tick(world, pos);
                    }
                }
            }
        }
    }

    fn random_tick(&mut self, world: usize, pos: [i32; 3]) {
        if !(ceres_world::MIN_Y..=ceres_world::MAX_Y).contains(&pos[1]) || !self.worlds.worlds[world].is_loaded(pos) {
            return;
        }
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        let name = self.gd.block_name(block);
        // copper slowly turns green (vanilla: about an hour per stage)
        if name.contains("copper") && rand::random::<u32>() % 24 == 0 {
            if let Some(next) = useitems::copper_oxidized(name) {
                if let Some(id) = self.gd.with_name(block, &next) {
                    self.change_block(world, pos, id);
                }
                return;
            }
        }
        if !is_crop(name) {
            let name = name.to_string();
            if self.plant_random_tick(world, pos, block, &name) {
                return;
            }
        }
        let name = self.gd.block_name(block);
        if is_crop(name) {
            let Some(g) = growth(&self.gd, block) else { return };
            // grows on farmland, with light (not checked) about one stage in three ticks
            let below = self.worlds.worlds[world].get_block([pos[0], pos[1] - 1, pos[2]], &self.gd);
            if g < 7 && self.gd.block_name(below) == "minecraft:farmland" && rand::random::<u32>() % 3 == 0 {
                if let Some(id) = self.gd.with_states(block, &[("growth", Tag::Int(g as i32 + 1))]) {
                    self.change_block(world, pos, id);
                }
            }
        } else if name == "minecraft:fire" {
            // fire burns out unless it sits on netherrack
            let below = self.worlds.worlds[world].get_block([pos[0], pos[1] - 1, pos[2]], &self.gd);
            if self.gd.block_name(below) != "minecraft:netherrack" && rand::random::<u32>() % 4 == 0 {
                let air = self.gd.air_id;
                self.change_block(world, pos, air);
            }
        }
    }
}

const LEVEL_EVENT_BONE_MEAL: i32 = 2005;
