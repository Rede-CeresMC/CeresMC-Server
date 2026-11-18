//! Tool, weapon and armor wear. Bedrock keeps the used-up durability in the item's NBT `Damage`
//! tag; the item breaks when it reaches the maximum. Unbreaking skips some of the wear.
//! Values are vanilla's (wiki); data-driven items (spears) are not covered yet.

use super::*;
use ceres_codec::nbt::{self, Flavor, Tag};
use ceres_protocol::{ItemDesc, window};

const ENCHANT_UNBREAKING: i16 = 17;

/// Maximum durability of a vanilla item, by name.
pub(crate) fn max_durability(name: &str) -> Option<u32> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let tiered = |prefix: &str| -> Option<u32> {
        Some(match prefix {
            "wooden" => 59,
            "stone" => 131,
            "copper" => 190,
            "iron" => 250,
            "golden" => 32,
            "diamond" => 1561,
            "netherite" => 2031,
            _ => return None,
        })
    };
    // [helmet, chestplate, leggings, boots]
    let armor = |prefix: &str| -> Option<[u32; 4]> {
        Some(match prefix {
            "leather" => [55, 80, 75, 65],
            "chainmail" | "iron" => [165, 240, 225, 195],
            "copper" => [121, 176, 165, 143],
            "golden" => [77, 112, 105, 91],
            "diamond" => [363, 528, 495, 429],
            "netherite" => [407, 592, 555, 481],
            _ => return None,
        })
    };
    if let Some((prefix, kind)) = n.rsplit_once('_') {
        match kind {
            "pickaxe" | "axe" | "shovel" | "hoe" | "sword" => return tiered(prefix),
            "helmet" => return armor(prefix).map(|a| a[0]),
            "chestplate" => return armor(prefix).map(|a| a[1]),
            "leggings" => return armor(prefix).map(|a| a[2]),
            "boots" => return armor(prefix).map(|a| a[3]),
            _ => {}
        }
    }
    Some(match n {
        "turtle_helmet" => 275,
        "shears" => 238,
        "bow" => 384,
        "crossbow" => 465,
        "trident" => 250,
        "fishing_rod" => 384,
        "flint_and_steel" => 64,
        "carrot_on_a_stick" => 25,
        "warped_fungus_on_a_stick" => 100,
        "shield" => 336,
        "elytra" => 432,
        "brush" => 64,
        "mace" => 500,
        _ => return None,
    })
}

/// The item's NBT (if any) and the bytes that follow it in the extra data (can-place-on lists).
fn split_extra(item: &ItemDesc) -> Option<(Tag, Vec<u8>)> {
    if item.extra.len() <= 3 || item.extra[0] != 0xff || item.extra[1] != 0xff {
        return None;
    }
    let mut r = R::new(&item.extra[3..]);
    let (_, tag) = nbt::read_root(&mut r, Flavor::LittleEndian).ok()?;
    Some((tag, r.rest().to_vec()))
}

pub(crate) fn item_damage(item: &ItemDesc) -> u32 {
    split_extra(item)
        .and_then(|(t, _)| t.get("Damage").and_then(|v| v.as_i64()))
        .unwrap_or(0)
        .max(0) as u32
}

fn set_item_damage(item: &mut ItemDesc, damage: u32) {
    let (mut tag, tail) = split_extra(item).unwrap_or_else(|| (Tag::Compound(Vec::new()), vec![0, 0, 0, 0, 0, 0, 0, 0]));
    if tag.get("Unbreakable").and_then(|v| v.as_i64()).unwrap_or(0) != 0 {
        return;
    }
    tag.set("Damage", Tag::Int(damage as i32));
    let mut w = ceres_codec::binary::W::new();
    w.u16_le(0xffff);
    w.u8(1);
    nbt::write_root(&mut w, "", &tag, Flavor::LittleEndian);
    w.bytes(&tail);
    item.extra = w.0;
}

/// Enchantment command names, by Bedrock id.
pub(crate) const ENCHANT_NAMES: [&str; 41] = [
    "protection", "fire_protection", "feather_falling", "blast_protection", "projectile_protection", "thorns",
    "respiration", "depth_strider", "aqua_affinity", "sharpness", "smite", "bane_of_arthropods", "knockback",
    "fire_aspect", "looting", "efficiency", "silk_touch", "unbreaking", "fortune", "power", "punch", "flame",
    "infinity", "luck_of_the_sea", "lure", "frost_walker", "mending", "binding", "vanishing", "impaling",
    "riptide", "loyalty", "channeling", "multishot", "piercing", "quick_charge", "soul_speed", "swift_sneak",
    "wind_burst", "density", "breach",
];

/// Adds (or changes the level of) an enchantment in the item's `ench` list.
pub(crate) fn set_enchant(item: &mut ItemDesc, id: i16, level: i16) {
    let (mut tag, tail) = split_extra(item).unwrap_or_else(|| (Tag::Compound(Vec::new()), vec![0, 0, 0, 0, 0, 0, 0, 0]));
    let mut list = match tag.get("ench") {
        Some(Tag::List(_, l)) => l.clone(),
        _ => Vec::new(),
    };
    list.retain(|e| e.get("id").and_then(|v| v.as_i64()) != Some(id as i64));
    list.push(Tag::Compound(vec![("id".into(), Tag::Short(id)), ("lvl".into(), Tag::Short(level))]));
    tag.set("ench", Tag::List(10, list));
    let mut w = ceres_codec::binary::W::new();
    w.u16_le(0xffff);
    w.u8(1);
    nbt::write_root(&mut w, "", &tag, Flavor::LittleEndian);
    w.bytes(&tail);
    item.extra = w.0;
}

fn unbreakable(item: &ItemDesc) -> bool {
    split_extra(item).is_some_and(|(t, _)| t.get("Unbreakable").and_then(|v| v.as_i64()).unwrap_or(0) != 0)
}

/// Wears `item` by `amount` (Unbreaking may skip each point). Returns true if it broke.
fn wear(item: &mut ItemDesc, amount: u32, gd: &GameData) -> bool {
    let Some(max) = gd.items_by_id.get(&item.id).and_then(|i| max_durability(&i.name)) else { return false };
    if item.is_empty() || unbreakable(item) {
        return false;
    }
    let unbreaking = blocks::enchant_level(item, ENCHANT_UNBREAKING) as u32;
    let applied = (0..amount).filter(|_| unbreaking == 0 || rand::random::<u32>() % (unbreaking + 1) == 0).count() as u32;
    if applied == 0 {
        return false;
    }
    let damage = item_damage(item) + applied;
    if damage >= max {
        *item = ItemDesc::default();
        return true;
    }
    set_item_damage(item, damage);
    false
}

impl Server {
    fn wears(&self, sid: SessionId) -> bool {
        self.players.get(&sid).is_some_and(|p| p.gamemode == 0 || p.gamemode == 2)
    }

    fn play_break_sound(&mut self, sid: SessionId) {
        let Some(p) = self.players.get(&sid) else { return };
        let (world, pos) = (p.world, p.pos);
        let pk = protocol::level_sound_event("break", pos, -1);
        self.broadcast(&pk, |_, q| q.world == world);
    }

    /// Wears the held item (mining: 1 for tools, 2 for swords; hitting: 1 for swords, 2 for tools).
    pub(crate) fn wear_held_item(&mut self, sid: SessionId, attacking: bool) {
        if !self.wears(sid) {
            return;
        }
        let Some(p) = self.players.get_mut(&sid) else { return };
        let slot = (p.inventory.held_slot as usize).min(8);
        let mut item = p.inventory.main[slot].clone();
        let Some(tool) = self.gd.tool_info(item.id) else { return };
        let sword = tool.tool == ceres_world::Tool::Sword;
        let amount = if sword == attacking { 1 } else { 2 };
        let broke = wear(&mut item, amount, &self.gd);
        p.inventory.set_main(slot, item);
        p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
        if broke {
            self.broadcast_equipment(sid);
            self.play_break_sound(sid);
        }
    }

    /// Wears the held item by `amount`, whatever kind of tool it is (flint and steel, hoes...).
    pub(crate) fn wear_held(&mut self, sid: SessionId, amount: u32) {
        if !self.wears(sid) {
            return;
        }
        let Some(p) = self.players.get_mut(&sid) else { return };
        let slot = (p.inventory.held_slot as usize).min(8);
        let mut item = p.inventory.main[slot].clone();
        let broke = wear(&mut item, amount, &self.gd);
        p.inventory.set_main(slot, item);
        p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
        if broke {
            self.broadcast_equipment(sid);
            self.play_break_sound(sid);
        }
    }

    /// Each armor piece loses max(1, damage / 4) when its wearer is hurt through armor.
    pub(crate) fn wear_armor(&mut self, sid: SessionId, raw_damage: f32) {
        if !self.wears(sid) {
            return;
        }
        let amount = ((raw_damage / 4.0) as u32).max(1);
        let Some(p) = self.players.get_mut(&sid) else { return };
        let mut broke = false;
        let mut changed = false;
        for i in 0..4 {
            let mut piece = p.inventory.armor[i].clone();
            if piece.is_empty() {
                continue;
            }
            broke |= wear(&mut piece, amount, &self.gd);
            if piece.is_empty() {
                p.inventory.armor[i] = piece;
            } else {
                piece.stack_id = p.inventory.alloc_id();
                p.inventory.armor[i] = piece;
            }
            changed = true;
        }
        if !changed {
            return;
        }
        p.inventory.dirty = true;
        p.out.push(protocol::inventory_content(window::ARMOR as u32, &p.inventory.armor));
        self.broadcast_equipment(sid);
        if broke {
            self.play_break_sound(sid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durability_table() {
        assert_eq!(max_durability("minecraft:diamond_pickaxe"), Some(1561));
        assert_eq!(max_durability("minecraft:wooden_sword"), Some(59));
        assert_eq!(max_durability("minecraft:iron_chestplate"), Some(240));
        assert_eq!(max_durability("minecraft:leather_boots"), Some(65));
        assert_eq!(max_durability("minecraft:shears"), Some(238));
        assert_eq!(max_durability("minecraft:stone"), None);
        assert_eq!(max_durability("minecraft:pickaxe"), None);
    }

    #[test]
    fn damage_round_trip_keeps_tail() {
        let mut it = ItemDesc { id: 1, count: 1, extra: ceres_world::extra_data(None), ..Default::default() };
        set_item_damage(&mut it, 7);
        assert_eq!(item_damage(&it), 7);
        set_item_damage(&mut it, 9);
        assert_eq!(item_damage(&it), 9);
        assert_eq!(&it.extra[it.extra.len() - 8..], &[0u8; 8]);
    }
}
