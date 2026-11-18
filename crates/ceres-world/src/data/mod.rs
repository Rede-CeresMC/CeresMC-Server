//! Vanilla data from bedrock-data, loaded once. Static packets are encoded at boot and reused for
//! every player.

use ceres_codec::binary::{R, Res, W};
use ceres_codec::nbt::{self, Flavor, Tag};
use ceres_protocol::{id, packet};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

mod biomes;
mod voxel_shapes;

use biomes::*;
use voxel_shapes::*;

pub struct GameData {
    /// state key (see nbt::state_key) -> hashed network runtime id
    pub block_ids: HashMap<String, i32>,
    pub air_id: i32,
    pub fallback_id: i32,
    pub block_definitions: Vec<u8>,
    pub block_definition_count: u32,
    pub item_registry: Vec<u8>,
    pub actor_identifiers: Vec<u8>,
    pub biome_definitions: Vec<u8>,
    pub voxel_shapes: Vec<u8>,
    pub jigsaw_structures: Vec<u8>,
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

impl GameData {
    pub fn load(dir: &Path) -> Res<GameData> {
        // ---- block palette
        let palette = load_gz_nbt(&dir.join("block_palette.nbt"))?;
        let mut block_ids = HashMap::new();
        for b in palette.get("blocks").and_then(|t| t.as_list()).ok_or("block_palette: no blocks")? {
            let name = b.get("name").and_then(|t| t.as_str()).ok_or("block without name")?;
            let nid = b.get("network_id").and_then(|t| t.as_i64()).ok_or("block without network_id")? as i32;
            block_ids.insert(nbt::state_key(name, b.get("states")), nid);
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
        let item_registry = packet(id::ITEM_REGISTRY, |w| {
            w.uvarint(list.len() as u32);
            for it in list {
                let name = it["name"].as_str().unwrap_or("");
                w.string(name);
                w.i16_le(it["id"].as_i64().unwrap_or(0) as i16);
                w.bool(it["component_based"].as_bool().unwrap_or(false));
                w.varint(it["version"].as_i64().unwrap_or(0) as i32);
                match components.get(name) {
                    Some(c) => w.bytes(&nbt::to_network(c)),
                    None => w.bytes(&empty),
                }
            }
        });

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
            block_ids,
            air_id,
            fallback_id,
            block_definitions: bd.0,
            block_definition_count,
            item_registry,
            actor_identifiers,
            biome_definitions,
            voxel_shapes,
            jigsaw_structures,
        })
    }

    pub fn lookup_block(&self, state: &Tag) -> i32 {
        let Some(name) = state.get("name").and_then(|t| t.as_str()) else { return self.fallback_id };
        let name = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
        self.block_ids.get(&nbt::state_key(&name, state.get("states"))).copied().unwrap_or(self.fallback_id)
    }
}
