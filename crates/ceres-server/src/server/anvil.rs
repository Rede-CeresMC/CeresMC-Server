//! Anvil: renaming, repairing (a second item of the same kind, or the repair material at 25% of
//! the durability per unit) and combining enchantments (books too), with the Java/Bedrock costs
//! and prior-work penalty. Using an anvil may wear it (12%): anvil -> chipped -> damaged -> gone.

use super::*;
use ceres_codec::nbt::{self, Flavor, Tag};
use ceres_protocol::ItemDesc;

pub(crate) const ANVIL_UI: u8 = 0xfb;
pub(crate) const UI_ANVIL_INPUT: u8 = 0;
pub(crate) const UI_ANVIL_MATERIAL: u8 = 1;
pub(crate) const SLOT_ANVIL_INPUT: u8 = 1;
pub(crate) const SLOT_ANVIL_MATERIAL: u8 = 2;
const TOO_EXPENSIVE: u32 = 40;
const SOUND_ANVIL_BREAK: i32 = 1020;
const SOUND_ANVIL_USE: i32 = 1021;

/// What the anvil makes: the item, the level cost, and how many of the material it uses.
pub(crate) struct AnvilResult {
    pub item: ItemDesc,
    pub cost: u32,
    pub material_used: u16,
}

fn split(item: &ItemDesc) -> (Tag, Vec<u8>) {
    if item.extra.len() > 3 && item.extra[0] == 0xff && item.extra[1] == 0xff {
        let mut r = R::new(&item.extra[3..]);
        if let Ok((_, tag)) = nbt::read_root(&mut r, Flavor::LittleEndian) {
            return (tag, r.rest().to_vec());
        }
    }
    (Tag::Compound(Vec::new()), vec![0; 8])
}

fn join(item: &mut ItemDesc, tag: &Tag, tail: &[u8]) {
    let mut w = ceres_codec::binary::W::new();
    w.u16_le(0xffff);
    w.u8(1);
    nbt::write_root(&mut w, "", tag, Flavor::LittleEndian);
    w.bytes(tail);
    item.extra = w.0;
}

fn repair_cost(item: &ItemDesc) -> u32 {
    split(item).0.get("RepairCost").and_then(|v| v.as_i64()).unwrap_or(0).max(0) as u32
}

fn custom_name(item: &ItemDesc) -> Option<String> {
    split(item).0.get("display").and_then(|d| d.get("Name")).and_then(|n| n.as_str()).map(str::to_string)
}

fn max_level(id: i16) -> i16 {
    match id {
        0..=4 | 7 => 4,
        9..=11 | 15 | 19 => 5,
        5 | 6 | 14 | 17 | 18 | 36 | 37 | 30 | 31 | 34 | 35 => 3,
        12 | 13 | 20 | 25 => 2,
        _ => 1,
    }
}

/// Cost multiplier of an enchantment (by rarity), halved when it comes from a book.
fn multiplier(id: i16, from_book: bool) -> u32 {
    let m = match id {
        0 | 9 | 15 | 19 => 1,
        1 | 2 | 4 | 10 | 11 | 12 | 17 => 2,
        3 | 6 | 8 | 13 | 14 | 18 | 20 | 21 | 26 | 7 | 25 | 31 | 32 | 33 => 4,
        _ => 8,
    };
    if from_book { (m / 2).max(1) } else { m }
}

fn conflicts(a: i16, b: i16) -> bool {
    let group = |id: i16| match id {
        0 | 1 | 3 | 4 => 1,
        9..=11 => 2,
        16 | 18 => 3,
        22 | 26 => 4, // infinity / mending
        7 | 25 => 5,  // depth strider / frost walker
        _ => 0,
    };
    a != b && group(a) != 0 && group(a) == group(b)
}

/// Whether an enchantment fits an item (books take anything).
fn applies(name: &str, id: i16) -> bool {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    if n == "book" || n == "enchanted_book" {
        return true;
    }
    let piece = n.rsplit('_').next().unwrap_or(n);
    match piece {
        "sword" => matches!(id, 9..=14 | 17 | 26 | 28),
        "axe" => matches!(id, 9..=11 | 15..=18 | 26 | 28),
        "pickaxe" | "shovel" | "hoe" => matches!(id, 15..=18 | 26 | 28),
        "bow" => matches!(id, 17 | 19..=22 | 26 | 28),
        "helmet" => matches!(id, 0..=6 | 8 | 17 | 26..=28),
        "chestplate" | "leggings" => matches!(id, 0 | 1 | 3..=5 | 17 | 26..=28 | 37),
        "boots" => matches!(id, 0..=5 | 7 | 17 | 25..=28 | 36),
        "rod" => matches!(id, 17 | 23 | 24 | 26 | 28),
        "trident" => matches!(id, 17 | 26 | 28..=32),
        "crossbow" => matches!(id, 17 | 26 | 28 | 33..=35),
        "shears" | "elytra" | "shield" => matches!(id, 17 | 26..=28),
        _ => false,
    }
}

/// The material that repairs an item.
fn repair_material(name: &str, material: &str, gd: &GameData) -> bool {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let m = material;
    let tagged = |tag: &str| gd.item_tags.get(tag).is_some_and(|s| s.contains(m));
    match n.split('_').next().unwrap_or("") {
        "wooden" => tagged("minecraft:planks") || m.ends_with("_planks"),
        "stone" => matches!(m, "minecraft:cobblestone" | "minecraft:blackstone" | "minecraft:cobbled_deepslate"),
        "iron" | "chainmail" => m == "minecraft:iron_ingot",
        "golden" => m == "minecraft:gold_ingot",
        "diamond" => m == "minecraft:diamond",
        "netherite" => m == "minecraft:netherite_ingot",
        "leather" => m == "minecraft:leather",
        "copper" => m == "minecraft:copper_ingot",
        _ => match n {
            "turtle_helmet" => m == "minecraft:turtle_scute" || m == "minecraft:scute",
            "elytra" => m == "minecraft:phantom_membrane",
            "shield" => m.ends_with("_planks"),
            "mace" => m == "minecraft:breeze_rod",
            _ => false,
        },
    }
}

/// The anvil's result for these inputs and optional new name (None: nothing to do).
pub(crate) fn combine(left: &ItemDesc, right: &ItemDesc, rename: Option<&str>, gd: &GameData) -> Option<AnvilResult> {
    if left.is_empty() {
        return None;
    }
    let left_name = gd.items_by_id.get(&left.id)?.name.clone();
    let mut out = left.clone();
    out.count = left.count;
    out.stack_id = 0;
    let mut cost = 0u32;
    let mut material_used = 0u16;
    let max = durability::max_durability(&left_name);

    if !right.is_empty() {
        let right_name = gd.items_by_id.get(&right.id)?.name.clone();
        let from_book = right_name == "minecraft:enchanted_book";
        if let (Some(max), true) = (max, repair_material(&left_name, &right_name, gd)) {
            // material: each unit repairs a quarter of the durability
            let mut damage = durability::item_damage(&out);
            if damage == 0 {
                return None;
            }
            while damage > 0 && material_used < right.count {
                damage = damage.saturating_sub(max / 4);
                material_used += 1;
                cost += 1;
            }
            durability::set_damage(&mut out, damage);
        } else if right.id == left.id || from_book {
            if right.id == left.id {
                if let Some(max) = max {
                    let left_dur = |it: &ItemDesc| max.saturating_sub(durability::item_damage(it));
                    let total = (left_dur(left) + left_dur(right) + max * 12 / 100).min(max);
                    if total > left_dur(left) {
                        durability::set_damage(&mut out, max - total);
                        cost += 2;
                    }
                }
            }
            let mut have = durability::enchantments(&out);
            let mut any = false;
            for (id, lvl) in durability::enchantments(right) {
                if !applies(&left_name, id) || have.iter().any(|(h, _)| conflicts(*h, id)) {
                    cost += 1; // vanilla charges for incompatible ones too
                    continue;
                }
                let current = have.iter().find(|(h, _)| *h == id).map(|(_, l)| *l).unwrap_or(0);
                let new = if current == lvl { (lvl + 1).min(max_level(id)) } else { lvl.max(current) };
                durability::set_enchant(&mut out, id, new);
                have.retain(|(h, _)| *h != id);
                have.push((id, new));
                cost += new as u32 * multiplier(id, from_book);
                any = true;
            }
            if !any && cost == 0 {
                return None;
            }
            material_used = 1;
        } else {
            return None;
        }
    }

    // renaming
    if let Some(name) = rename.map(str::trim).filter(|n| !n.is_empty()) {
        let name: String = name.chars().take(30).collect();
        if custom_name(&out).as_deref() != Some(name.as_str()) {
            let (mut tag, tail) = split(&out);
            let mut display = tag.get("display").cloned().unwrap_or(Tag::Compound(Vec::new()));
            display.set("Name", Tag::String(name));
            tag.set("display", display);
            join(&mut out, &tag, &tail);
            cost += 1;
        }
    }
    if cost == 0 {
        return None;
    }
    // prior work penalty, then the next one grows
    let prior = repair_cost(left) + if right.is_empty() { 0 } else { repair_cost(right) };
    cost += prior;
    let (mut tag, tail) = split(&out);
    let next = repair_cost(left).max(if right.is_empty() { 0 } else { repair_cost(right) }) * 2 + 1;
    tag.set("RepairCost", Tag::Int(next as i32));
    join(&mut out, &tag, &tail);
    Some(AnvilResult { item: out, cost, material_used })
}

pub(crate) fn too_expensive(cost: u32) -> bool {
    cost >= TOO_EXPENSIVE
}

impl Server {
    /// After a successful anvil use: pay the levels, play the sound, maybe wear the anvil.
    pub(crate) fn finish_anvil(&mut self, sid: SessionId) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(cost) = p.inventory.anvil_paid.take() else { return };
        let creative = p.gamemode == 1;
        let Some(c) = p.inventory.container.as_ref() else { return };
        let (world, pos) = (c.world, c.pos);
        if !creative {
            self.add_xp_levels(sid, -(cost as i64));
        }
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        // 12% chance the anvil wears down (not in creative)
        if !creative && rand::random::<u32>() % 100 < 12 {
            let block = self.worlds.worlds[world].get_block(pos, &self.gd);
            let next = match self.gd.block_name(block) {
                "minecraft:anvil" => Some("minecraft:chipped_anvil"),
                "minecraft:chipped_anvil" => Some("minecraft:damaged_anvil"),
                _ => None,
            };
            match next.and_then(|n| self.gd.with_name(block, n)) {
                Some(id) => {
                    self.change_block(world, pos, id);
                }
                None => {
                    let air = self.gd.air_id;
                    self.close_container(sid, true);
                    self.change_block(world, pos, air);
                    self.broadcast(&protocol::level_event(SOUND_ANVIL_BREAK, center, 0), |_, q| q.world == world);
                    return;
                }
            }
        }
        self.broadcast(&protocol::level_event(SOUND_ANVIL_USE, center, 0), |_, q| q.world == world);
    }
}
