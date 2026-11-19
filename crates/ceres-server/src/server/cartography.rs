//! Cartography table (logic of PowerNukkitX's `CraftRecipeOptionalProcessor`, LGPL-3.0): the
//! client sends the same "craft recipe optional" request as the anvil, the server works out the
//! result from the two slots and takes one item from each:
//! - paper: an empty map; an empty or filled map alone: the same map (to rename it);
//! - paper or a map + compass: a locator map (shows where its carriers are);
//! - filled map + glass pane: a locked copy (a new map that is never drawn again);
//! - filled map + empty map: two copies;
//! - filled map + paper: zoomed out one scale (a new map, drawn when first seen).
//!
//! The new maps (locked, zoomed out) are made by the server the first time they are held or
//! asked for, from what their item carries (`ceres_lock_of`, or the scale and old centre).

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::ItemDesc;
use durability as dur;

pub(crate) const CARTOGRAPHY_UI: u8 = 0xfa;
pub(crate) const UI_CARTO_INPUT: u8 = 55;
pub(crate) const UI_CARTO_ADDITIONAL: u8 = 56;
pub(crate) const SLOT_CARTO_INPUT: u8 = 12;
pub(crate) const SLOT_CARTO_ADDITIONAL: u8 = 13;

/// Locator maps (empty or filled) have the data value 2 (as PowerNukkitX).
pub(crate) const LOCATOR: u32 = 2;
/// Locked filled maps have the data value 6.
pub(crate) const LOCKED: u32 = 6;

fn new_map_id() -> i64 {
    (rand::random::<u32>() as i64) | 1 << 34
}

/// The table's result and how many of each slot it takes (always one of each filled slot);
/// None when the two slots make nothing.
pub(crate) fn result(input: &ItemDesc, additional: &ItemDesc, rename: Option<&str>, gd: &GameData) -> Option<ItemDesc> {
    let name = |i: &ItemDesc| if i.is_empty() { "" } else { gd.items_by_id.get(&i.id).map(|x| x.name.as_str()).unwrap_or("") };
    let (a, b) = (name(input), name(additional));
    let simple = |n: &str, meta: u32| {
        let info = gd.find_item(n)?;
        Some(ItemDesc { id: info.id, count: 1, meta, stack_id: 0, block_runtime_id: info.block_runtime_id, extra: ceres_world::extra_data(None) })
    };
    let one = |i: &ItemDesc| {
        let mut o = i.clone();
        o.count = 1;
        o.stack_id = 0;
        o
    };
    let locked = input.meta == LOCKED;
    let mut out = match (a, b) {
        ("minecraft:paper", "") => simple("minecraft:empty_map", 0)?,
        ("minecraft:empty_map" | "minecraft:filled_map", "") => one(input),
        ("minecraft:paper", "minecraft:compass") => simple("minecraft:empty_map", LOCATOR)?,
        ("minecraft:empty_map", "minecraft:compass") => {
            let mut o = one(input);
            o.meta = LOCATOR;
            o
        }
        ("minecraft:filled_map", "minecraft:compass") if !locked => {
            let mut o = one(input);
            o.meta = LOCATOR;
            dur::set_item_field(&mut o, "map_display_players", Some(Tag::Byte(1)));
            o
        }
        ("minecraft:filled_map", "minecraft:glass_pane") if !locked => {
            let Some(Tag::Long(old)) = dur::item_field(input, "map_uuid") else { return None };
            let mut o = one(input);
            o.meta = LOCKED;
            dur::set_item_field(&mut o, "map_uuid", Some(Tag::Long(new_map_id())));
            dur::set_item_field(&mut o, "ceres_lock_of", Some(Tag::Long(old)));
            dur::set_item_field(&mut o, "map_display_players", Some(Tag::Byte(0)));
            o
        }
        ("minecraft:filled_map", "minecraft:empty_map") => {
            let mut o = one(input);
            o.count = 2;
            o
        }
        ("minecraft:filled_map", "minecraft:paper") if !locked => {
            let int = |k: &str| match dur::item_field(input, k) {
                Some(Tag::Int(v)) => Some(v),
                _ => None,
            };
            let scale = int("map_scale").unwrap_or(0);
            let (Some(x), Some(z)) = (int("ceres_map_x"), int("ceres_map_z")) else { return None };
            if scale >= 4 {
                return None;
            }
            let mut o = one(input);
            dur::set_item_field(&mut o, "map_uuid", Some(Tag::Long(new_map_id())));
            dur::set_item_field(&mut o, "map_scale", Some(Tag::Int(scale + 1)));
            dur::set_item_field(&mut o, "ceres_map_x", Some(Tag::Int(x)));
            dur::set_item_field(&mut o, "ceres_map_z", Some(Tag::Int(z)));
            o
        }
        _ => return None,
    };
    if let Some(n) = rename.filter(|n| !n.trim().is_empty() && n.chars().count() <= 64) {
        super::anvil::set_name(&mut out, n);
    }
    Some(out)
}

impl Server {
    /// After a successful use of the table: its sound for everyone near.
    pub(crate) fn finish_cartography(&mut self, sid: SessionId) {
        let Some(c) = self.players.get(&sid).and_then(|p| p.inventory.container.as_ref()) else { return };
        let (world, pos) = (c.world, c.pos);
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        self.broadcast(&protocol::level_sound_event("block.cartography_table.use", center, -1), |_, q| q.world == world);
    }
}
