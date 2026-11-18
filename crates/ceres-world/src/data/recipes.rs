//! Crafting recipes from recipes.json and item_tags.json: the CraftingData packet the client needs
//! to show recipes, plus the server-side table used to check crafts.
//! Layout follows Altay's CraftingDataCache (null UUIDs, priority 50, always unlocked).

use super::*;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ceres_protocol::ItemDesc;
use std::collections::HashSet;

pub(crate) const CRAFTING_DATA: u32 = 0x34;
const WILDCARD_META: i32 = 0x7fff;
const UNLOCK_ALWAYS: i32 = 1;

#[derive(Clone, Debug)]
pub enum Ingredient {
    /// item name; aux = None matches any meta
    Name { name: String, aux: Option<i32> },
    Tag(String),
}

#[derive(Clone, Debug)]
pub struct Recipe {
    /// ingredients with their count, empty grid cells left out
    pub inputs: Vec<(Ingredient, u16)>,
    pub output: ItemDesc,
    /// "crafting_table", "stonecutter", "furnace"...
    pub block: String,
    /// needs the 3x3 grid of a crafting table
    pub big: bool,
}

fn ingredient_of(v: &serde_json::Value) -> Option<(Ingredient, u16)> {
    let count = v["count"].as_u64().unwrap_or(1).clamp(1, 64) as u16;
    match v["type"].as_str()? {
        "name" => {
            let aux = v["auxValue"].as_i64().unwrap_or(-1) as i32;
            let aux = if aux < 0 || aux == WILDCARD_META { None } else { Some(aux) };
            Some((Ingredient::Name { name: v["itemId"].as_str()?.to_string(), aux }, count))
        }
        "item_tag" => Some((Ingredient::Tag(v["itemTag"].as_str()?.to_string()), count)),
        _ => None,
    }
}

/// RecipeIngredient as recipes carry it since protocol 2225: a "valid" flag, the descriptor type as a
/// string, the descriptor, and the count as a varint. (Item stack requests still use the older form.)
fn write_ingredient(w: &mut W, ing: Option<&(Ingredient, u16)>) {
    match ing {
        Some((Ingredient::Name { name, aux }, count)) => {
            w.uvarint(1);
            w.string("name");
            w.string(name);
            w.varint(aux.unwrap_or(WILDCARD_META));
            w.varint(*count as i32);
        }
        Some((Ingredient::Tag(tag), count)) => {
            w.uvarint(1);
            w.string("item_tag");
            w.string(tag);
            w.varint(WILDCARD_META);
            w.varint(*count as i32);
        }
        None => {
            w.uvarint(0);
            w.varint(WILDCARD_META);
            w.varint(0);
        }
    }
}

/// ItemStack without stack id (CommonTypes::putItemStackWithoutStackId).
fn write_stack(w: &mut W, it: &ItemDesc) {
    w.varint(it.id as i32);
    w.u16_le(it.count);
    w.uvarint(it.meta);
    w.varint(it.block_runtime_id);
    w.uvarint(it.extra.len() as u32);
    w.bytes(&it.extra);
}

fn output_of(v: &serde_json::Value, items: &HashMap<String, ItemInfo>) -> Option<ItemDesc> {
    let info = items.get(v["id"].as_str()?)?;
    let nbt = v["nbt_b64"].as_str().and_then(|b| STANDARD.decode(b).ok());
    Some(ItemDesc {
        id: info.id,
        count: v["count"].as_u64().unwrap_or(1).clamp(1, 64) as u16,
        meta: v["damage"].as_u64().unwrap_or(0) as u32,
        stack_id: 0,
        block_runtime_id: info.block_runtime_id,
        extra: extra_data(nbt.as_deref()),
    })
}

fn write_unlocking(w: &mut W) {
    w.bool(true);
    w.varint(UNLOCK_ALWAYS);
    w.bool(false); // no unlocking ingredients
}

pub(crate) struct CraftingTables {
    pub(crate) packet: Vec<u8>,
    pub(crate) recipes: HashMap<i32, Recipe>,
    pub(crate) item_tags: HashMap<String, HashSet<String>>,
}

pub(crate) fn load_recipes(dir: &Path, items: &HashMap<String, ItemInfo>) -> Res<CraftingTables> {
    let tags_json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.join("item_tags.json")).map_err(|e| format!("item_tags.json: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    let mut item_tags: HashMap<String, HashSet<String>> = HashMap::new();
    if let Some(obj) = tags_json.as_object() {
        for (tag, list) in obj {
            let set = list.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
            item_tags.insert(tag.clone(), set.unwrap_or_default());
        }
    }

    let data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("recipes.json")).map_err(|e| format!("recipes.json: {e}"))?)
            .map_err(|e| e.to_string())?;

    let mut shaped = W::with_capacity(512 * 1024);
    let mut shaped_n = 0u32;
    let mut shapeless = W::with_capacity(512 * 1024);
    let mut shapeless_n = 0u32;
    let mut recipes = HashMap::new();
    let mut net_id = 0i32;

    for r in data["recipes"].as_array().ok_or("recipes.json: no recipes")? {
        let ty = r["type"].as_i64().unwrap_or(-1);
        let block = r["block"].as_str().unwrap_or("");
        let name = r["id"].as_str().unwrap_or("");
        let priority = r["priority"].as_i64().unwrap_or(0) as i32;
        if block == "deprecated" || !(ty == 0 || ty == 1 || ty == 3) {
            continue; // multi, smithing and deprecated recipes are not supported yet
        }
        let outputs: Vec<ItemDesc> =
            r["output"].as_array().map(|a| a.iter().filter_map(|o| output_of(o, items)).collect()).unwrap_or_default();
        let Some(first) = outputs.first().cloned() else { continue };

        if ty == 0 {
            // shaped: the shape rows use one character per cell, ' ' is empty
            let Some(shape) = r["shape"].as_array() else { continue };
            let rows: Vec<Vec<char>> = shape.iter().filter_map(|s| s.as_str()).map(|s| s.chars().collect()).collect();
            let height = rows.len();
            let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
            if height == 0 || width == 0 {
                continue;
            }
            let keys: HashMap<String, (Ingredient, u16)> = r["input"]
                .as_object()
                .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), ingredient_of(v)?))).collect())
                .unwrap_or_default();
            let mut cells: Vec<Option<(Ingredient, u16)>> = Vec::with_capacity(width * height);
            let mut ok = true;
            for row in &rows {
                for col in 0..width {
                    match row.get(col) {
                        None | Some(' ') => cells.push(None),
                        Some(c) => match keys.get(&c.to_string()) {
                            Some(i) => cells.push(Some(i.clone())),
                            None => ok = false,
                        },
                    }
                }
            }
            if !ok {
                continue;
            }
            net_id += 1;
            shaped.string(name);
            shaped.varint(width as i32);
            shaped.varint(height as i32);
            shaped.uvarint((width * height) as u32);
            for cell in &cells {
                write_ingredient(&mut shaped, cell.as_ref());
            }
            shaped.uvarint(outputs.len() as u32);
            for o in &outputs {
                write_stack(&mut shaped, o);
            }
            shaped.bytes(&[0u8; 16]);
            shaped.string(block);
            shaped.varint(priority);
            shaped.bool(true); // symmetric
            write_unlocking(&mut shaped);
            shaped.varint(net_id);
            shaped_n += 1;
            recipes.insert(
                net_id,
                Recipe {
                    inputs: cells.into_iter().flatten().collect(),
                    output: first,
                    block: block.to_string(),
                    big: width > 2 || height > 2,
                },
            );
        } else {
            let inputs: Vec<(Ingredient, u16)> =
                r["input"].as_array().map(|a| a.iter().filter_map(ingredient_of).collect()).unwrap_or_default();
            if inputs.is_empty() {
                continue;
            }
            net_id += 1;
            shapeless.string(name);
            shapeless.uvarint(inputs.len() as u32);
            for i in &inputs {
                write_ingredient(&mut shapeless, Some(i));
            }
            shapeless.uvarint(outputs.len() as u32);
            for o in &outputs {
                write_stack(&mut shapeless, o);
            }
            shapeless.bytes(&[0u8; 16]);
            shapeless.string(block);
            shapeless.varint(priority);
            write_unlocking(&mut shapeless);
            shapeless.varint(net_id);
            shapeless_n += 1;
            let big = inputs.iter().map(|(_, c)| *c as usize).sum::<usize>() > 4;
            recipes.insert(net_id, Recipe { inputs, output: first, block: block.to_string(), big });
        }
    }

    let id_of = |name: &str| items.get(name).map(|i| i.id as i32);
    let packet = packet(CRAFTING_DATA, |w| {
        w.uvarint(shaped_n);
        w.bytes(&shaped.0);
        w.uvarint(shapeless_n);
        w.bytes(&shapeless.0);
        for _ in 0..6 {
            w.uvarint(0); // multi, user data shapeless, chemistry x2, smithing transform, smithing trim
        }
        let potions: Vec<[i32; 6]> = data["potionMixes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        Some([
                            id_of(m["inputId"].as_str()?)?,
                            m["inputMeta"].as_i64()? as i32,
                            id_of(m["reagentId"].as_str()?)?,
                            m["reagentMeta"].as_i64()? as i32,
                            id_of(m["outputId"].as_str()?)?,
                            m["outputMeta"].as_i64()? as i32,
                        ])
                    })
                    .collect()
            })
            .unwrap_or_default();
        w.uvarint(potions.len() as u32);
        for p in &potions {
            for v in p {
                w.varint(*v);
            }
        }
        let containers: Vec<[i32; 3]> = data["containerMixes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        Some([
                            id_of(m["inputId"].as_str()?)?,
                            id_of(m["reagentId"].as_str()?)?,
                            id_of(m["outputId"].as_str()?)?,
                        ])
                    })
                    .collect()
            })
            .unwrap_or_default();
        w.uvarint(containers.len() as u32);
        for c in &containers {
            for v in c {
                w.varint(*v);
            }
        }
        w.uvarint(0); // material reducer recipes
        w.bool(true); // clean recipes
    });
    Ok(CraftingTables { packet, recipes, item_tags })
}

impl GameData {
    /// True if `item` (by name and meta) satisfies the ingredient.
    pub fn ingredient_matches(&self, ing: &Ingredient, item: &ItemDesc) -> bool {
        let Some(info) = self.items_by_id.get(&item.id) else { return false };
        match ing {
            Ingredient::Name { name, aux } => {
                *name == info.name && aux.is_none_or(|a| a as u32 == item.meta)
            }
            Ingredient::Tag(tag) => self.item_tags.get(tag).is_some_and(|set| set.contains(&info.name)),
        }
    }
}
