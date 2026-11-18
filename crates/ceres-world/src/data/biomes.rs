//! BiomeDefinitionList built from biome_definitions.nbt.

use super::*;

/// Mirrors BiomeDefinitionListPacket::fromDefinitions(): rebuilds the string pool in first-use order.
pub(crate) fn encode_biomes(root: &Tag) -> Res<Vec<u8>> {
    let strings_in: Vec<&str> = root
        .get("biomeStringList")
        .and_then(|t| t.as_list())
        .ok_or("biome_definitions: no biomeStringList")?
        .iter()
        .map(|t| t.as_str().unwrap_or(""))
        .collect();
    let mut pool: Vec<String> = Vec::new();
    let mut index: HashMap<String, u16> = HashMap::new();
    let mut add = |s: &str| -> u16 {
        if let Some(i) = index.get(s) {
            return *i;
        }
        let i = pool.len() as u16;
        pool.push(s.to_string());
        index.insert(s.to_string(), i);
        i
    };
    let lookup = |i: i64| -> &str { strings_in.get((i & 0xffff) as usize).copied().unwrap_or("") };

    let entries = root.get("biomeData").and_then(|t| t.as_list()).ok_or("biome_definitions: no biomeData")?;
    let mut body = W::new();
    body.uvarint(entries.len() as u32);
    for e in entries {
        let d = e.get("data").ok_or("biome without data")?;
        let f = |k: &str| d.get(k).and_then(|t| t.as_f32()).unwrap_or(0.0);
        let name_idx = add(lookup(e.get("index").and_then(|t| t.as_i64()).unwrap_or(0)));
        body.u16_le(name_idx);
        body.u16_le(d.get("id").and_then(|t| t.as_i64()).unwrap_or(0) as u16);
        body.f32_le(f("temperature"));
        body.f32_le(f("downfall"));
        body.f32_le(f("foliageSnow"));
        body.f32_le(f("depth"));
        body.f32_le(f("scale"));
        body.u32_le(d.get("mapWaterColorARGB").and_then(|t| t.as_i64()).unwrap_or(0) as u32);
        body.bool(d.get("rain").and_then(|t| t.as_i64()).unwrap_or(0) != 0);
        match d.get("tags").and_then(|t| t.get("tags")).and_then(|t| t.as_list()) {
            Some(tags) => {
                body.bool(true);
                body.uvarint(tags.len() as u32);
                for t in tags {
                    let idx = add(lookup(t.as_i64().unwrap_or(0)));
                    body.u16_le(idx);
                }
            }
            None => body.bool(false),
        }
        body.bool(false); // chunk gen data
    }
    Ok(packet(id::BIOME_DEFINITION_LIST, |w| {
        w.bytes(&body.0);
        w.uvarint(pool.len() as u32);
        for s in &pool {
            w.string(s);
        }
    }))
}
