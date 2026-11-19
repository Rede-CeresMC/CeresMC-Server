//! Entity loot tables (`loot_tables/entities/*.json`): pools, rolls, tiers, weighted entries,
//! nested tables, the conditions mobs use (killed by a player, looting, difficulty, properties)
//! and the functions (counts, looting, smelting when on fire, damage, enchantments, colours).

use super::defs::PropVal;
use super::json::*;
use crate::server::Server;
use ceres_protocol::ItemDesc;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct LootCx<'a> {
    pub killed_by_player: bool,
    pub killed_by_pet: bool,
    pub killer: Option<String>,
    pub looting: u32,
    pub on_fire: bool,
    pub baby: bool,
    pub variant: i32,
    pub mark_variant: i32,
    pub color: u8,
    pub props: Option<&'a HashMap<String, PropVal>>,
    pub difficulty: i32,
    /// fishing luck (Luck of the Sea): entries with a quality get `quality * luck` more weight
    pub luck: f32,
}

/// Bedrock's old data values for the items that became one item per colour.
fn colored(name: &str, data: i32) -> Option<String> {
    let colour = super::WOOL.get(data.clamp(0, 15) as usize)?;
    match name {
        "minecraft:wool" => Some(format!("minecraft:{colour}_wool")),
        "minecraft:carpet" => Some(format!("minecraft:{colour}_carpet")),
        "minecraft:concrete" => Some(format!("minecraft:{colour}_concrete")),
        "minecraft:stained_glass" => Some(format!("minecraft:{colour}_stained_glass")),
        "minecraft:bed" => Some(format!("minecraft:{colour}_bed")),
        _ => None,
    }
}

fn smelted(name: &str) -> Option<&'static str> {
    Some(match name {
        "minecraft:beef" => "minecraft:cooked_beef",
        "minecraft:porkchop" => "minecraft:cooked_porkchop",
        "minecraft:chicken" => "minecraft:cooked_chicken",
        "minecraft:mutton" => "minecraft:cooked_mutton",
        "minecraft:rabbit" => "minecraft:cooked_rabbit",
        "minecraft:cod" => "minecraft:cooked_cod",
        "minecraft:salmon" => "minecraft:cooked_salmon",
        _ => return None,
    })
}

impl Server {
    /// Rolls a loot table; returns the item stacks it gives.
    pub(crate) fn roll_loot(&self, path: &str, cx: &LootCx) -> Vec<ItemDesc> {
        let mut out = Vec::new();
        self.roll_into(path, cx, &mut out, 0);
        out
    }

    fn roll_into(&self, path: &str, cx: &LootCx, out: &mut Vec<ItemDesc>, depth: u32) {
        if depth > 6 {
            return;
        }
        let path = path.replace('\\', "/");
        let Some(table) = self.mobsys.reg.loot.get(&path).or_else(|| self.mobsys.reg.loot.get(path.trim_start_matches('/'))) else {
            return;
        };
        let pools = table.get("pools").and_then(|p| p.as_array()).cloned().unwrap_or_default();
        for pool in &pools {
            if !conditions(pool.get("conditions"), cx) {
                continue;
            }
            let entries: Vec<&Value> = pool.get("entries").and_then(|e| e.as_array()).map(|a| a.iter().collect()).unwrap_or_default();
            let entries: Vec<&Value> = entries.into_iter().filter(|e| conditions(e.get("conditions"), cx)).collect();
            if entries.is_empty() {
                continue;
            }
            if let Some(tiers) = pool.get("tiers") {
                // armour tiers: start at a random tier and maybe climb
                let initial = num(tiers, "initial_range", 1.0).max(1.0) as usize;
                let bonus_rolls = num(tiers, "bonus_rolls", 0.0) as usize;
                let bonus_chance = num(tiers, "bonus_chance", 0.0);
                let mut tier = rand::random::<u32>() as usize % initial;
                for _ in 0..bonus_rolls {
                    if chance(bonus_chance) {
                        tier += 1;
                    }
                }
                if let Some(e) = entries.get(tier.min(entries.len() - 1)) {
                    self.entry(e, cx, out, depth);
                }
                continue;
            }
            let (lo, hi) = range(pool.get("rolls"), (1.0, 1.0));
            let rolls = rand_i(lo.round() as i32, hi.round() as i32).max(0);
            let weight = |e: &Value| (num(e, "weight", 1.0) + num(e, "quality", 0.0) * cx.luck).floor().max(0.0);
            let total: f32 = entries.iter().map(|e| weight(e)).sum();
            for _ in 0..rolls {
                let mut pick = rand::random::<f32>() * total;
                let mut chosen = entries[entries.len() - 1];
                for e in &entries {
                    let w = weight(e);
                    if pick < w {
                        chosen = e;
                        break;
                    }
                    pick -= w;
                }
                self.entry(chosen, cx, out, depth);
            }
        }
    }

    fn entry(&self, e: &Value, cx: &LootCx, out: &mut Vec<ItemDesc>, depth: u32) {
        match string(e, "type").unwrap_or("item") {
            "loot_table" => {
                if let Some(n) = string(e, "name") {
                    self.roll_into(n, cx, out, depth + 1);
                }
            }
            "item" => {
                let Some(raw) = string(e, "name") else { return };
                let (mut name, mut data) = item_ref(raw);
                let mut count = 1i32;
                let mut damage: Option<f32> = None;
                let mut enchant = false;
                for f in e.get("functions").and_then(|f| f.as_array()).into_iter().flatten() {
                    if !conditions(f.get("conditions"), cx) {
                        continue;
                    }
                    let fname = string(f, "function").unwrap_or("");
                    match fname.strip_prefix("minecraft:").unwrap_or(fname) {
                        "set_count" => {
                            let (a, b) = range(f.get("count"), (1.0, 1.0));
                            count = rand_i(a.round() as i32, b.round() as i32);
                        }
                        "looting_enchant" => {
                            let (a, b) = range(f.get("count"), (0.0, 1.0));
                            for _ in 0..cx.looting {
                                count += rand_i(a.round() as i32, b.round() as i32);
                            }
                        }
                        "furnace_smelt" => {
                            if cx.on_fire {
                                if let Some(s) = smelted(&name) {
                                    name = s.to_string();
                                }
                            }
                        }
                        "set_data" => data = Some(range(f.get("data"), (0.0, 0.0)).0 as i32),
                        "random_aux_value" => {
                            let (a, b) = range(f.get("values"), (0.0, 0.0));
                            data = Some(rand_i(a as i32, b as i32));
                        }
                        "set_data_from_color_index" => data = Some(cx.color as i32),
                        "set_damage" => {
                            let (a, b) = range(f.get("damage"), (1.0, 1.0));
                            damage = Some(rand_f(a, b));
                        }
                        "enchant_random_gear" => enchant |= chance(num(f, "chance", 1.0)),
                        "enchant_with_levels" | "enchant_randomly" | "specific_enchants" => enchant = true,
                        _ => {}
                    }
                }
                if count <= 0 {
                    return;
                }
                if let Some(c) = data.and_then(|d| colored(&name, d)) {
                    name = c;
                    data = None;
                }
                let Some(info) = self.gd.find_item(&name) else { return };
                let max = self.gd.max_stack(info.id).max(1) as i32;
                let mut item = ItemDesc {
                    id: info.id,
                    count: count.min(max) as u16,
                    meta: data.unwrap_or(0).max(0) as u32,
                    stack_id: 0,
                    block_runtime_id: info.block_runtime_id,
                    extra: ceres_world::extra_data(None),
                };
                if let (Some(d), Some(maxd)) = (damage, crate::server::durability::max_durability(&name)) {
                    let used = ((1.0 - d.clamp(0.0, 1.0)) * maxd as f32) as u32;
                    crate::server::durability::set_damage(&mut item, used.min(maxd.saturating_sub(1)));
                }
                if enchant {
                    let (id, lvl) = if name.contains("bow") {
                        (19, rand_i(1, 3)) // power
                    } else if name.ends_with("_sword") || name.ends_with("_axe") || name.ends_with("_spear") {
                        (9, rand_i(1, 3)) // sharpness
                    } else {
                        (0, rand_i(1, 2)) // protection
                    };
                    crate::server::durability::set_enchant(&mut item, id, lvl as i16);
                }
                out.push(item);
            }
            _ => {}
        }
    }
}

fn conditions(c: Option<&Value>, cx: &LootCx) -> bool {
    let Some(Value::Array(list)) = c else { return true };
    list.iter().all(|c| {
        let name = string(c, "condition").unwrap_or("");
        match name.strip_prefix("minecraft:").unwrap_or(name) {
            "killed_by_player" => cx.killed_by_player,
            "killed_by_player_or_pets" => cx.killed_by_player || cx.killed_by_pet,
            "killed_by_entity" | "damaged_by_entity" => {
                let want = string(c, "entity_type").unwrap_or("");
                cx.killer.as_deref() == Some(want)
            }
            "random_chance" => chance(num(c, "chance", 1.0)),
            "random_chance_with_looting" => chance(num(c, "chance", 1.0) + num(c, "looting_multiplier", 0.0) * cx.looting as f32),
            "random_difficulty_chance" => {
                let key = ["peaceful", "easy", "normal", "hard"][cx.difficulty.clamp(0, 3) as usize];
                chance(c.get(key).and_then(|v| as_num(Some(v))).unwrap_or(num(c, "default_chance", 1.0)))
            }
            "random_regional_difficulty_chance" => chance(num(c, "max_chance", 1.0) * (0.25 + 0.25 * cx.difficulty as f32)),
            "is_baby" => cx.baby,
            "has_variant" => as_num(c.get("value")).is_some_and(|v| v as i32 == cx.variant),
            "has_mark_variant" => as_num(c.get("value")).is_some_and(|v| v as i32 == cx.mark_variant),
            "entity_properties" => c.get("properties").and_then(|p| p.as_object()).is_none_or(|p| {
                p.iter().all(|(k, v)| match k.as_str() {
                    "on_fire" => v.as_bool() == Some(cx.on_fire),
                    "is_baby" => v.as_bool() == Some(cx.baby),
                    _ => cx.props.and_then(|pr| pr.get(k)).is_some_and(|pv| *pv == pv_from(v, pv)),
                })
            }),
            "bool_property" => {
                let prop = string(c, "property").or(string(c, "domain")).unwrap_or("");
                cx.props.and_then(|p| p.get(prop)).is_some_and(|v| *v == PropVal::Bool(true))
            }
            "passenger_of_entity" => false,
            "entity_killed" => true,
            _ => true,
        }
    })
}

fn pv_from(v: &Value, like: &PropVal) -> PropVal {
    match like {
        PropVal::Bool(_) => PropVal::Bool(v.as_bool().unwrap_or(false)),
        PropVal::Int(_) => PropVal::Int(as_num(Some(v)).unwrap_or(0.0) as i32),
        PropVal::Float(_) => PropVal::Float(as_num(Some(v)).unwrap_or(0.0)),
        PropVal::Enum(_) => PropVal::Enum(v.as_str().unwrap_or("").to_string()),
    }
}
