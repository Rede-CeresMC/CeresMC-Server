//! Vanilla data from bedrock-data, loaded once. Static packets are encoded at boot and reused for
//! every player.

use ceres_codec::binary::{R, Res, W};
use ceres_codec::nbt::{self, Flavor, Tag};
use ceres_protocol::{id, packet};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::rc::Rc;

mod biomes;
mod blocks;
mod creative;
pub mod recipes;
mod mining;
pub use mining::{BreakInfo, Tool, ToolInfo};
mod voxel_shapes;

use biomes::*;
pub use blocks::BlockStateInfo;
use creative::*;
pub use creative::{CreativeEntry, extra_data};
pub use recipes::{Ingredient, Recipe};
use recipes::*;
use voxel_shapes::*;

#[derive(Clone, Debug)]
pub struct ItemInfo {
    pub name: String,
    pub id: i16,
    pub max_stack: u8,
    pub block_runtime_id: i32,
}

pub struct GameData {
    /// state key (see nbt::state_key) -> hashed network runtime id
    pub block_ids: HashMap<String, i32>,
    /// network id -> BlockStateInfo (name + states compound)
    pub block_states: HashMap<i32, BlockStateInfo>,
    /// "minecraft:stone" -> first/default block runtime id
    pub default_blocks: HashMap<String, i32>,
    /// network id -> little-endian NBT {name, states, version} as stored in worlds
    pub state_nbt: HashMap<i32, Rc<[u8]>>,
    pub air_id: i32,
    pub fallback_id: i32,
    pub items_by_name: HashMap<String, ItemInfo>,
    pub items_by_id: HashMap<i16, ItemInfo>,
    /// 0-based creative item net id -> item details
    pub creative_items: Vec<CreativeEntry>,
    /// block runtime id -> matching creative item (for BlockPickRequest and survival drops)
    pub block_to_item: HashMap<i32, CreativeEntry>,
    pub block_definitions: Vec<u8>,
    pub block_definition_count: u32,
    pub item_registry: Vec<u8>,
    pub actor_identifiers: Vec<u8>,
    pub biome_definitions: Vec<u8>,
    pub voxel_shapes: Vec<u8>,
    pub jigsaw_structures: Vec<u8>,
    pub creative_content: Vec<u8>,
    /// CraftingData packet (every recipe the client may show)
    pub crafting_data: Vec<u8>,
    /// recipe network id -> recipe, to check crafts
    pub recipes: HashMap<i32, Recipe>,
    pub item_tags: HashMap<String, std::collections::HashSet<String>>,
    /// smelting recipes: (furnace kind, input) -> outputs
    pub smelting: recipes::Smelting,
    /// brewing stand recipes
    pub brewing: recipes::Brewing,
    /// block name -> hardness, right tool and needed tier
    pub break_info: HashMap<String, BreakInfo>,
    pub creative_item_count: usize,
}

fn load_gz_nbt(path: &Path) -> Res<Tag> {
    let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut data = Vec::new();
    if raw.starts_with(&[0x1f, 0x8b]) {
        flate2::read::GzDecoder::new(&raw[..]).read_to_end(&mut data).map_err(|e| e.to_string())?;
    } else if raw.first() == Some(&0x78) {
        flate2::read::ZlibDecoder::new(&raw[..]).read_to_end(&mut data).map_err(|e| e.to_string())?;
    } else {
        data = raw;
    }
    Ok(nbt::read_root(&mut R::new(&data), Flavor::BigEndian)?.1)
}

fn infer_max_stack(name: &str, comp: Option<&Tag>) -> u8 {
    if let Some(c) = comp {
        if let Some(m) = c
            .get("components")
            .and_then(|t| t.get("item_properties"))
            .and_then(|t| t.get("max_stack_size"))
            .and_then(|t| t.as_i64())
        {
            return (m as u8).clamp(1, 64);
        }
    }
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    if short.ends_with("_sword")
        || short.ends_with("_pickaxe")
        || short.ends_with("_axe")
        || short.ends_with("_shovel")
        || short.ends_with("_hoe")
        || short.ends_with("_helmet")
        || short.ends_with("_chestplate")
        || short.ends_with("_leggings")
        || short.ends_with("_boots")
        || short.ends_with("_boat")
        || short.ends_with("_raft")
        || short.ends_with("_minecart")
        || short.ends_with("_horse_armor")
        || short.ends_with("_shulker_box")
        || short.ends_with("_bed")
        || short.ends_with("_spear")
        || short.starts_with("music_disc_")
        || matches!(
            short,
            "bow"
                | "crossbow"
                | "trident"
                | "mace"
                | "shield"
                | "shears"
                | "flint_and_steel"
                | "fishing_rod"
                | "carrot_on_a_stick"
                | "warped_fungus_on_a_stick"
                | "elytra"
                | "totem_of_undying"
                | "spyglass"
                | "brush"
                | "goat_horn"
                | "potion"
                | "splash_potion"
                | "lingering_potion"
                | "water_bucket"
                | "lava_bucket"
                | "milk_bucket"
                | "powder_snow_bucket"
                | "axolotl_bucket"
                | "cod_bucket"
                | "salmon_bucket"
                | "pufferfish_bucket"
                | "tropical_fish_bucket"
                | "tadpole_bucket"
                | "saddle"
                | "writable_book"
                | "enchanted_book"
                | "mushroom_stew"
                | "rabbit_stew"
                | "beetroot_soup"
                | "suspicious_stew"
                | "bed"
                | "shulker_box"
                | "undyed_shulker_box"
                | "minecart"
                | "cake"
        )
    {
        1
    } else if short.ends_with("_sign")
        || short.ends_with("_hanging_sign")
        || short.ends_with("_banner")
        || matches!(
            short,
            "ender_pearl"
                | "snowball"
                | "egg"
                | "blue_egg"
                | "brown_egg"
                | "bucket"
                | "honey_bottle"
                | "armor_stand"
                | "written_book"
        )
    {
        16
    } else {
        64
    }
}

impl GameData {
    pub fn load(dir: &Path) -> Res<GameData> {
        // ---- block palette
        let palette = load_gz_nbt(&dir.join("block_palette.nbt"))?;
        let mut block_ids = HashMap::new();
        let mut block_states = HashMap::new();
        let mut default_blocks = HashMap::new();
        let mut state_nbt = HashMap::new();
        for b in palette.get("blocks").and_then(|t| t.as_list()).ok_or("block_palette: no blocks")? {
            let name = b.get("name").and_then(|t| t.as_str()).ok_or("block without name")?;
            let nid = b.get("network_id").and_then(|t| t.as_i64()).ok_or("block without network_id")? as i32;
            let st = b.get("states").cloned().unwrap_or_else(Tag::empty_compound);
            block_ids.insert(nbt::state_key(name, Some(&st)), nid);
            default_blocks.entry(name.to_string()).or_insert(nid);
            block_states.insert(nid, BlockStateInfo { name: name.to_string(), states: st.clone() });
            let disk = Tag::Compound(vec![
                ("name".into(), Tag::String(name.to_string())),
                ("states".into(), st),
                ("version".into(), b.get("version").cloned().unwrap_or(Tag::Int(0))),
            ]);
            let mut w = W::new();
            nbt::write_root(&mut w, "", &disk, Flavor::LittleEndian);
            state_nbt.insert(nid, Rc::from(w.0));
        }
        let air_id = *block_ids.get("minecraft:air").ok_or("no air in palette")?;
        let fallback_id = block_ids.get("minecraft:info_update").copied().unwrap_or(air_id);

        // ---- data-driven block definitions (sent inside StartGame)
        let defs = load_gz_nbt(&dir.join("block_definitions.nbt"))?;
        let mut bd = W::new();
        let mut block_definition_count = 0;
        for b in defs.get("blocks").and_then(|t| t.as_list()).unwrap_or(&[]) {
            let name = b.get("name").and_then(|t| t.as_str()).ok_or("definition without name")?;
            let props = b.get("properties").cloned().unwrap_or_else(Tag::empty_compound);
            bd.string(name);
            bd.bytes(&nbt::to_network(&props));
            block_definition_count += 1;
        }

        // ---- items
        let items: serde_json::Value = serde_json::from_slice(
            &std::fs::read(dir.join("item_palette.json")).map_err(|e| format!("item_palette.json: {e}"))?,
        )
        .map_err(|e| e.to_string())?;
        let components = load_gz_nbt(&dir.join("item_components.nbt")).unwrap_or_else(|_| Tag::empty_compound());
        let empty = nbt::to_network(&Tag::empty_compound());
        let list = items["items"].as_array().ok_or("item_palette.json: no items")?;
        let mut items_by_name = HashMap::with_capacity(list.len());
        let mut items_by_id = HashMap::with_capacity(list.len());
        let item_registry = packet(id::ITEM_REGISTRY, |w| {
            w.uvarint(list.len() as u32);
            for it in list {
                let name = it["name"].as_str().unwrap_or("");
                let iid = it["id"].as_i64().unwrap_or(0) as i16;
                w.string(name);
                w.i16_le(iid);
                w.bool(it["component_based"].as_bool().unwrap_or(false));
                w.varint(it["version"].as_i64().unwrap_or(0) as i32);
                let comp = components.get(name);
                match comp {
                    Some(c) => w.bytes(&nbt::to_network(c)),
                    None => w.bytes(&empty),
                }
                let max_stack = infer_max_stack(name, comp);
                let block_runtime_id = default_blocks.get(name).copied().unwrap_or(0);
                let info = ItemInfo { name: name.to_string(), id: iid, max_stack, block_runtime_id };
                items_by_name.insert(name.to_string(), info.clone());
                items_by_id.insert(iid, info);
            }
        });

        // ---- creative inventory
        let item_ids: HashMap<String, i32> =
            list.iter().filter_map(|it| Some((it["name"].as_str()?.to_string(), it["id"].as_i64()? as i32))).collect();
        let (creative_content, creative_items) = encode_creative(&dir.join("creative_items.json"), &item_ids)?;
        let creative_item_count = creative_items.len();

        let mut block_to_item: HashMap<i32, CreativeEntry> = HashMap::new();
        for ce in &creative_items {
            if ce.block_runtime_id != 0 {
                block_to_item.entry(ce.block_runtime_id).or_insert_with(|| ce.clone());
                // Also refine items_by_name block_runtime_id with the canonical creative state!
                if let Some(info) = items_by_name.get_mut(&ce.name) {
                    if info.block_runtime_id == 0 || ce.meta == 0 {
                        info.block_runtime_id = ce.block_runtime_id;
                        if let Some(by_id) = items_by_id.get_mut(&info.id) {
                            by_id.block_runtime_id = ce.block_runtime_id;
                        }
                    }
                }
            }
        }
        // Map oriented block states (e.g. rotated stairs, slabs) back to their base block's item!
        for (&bid, binfo) in &block_states {
            if bid == air_id || block_to_item.contains_key(&bid) {
                continue;
            }
            let base_name = if binfo.name.ends_with("_wall_sign") {
                binfo.name.replace("_wall_sign", "_sign")
            } else if binfo.name.ends_with("_standing_sign") {
                binfo.name.replace("_standing_sign", "_sign")
            } else if binfo.name.contains("_double_slab") {
                binfo.name.replace("_double_slab", "_slab")
            } else {
                binfo.name.clone()
            };
            if let Some(&def_bid) = default_blocks.get(&base_name) {
                if let Some(ce) = block_to_item.get(&def_bid).cloned() {
                    block_to_item.insert(bid, ce);
                    continue;
                }
            }
            if let Some(info) = items_by_name.get(&base_name) {
                block_to_item.insert(
                    bid,
                    CreativeEntry {
                        name: info.name.clone(),
                        id: info.id,
                        meta: 0,
                        block_runtime_id: info.block_runtime_id,
                        extra: extra_data(None),
                    },
                );
            }
        }

        // ---- crafting
        let crafting = load_recipes(dir, &items_by_name)?;

        // ---- actor identifiers
        let actors = load_gz_nbt(&dir.join("entity_identifiers.nbt"))?;
        let actor_identifiers = packet(id::AVAILABLE_ACTOR_IDENTIFIERS, |w| w.bytes(&nbt::to_network(&actors)));

        // ---- biomes
        let biome_definitions = encode_biomes(&load_gz_nbt(&dir.join("biome_definitions.nbt"))?)?;

        // ---- voxel shapes
        let voxel_shapes = encode_voxel_shapes(&dir.join("voxel_shapes.json"))?;

        // ---- jigsaw structures (empty)
        let empty_list = || Tag::List(10, Vec::new());
        let jigsaw = Tag::Compound(vec![
            ("processors".into(), empty_list()),
            ("template_pools".into(), empty_list()),
            ("jigsaws".into(), empty_list()),
            ("structure_sets".into(), empty_list()),
        ]);
        let jigsaw_structures = packet(id::JIGSAW_STRUCTURE_DATA, |w| w.bytes(&nbt::to_network(&jigsaw)));

        Ok(GameData {
            crafting_data: crafting.packet,
            recipes: crafting.recipes,
            item_tags: crafting.item_tags,
            smelting: crafting.smelting,
            brewing: crafting.brewing,
            break_info: mining::load_break_info(dir)?,
            block_ids,
            block_states,
            default_blocks,
            state_nbt,
            air_id,
            fallback_id,
            items_by_name,
            items_by_id,
            creative_items,
            block_to_item,
            block_definitions: bd.0,
            block_definition_count,
            item_registry,
            actor_identifiers,
            biome_definitions,
            voxel_shapes,
            jigsaw_structures,
            creative_content,
            creative_item_count,
        })
    }

    pub fn lookup_block(&self, state: &Tag) -> i32 {
        let Some(name) = state.get("name").and_then(|t| t.as_str()) else { return self.fallback_id };
        let name = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
        self.block_ids.get(&nbt::state_key(&name, state.get("states"))).copied().unwrap_or(self.fallback_id)
    }

    pub fn max_stack(&self, item_id: i16) -> u8 {
        self.items_by_id.get(&item_id).map(|i| i.max_stack).unwrap_or(64)
    }

    /// Resolves an item name ("diamond_sword" or "minecraft:diamond_sword") to ItemInfo.
    pub fn find_item(&self, query: &str) -> Option<&ItemInfo> {
        let q = query.to_ascii_lowercase();
        let full = if q.contains(':') { q } else { format!("minecraft:{q}") };
        self.items_by_name.get(&full)
    }
}
