//! What a broken block drops: vanilla loot for the common blocks (ores give their item, stone
//! gives cobblestone, glass gives nothing...), with Silk Touch and Fortune. Blocks not listed
//! drop themselves.

const ENCHANT_FORTUNE: i16 = 18;
const ENCHANT_SILK_TOUCH: i16 = 16;

/// One possible drop: item, count range, chance in 1/1000.
struct Drop {
    item: &'static str,
    min: u32,
    max: u32,
    chance: u32,
    /// Fortune adds to the count (ores)
    fortune: bool,
}

const fn always(item: &'static str, min: u32, max: u32, fortune: bool) -> Drop {
    Drop { item, min, max, chance: 1000, fortune }
}

const fn maybe(item: &'static str, chance: u32) -> Drop {
    Drop { item, min: 1, max: 1, chance, fortune: false }
}

pub(crate) enum Loot {
    /// the block's own item
    Itself,
    /// these items
    Items(Vec<(String, u32)>),
}

/// Loot table of a block, None = drops itself.
fn table(name: &str) -> Option<Vec<Drop>> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let ore = n.strip_prefix("deepslate_").unwrap_or(n);
    Some(match ore {
        "coal_ore" => vec![always("minecraft:coal", 1, 1, true)],
        "diamond_ore" => vec![always("minecraft:diamond", 1, 1, true)],
        "emerald_ore" => vec![always("minecraft:emerald", 1, 1, true)],
        "lapis_ore" => vec![always("minecraft:lapis_lazuli", 4, 9, true)],
        "redstone_ore" | "lit_redstone_ore" => vec![always("minecraft:redstone", 4, 5, true)],
        "iron_ore" => vec![always("minecraft:raw_iron", 1, 1, true)],
        "gold_ore" => vec![always("minecraft:raw_gold", 1, 1, true)],
        "copper_ore" => vec![always("minecraft:raw_copper", 2, 5, true)],
        "quartz_ore" | "nether_quartz_ore" => vec![always("minecraft:quartz", 1, 1, true)],
        "nether_gold_ore" => vec![always("minecraft:gold_nugget", 2, 6, true)],
        _ => match n {
            "stone" => vec![always("minecraft:cobblestone", 1, 1, false)],
            "deepslate" => vec![always("minecraft:cobbled_deepslate", 1, 1, false)],
            "grass_block" | "mycelium" | "podzol" | "grass_path" | "dirt_path" | "farmland" => {
                vec![always("minecraft:dirt", 1, 1, false)]
            }
            "gravel" => vec![maybe("minecraft:flint", 100)],
            "short_grass" | "tall_grass" | "fern" | "large_fern" | "tallgrass" | "double_plant" => {
                vec![maybe("minecraft:wheat_seeds", 125)]
            }
            "glowstone" => vec![always("minecraft:glowstone_dust", 2, 4, true)],
            "sea_lantern" => vec![always("minecraft:prismarine_crystals", 2, 3, true)],
            "melon_block" => vec![always("minecraft:melon_slice", 3, 7, true)],
            "clay" => vec![always("minecraft:clay_ball", 4, 4, false)],
            "bookshelf" => vec![always("minecraft:book", 3, 3, false)],
            "snow" => vec![always("minecraft:snowball", 4, 4, false)],
            "snow_layer" => vec![always("minecraft:snowball", 1, 1, false)],
            "oak_leaves" | "dark_oak_leaves" => vec![
                maybe(if n == "oak_leaves" { "minecraft:oak_sapling" } else { "minecraft:dark_oak_sapling" }, 50),
                maybe("minecraft:stick", 20),
                maybe("minecraft:apple", 5),
            ],
            "spruce_leaves" | "birch_leaves" | "acacia_leaves" | "cherry_leaves" | "pale_oak_leaves" => {
                let sapling: &'static str = match n {
                    "spruce_leaves" => "minecraft:spruce_sapling",
                    "birch_leaves" => "minecraft:birch_sapling",
                    "acacia_leaves" => "minecraft:acacia_sapling",
                    "cherry_leaves" => "minecraft:cherry_sapling",
                    _ => "minecraft:pale_oak_sapling",
                };
                vec![maybe(sapling, 50), maybe("minecraft:stick", 20)]
            }
            "jungle_leaves" => vec![maybe("minecraft:jungle_sapling", 25), maybe("minecraft:stick", 20)],
            "mangrove_leaves" | "azalea_leaves" | "azalea_leaves_flowered" => vec![maybe("minecraft:stick", 20)],
            _ if n == "glass" || n.ends_with("_glass") || n.ends_with("glass_pane") || n == "ice" || n == "packed_ice"
                || n == "blue_ice" =>
            {
                vec![]
            }
            _ => return None,
        },
    })
}

/// Fortune: ores get a random bonus multiplier, the others up to `level` extra items.
fn fortune_count(base: u32, level: u8, ore: bool) -> u32 {
    if level == 0 {
        return base;
    }
    if ore {
        // vanilla: chance 2/(level+2) of no bonus, otherwise x(2..=level+1)
        let roll = rand::random::<u32>() % (level as u32 + 2);
        if roll < 2 { base } else { base * roll }
    } else {
        base + rand::random::<u32>() % (level as u32 + 1)
    }
}

/// Blocks shears collect whole (leaves, cobwebs, grass, vines...).
fn shears_keep(name: &str) -> bool {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    n.ends_with("_leaves")
        || n.ends_with("leaves_flowered")
        || matches!(
            n,
            "web" | "cobweb" | "short_grass" | "tall_grass" | "fern" | "large_fern" | "tallgrass" | "vine" | "dead_bush"
                | "seagrass" | "glow_lichen" | "hanging_roots" | "nether_sprouts" | "twisting_vines" | "weeping_vines"
        )
}

/// What breaking `name` with an item enchanted with these levels drops.
pub(crate) fn block_loot(name: &str, fortune: u8, silk_touch: bool, shears: bool) -> Loot {
    if shears && shears_keep(name) {
        return Loot::Itself;
    }
    let Some(table) = table(name) else { return Loot::Itself };
    if silk_touch {
        return Loot::Itself;
    }
    let ore = name.ends_with("_ore");
    let mut out = Vec::new();
    for d in table {
        // Fortune also improves the chance of rare drops (seeds, flint, saplings)
        let chance = if d.chance < 1000 { (d.chance * (1 + fortune as u32)).min(1000) } else { 1000 };
        if rand::random::<u32>() % 1000 >= chance {
            continue;
        }
        let mut count = d.min + if d.max > d.min { rand::random::<u32>() % (d.max - d.min + 1) } else { 0 };
        if d.fortune {
            count = fortune_count(count, fortune, ore);
        }
        if count > 0 {
            out.push((d.item.to_string(), count));
        }
    }
    Loot::Items(out)
}

pub(crate) fn loot_enchants(item: &ceres_protocol::ItemDesc) -> (u8, bool) {
    (
        super::blocks::enchant_level(item, ENCHANT_FORTUNE),
        super::blocks::enchant_level(item, ENCHANT_SILK_TOUCH) > 0,
    )
}
