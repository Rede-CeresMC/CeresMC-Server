//! Bedrock filters (`{"test": "is_family", "subject": "other", "value": "player"}`, `all_of`,
//! `any_of`, `none_of`), evaluated against the mob and the entities around one decision.
//! Tests this server cannot answer (riding, leashes, villages...) read as false.

use super::defs::PropVal;
use super::json::*;
use super::{Ent, Env, Mob};
use crate::server::{Player, Server};
use ceres_net::transport::SessionId;
use serde_json::Value;

#[derive(Clone, Copy)]
pub(crate) enum Subj<'a> {
    P(SessionId, &'a Player),
    M(&'a Mob),
}

impl<'a> Subj<'a> {
    pub(crate) fn ent(&self) -> Ent {
        match self {
            Subj::P(s, _) => Ent::Player(*s),
            Subj::M(m) => Ent::Mob(m.entity_id),
        }
    }
    pub(crate) fn feet(&self) -> [f32; 3] {
        match self {
            Subj::P(_, p) => [p.pos[0], p.pos[1] - super::EYE_HEIGHT, p.pos[2]],
            Subj::M(m) => m.pos,
        }
    }
    pub(crate) fn world(&self) -> usize {
        match self {
            Subj::P(_, p) => p.world,
            Subj::M(m) => m.world,
        }
    }
    pub(crate) fn is_family(&self, f: &str) -> bool {
        match self {
            Subj::P(..) => matches!(f, "player" | "mob"),
            Subj::M(m) => m.t.is(f),
        }
    }
    pub(crate) fn health(&self) -> (f32, f32) {
        match self {
            Subj::P(_, p) => (p.health, 20.0),
            Subj::M(m) => (m.health, m.t.max_health),
        }
    }
    fn env<'b>(&self, s: &'b Server) -> Option<&'b Env>
    where
        'a: 'b,
    {
        match self {
            Subj::P(sid, _) => s.mobsys.penv.get(sid),
            Subj::M(_) => None,
        }
    }
    fn held_name(&self, s: &Server) -> (String, u32) {
        match self {
            Subj::P(_, p) => {
                let h = p.inventory.held_item();
                (s.gd.items_by_id.get(&h.id).map(|i| i.name.clone()).unwrap_or_default(), h.meta)
            }
            Subj::M(m) => (s.gd.items_by_id.get(&m.equip.hand.id).map(|i| i.name.clone()).unwrap_or_default(), m.equip.hand.meta),
        }
    }
    /// item names in a slot domain ("hand", "head", "torso", "leg", "feet", "armor", "offhand", "any", "inventory")
    fn equipment(&self, s: &Server, domain: &str) -> Vec<(String, u32, i16)> {
        let name = |it: &ceres_protocol::ItemDesc| (s.gd.items_by_id.get(&it.id).map(|i| i.name.clone()).unwrap_or_default(), it.meta, it.id);
        let (hand, off, armor, inv): (_, _, Vec<_>, Vec<_>) = match self {
            Subj::P(_, p) => (
                name(p.inventory.held_item()),
                name(&p.inventory.offhand),
                p.inventory.armor.iter().map(name).collect(),
                p.inventory.main.iter().map(name).collect(),
            ),
            Subj::M(m) => (name(&m.equip.hand), name(&m.equip.offhand), m.equip.armor.iter().map(name).collect(), Vec::new()),
        };
        let mut out = Vec::new();
        match domain {
            "hand" => out.push(hand),
            "offhand" => out.push(off),
            "head" => out.extend(armor.first().cloned()),
            "torso" => out.extend(armor.get(1).cloned()),
            "leg" => out.extend(armor.get(2).cloned()),
            "feet" => out.extend(armor.get(3).cloned()),
            "armor" => out.extend(armor),
            "inventory" => out.extend(inv),
            _ => {
                out.push(hand);
                out.push(off);
                out.extend(armor);
                out.extend(inv);
            }
        }
        out.retain(|(n, _, id)| !n.is_empty() && *id != 0);
        out
    }
}

pub(crate) struct Cx<'a> {
    pub s: &'a Server,
    pub me: Option<Subj<'a>>,
    pub other: Option<Subj<'a>>,
    pub target: Option<Subj<'a>>,
    pub player: Option<Subj<'a>>,
    pub parent: Option<Subj<'a>>,
    pub damager: Option<Subj<'a>>,
    pub block: Option<i32>,
    pub cause: Option<&'a str>,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(s: &'a Server, me: &'a Mob) -> Cx<'a> {
        let target = me.target.and_then(|t| s.subj(t));
        Cx { s, me: Some(Subj::M(me)), other: None, target, player: None, parent: None, damager: None, block: None, cause: None }
    }
    pub(crate) fn with_other(mut self, other: Option<Ent>) -> Self {
        self.other = other.and_then(|o| self.s.subj(o));
        if let Some(Subj::P(..)) = self.other {
            self.player = self.other;
        }
        if self.damager.is_none() {
            self.damager = self.other;
        }
        self
    }
    pub(crate) fn with_other_subj(mut self, other: Subj<'a>) -> Self {
        self.other = Some(other);
        if let Subj::P(..) = other {
            self.player = Some(other);
        }
        self
    }
}

/// True when `f` (a filter object, a list meaning all_of, or nothing) passes.
pub(crate) fn test(f: Option<&Value>, cx: &Cx) -> bool {
    match f {
        None | Some(Value::Null) => true,
        Some(Value::Array(a)) => a.iter().all(|x| test(Some(x), cx)),
        Some(Value::Object(o)) => {
            if o.is_empty() {
                return true;
            }
            let mut ok = true;
            // the older spelling: AND / OR / NOT
            if let Some(v) = o.get("AND") {
                ok &= list(v).iter().all(|x| test(Some(x), cx));
            }
            if let Some(v) = o.get("OR") {
                let l = list(v);
                ok &= l.is_empty() || l.iter().any(|x| test(Some(x), cx));
            }
            if let Some(v) = o.get("NOT") {
                ok &= !list(v).iter().any(|x| test(Some(x), cx));
            }
            if let Some(v) = o.get("all_of") {
                ok &= list(v).iter().all(|x| test(Some(x), cx));
            }
            if let Some(v) = o.get("any_of") {
                let l = list(v);
                ok &= l.is_empty() || l.iter().any(|x| test(Some(x), cx));
            }
            if let Some(v) = o.get("none_of") {
                ok &= !list(v).iter().any(|x| test(Some(x), cx));
            }
            if let Some(Value::String(t)) = o.get("test") {
                ok &= single(t, f.unwrap(), cx);
            }
            ok
        }
        _ => true,
    }
}

fn list(v: &Value) -> Vec<&Value> {
    match v {
        Value::Array(a) => a.iter().collect(),
        other => vec![other],
    }
}

enum TV {
    B(bool),
    N(f32),
}

fn compare(actual: TV, op: &str, value: Option<&Value>) -> bool {
    match actual {
        TV::B(b) => {
            let want = match value {
                Some(Value::Bool(v)) => *v,
                _ => true,
            };
            let eq = b == want;
            match op {
                "!=" | "<>" | "not" => !eq,
                _ => eq,
            }
        }
        TV::N(n) => {
            let v = as_num(value).unwrap_or(0.0);
            match op {
                "!=" | "<>" | "not" => (n - v).abs() > 1e-4,
                "<" => n < v,
                "<=" => n <= v,
                ">" => n > v,
                ">=" => n >= v,
                _ => (n - v).abs() <= 1e-4,
            }
        }
    }
}

fn single(t: &str, f: &Value, cx: &Cx) -> bool {
    // older files write the operator and subject as numbers
    let op = match f.get("operator") {
        Some(Value::Number(n)) => match n.as_i64().unwrap_or(0) {
            1 => "!=",
            2 => "<",
            3 => "<=",
            4 => ">",
            5 => ">=",
            _ => "==",
        },
        _ => string(f, "operator").unwrap_or("=="),
    };
    let value = f.get("value");
    let sv = value.and_then(|v| v.as_str()).unwrap_or("");
    let subject = match f.get("subject") {
        Some(Value::Number(n)) => match n.as_i64().unwrap_or(0) {
            1 => "other",
            2 => "parent",
            3 => "player",
            4 => "target",
            5 => "baby",
            6 => "damager",
            7 => "block",
            8 => "item",
            _ => "self",
        },
        _ => string(f, "subject").unwrap_or("self"),
    };
    let subj = match subject {
        "self" => cx.me,
        "other" => cx.other,
        "target" => cx.target,
        "player" => cx.player.or(cx.other),
        "parent" => cx.parent,
        "damager" => cx.damager.or(cx.other),
        "baby" => cx.other,
        "block" | "item" => cx.me,
        _ => cx.me,
    };
    let s = cx.s;
    let domain = string(f, "domain").unwrap_or("");
    let result: TV = match t {
        // no entity behind the damage: lightning counts as a "lightning" other
        "is_family" => TV::B(subj.is_some_and(|x| x.is_family(sv)) || (subj.is_none() && subject == "other" && cx.cause == Some("lightning") && sv == "lightning")),
        "has_component" => TV::B(match subj {
            Some(Subj::M(m)) => m.comps.contains_key(sv),
            _ => false,
        }),
        "is_baby" => TV::B(matches!(subj, Some(Subj::M(m)) if m.t.baby)),
        "has_equipment" | "has_equipment_tag" => {
            let Some(x) = subj else { return compare(TV::B(false), op, None) };
            let items = x.equipment(s, if domain.is_empty() { "any" } else { domain });
            let found = if t == "has_equipment" {
                let (want, data) = item_ref(sv);
                items.iter().any(|(n, meta, _)| *n == want && data.is_none_or(|d| d as u32 == *meta))
            } else {
                let tag = if sv.contains(':') { sv.to_string() } else { format!("minecraft:{sv}") };
                items.iter().any(|(n, _, _)| {
                    s.gd.item_tags.get(&tag).or_else(|| s.gd.item_tags.get(sv)).is_some_and(|set| set.contains(n))
                        || super::items::implied_tag(n, &tag)
                })
            };
            TV::B(found)
        }
        "all_slots_empty" => TV::B(subj.is_some_and(|x| x.equipment(s, if domain.is_empty() { "any" } else { domain }).is_empty())),
        "has_ranged_weapon" => TV::B(subj.is_some_and(|x| {
            let (n, _) = x.held_name(s);
            matches!(n.as_str(), "minecraft:bow" | "minecraft:crossbow" | "minecraft:trident")
        })),
        "is_sneak_held" | "is_sneaking" => TV::B(matches!(subj, Some(Subj::P(_, p)) if p.actor_state.sneaking)),
        "is_sprinting" => TV::B(matches!(subj, Some(Subj::P(_, p)) if p.actor_state.sprinting)),
        "has_ability" => TV::B(match subj {
            Some(Subj::P(_, p)) => match sv {
                "instabuild" | "invulnerable" | "mayfly" => p.gamemode == 1 || (sv == "invulnerable" && p.gamemode == 6),
                "flying" => p.flying,
                _ => false,
            },
            _ => false,
        }),
        "in_water" | "in_contact_with_water" => TV::B(env_of(subj, cx).is_some_and(|e| e.in_water)),
        "in_water_or_rain" => TV::B(env_of(subj, cx).is_some_and(|e| e.in_water || (e.sky && s.raining(subj.map(|x| x.world()).unwrap_or(0))))),
        "is_underwater" => TV::B(env_of(subj, cx).is_some_and(|e| e.underwater)),
        "in_lava" => TV::B(env_of(subj, cx).is_some_and(|e| e.in_lava)),
        "on_hot_block" => TV::B(env_of(subj, cx).is_some_and(|e| matches!(s.gd.block_name(e.below), "minecraft:magma" | "minecraft:campfire" | "minecraft:soul_campfire"))),
        "on_ground" => TV::B(match subj {
            Some(Subj::M(m)) => m.on_ground,
            Some(Subj::P(_, p)) => !p.flying,
            None => false,
        }),
        "on_fire" | "taking_fire_damage" => TV::B(match subj {
            Some(Subj::M(m)) => m.fire_ticks > 0,
            Some(Subj::P(_, p)) => p.fire_ticks > 0,
            None => false,
        }),
        "is_underground" => TV::B(env_of(subj, cx).is_some_and(|e| e.underground)),
        "is_snow_covered" => TV::B(env_of(subj, cx).is_some_and(|e| s.gd.biomes.get(&e.biome).is_some_and(|b| b.temperature < 0.15))),
        "is_brightness" => TV::N(env_of(subj, cx).map(|e| e.light as f32 / 15.0).unwrap_or(1.0)),
        "light_level" => TV::N(env_of(subj, cx).map(|e| e.light as f32).unwrap_or(15.0)),
        "is_temperature_value" => TV::N(env_of(subj, cx).and_then(|e| s.gd.biomes.get(&e.biome)).map(|b| b.temperature).unwrap_or(0.8)),
        "is_temperature_type" => {
            let temp = env_of(subj, cx).and_then(|e| s.gd.biomes.get(&e.biome)).map(|b| b.temperature).unwrap_or(0.8);
            let kind = if temp < 0.15 { "cold" } else if temp < 1.0 { "mild" } else if temp >= 1.5 { "warm" } else { "ocean" };
            TV::B(kind == sv)
        }
        "has_biome_tag" | "is_biome" => TV::B(env_of(subj, cx).and_then(|e| s.gd.biomes.get(&e.biome)).is_some_and(|b| {
            b.tags.iter().any(|t| t == sv) || b.name == sv || b.name.strip_prefix("minecraft:") == Some(sv)
        })),
        "is_block" | "in_block" | "is_waterlogged" => {
            let block = if t == "is_block" { cx.block.or(env_of(subj, cx).map(|e| e.below)) } else { env_of(subj, cx).map(|e| e.at) };
            let want = if sv.contains(':') { sv.to_string() } else { format!("minecraft:{sv}") };
            TV::B(block.is_some_and(|b| s.gd.block_name(b) == want || (t == "is_waterlogged" && s.gd.block_name(b).contains("water"))))
        }
        "is_daytime" => TV::B(s.is_day(subj.map(|x| x.world()).unwrap_or(0))),
        "hourly_clock_time" => TV::N(s.worlds.worlds.get(subj.map(|x| x.world()).unwrap_or(0)).map(|w| w.time.rem_euclid(24000) as f32).unwrap_or(0.0)),
        "moon_intensity" => TV::N(0.5),
        "weather" | "weather_at_position" => {
            let w = subj.map(|x| x.world()).unwrap_or(0);
            let now = match s.worlds.worlds.get(w).map(|w| w.weather) {
                Some(ceres_world::Weather::Thunder) => "thunderstorm",
                Some(ceres_world::Weather::Rain) => "rain",
                _ => "clear",
            };
            TV::B(now == sv || (sv == "precipitation" && now != "clear"))
        }
        "is_difficulty" => TV::B(["peaceful", "easy", "normal", "hard"].get(s.cfg.difficulty as usize) == Some(&sv)),
        "is_game_rule" => TV::B(match domain {
            "domobspawning" => s.cfg.spawn_monsters || s.cfg.spawn_animals,
            "mobgriefing" => s.cfg.mob_griefing,
            "dodaylightcycle" => true,
            _ => true,
        }),
        "actor_health" => TV::N(subj.map(|x| x.health().0).unwrap_or(0.0)),
        "is_missing_health" => TV::B(subj.is_some_and(|x| x.health().0 < x.health().1)),
        "has_target" => TV::B(matches!(subj, Some(Subj::M(m)) if m.target.is_some())),
        "is_target" => TV::B(matches!((cx.me, cx.other), (Some(Subj::M(m)), Some(o)) if m.target == Some(o.ent()))),
        "target_distance" => TV::N(match (subj, cx.target) {
            (Some(x), Some(t)) => dist(x.feet(), t.feet()),
            _ => f32::MAX,
        }),
        "distance_to_nearest_player" => TV::N(subj.map(|x| s.nearest_player_dist(x.world(), x.feet())).unwrap_or(f32::MAX)),
        "owner_distance" => TV::N(match subj {
            Some(Subj::M(m)) => m.owner.as_ref().and_then(|o| s.online_named(o)).map(|(_, p)| dist(m.pos, [p.pos[0], p.pos[1] - super::EYE_HEIGHT, p.pos[2]])).unwrap_or(f32::MAX),
            _ => f32::MAX,
        }),
        "home_distance" => TV::N(match subj {
            Some(Subj::M(m)) => m.home.map(|h| dist(m.pos, [h[0] as f32 + 0.5, h[1] as f32, h[2] as f32 + 0.5])).unwrap_or(f32::MAX),
            _ => f32::MAX,
        }),
        "is_owner" => TV::B(match (cx.me, cx.other) {
            (Some(Subj::M(m)), Some(Subj::P(_, p))) => m.owner.as_deref() == Some(p.name.as_str()),
            _ => false,
        }),
        "trusts" => TV::B(match (cx.me, cx.other) {
            (Some(Subj::M(m)), Some(Subj::P(_, p))) => m.owner.as_deref() == Some(p.name.as_str()) || m.trusted.contains(&p.name),
            _ => false,
        }),
        "is_tamed" => TV::B(matches!(subj, Some(Subj::M(m)) if m.t.tamed)),
        "is_sitting" => TV::B(matches!(subj, Some(Subj::M(m)) if m.sitting)),
        "is_sleeping" => TV::B(match subj {
            Some(Subj::M(m)) => m.sleeping,
            Some(Subj::P(_, p)) => p.actor_state.sleeping.is_some(),
            None => false,
        }),
        "is_persistent" => TV::B(matches!(subj, Some(Subj::M(m)) if m.persistent || m.t.persistent)),
        "has_nametag" => TV::B(matches!(subj, Some(Subj::M(m)) if m.name_tag.is_some())),
        "is_variant" => TV::N(match subj {
            Some(Subj::M(m)) => m.t.variant.unwrap_or(0) as f32,
            _ => 0.0,
        }),
        "is_mark_variant" => TV::N(match subj {
            Some(Subj::M(m)) => m.t.mark_variant.unwrap_or(0) as f32,
            _ => 0.0,
        }),
        "is_skin_id" => TV::N(match subj {
            Some(Subj::M(m)) => m.t.skin_id.unwrap_or(0) as f32,
            _ => 0.0,
        }),
        "is_color" => {
            let names = super::WOOL;
            let c = match subj {
                Some(Subj::M(m)) => m.t.color.unwrap_or(0),
                _ => 0,
            };
            TV::B(names.get(c as usize) == Some(&sv) || as_num(value).is_some_and(|n| n as u8 == c))
        }
        "is_moving" => TV::B(match subj {
            Some(Subj::M(m)) => m.walk_speed > 0.001 || m.motion.iter().any(|v| v.abs() > 0.02),
            Some(Subj::P(..)) => false,
            None => false,
        }),
        "is_panicking" => TV::B(matches!(subj, Some(Subj::M(m)) if m.panicking)),
        "is_avoiding_mobs" => TV::B(matches!(subj, Some(Subj::M(m)) if m.avoiding)),
        "is_navigating" => TV::B(matches!(subj, Some(Subj::M(m)) if m.ctl.dest.is_some())),
        "inactivity_timer" => TV::N(match subj {
            Some(Subj::M(m)) => (m.inactive_ticks / 20) as f32,
            _ => 0.0,
        }),
        "y_rotation" => TV::N(match subj {
            Some(Subj::M(m)) => m.yaw,
            Some(Subj::P(_, p)) => p.rot[1],
            None => 0.0,
        }),
        "has_mob_effect" => TV::B(match subj {
            Some(Subj::P(_, p)) => super::super::effects::effect_id(sv).is_some_and(|id| p.effects.contains_key(&id)) || (sv.is_empty() && !p.effects.is_empty()),
            Some(Subj::M(m)) => m.effects.iter().any(|(n, _)| n == sv) || (sv.is_empty() && !m.effects.is_empty()),
            None => false,
        }),
        "was_last_hurt_by" => TV::B(match (cx.me, cx.other) {
            (Some(Subj::M(m)), Some(o)) => m.last_hurt_by == Some(o.ent()),
            _ => false,
        }),
        "has_damage" => TV::B(cx.cause.is_some_and(|c| c == sv || sv == "any" || (sv == "fatal" && false))),
        "random_chance" => TV::B({
            let n = as_num(value).unwrap_or(2.0).max(1.0) as u32;
            rand::random::<u32>() % n == 0
        }),
        "enum_property" | "bool_property" | "int_property" | "float_property" | "has_property" => {
            let pv = match subj {
                Some(Subj::M(m)) => m.props.get(domain).or_else(|| if t == "has_property" { m.props.get(sv) } else { None }),
                _ => None,
            };
            match (t, pv) {
                ("has_property", p) => TV::B(p.is_some()),
                ("enum_property", Some(PropVal::Enum(e))) => TV::B(e == sv),
                ("bool_property", Some(PropVal::Bool(b))) => TV::B(*b),
                ("int_property", Some(PropVal::Int(i))) => TV::N(*i as f32),
                ("float_property", Some(PropVal::Float(f))) => TV::N(*f),
                ("int_property" | "float_property", _) => TV::N(0.0),
                _ => TV::B(false),
            }
        }
        "rider_count" => TV::N(0.0),
        "is_riding" | "is_riding_self" | "is_leashed" | "is_leashed_to" | "is_vehicle_family" | "is_controlling_passenger_family"
        | "in_caravan" | "in_nether" | "in_end" | "is_in_village" | "has_container_open" | "is_raider" | "is_bound_to_creaking_heart"
        | "has_trade_supply" | "has_silk_touch" | "has_damaged_equipment" | "has_same_equipment_in_slot_as"
        | "actor_has_item_with_enchantment_in_slot" => TV::B(false),
        "redstone_strength_at_position" => TV::N(0.0),
        "is_visible" => TV::B(true),
        "surface_mob" => TV::B(true),
        "is_altitude" => TV::N(subj.map(|x| x.feet()[1]).unwrap_or(0.0)),
        _ => TV::B(false),
    };
    compare(result, op, value)
}

fn env_of<'a>(subj: Option<Subj<'a>>, cx: &'a Cx) -> Option<&'a Env> {
    match subj? {
        Subj::M(m) => Some(&m.env),
        p @ Subj::P(..) => p.env(cx.s),
    }
}

pub(crate) fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
