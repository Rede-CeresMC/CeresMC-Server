//! Furnaces, blast furnaces and smokers: fuel, cooking time, lit/unlit block and the progress
//! arrows (ContainerSetData). Contents stay in the vanilla block entity (Items, BurnTime,
//! CookTime, BurnDuration). A furnace only ticks while it has something to do.
//! TODO: XP from smelting, furnaces that were burning when the server stopped (they resume when
//! someone opens them), hoppers.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::ItemDesc;

pub(crate) const UI_FUEL: u8 = 24;
pub(crate) const UI_FURNACE: u8 = 25;
pub(crate) const UI_RESULT: u8 = 26;
pub(crate) const UI_BLAST_FURNACE: u8 = 45;
pub(crate) const UI_SMOKER: u8 = 46;

const SET_DATA_PROGRESS: i32 = 0;
const SET_DATA_FUEL_LEFT: i32 = 1;
const SET_DATA_FUEL_MAX: i32 = 2;

pub(crate) fn is_furnace_ui(ui: u8) -> bool {
    matches!(ui, UI_FURNACE | UI_BLAST_FURNACE | UI_SMOKER)
}

/// (recipe block name, cooking ticks, window type) for a furnace UI id.
fn furnace_info(ui: u8) -> (&'static str, u16, u8) {
    match ui {
        UI_BLAST_FURNACE => ("blast_furnace", 100, protocol::WINDOW_TYPE_BLAST_FURNACE),
        UI_SMOKER => ("smoker", 100, protocol::WINDOW_TYPE_SMOKER),
        _ => ("furnace", 200, protocol::WINDOW_TYPE_FURNACE),
    }
}

pub(crate) fn furnace_window_type(ui: u8) -> u8 {
    furnace_info(ui).2
}

/// Unlit and lit block names of each furnace kind.
pub(crate) fn furnace_names(ui: u8) -> (&'static str, &'static str) {
    match ui {
        UI_BLAST_FURNACE => ("minecraft:blast_furnace", "minecraft:lit_blast_furnace"),
        UI_SMOKER => ("minecraft:smoker", "minecraft:lit_smoker"),
        _ => ("minecraft:furnace", "minecraft:lit_furnace"),
    }
}

/// Ticks of burning an item gives (vanilla values).
pub(crate) fn fuel_ticks(name: &str) -> Option<u16> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    Some(match n {
        "lava_bucket" => 20000,
        "coal_block" => 16000,
        "dried_kelp_block" => 4000,
        "blaze_rod" => 2400,
        "coal" | "charcoal" => 1600,
        "boat" | "chest_boat" => 1200,
        "bamboo_raft" | "bamboo_chest_raft" => 1200,
        "bookshelf" | "chiseled_bookshelf" | "lectern" | "chest" | "trapped_chest" | "barrel" | "crafting_table"
        | "cartography_table" | "fletching_table" | "smithing_table" | "loom" | "composter" | "jukebox" | "note_block"
        | "daylight_detector" | "beehive" | "bee_nest" => 300,
        "stick" | "bamboo" | "scaffolding" => if n == "stick" { 100 } else { 50 },
        "bowl" | "dead_bush" | "sapling" => 100,
        "wooden_sword" | "wooden_pickaxe" | "wooden_axe" | "wooden_shovel" | "wooden_hoe" | "bow" | "crossbow"
        | "fishing_rod" | "ladder" => 200,
        _ if n.ends_with("_planks") || n.ends_with("_log") || n.ends_with("_wood") || n.ends_with("_stem")
            || n.ends_with("_hyphae") || n.ends_with("_fence") || n.ends_with("_fence_gate")
            || n.ends_with("_stairs") && is_wood(n) || n.ends_with("_trapdoor") && is_wood(n)
            || n.ends_with("_pressure_plate") && is_wood(n) =>
        {
            300
        }
        _ if n.ends_with("_slab") && is_wood(n) => 150,
        _ if n.ends_with("_door") && is_wood(n) || n.ends_with("_sign") || n.ends_with("_button") && is_wood(n) => {
            200
        }
        _ if n.ends_with("_sapling") || n.ends_with("_carpet") => 100,
        _ if n.ends_with("_wool") => 100,
        _ => return None,
    })
}

fn is_wood(n: &str) -> bool {
    ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "cherry", "bamboo", "pale_oak", "wooden"]
        .iter()
        .any(|w| n.starts_with(w))
        && !n.contains("crimson")
        && !n.contains("warped")
}

#[derive(Clone)]
pub(crate) struct FurnaceState {
    ui: u8,
    items: Vec<ItemDesc>,
    burn_time: u16,
    burn_duration: u16,
    cook_time: u16,
}

impl FurnaceState {
    pub(crate) fn items(&self) -> Vec<ItemDesc> {
        self.items.clone()
    }
    pub(crate) fn set_items(&mut self, items: Vec<ItemDesc>) {
        self.items = items;
    }
}

fn tag_u16(t: &Tag, key: &str) -> u16 {
    t.get(key).and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, u16::MAX as i64) as u16
}

impl Server {
    /// The smelting result of `input` in this kind of furnace.
    fn smelt_result(&self, ui: u8, input: &ItemDesc) -> Option<ItemDesc> {
        let (block, _, _) = furnace_info(ui);
        let name = &self.gd.items_by_id.get(&input.id)?.name;
        self.gd.smelting.get(&(block.to_string(), name.clone())).and_then(|list| {
            list.iter().find(|(aux, _)| aux.is_none_or(|a| a as u32 == input.meta)).map(|(_, out)| out.clone())
        })
    }

    /// Starts ticking a furnace (its contents changed or someone opened it).
    pub(crate) fn wake_furnace(&mut self, world: usize, pos: [i32; 3], ui: u8) {
        let tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd);
        let items = containers::items_from_tile(tile.as_ref(), &self.gd, 3);
        let t = tile.unwrap_or(Tag::Compound(Vec::new()));
        let state = FurnaceState {
            ui,
            items,
            burn_time: tag_u16(&t, "BurnTime"),
            burn_duration: tag_u16(&t, "BurnDuration"),
            cook_time: tag_u16(&t, "CookTime"),
        };
        self.furnaces.insert((world, pos), state);
    }

    /// Writes a furnace back to its block entity.
    fn save_furnace(&mut self, world: usize, pos: [i32; 3]) {
        let Some(f) = self.furnaces.get(&(world, pos)).cloned() else { return };
        let tile_id = match f.ui {
            UI_BLAST_FURNACE => "BlastFurnace",
            UI_SMOKER => "Smoker",
            _ => "Furnace",
        };
        let mut tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd).unwrap_or_else(|| {
            Tag::Compound(vec![("id".into(), Tag::String(tile_id.into())), ("isMovable".into(), Tag::Byte(1))])
        });
        tile.set("Items", containers::items_to_list(&f.items, &self.gd));
        tile.set("BurnTime", Tag::Short(f.burn_time as i16));
        tile.set("CookTime", Tag::Short(f.cook_time as i16));
        tile.set("BurnDuration", Tag::Short(f.burn_duration as i16));
        self.worlds.worlds[world].set_block_entity(pos, tile, &self.gd);
    }

    /// Players looking at this furnace.
    fn furnace_viewers(&self, world: usize, pos: [i32; 3]) -> Vec<SessionId> {
        self.players
            .iter()
            .filter(|(_, q)| q.inventory.container.as_ref().is_some_and(|c| c.world == world && c.pos == pos))
            .map(|(s, _)| *s)
            .collect()
    }

    /// Lit <-> unlit furnace block, keeping its facing and contents.
    fn set_furnace_lit(&mut self, world: usize, pos: [i32; 3], ui: u8, lit: bool) {
        let old = self.worlds.worlds[world].get_block(pos, &self.gd);
        let (unlit, lit_name) = furnace_names(ui);
        let target = if lit { lit_name } else { unlit };
        if self.gd.block_name(old) == target {
            return;
        }
        let Some(new) = self.gd.with_name(old, target) else { return };
        let tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd);
        if self.worlds.worlds[world].set_block(pos, new, &self.gd) {
            if let Some(t) = tile {
                self.worlds.worlds[world].set_block_entity(pos, t, &self.gd);
            }
            self.broadcast(&protocol::update_block(pos, new), |_, q| q.world == world);
        }
    }

    /// One tick of every working furnace.
    pub(crate) fn tick_furnaces(&mut self) {
        let keys: Vec<(usize, [i32; 3])> = self.furnaces.keys().copied().collect();
        for (world, pos) in keys {
            // the block may have been broken or replaced
            let block = self.worlds.worlds[world].get_block(pos, &self.gd);
            let Some(mut f) = self.furnaces.get(&(world, pos)).cloned() else { continue };
            let (unlit, lit) = furnace_names(f.ui);
            let name = self.gd.block_name(block);
            if name != unlit && name != lit {
                self.furnaces.remove(&(world, pos));
                continue;
            }
            let (_, cook_ticks, _) = furnace_info(f.ui);
            let result = if f.items[0].is_empty() { None } else { self.smelt_result(f.ui, &f.items[0]) };
            let can_smelt = result.as_ref().is_some_and(|out| {
                let r = &f.items[2];
                r.is_empty()
                    || (r.same_type(out) && (r.count + out.count) as u32 <= self.gd.max_stack(r.id) as u32)
            });
            let mut items_changed = false;
            if f.burn_time > 0 {
                f.burn_time -= 1;
            }
            if f.burn_time == 0 && can_smelt {
                let fuel = f.items[1].clone();
                let ticks = self.gd.items_by_id.get(&fuel.id).and_then(|i| fuel_ticks(&i.name));
                if let (Some(t), false) = (ticks, fuel.is_empty()) {
                    let is_lava = self.gd.items_by_id.get(&fuel.id).is_some_and(|i| i.name == "minecraft:lava_bucket");
                    f.burn_time = t;
                    f.burn_duration = t;
                    let mut rest = fuel;
                    rest.count -= 1;
                    if rest.count == 0 {
                        rest = ItemDesc::default();
                    }
                    if is_lava {
                        if let Some(b) = self.gd.items_by_name.get("minecraft:bucket") {
                            rest = ItemDesc {
                                id: b.id,
                                count: 1,
                                meta: 0,
                                stack_id: 0,
                                block_runtime_id: 0,
                                extra: ceres_world::extra_data(None),
                            };
                        }
                    }
                    f.items[1] = rest;
                    items_changed = true;
                }
            }
            if f.burn_time > 0 && can_smelt {
                f.cook_time += 1;
                if f.cook_time >= cook_ticks {
                    f.cook_time = 0;
                    let out = result.unwrap();
                    f.items[0].count -= 1;
                    if f.items[0].count == 0 {
                        f.items[0] = ItemDesc::default();
                    }
                    if f.items[2].is_empty() {
                        f.items[2] = out;
                    } else {
                        f.items[2].count += out.count;
                    }
                    items_changed = true;
                }
            } else if f.cook_time > 0 {
                f.cook_time = f.cook_time.saturating_sub(2);
            }
            let burning = f.burn_time > 0;
            let idle = !burning && f.cook_time == 0;
            self.furnaces.insert((world, pos), f.clone());
            self.set_furnace_lit(world, pos, f.ui, burning);
            if items_changed || idle || self.ticks % 20 == 0 {
                self.save_furnace(world, pos);
            }
            // what the viewers see
            for sid in self.furnace_viewers(world, pos) {
                let Some(q) = self.players.get_mut(&sid) else { continue };
                let Some(window) = q.inventory.container.as_ref().map(|c| c.window) else { continue };
                if items_changed {
                    let mut fresh = f.items.clone();
                    for it in fresh.iter_mut().filter(|it| !it.is_empty()) {
                        it.stack_id = q.inventory.alloc_id();
                    }
                    if let Some(c) = q.inventory.container.as_mut() {
                        c.items = fresh.clone();
                        c.changed = false;
                    }
                    q.out.push(protocol::inventory_content(window as u32, &fresh));
                }
                q.out.push(protocol::container_set_data(window, SET_DATA_PROGRESS, f.cook_time as i32));
                q.out.push(protocol::container_set_data(window, SET_DATA_FUEL_LEFT, f.burn_time as i32));
                q.out.push(protocol::container_set_data(window, SET_DATA_FUEL_MAX, f.burn_duration as i32));
            }
            if idle {
                self.furnaces.remove(&(world, pos));
            }
        }
    }
}
