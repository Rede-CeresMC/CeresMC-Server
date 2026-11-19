//! Block containers: chests, trapped chests, barrels and shulker boxes.
//! Contents live in the block entity (vanilla "Items" list), so worlds stay compatible with
//! BDS and PocketMine.
//! TODO: double chests (pairing), hoppers, comparators.

use super::inventory::{OpenContainer, STONECUTTER_UI, WORKBENCH_UI};
use super::*;
use ceres_codec::binary::W;
use ceres_codec::nbt::{self, Flavor, Tag};
use ceres_protocol::{ItemDesc, block_container};
use ceres_world::extra_data;

const CONTAINER_SLOTS: usize = 27;

/// (container UI id, block entity id, plays the lid animation) for a container block name.
/// The crafting table has no block entity: its grid lives in the player's inventory.
pub(crate) fn container_kind(name: &str) -> Option<(u8, &'static str, bool)> {
    match name {
        "minecraft:crafting_table" => Some((WORKBENCH_UI, "", false)),
        "minecraft:stonecutter_block" => Some((STONECUTTER_UI, "", false)),
        "minecraft:enchanting_table" => Some((enchanting::ENCHANT_UI, "", false)),
        "minecraft:grindstone" => Some((grindstone::GRINDSTONE_UI, "", false)),
        "minecraft:anvil" | "minecraft:chipped_anvil" | "minecraft:damaged_anvil" => Some((anvil::ANVIL_UI, "", false)),
        "minecraft:cartography_table" => Some((cartography::CARTOGRAPHY_UI, "", false)),
        "minecraft:loom" => Some((loom::LOOM_UI, "", false)),
        "minecraft:furnace" | "minecraft:lit_furnace" => Some((furnace::UI_FURNACE, "Furnace", false)),
        "minecraft:blast_furnace" | "minecraft:lit_blast_furnace" => {
            Some((furnace::UI_BLAST_FURNACE, "BlastFurnace", false))
        }
        "minecraft:smoker" | "minecraft:lit_smoker" => Some((furnace::UI_SMOKER, "Smoker", false)),
        "minecraft:chest" | "minecraft:trapped_chest" => Some((block_container::CHEST, "Chest", true)),
        "minecraft:barrel" => Some((block_container::BARREL, "Barrel", false)),
        n if n.ends_with("shulker_box") => Some((block_container::SHULKER_BOX, "ShulkerBox", true)),
        "minecraft:hopper" => Some((block_container::CHEST, "Hopper", false)),
        "minecraft:dispenser" => Some((block_container::CHEST, "Dispenser", false)),
        "minecraft:dropper" => Some((block_container::CHEST, "Dropper", false)),
        "minecraft:brewing_stand" => Some((brewing::UI_BREWING_INPUT, "BrewingStand", false)),
        _ => None,
    }
}

/// Item -> vanilla item NBT (Name, Count, Damage, Block, tag).
pub(crate) fn item_to_vanilla(it: &ItemDesc, slot: Option<u8>, gd: &GameData) -> Option<Tag> {
    let info = gd.items_by_id.get(&it.id)?;
    let mut fields = Vec::new();
    if let Some(s) = slot {
        fields.push(("Slot".to_string(), Tag::Byte(s as i8)));
    }
    fields.push(("Name".into(), Tag::String(info.name.clone())));
    fields.push(("Count".into(), Tag::Byte(it.count.min(127) as i8)));
    fields.push(("Damage".into(), Tag::Short(it.meta as i16)));
    fields.push(("WasPickedUp".into(), Tag::Byte(0)));
    if it.block_runtime_id != 0 {
        if let Some(disk) = gd.state_nbt.get(&it.block_runtime_id) {
            if let Ok((_, block)) = nbt::read_root(&mut R::new(disk), Flavor::LittleEndian) {
                fields.push(("Block".into(), block));
            }
        }
    }
    // network extra data: 0xffff, version 1, little-endian NBT, then can-place-on/can-destroy
    if it.extra.len() > 3 && it.extra[0] == 0xff && it.extra[1] == 0xff {
        if let Ok((_, tag)) = nbt::read_root(&mut R::new(&it.extra[3..]), Flavor::LittleEndian) {
            fields.push(("tag".into(), tag));
        }
    }
    Some(Tag::Compound(fields))
}

/// Vanilla item NBT -> item (unknown items are dropped).
pub(crate) fn item_from_vanilla(t: &Tag, gd: &GameData) -> Option<ItemDesc> {
    let name = t.get("Name")?.as_str()?;
    let full = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
    let info = gd.items_by_name.get(&full)?;
    let count = t.get("Count").and_then(|v| v.as_i64()).unwrap_or(1).clamp(1, 255) as u16;
    let meta = t.get("Damage").and_then(|v| v.as_i64()).unwrap_or(0).max(0) as u32;
    let block_runtime_id = match t.get("Block") {
        Some(b @ Tag::Compound(_)) => gd.lookup_block(b),
        _ => info.block_runtime_id,
    };
    let extra = match t.get("tag") {
        Some(tag @ Tag::Compound(_)) => {
            let mut w = W::new();
            nbt::write_root(&mut w, "", tag, Flavor::LittleEndian);
            extra_data(Some(&w.0))
        }
        _ => extra_data(None),
    };
    Some(ItemDesc { id: info.id, count, meta, stack_id: 0, block_runtime_id, extra })
}

/// Slots of a container, by its block entity id (chest and hopper share a UI id).
pub(crate) fn slots_of(tile_id: &str) -> usize {
    match tile_id {
        "Hopper" => hopper::HOPPER_SLOTS,
        "Dispenser" | "Dropper" => dispenser::SLOTS,
        "BrewingStand" => brewing::BREWING_SLOTS,
        "Furnace" | "BlastFurnace" | "Smoker" => 3,
        _ => CONTAINER_SLOTS,
    }
}

fn window_type_of(ui: u8, tile_id: &str) -> u8 {
    if furnace::is_furnace_ui(ui) {
        furnace::furnace_window_type(ui)
    } else if tile_id == "Hopper" {
        protocol::WINDOW_TYPE_HOPPER
    } else if tile_id == "Dispenser" {
        protocol::WINDOW_TYPE_DISPENSER
    } else if tile_id == "Dropper" {
        protocol::WINDOW_TYPE_DROPPER
    } else if tile_id == "BrewingStand" {
        protocol::WINDOW_TYPE_BREWING_STAND
    } else {
        protocol::WINDOW_TYPE_CONTAINER
    }
}

pub(crate) fn items_from_tile(tile: Option<&Tag>, gd: &GameData, slots: usize) -> Vec<ItemDesc> {
    let mut items = vec![ItemDesc::default(); slots];
    if let Some(list) = tile.and_then(|t| t.get("Items")).and_then(|t| t.as_list()) {
        for entry in list {
            let Some(slot) = entry.get("Slot").and_then(|v| v.as_i64()) else { continue };
            if (0..slots as i64).contains(&slot) {
                if let Some(it) = item_from_vanilla(entry, gd) {
                    items[slot as usize] = it;
                }
            }
        }
    }
    items
}

pub(crate) fn items_to_list(items: &[ItemDesc], gd: &GameData) -> Tag {
    Tag::List(
        10,
        items
            .iter()
            .enumerate()
            .filter(|(_, it)| !it.is_empty())
            .filter_map(|(i, it)| item_to_vanilla(it, Some(i as u8), gd))
            .collect(),
    )
}

/// Vanilla open/close sound of a container block.
fn container_sound(name: &str, open: bool) -> Option<&'static str> {
    Some(match (name, open) {
        ("minecraft:chest" | "minecraft:trapped_chest", true) => "chest.open",
        ("minecraft:chest" | "minecraft:trapped_chest", false) => "chest.closed",
        ("minecraft:barrel", true) => "block.barrel.open",
        ("minecraft:barrel", false) => "block.barrel.close",
        (n, true) if n.ends_with("shulker_box") => "shulkerbox.open",
        (n, false) if n.ends_with("shulker_box") => "shulkerbox.closed",
        _ => return None,
    })
}

impl Server {
    fn play_container_sound(&mut self, world: usize, pos: [i32; 3], open: bool) {
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        if let Some(sound) = container_sound(self.gd.block_name(block), open) {
            let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
            self.broadcast(&protocol::level_sound_event(sound, center, -1), |_, q| q.world == world);
        }
    }

    /// Opens the container at `pos` for a player. Returns false if the block is not a container.
    pub(crate) fn open_container(&mut self, sid: SessionId, pos: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let world = p.world;
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        let Some((ui, tile_id, lid)) = container_kind(self.gd.block_name(block)) else { return false };
        // a container above the chest lid blocks opening, like vanilla
        if lid && tile_id == "Chest" {
            let above = self.worlds.worlds[world].get_block([pos[0], pos[1] + 1, pos[2]], &self.gd);
            if self.gd.is_opaque_cube(above) {
                return true;
            }
        }
        self.close_container(sid, false);
        // crafting table and stonecutter: no storage of their own
        let workbench = tile_id.is_empty();
        let double = if tile_id == "Chest" { self.double_chest_items(world, pos) } else { None };
        let (pos, pair) = match &double {
            Some((first, second, _)) => (*first, Some(*second)),
            None => (pos, None),
        };
        let mut items = if let Some((_, _, items)) = double {
            items
        } else if workbench {
            Vec::new()
        } else {
            // a working furnace holds the newest contents itself
            if furnace::is_furnace_ui(ui) && !self.furnaces.contains_key(&(world, pos)) {
                self.wake_furnace(world, pos, ui);
            }
            let tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd);
            if tile_id == "BrewingStand" && !self.brewers.contains_key(&(world, pos)) {
                self.wake_brewing(world, pos);
            }
            match (self.furnaces.get(&(world, pos)), self.brewers.get(&(world, pos))) {
                (Some(f), _) if furnace::is_furnace_ui(ui) => f.items(),
                (_, Some(b)) if tile_id == "BrewingStand" => b.items(),
                _ => items_from_tile(tile.as_ref(), &self.gd, slots_of(tile_id)),
            }
        };
        let Some(p) = self.players.get_mut(&sid) else { return false };
        for it in items.iter_mut().filter(|it| !it.is_empty()) {
            it.stack_id = p.inventory.alloc_id();
        }
        p.window_counter = p.window_counter % 99 + 1;
        let window = p.window_counter;
        if workbench {
            let ty = match ui {
                STONECUTTER_UI => protocol::WINDOW_TYPE_STONECUTTER,
                enchanting::ENCHANT_UI => protocol::WINDOW_TYPE_ENCHANTMENT,
                grindstone::GRINDSTONE_UI => protocol::WINDOW_TYPE_GRINDSTONE,
                anvil::ANVIL_UI => protocol::WINDOW_TYPE_ANVIL,
                cartography::CARTOGRAPHY_UI => protocol::WINDOW_TYPE_CARTOGRAPHY,
                loom::LOOM_UI => protocol::WINDOW_TYPE_LOOM,
                _ => protocol::WINDOW_TYPE_WORKBENCH,
            };
            p.out.push(protocol::open_block_container(window, ty, pos));
        } else {
            p.out.push(protocol::open_block_container(window, window_type_of(ui, tile_id), pos));
            p.out.push(protocol::inventory_content(window as u32, &items));
        }
        p.inventory.container = Some(OpenContainer { ui, window, world, pos, pair, items, changed: false });
        if self.container_viewers(world, pos) == 1 {
            self.play_container_sound(world, pos, true);
        }
        if lid {
            let viewers = self.container_viewers(world, pos);
            for half in std::iter::once(pos).chain(pair) {
                self.broadcast(&protocol::block_event(half, 1, viewers as i32), |_, q| q.world == world);
            }
        }
        true
    }

    fn container_viewers(&self, world: usize, pos: [i32; 3]) -> usize {
        self.players
            .values()
            .filter(|q| q.inventory.container.as_ref().is_some_and(|c| c.world == world && c.pos == pos))
            .count()
    }

    /// The open container may only be used while the player is alive, in its world, within reach,
    /// and the block is still that kind of container.
    pub(crate) fn container_still_valid(&mut self, sid: SessionId) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let Some(c) = p.inventory.container.as_ref() else { return true };
        if p.dead || p.world != c.world || !Self::in_reach(p, c.pos) {
            return false;
        }
        let (world, pos, ui) = (c.world, c.pos, c.ui);
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        container_kind(self.gd.block_name(block)).is_some_and(|(u, _, _)| u == ui)
    }

    /// Writes an open container back to its block entity and shows the change to other viewers.
    pub(crate) fn sync_container(&mut self, sid: SessionId) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(c) = p.inventory.container.as_mut() else { return };
        if !c.changed
            || c.ui == WORKBENCH_UI
            || c.ui == STONECUTTER_UI
            || c.ui == enchanting::ENCHANT_UI
            || c.ui == grindstone::GRINDSTONE_UI
            || c.ui == anvil::ANVIL_UI
            || c.ui == cartography::CARTOGRAPHY_UI
            || c.ui == loom::LOOM_UI
        {
            return;
        }
        c.changed = false;
        let (world, pos, pair, items) = (c.world, c.pos, c.pair, c.items.clone());
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        let Some((_, tile_id, _)) = container_kind(self.gd.block_name(block)) else { return };
        // the view must still match what is stored (27 per chest half), or nothing is written
        let expected = if pair.is_some() { 2 * CONTAINER_SLOTS } else { slots_of(tile_id) };
        if items.len() != expected {
            return;
        }
        // a double chest keeps 27 items in each half
        let parts: Vec<([i32; 3], &[ItemDesc])> = match pair {
            Some(second) => vec![(pos, &items[..CONTAINER_SLOTS]), (second, &items[CONTAINER_SLOTS..])],
            None => vec![(pos, &items[..])],
        };
        for (half, half_items) in parts {
            let mut tile = self.worlds.worlds[world].get_block_entity(half, &self.gd).unwrap_or_else(|| {
                Tag::Compound(vec![("id".into(), Tag::String(tile_id.into())), ("isMovable".into(), Tag::Byte(1))])
            });
            tile.set("Items", items_to_list(half_items, &self.gd));
            self.worlds.worlds[world].set_block_entity(half, tile, &self.gd);
        }
        // a brewing stand starts working on what the player put in
        if tile_id == "BrewingStand" {
            match self.brewers.get_mut(&(world, pos)) {
                Some(b) => b.set_items(items.clone()),
                None => self.wake_brewing(world, pos),
            }
        }
        // a furnace starts working on what the player put in
        if let Some((ui, _, _)) = container_kind(self.gd.block_name(block)).filter(|(u, _, _)| furnace::is_furnace_ui(*u)) {
            match self.furnaces.get_mut(&(world, pos)) {
                Some(f) => f.set_items(items.clone()),
                None => self.wake_furnace(world, pos, ui),
            }
        }
        // everyone else looking at the same container sees the new contents
        let others: Vec<SessionId> = self
            .players
            .iter()
            .filter(|(o, q)| {
                **o != sid
                    && q.inventory
                        .container
                        .as_ref()
                        .is_some_and(|c| c.world == world && c.pos == pos && c.pair == pair && c.items.len() == items.len())
            })
            .map(|(o, _)| *o)
            .collect();
        for o in others {
            let q = self.players.get_mut(&o).unwrap();
            let mut fresh = items.clone();
            for it in fresh.iter_mut().filter(|it| !it.is_empty()) {
                it.stack_id = q.inventory.alloc_id();
            }
            let c = q.inventory.container.as_mut().unwrap();
            c.items = fresh.clone();
            let window = c.window;
            q.out.push(protocol::inventory_content(window as u32, &fresh));
        }
    }

    /// Closes (after saving) every window open on any of these blocks.
    pub(crate) fn close_containers_at(&mut self, world: usize, positions: &[[i32; 3]]) {
        let viewers: Vec<SessionId> = self
            .players
            .iter()
            .filter(|(_, q)| {
                q.inventory.container.as_ref().is_some_and(|c| {
                    c.world == world && positions.iter().any(|p| c.pos == *p || c.pair == Some(*p))
                })
            })
            .map(|(s, _)| *s)
            .collect();
        for s in viewers {
            self.close_container(s, true);
        }
    }

    /// Closes the player's open container. `by_server` tells the client to close its window too.
    pub(crate) fn close_container(&mut self, sid: SessionId, by_server: bool) {
        self.sync_container(sid);
        self.return_craft_grid(sid);
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(c) = p.inventory.container.take() else { return };
        if by_server {
            let block = self.worlds.worlds[c.world].get_block(c.pos, &self.gd);
            let tile_id = container_kind(self.gd.block_name(block)).map_or("", |(_, t, _)| t);
            let ty = match c.ui {
                WORKBENCH_UI => protocol::WINDOW_TYPE_WORKBENCH,
                STONECUTTER_UI => protocol::WINDOW_TYPE_STONECUTTER,
                enchanting::ENCHANT_UI => protocol::WINDOW_TYPE_ENCHANTMENT,
                grindstone::GRINDSTONE_UI => protocol::WINDOW_TYPE_GRINDSTONE,
                anvil::ANVIL_UI => protocol::WINDOW_TYPE_ANVIL,
                cartography::CARTOGRAPHY_UI => protocol::WINDOW_TYPE_CARTOGRAPHY,
                loom::LOOM_UI => protocol::WINDOW_TYPE_LOOM,
                _ => window_type_of(c.ui, tile_id),
            };
            p.out.push(protocol::container_close(c.window, ty, true));
        }
        if self.container_viewers(c.world, c.pos) == 0 {
            self.play_container_sound(c.world, c.pos, false);
        }
        let block = self.worlds.worlds[c.world].get_block(c.pos, &self.gd);
        if let Some((_, _, true)) = container_kind(self.gd.block_name(block)) {
            let viewers = self.container_viewers(c.world, c.pos);
            for half in std::iter::once(c.pos).chain(c.pair) {
                self.broadcast(&protocol::block_event(half, 1, viewers as i32), |_, q| q.world == c.world);
            }
        }
    }

    /// Items stored in a container block, removed from it (used when the block is broken).
    pub(crate) fn take_container_items(&mut self, world: usize, pos: [i32; 3]) -> Vec<ItemDesc> {
        self.unpair_chest(world, pos);
        let tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd);
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        let slots = container_kind(self.gd.block_name(block)).map_or(CONTAINER_SLOTS, |(_, t, _)| slots_of(t));
        let items: Vec<ItemDesc> = match (self.furnaces.remove(&(world, pos)), self.brewers.remove(&(world, pos))) {
            (Some(f), _) => f.items(),
            (_, Some(b)) => b.items(),
            _ => items_from_tile(tile.as_ref(), &self.gd, slots),
        }
        .into_iter()
        .filter(|i| !i.is_empty())
        .collect();
        // close the window for anyone looking at it
        let viewers: Vec<SessionId> = self
            .players
            .iter()
            .filter(|(_, q)| {
                q.inventory.container.as_ref().is_some_and(|c| c.world == world && (c.pos == pos || c.pair == Some(pos)))
            })
            .map(|(s, _)| *s)
            .collect();
        for s in viewers {
            if let Some(q) = self.players.get_mut(&s) {
                if let Some(c) = q.inventory.container.take() {
                    q.out.push(protocol::container_close(c.window, protocol::WINDOW_TYPE_CONTAINER, true));
                }
            }
        }
        items
    }
}
