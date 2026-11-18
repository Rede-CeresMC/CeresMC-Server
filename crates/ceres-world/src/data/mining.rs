//! How long blocks take to break: hardness (block_properties_table.json), the right tool and the
//! minimum tool tier (block_tags.json), and the tool held (item_tags.json).
//! Same formula as vanilla/PocketMine: hardness x 1.5 when the tool can harvest the block (x 5 when
//! it cannot), divided by the tool's mining speed.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    None,
    Pickaxe,
    Axe,
    Shovel,
    Hoe,
    Shears,
    Sword,
}

#[derive(Clone, Copy, Debug)]
pub struct BreakInfo {
    /// seconds of base work; negative = unbreakable
    pub hardness: f32,
    /// the tool that mines it faster
    pub tool: Tool,
    /// tier the tool needs for the block to drop anything (0 = no requirement)
    pub min_tier: u8,
    /// 1.0 for blocks light cannot pass (full opaque blocks and some others)
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ToolInfo {
    pub tool: Tool,
    /// 1 wood/gold, 2 stone/copper, 3 iron, 4 diamond, 5 netherite
    pub tier: u8,
    /// mining speed on blocks of its kind
    pub speed: f32,
}

const UNKNOWN_HARDNESS: f32 = 1.5;

const TOOL_TAGS: [(&str, Tool); 6] = [
    ("pickaxe", Tool::Pickaxe),
    ("axe", Tool::Axe),
    ("shovel", Tool::Shovel),
    ("hoe", Tool::Hoe),
    ("shears", Tool::Shears),
    ("sword", Tool::Sword),
];

fn read_json(path: &Path) -> Res<serde_json::Value> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
    serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{name}: {e}"))?).map_err(|e| format!("{name}: {e}"))
}

pub(crate) fn load_break_info(dir: &Path) -> Res<HashMap<String, BreakInfo>> {
    let props = read_json(&dir.join("block_properties_table.json"))?;
    let tags = read_json(&dir.join("block_tags.json"))?;
    let members = |tag: &str| -> Vec<String> {
        tags[tag].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    let mut out: HashMap<String, BreakInfo> = HashMap::new();
    for (name, p) in props.as_object().ok_or("block_properties_table.json: not an object")? {
        let Some(hardness) = p["hardness"].as_f64() else { continue };
        let opacity = p["opacity"].as_f64().unwrap_or(0.0) as f32;
        out.insert(name.clone(), BreakInfo { hardness: hardness as f32, tool: Tool::None, min_tier: 0, opacity });
    }
    for (tag, tool) in TOOL_TAGS {
        for name in members(&format!("minecraft:is_{tag}_item_destructible")) {
            if let Some(b) = out.get_mut(&name) {
                if b.tool == Tool::None {
                    b.tool = tool;
                }
            }
        }
    }
    // stone-like blocks drop nothing without a pickaxe; ores and the like need a better tier
    for name in members("minecraft:stone") {
        if let Some(b) = out.get_mut(&name).filter(|b| b.tool == Tool::Pickaxe) {
            b.min_tier = b.min_tier.max(1);
        }
    }
    for (tag, tier) in [("stone", 2), ("iron", 3), ("diamond", 4)] {
        for name in members(&format!("minecraft:{tag}_tier_destructible")) {
            if let Some(b) = out.get_mut(&name) {
                b.min_tier = b.min_tier.max(tier);
            }
        }
    }
    Ok(out)
}

impl GameData {
    /// Tool kind, tier and speed of an item (None for anything that is not a tool).
    pub fn tool_info(&self, item_id: i16) -> Option<ToolInfo> {
        let name = &self.items_by_id.get(&item_id)?.name;
        let has = |tag: &str| self.item_tags.get(tag).is_some_and(|s| s.contains(name));
        let tool = TOOL_TAGS.iter().find(|(t, _)| has(&format!("minecraft:is_{t}"))).map(|(_, t)| *t)?;
        let (tier, speed) = if has("minecraft:netherite_tier") {
            (5, 9.0)
        } else if has("minecraft:diamond_tier") {
            (4, 8.0)
        } else if has("minecraft:iron_tier") {
            (3, 6.0)
        } else if has("minecraft:copper_tier") {
            (2, 5.0)
        } else if has("minecraft:stone_tier") {
            (2, 4.0)
        } else if has("minecraft:golden_tier") {
            (1, 12.0)
        } else if has("minecraft:wooden_tier") {
            (1, 2.0)
        } else {
            (1, 1.5) // shears and other untiered tools
        };
        Some(ToolInfo { tool, tier, speed })
    }

    /// Break info of a block state (None when unknown).
    pub fn break_info(&self, block: i32) -> Option<BreakInfo> {
        self.break_info.get(self.block_name(block)).copied()
    }

    /// A full opaque block (stone, dirt, planks...): it keeps a chest lid shut.
    pub fn is_opaque_cube(&self, block: i32) -> bool {
        let name = self.block_name(block);
        self.break_info(block).is_some_and(|b| b.opacity >= 1.0) && !name.ends_with("_slab")
    }

    /// Whether the held item lets the block drop anything.
    pub fn can_harvest(&self, block: i32, held_id: i16) -> bool {
        let Some(b) = self.break_info(block) else { return true };
        if b.min_tier == 0 {
            return true;
        }
        self.tool_info(held_id).is_some_and(|t| t.tool == b.tool && t.tier >= b.min_tier)
    }

    /// Seconds a survival player needs to break `block` with `held_id` (with `efficiency` levels).
    /// None = unbreakable.
    pub fn break_time(&self, block: i32, held_id: i16, efficiency: u8) -> Option<f32> {
        // every palette block has an entry; anything else is treated as a plain 1.5-hardness block
        // so an unexpected id can never be broken instantly
        let Some(b) = self.break_info(block) else { return Some(UNKNOWN_HARDNESS * 1.5) };
        if b.hardness < 0.0 {
            return None;
        }
        if b.hardness == 0.0 {
            return Some(0.0);
        }
        let mut speed = 1.0;
        if let Some(t) = self.tool_info(held_id).filter(|t| t.tool == b.tool && b.tool != Tool::None) {
            speed = t.speed;
            if efficiency > 0 {
                speed += (efficiency as f32).powi(2) + 1.0;
            }
        }
        let base = if self.can_harvest(block, held_id) { 1.5 } else { 5.0 };
        Some(b.hardness * base / speed)
    }
}
