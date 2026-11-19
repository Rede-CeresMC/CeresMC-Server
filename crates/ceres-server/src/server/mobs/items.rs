//! Item knowledge the mobs need: which slot an item goes in, which of two is better, armour
//! points, a name tag's name, crop drops and blocks explosions cannot break.

use ceres_codec::binary::R;
use ceres_codec::nbt::{self, Flavor};
use ceres_protocol::ItemDesc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Slot {
    Hand,
    Offhand,
    Head,
    Chest,
    Legs,
    Feet,
}

pub(crate) fn slot_of(name: &str) -> Slot {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    if n.ends_with("_helmet") || n.ends_with("_head") || n.ends_with("_skull") || n == "carved_pumpkin" || n == "turtle_helmet" {
        Slot::Head
    } else if n.ends_with("_chestplate") || n == "elytra" {
        Slot::Chest
    } else if n.ends_with("_leggings") {
        Slot::Legs
    } else if n.ends_with("_boots") {
        Slot::Feet
    } else if n == "shield" || n == "totem_of_undying" {
        Slot::Offhand
    } else {
        Slot::Hand
    }
}

/// Rough quality of a tool or armour piece (for picking up a better one).
pub(crate) fn tier(name: &str) -> i32 {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let material = if n.starts_with("netherite") {
        7
    } else if n.starts_with("diamond") {
        6
    } else if n.starts_with("iron") {
        5
    } else if n.starts_with("chainmail") {
        4
    } else if n.starts_with("copper") {
        3
    } else if n.starts_with("golden") || n.starts_with("gold") {
        2
    } else if n.starts_with("stone") || n.starts_with("leather") {
        1
    } else if n.starts_with("wooden") {
        0
    } else {
        -1
    };
    let kind = if n.ends_with("_sword") || n.ends_with("_spear") {
        10
    } else if n.ends_with("_axe") {
        8
    } else if n.contains("_helmet") || n.contains("_chestplate") || n.contains("_leggings") || n.contains("_boots") {
        10
    } else {
        0
    };
    kind + material
}

pub(crate) fn armor_points(name: &str) -> i32 {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let (mat, piece) = match n.rsplit_once('_') {
        Some(x) => x,
        None => return 0,
    };
    let table: [i32; 4] = match mat {
        "leather" => [1, 3, 2, 1],
        "golden" => [2, 5, 3, 1],
        "chainmail" => [2, 5, 4, 1],
        "copper" => [2, 4, 3, 1],
        "iron" => [2, 6, 5, 2],
        "diamond" | "netherite" => [3, 8, 6, 3],
        "turtle" => [2, 0, 0, 0],
        _ => return 0,
    };
    match piece {
        "helmet" => table[0],
        "chestplate" => table[1],
        "leggings" => table[2],
        "boots" => table[3],
        _ => 0,
    }
}

/// Tags the item_tags file does not list but entity filters use.
pub(crate) fn implied_tag(name: &str, tag: &str) -> bool {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    match tag.strip_prefix("minecraft:").unwrap_or(tag) {
        "is_sword" => n.ends_with("_sword"),
        "is_axe" => n.ends_with("_axe"),
        "is_pickaxe" => n.ends_with("_pickaxe"),
        "is_shovel" => n.ends_with("_shovel"),
        "is_hoe" => n.ends_with("_hoe"),
        "is_spear" => n.ends_with("_spear"),
        "is_armor" => n.ends_with("_helmet") || n.ends_with("_chestplate") || n.ends_with("_leggings") || n.ends_with("_boots"),
        "is_food" | "is_meat" => matches!(n, "beef" | "porkchop" | "chicken" | "mutton" | "rabbit" | "cooked_beef" | "cooked_porkchop" | "cooked_chicken" | "cooked_mutton" | "cooked_rabbit" | "rotten_flesh"),
        "is_fish" => matches!(n, "cod" | "salmon" | "cooked_cod" | "cooked_salmon" | "tropical_fish" | "pufferfish"),
        "is_tool" => n.ends_with("_sword") || n.ends_with("_axe") || n.ends_with("_pickaxe") || n.ends_with("_shovel") || n.ends_with("_hoe"),
        "golden_tier" => n.starts_with("golden_"),
        "piglin_loved" => n.starts_with("golden_") || n.starts_with("gold") || n == "raw_gold" || n == "clock" || n == "bell",
        _ => false,
    }
}

pub(crate) fn crop_drop(block: &str) -> Option<&'static str> {
    Some(match block.strip_prefix("minecraft:").unwrap_or(block) {
        "wheat" => "minecraft:wheat",
        "carrots" => "minecraft:carrot",
        "potatoes" => "minecraft:potato",
        "beetroot" => "minecraft:beetroot",
        _ => return None,
    })
}

/// Blocks an explosion leaves alone (very blast resistant or not blocks at all).
pub(crate) fn blast_proof(block: &str) -> bool {
    matches!(
        block.strip_prefix("minecraft:").unwrap_or(block),
        "bedrock" | "obsidian" | "crying_obsidian" | "barrier" | "end_portal_frame" | "end_portal" | "portal" | "command_block"
            | "repeating_command_block" | "chain_command_block" | "structure_block" | "jigsaw" | "reinforced_deepslate" | "enchanting_table"
            | "anvil" | "ender_chest" | "respawn_anchor" | "netherite_block" | "ancient_debris" | "water" | "flowing_water" | "lava"
            | "flowing_lava" | "light_block" | "allow" | "deny" | "border_block" | "end_gateway"
    )
}

/// The name an anvil gave an item (name tags).
pub(crate) fn custom_name(item: &ItemDesc) -> Option<String> {
    if item.extra.len() <= 3 || item.extra[0] != 0xff || item.extra[1] != 0xff {
        return None;
    }
    let (_, tag) = nbt::read_root(&mut R::new(&item.extra[3..]), Flavor::LittleEndian).ok()?;
    tag.get("display").and_then(|d| d.get("Name")).and_then(|n| n.as_str()).map(|s| s.chars().take(64).collect())
}
