//! What a mob is right now: its definition's components merged with the component groups it has,
//! read once (whenever the groups change) into the fields the AI uses every tick, plus its goals.

use super::json::*;
use serde_json::{Map, Value};
use std::rc::Rc;

pub(crate) type Comps = Map<String, Value>;

/// Merges the base components with the active groups (later groups win).
pub(crate) fn merge(def: &super::defs::EntityDef, groups: &[String]) -> Comps {
    let mut c = def.components.clone();
    for g in groups {
        if let Some(grp) = def.groups.get(g) {
            for (k, v) in grp {
                c.insert(k.clone(), v.clone());
            }
        }
    }
    c
}

/// Goal controls: two running goals may not share one.
pub(crate) const MOVE: u8 = 1;
pub(crate) const LOOK: u8 = 2;
pub(crate) const JUMP: u8 = 4;
pub(crate) const TARGET: u8 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum G {
    // choosing a target
    HurtByTarget,
    NearestAttackable,
    OwnerHurtByTarget,
    OwnerHurtTarget,
    DefendTrusted,
    DefendVillage,
    TargetWhenPushed,
    TargetHighestDamage,
    // staying alive
    Float,
    Panic,
    AvoidMob,
    AvoidBlock,
    FleeSun,
    Breathe,
    MoveToWater,
    MoveToLand,
    MoveToLava,
    // animals
    Tempt,
    Breed,
    FollowParent,
    FollowOwner,
    TeleportToOwner,
    StayWhileSitting,
    FollowMob,
    Beg,
    // wandering
    Stroll,
    Swim,
    Fly,
    Hover,
    StayNearHome,
    Breach,
    SlimeHop,
    // fighting
    Melee,
    Leap,
    Ranged,
    Swell,
    Ram,
    // looking
    LookAtPlayer,
    LookAtEntity,
    LookAtTarget,
    LookAround,
    // using blocks and items
    MoveToBlock,
    EatBlock,
    RaidGarden,
    Harvest,
    StompEgg,
    LayEgg,
    PickupItems,
    GiveToOwner,
    Sleep,
    // events and timers
    TimerFlag,
    SendEvent,
    Summon,
    // a pause with a pose (nap, sniff, roar, celebrate...)
    Idle,
    // the ender dragon's flight
    Dragon,
    /// riding, trading and the like: nothing on this server yet
    Passive,
}

impl G {
    pub(crate) fn controls(self) -> u8 {
        use G::*;
        match self {
            HurtByTarget | NearestAttackable | OwnerHurtByTarget | OwnerHurtTarget | DefendTrusted | DefendVillage
            | TargetWhenPushed | TargetHighestDamage => TARGET,
            Float => JUMP,
            LookAtPlayer | LookAtEntity | LookAtTarget | LookAround | Beg => LOOK,
            Leap => JUMP,
            TimerFlag | Passive => 0,
            _ => MOVE | LOOK,
        }
    }
}

pub(crate) fn goal_kind(name: &str) -> Option<G> {
    use G::*;
    let n = name.strip_prefix("minecraft:behavior.")?;
    Some(match n {
        "hurt_by_target" => HurtByTarget,
        "nearest_attackable_target" | "nearest_prioritized_attackable_target" => NearestAttackable,
        "owner_hurt_by_target" => OwnerHurtByTarget,
        "owner_hurt_target" => OwnerHurtTarget,
        "defend_trusted_target" => DefendTrusted,
        "defend_village_target" => DefendVillage,
        "target_when_pushed" => TargetWhenPushed,
        "wither_target_highest_damage" => TargetHighestDamage,
        "float" | "slime_float" => Float,
        "panic" | "scared" | "squid_flee" => Panic,
        "avoid_mob_type" => AvoidMob,
        "avoid_block" => AvoidBlock,
        "flee_sun" | "find_cover" | "move_indoors" | "hide" => FleeSun,
        "swim_up_for_breath" => Breathe,
        "move_to_water" => MoveToWater,
        "move_to_land" | "squid_out_of_water" => MoveToLand,
        "move_to_liquid" | "rise_to_liquid_level" => MoveToLava,
        "tempt" | "float_tempt" => Tempt,
        "breed" | "make_love" => Breed,
        "follow_parent" => FollowParent,
        "follow_owner" | "pet_sleep_with_owner" => FollowOwner,
        "teleport_to_owner" => TeleportToOwner,
        "stay_while_sitting" => StayWhileSitting,
        "follow_mob" | "follow_caravan" | "follow_target_captain" | "swim_with_entity" => FollowMob,
        "beg" | "look_at_trading_player" | "trade_interest" => Beg,
        "random_stroll" | "stroll_towards_village" | "move_through_village" | "move_to_village" | "explore_outskirts"
        | "mingle" | "play" | "move_to_random_block" | "random_search_and_dig" | "find_underwater_treasure" => Stroll,
        "random_swim" | "swim_wander" | "swim_idle" | "squid_idle" | "squid_dive" | "squid_move_away_from_ground" => Swim,
        "random_fly" | "float_wander" | "wither_random_attack_pos_goal" | "circle_around_anchor" => Fly,
        "random_hover" => Hover,
        "move_towards_home_restriction" | "move_towards_dwelling_restriction" | "stay_near_noteblock" => StayNearHome,
        "random_breach" => Breach,
        "slime_keep_on_jumping" | "slime_random_direction" | "jump_to_block" => SlimeHop,
        "melee_box_attack" | "melee_attack" | "ocelotattack" | "slime_attack" | "stomp_attack" | "delayed_attack"
        | "eat_mob" | "use_kinetic_weapon" | "aquatic_charge_attack" | "charge_attack" | "swoop_attack"
        | "stalk_and_pounce_on_target" | "guardian_attack" | "knockback_roar" | "move_towards_target"
        | "move_around_target" | "jump_around_target" | "hold_ground" => Melee,
        "leap_at_target" => Leap,
        "ranged_attack" | "fire_at_target" | "sonic_boom" | "charge_held_item" => Ranged,
        "swell" => Swell,
        "ram_attack" => Ram,
        "look_at_player" => LookAtPlayer,
        "look_at_entity" => LookAtEntity,
        "look_at_target" => LookAtTarget,
        "random_look_around" | "random_look_around_and_sit" => LookAround,
        "move_to_block" | "inspect_bookshelf" | "work" | "work_composter" | "ocelot_sit_on_block" | "go_and_give_items_to_noteblock"
        | "take_flower" | "offer_flower" | "go_home" | "fertilize_farm_block" => MoveToBlock,
        "eat_block" => EatBlock,
        "raid_garden" => RaidGarden,
        "harvest_farm_block" => Harvest,
        "stomp_turtle_egg" => StompEgg,
        "lay_egg" => LayEgg,
        "pickup_items" | "share_items" | "transport_items" | "take_block" => PickupItems,
        "go_and_give_items_to_owner" | "drop_item_for" => GiveToOwner,
        "sleep" | "nap" => Sleep,
        "timer_flag_1" | "timer_flag_2" | "timer_flag_3" => TimerFlag,
        "send_event" | "drink_potion" | "drink_milk" | "eat_carried_item" | "sneeze" => SendEvent,
        "summon_entity" | "skeleton_horse_trap" | "silverfish_wake_up_friends" | "silverfish_merge_with_stone" => Summon,
        "celebrate" | "celebrate_survive" | "lay_down" | "play_dead" | "roll" | "croak" | "sniff" | "dig" | "emerge" | "roar"
        | "investigate_suspicious_location" | "random_sitting" | "snacking" | "admire_item" | "barter" | "place_block" => Idle,
        n if n.starts_with("dragon") => Dragon,
        "mount_pathing" | "find_mount" | "controlled_by_player" | "player_ride_tamed" | "run_around_like_crazy"
        | "trade_with_player" | "open_door" | "restrict_open_door" | "restrict_sun" | "equip_item" => Passive,
        _ => Passive,
    })
}

#[derive(Clone, Debug)]
pub(crate) struct GoalDef {
    pub kind: G,
    pub name: String,
    pub priority: i32,
    pub p: Value,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum NavKind {
    #[default]
    Walk,
    Swim,
    Fly,
    Climb,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Traits {
    pub max_health: f32,
    pub start_health: f32,
    /// movement attribute (vanilla units)
    pub speed: f32,
    pub underwater_speed: f32,
    pub flying_speed: f32,
    pub damage: (f32, f32),
    pub attack_effect: Option<(String, f32)>,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub families: Rc<Vec<String>>,
    pub baby: bool,
    pub nav: NavKind,
    pub avoid_water: bool,
    pub can_swim: bool,
    pub walks_on_lava: bool,
    pub can_fly: bool,
    /// the ender dragon: its model faces backwards and it dies slowly
    pub dragon: bool,
    pub gravity: bool,
    pub collision: bool,
    pub breathes_air: bool,
    pub breathes_water: bool,
    pub air_ticks: i32,
    pub burns_in_daylight: bool,
    pub fire_immune: bool,
    pub knockback_resistance: f32,
    pub follow_range: f32,
    pub variant: Option<i32>,
    pub mark_variant: Option<i32>,
    pub skin_id: Option<i32>,
    pub color: Option<u8>,
    pub color2: Option<u8>,
    pub tamed: bool,
    pub saddled: bool,
    pub chested: bool,
    pub sheared: bool,
    pub charged: bool,
    pub ignited: bool,
    pub stunned: bool,
    pub captain: bool,
    pub persistent: bool,
    pub sittable: bool,
    pub goals: Vec<GoalDef>,
}

impl Traits {
    pub(crate) fn read(c: &Comps) -> Traits {
        let g = |k: &str| c.get(k);
        let has = |k: &str| c.contains_key(k);
        let empty = Value::Null;
        let health = g("minecraft:health").unwrap_or(&empty);
        let max_health = range(health.get("max"), (0.0, 0.0)).1.max(range(health.get("value"), (0.0, 0.0)).1).max(1.0);
        let start_health = range(health.get("value"), (max_health, max_health));
        let start_health = rand_f(start_health.0, start_health.1).clamp(1.0, max_health);
        let attack = g("minecraft:attack").unwrap_or(&empty);
        let damage = range(attack.get("damage"), (0.0, 0.0));
        let attack_effect = string(attack, "effect_name").map(|e| (e.to_string(), num(attack, "effect_duration", 0.0)));
        let cb = g("minecraft:collision_box").unwrap_or(&empty);
        let scale = g("minecraft:scale").map(|s| num(s, "value", 1.0)).unwrap_or(1.0).max(0.01);
        let families = g("minecraft:type_family").map(|f| str_list(f.get("family"))).unwrap_or_default();
        let nav = ["minecraft:navigation.walk", "minecraft:navigation.swim", "minecraft:navigation.fly", "minecraft:navigation.hover", "minecraft:navigation.float", "minecraft:navigation.climb", "minecraft:navigation.generic"]
            .iter()
            .find_map(|k| g(k).map(|v| (*k, v)));
        let nav_kind = match nav.map(|n| n.0) {
            Some("minecraft:navigation.swim") => NavKind::Swim,
            Some("minecraft:navigation.fly") | Some("minecraft:navigation.hover") | Some("minecraft:navigation.float") => NavKind::Fly,
            Some("minecraft:navigation.climb") => NavKind::Climb,
            Some("minecraft:navigation.generic") => {
                let v = nav.unwrap().1;
                if boolean(v, "can_walk", true) && !boolean(v, "can_swim", false) { NavKind::Walk } else if boolean(v, "can_walk", false) { NavKind::Walk } else { NavKind::Swim }
            }
            _ => NavKind::Walk,
        };
        let nav_v = nav.map(|n| n.1).unwrap_or(&empty);
        let physics = g("minecraft:physics").unwrap_or(&empty);
        let can_fly = has("minecraft:can_fly") || nav_kind == NavKind::Fly || (!boolean(physics, "has_gravity", true) && has("minecraft:movement"));
        let breath = g("minecraft:breathable").unwrap_or(&empty);
        let mut goals: Vec<GoalDef> = c
            .iter()
            .filter_map(|(k, v)| {
                let kind = goal_kind(k)?;
                Some(GoalDef { kind, name: k.clone(), priority: num(v, "priority", 10.0) as i32, p: v.clone() })
            })
            .collect();
        goals.sort_by_key(|g| g.priority);
        let flag = |k: &str| has(k);
        let int = |k: &str| g(k).map(|v| num(v, "value", 0.0) as i32);
        let dragon = g("minecraft:boss").is_some() && families.iter().any(|f| f == "dragon");
        let mut t = Traits {
            max_health,
            start_health,
            speed: g("minecraft:movement").map(|m| range(m.get("value"), (0.25, 0.25)).0).unwrap_or(0.25),
            underwater_speed: g("minecraft:underwater_movement").map(|m| num(m, "value", 0.02)).unwrap_or(0.0),
            flying_speed: g("minecraft:flying_speed").map(|m| num(m, "value", 0.1)).unwrap_or(0.0),
            damage,
            attack_effect,
            width: num(cb, "width", 0.6) * scale,
            height: num(cb, "height", 1.8) * scale,
            scale,
            families: Rc::new(families),
            baby: has("minecraft:is_baby"),
            nav: nav_kind,
            avoid_water: boolean(nav_v, "avoid_water", false),
            can_swim: boolean(nav_v, "can_swim", false) || nav_kind == NavKind::Swim,
            walks_on_lava: boolean(nav_v, "can_walk_in_lava", false) || has("minecraft:lava_movement"),
            can_fly,
            gravity: boolean(physics, "has_gravity", true) && !(can_fly && nav_kind == NavKind::Fly),
            dragon,
            collision: boolean(physics, "has_collision", true),
            breathes_air: boolean(breath, "breathes_air", true),
            breathes_water: boolean(breath, "breathes_water", false),
            air_ticks: (num(breath, "total_supply", 15.0) * 20.0) as i32,
            burns_in_daylight: has("minecraft:burns_in_daylight"),
            fire_immune: has("minecraft:fire_immune"),
            knockback_resistance: g("minecraft:knockback_resistance").map(|k| num(k, "value", 0.0)).unwrap_or(0.0),
            follow_range: g("minecraft:follow_range").map(|k| num(k, "value", 16.0).max(num(k, "max", 0.0))).unwrap_or(16.0),
            variant: int("minecraft:variant"),
            mark_variant: int("minecraft:mark_variant"),
            skin_id: int("minecraft:skin_id"),
            color: int("minecraft:color").map(|v| v.clamp(0, 15) as u8),
            color2: int("minecraft:color2").map(|v| v.clamp(0, 15) as u8),
            tamed: flag("minecraft:is_tamed"),
            saddled: flag("minecraft:is_saddled"),
            chested: flag("minecraft:is_chested"),
            sheared: flag("minecraft:is_sheared"),
            charged: flag("minecraft:is_charged"),
            ignited: flag("minecraft:is_ignited"),
            stunned: flag("minecraft:is_stunned"),
            captain: flag("minecraft:is_illager_captain"),
            persistent: flag("minecraft:persistent"),
            sittable: flag("minecraft:sittable"),
            goals,
        };
        if t.underwater_speed == 0.0 {
            t.underwater_speed = if t.nav == NavKind::Swim { t.speed.max(0.1) } else { t.speed * 0.5 };
        }
        if t.flying_speed == 0.0 {
            t.flying_speed = t.speed.max(0.1);
        }
        t
    }

    pub(crate) fn is(&self, family: &str) -> bool {
        self.families.iter().any(|f| f == family)
    }
}
