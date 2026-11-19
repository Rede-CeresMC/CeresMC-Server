//! Behaviour pack data: entity definitions (components, component groups, events, properties),
//! loot tables, spawn rules and trade tables. The vanilla pack ships in `data/behavior/`; packs
//! put in `behavior_packs/` (folders) are loaded after it and replace entities, loot tables and
//! spawn rules with the same identifier/path.

use super::json::*;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

pub(crate) const VANILLA_DIR: &str = "data/behavior";
pub(crate) const PACKS_DIR: &str = "behavior_packs";

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PropKind {
    Int(i32, i32),
    Float(f32, f32),
    Bool,
    Enum(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PropVal {
    Int(i32),
    Float(f32),
    Bool(bool),
    Enum(String),
}

impl PropVal {
    pub(crate) fn to_json(&self) -> Value {
        match self {
            PropVal::Int(i) => Value::from(*i),
            PropVal::Float(f) => Value::from(*f),
            PropVal::Bool(b) => Value::from(*b),
            PropVal::Enum(s) => Value::from(s.clone()),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PropDef {
    pub name: String,
    pub kind: PropKind,
    pub default: PropVal,
}

impl PropDef {
    /// A value for this property from JSON (an event's set_property, or a save file).
    pub(crate) fn parse(&self, v: &Value, query: &impl Fn(&str, &[f32]) -> f32) -> Option<PropVal> {
        Some(match &self.kind {
            PropKind::Int(lo, hi) => PropVal::Int((super::molang::value(Some(v), 0.0, query).round() as i32).clamp(*lo, *hi)),
            PropKind::Float(lo, hi) => PropVal::Float(super::molang::value(Some(v), 0.0, query).clamp(*lo, *hi)),
            PropKind::Bool => match v {
                Value::Bool(b) => PropVal::Bool(*b),
                other => PropVal::Bool(super::molang::value(Some(other), 0.0, query) != 0.0),
            },
            PropKind::Enum(values) => {
                let s = v.as_str()?.trim_matches('\'').to_string();
                if !values.contains(&s) {
                    return None;
                }
                PropVal::Enum(s)
            }
        })
    }
}

#[derive(Default)]
pub(crate) struct EntityDef {
    pub id: String,
    pub spawnable: bool,
    pub summonable: bool,
    pub components: Map<String, Value>,
    pub groups: HashMap<String, Map<String, Value>>,
    pub events: HashMap<String, Value>,
    pub properties: Vec<PropDef>,
}

impl EntityDef {
    fn parse(v: &Value) -> Option<EntityDef> {
        let e = v.get("minecraft:entity")?;
        let d = e.get("description")?;
        let id = d.get("identifier")?.as_str()?.to_string();
        let obj = |v: Option<&Value>| v.and_then(|v| v.as_object()).cloned().unwrap_or_default();
        let mut properties = Vec::new();
        if let Some(props) = d.get("properties").and_then(|p| p.as_object()) {
            for (name, p) in props {
                let kind = match string(p, "type").unwrap_or("int") {
                    "bool" => PropKind::Bool,
                    "float" => {
                        let (lo, hi) = range(p.get("range"), (f32::MIN, f32::MAX));
                        PropKind::Float(lo, hi)
                    }
                    "enum" => PropKind::Enum(str_list(p.get("values"))),
                    _ => {
                        let (lo, hi) = range(p.get("range"), (i32::MIN as f32, i32::MAX as f32));
                        PropKind::Int(lo as i32, hi as i32)
                    }
                };
                let default = match (&kind, p.get("default")) {
                    (PropKind::Bool, Some(Value::Bool(b))) => PropVal::Bool(*b),
                    (PropKind::Bool, _) => PropVal::Bool(false),
                    (PropKind::Enum(vals), Some(Value::String(s))) if vals.contains(s) => PropVal::Enum(s.clone()),
                    (PropKind::Enum(vals), _) => PropVal::Enum(vals.first().cloned().unwrap_or_default()),
                    (PropKind::Int(lo, hi), dv) => PropVal::Int((as_num(dv).unwrap_or(*lo as f32) as i32).clamp(*lo, *hi)),
                    (PropKind::Float(lo, hi), dv) => PropVal::Float(as_num(dv).unwrap_or(0.0).clamp(*lo, *hi)),
                };
                properties.push(PropDef { name: name.clone(), kind, default });
            }
        }
        Some(EntityDef {
            spawnable: boolean(d, "is_spawnable", false),
            summonable: boolean(d, "is_summonable", false),
            components: obj(e.get("components")),
            groups: obj(e.get("component_groups"))
                .into_iter()
                .filter_map(|(k, v)| v.as_object().map(|o| (k, o.clone())))
                .collect(),
            events: obj(e.get("events")).into_iter().collect(),
            properties,
            id,
        })
    }

    /// A living entity (health somewhere in its file), not a projectile, vehicle or the player.
    pub(crate) fn is_mob(&self) -> bool {
        self.id != "minecraft:player"
            && !self.components.contains_key("minecraft:projectile")
            && (self.components.contains_key("minecraft:health")
                || self.groups.values().any(|g| g.contains_key("minecraft:health"))
                || self.is_vehicle()
                // primed TNT: no health, only a lit fuse
                || self.components.contains_key("minecraft:explode"))
    }

    /// Boats and minecarts: rideable things with physics but no health.
    pub(crate) fn is_vehicle(&self) -> bool {
        self.components.contains_key("minecraft:rideable") && self.components.contains_key("minecraft:physics") && !self.components.contains_key("minecraft:health")
    }

    pub(crate) fn prop(&self, name: &str) -> Option<&PropDef> {
        self.properties.iter().find(|p| p.name == name)
    }
}

/// One `conditions` entry of a spawn rule.
pub(crate) struct SpawnCondition {
    pub c: Map<String, Value>,
    pub weight: f32,
}

pub(crate) struct SpawnRule {
    pub id: String,
    pub population: String,
    pub conditions: Vec<SpawnCondition>,
}

#[derive(Default)]
pub(crate) struct Registry {
    pub entities: HashMap<String, Rc<EntityDef>>,
    /// by path, e.g. "loot_tables/entities/cow.json"
    pub loot: HashMap<String, Value>,
    pub spawn_rules: Vec<SpawnRule>,
    /// by path, e.g. "trading/economy_trades/armorer_trades.json"
    pub trading: HashMap<String, Value>,
}

impl Registry {
    pub(crate) fn load() -> Registry {
        let mut r = Registry::default();
        let mut bad = 0;
        let mut dirs = vec![Path::new(VANILLA_DIR).to_path_buf()];
        if let Ok(rd) = std::fs::read_dir(PACKS_DIR) {
            let mut extra: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            extra.sort();
            dirs.extend(extra);
        }
        for dir in &dirs {
            for (rel, v) in read_tree(dir, "entities", &mut bad) {
                if let Some(d) = EntityDef::parse(&v) {
                    r.entities.insert(d.id.clone(), Rc::new(d));
                } else {
                    let _ = rel;
                    bad += 1;
                }
            }
            for (rel, v) in read_tree(dir, "loot_tables", &mut bad) {
                r.loot.insert(rel, v);
            }
            for (rel, v) in read_tree(dir, "trading", &mut bad) {
                r.trading.insert(rel, v);
            }
            for (_, v) in read_tree(dir, "spawn_rules", &mut bad) {
                let Some(s) = v.get("minecraft:spawn_rules") else {
                    bad += 1;
                    continue;
                };
                let Some(id) = s.get("description").and_then(|d| string(d, "identifier")) else { continue };
                let population = s.get("description").and_then(|d| string(d, "population_control")).unwrap_or("animal").to_string();
                let conditions = s
                    .get("conditions")
                    .and_then(|c| c.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| c.as_object().cloned())
                            .map(|c| {
                                let weight = c.get("minecraft:weight").map(|w| num(w, "default", 100.0)).unwrap_or(100.0);
                                SpawnCondition { c, weight }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                r.spawn_rules.retain(|x| x.id != id);
                r.spawn_rules.push(SpawnRule { id: id.to_string(), population, conditions });
            }
        }
        if r.entities.is_empty() {
            eprintln!("[Mobs] no entity definitions found in {VANILLA_DIR}/entities: mobs are disabled");
        } else {
            println!(
                "[Mobs] {} entity definitions, {} loot tables, {} spawn rules loaded{}",
                r.entities.len(),
                r.loot.len(),
                r.spawn_rules.len(),
                if bad > 0 { format!(" ({bad} files could not be read)") } else { String::new() }
            );
        }
        r
    }

    pub(crate) fn entity(&self, name: &str) -> Option<&Rc<EntityDef>> {
        if let Some(d) = self.entities.get(name) {
            return Some(d);
        }
        // older names the files still use
        let name = match name.strip_prefix("minecraft:").unwrap_or(name) {
            "pig_zombie" => "minecraft:zombie_pigman",
            "villager" if !self.entities.contains_key("minecraft:villager") => "minecraft:villager_v2",
            "zombie_villager" if !self.entities.contains_key("minecraft:zombie_villager") => "minecraft:zombie_villager_v2",
            "evoker" => "minecraft:evocation_illager",
            "evocation_fang" | "evoker_fangs" => "minecraft:evocation_fang",
            _ => name,
        };
        if let Some(d) = self.entities.get(name) {
            return Some(d);
        }
        let full = if name.contains(':') { name.to_ascii_lowercase() } else { format!("minecraft:{}", name.to_ascii_lowercase()) };
        self.entities.get(&full)
    }
}

/// Every .json under `<dir>/<sub>`, keyed by its path relative to `dir` with forward slashes.
fn read_tree(dir: &Path, sub: &str, bad: &mut usize) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.join(sub)];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")) {
                let rel = p.strip_prefix(dir).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                match std::fs::read_to_string(&p).ok().and_then(|t| parse_lenient(&t)) {
                    Some(v) => out.push((rel, v)),
                    None => {
                        eprintln!("[Mobs] could not read {}", p.display());
                        *bad += 1;
                    }
                }
            }
        }
    }
    out
}
