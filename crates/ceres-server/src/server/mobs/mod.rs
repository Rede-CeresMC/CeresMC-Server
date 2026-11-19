//! Mobs driven by the vanilla behaviour pack (`data/behavior/`, plus any pack in
//! `behavior_packs/`): every entity file is read as Bedrock does it, its components and component
//! groups merged, its events (add/remove groups, randomize, sequence, triggers, properties) run,
//! and its `minecraft:behavior.*` goals executed by priority with move/look/jump/target controls.
//!
//! On top of that: gravity, block collision, stepping and jumping, A* paths for walkers, swimming
//! and flying, melee and ranged attacks (data-driven projectiles), creeper fuses and explosions,
//! breeding, babies growing up, taming, sitting, tempting, panicking, burning in daylight,
//! drowning, sensors (environment, entity, target nearby, damage), timers, transformations,
//! loot tables, equipment tables, experience, natural spawning from the spawn rules (biome tags,
//! light approximated without a light engine, herds, density limits) and despawning.
//! `/summon <entity> [x y z] [event]` creates any entity of the pack. Mobs are saved in
//! `mobs.json` in each world's folder.

mod defs;
mod events;
pub(crate) mod fight;
pub(crate) use filters::dist as filters_dist;
mod filters;
mod goals;
pub(crate) mod items;
mod json;
pub(crate) mod loot;
mod molang;
mod nav;
mod ride;
mod spawn;
mod traits;
mod vehicles;

use super::*;
use ceres_protocol::{ItemDesc, Meta, actor_event, meta_key};
pub(crate) use defs::Registry;
use defs::{EntityDef, PropVal};
use filters::Subj;
use std::rc::Rc;
use traits::{Comps, GoalDef, Traits};

pub(crate) const EYE_HEIGHT: f32 = 1.62;

/// Debug lines for the mob AI (CERES_MOBDEBUG=1).
pub(crate) fn dbg(msg: impl FnOnce() -> String) {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var("CERES_MOBDEBUG").is_ok()) {
        eprintln!("[MobDbg] {}", msg());
    }
}
const MAX_MOBS: usize = 400;
/// mobs farther than this from every player stop thinking (vanilla simulation distance)
const ACTIVE_RANGE: f32 = 96.0;

pub(crate) const WOOL: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue",
    "brown", "green", "red", "black",
];

/// Something a mob can see, target or be hurt by.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum Ent {
    Player(SessionId),
    Mob(i64),
}

/// Where an entity stands, refreshed every few ticks.
#[derive(Default, Clone, Debug)]
pub(crate) struct Env {
    pub in_water: bool,
    pub in_lava: bool,
    pub underwater: bool,
    /// nothing above but air (approximated sky light)
    pub sky: bool,
    pub underground: bool,
    pub light: u8,
    pub biome: i32,
    /// block at the feet and under them
    pub at: i32,
    pub below: i32,
}

#[derive(Default, Clone)]
pub(crate) struct Equip {
    pub hand: ItemDesc,
    pub offhand: ItemDesc,
    pub armor: [ItemDesc; 4],
    /// hand, offhand, head, chest, legs, feet
    pub drop_chance: [f32; 6],
}

/// What the goals ask the body to do this tick.
#[derive(Default, Clone, Debug)]
pub(crate) struct Ctl {
    pub dest: Option<[f32; 3]>,
    pub speed: f32,
    pub look: Option<[f32; 3]>,
    pub jump: bool,
    /// stand still (sitting, eating, aiming...)
    pub hold: bool,
}

#[derive(Default, Clone, Debug)]
pub(crate) struct GoalSt {
    pub running: bool,
    pub t: u32,
    pub n: u32,
    pub dest: Option<[f32; 3]>,
    pub ent: Option<Ent>,
    /// earliest tick it may start again
    pub next: u64,
    pub phase: u8,
    pub f: f32,
    pub block: Option<[i32; 3]>,
}

#[derive(Clone, Debug)]
pub(crate) struct GoalRt {
    pub def: GoalDef,
    pub st: GoalSt,
}

#[derive(Default)]
pub(crate) struct Mob {
    pub entity_id: i64,
    pub def: Rc<EntityDef>,
    pub world: usize,
    pub pos: [f32; 3],
    pub motion: [f32; 3],
    pub yaw: f32,
    pub head_yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub on_ground: bool,
    pub groups: Vec<String>,
    pub comps: Rc<Comps>,
    pub t: Rc<Traits>,
    pub props: HashMap<String, PropVal>,
    pub goals: Vec<GoalRt>,
    pub env: Env,
    pub ctl: Ctl,
    pub walk_speed: f32,
    pub path: Vec<[i32; 3]>,
    pub path_dest: Option<[i32; 3]>,
    pub path_tick: u64,
    pub stuck: u32,
    pub climbing: bool,
    pub target: Option<Ent>,
    pub last_target: Option<Ent>,
    pub last_hurt_by: Option<Ent>,
    pub last_hurt_tick: u64,
    pub last_cause: String,
    pub last_hit_by_player: u64,
    pub owner: Option<String>,
    pub trusted: Vec<String>,
    pub love: u32,
    pub love_cooldown: u32,
    pub grow_ticks: u32,
    pub air: i32,
    pub fire_ticks: u32,
    pub fall: f32,
    pub hurt_cooldown: u32,
    pub attack_cooldown: u32,
    pub death_ticks: Option<u32>,
    /// tick the entity is removed at the end of this tick (despawned, transformed, exploded)
    pub gone: bool,
    /// component-driven timers: component name -> due tick
    pub timers: HashMap<String, u64>,
    pub spawn_due: Vec<u64>,
    pub transform_due: Option<u64>,
    pub angry_until: Option<u64>,
    pub fuse: Option<u32>,
    pub sitting: bool,
    pub sleeping: bool,
    pub panicking: bool,
    pub avoiding: bool,
    pub interested: bool,
    pub charging: bool,
    pub persistent: bool,
    /// variant given by the item that placed it (boat wood)
    pub item_variant: Option<i32>,
    pub name_tag: Option<String>,
    pub equip: Equip,
    pub carried_block: Option<i32>,
    /// armor stand pose (0-12)
    pub pose: i32,
    /// players riding it
    pub riders: Vec<SessionId>,
    /// player holding its lead
    pub leash: Option<SessionId>,
    /// fence its lead is tied to (a leash knot there)
    pub knot: Option<[i32; 3]>,
    pub home: Option<[i32; 3]>,
    /// mob effects on this mob: name, ticks left
    pub effects: Vec<(String, u32)>,
    pub inactive_ticks: u32,
    pub interact_ready: u64,
    pub barter_due: Option<u64>,
    pub age: u64,
    pub ambient_due: u64,
    /// last state of the target_nearby_sensor (0 none, 1 inside, 2 outside) and entity_sensor
    pub nearby_state: u8,
    pub sensor_state: Vec<bool>,
    pub sent_pos: [f32; 3],
    pub sent_yaw: f32,
    pub sent_head: f32,
    pub sent_meta: Vec<Meta>,
    pub sent_equip: Vec<i16>,
}

impl Mob {
    pub(crate) fn alive(&self) -> bool {
        self.entity_id != 0 && self.death_ticks.is_none() && !self.gone
    }
    pub(crate) fn id(&self) -> &str {
        &self.def.id
    }
    pub(crate) fn eye(&self) -> [f32; 3] {
        [self.pos[0], self.pos[1] + self.t.height * 0.85, self.pos[2]]
    }
    pub(crate) fn center(&self) -> [f32; 3] {
        [self.pos[0], self.pos[1] + self.t.height * 0.5, self.pos[2]]
    }
    pub(crate) fn hostile(&self) -> bool {
        self.t.is("monster")
    }
}

/// Mob state kept on the server between ticks.
#[derive(Default)]
pub(crate) struct MobSys {
    pub reg: Rc<Registry>,
    /// entity id -> index in `Server::mobs`
    pub index: HashMap<i64, usize>,
    pub penv: HashMap<SessionId, Env>,
    /// events for other mobs, run after the tick: (entity id, event, other)
    pub queued: Vec<(i64, String, Option<Ent>)>,
    /// paths that may still be computed this tick
    pub paths_left: u32,
    /// who last hurt a player, and when (for wolves defending their owner)
    pub player_hurt_by: HashMap<SessionId, (Ent, u64)>,
    /// what a player last attacked, and when
    pub player_attacked: HashMap<SessionId, (Ent, u64)>,
    /// leash knots on fences: (world, fence) -> knot entity id
    pub knots: HashMap<(usize, [i32; 3]), i64>,
    /// villagers attacked: (world, where, attacker, tick)
    pub villager_attacks: Vec<(usize, [f32; 3], Ent, u64)>,
    /// block runtime id -> nav class bits
    pub block_classes: HashMap<i32, u8>,
}

/// Portuguese names for the death messages.
fn display_name(id: &str) -> String {
    let n = json::short(id);
    let pt = match n {
        "zombie" => "um zumbi",
        "husk" => "um zumbi do deserto",
        "drowned" => "um afogado",
        "zombie_villager" | "zombie_villager_v2" => "um aldeão zumbi",
        "skeleton" => "um esqueleto",
        "stray" => "um esqueleto errante",
        "bogged" => "um esqueleto do pântano",
        "wither_skeleton" => "um esqueleto wither",
        "creeper" => "um creeper",
        "spider" => "uma aranha",
        "cave_spider" => "uma aranha de caverna",
        "enderman" => "um enderman",
        "endermite" => "um endermite",
        "silverfish" => "uma traça",
        "slime" => "um slime",
        "magma_cube" => "um cubo de magma",
        "witch" => "uma bruxa",
        "blaze" => "um blaze",
        "ghast" => "um ghast",
        "phantom" => "um phantom",
        "guardian" => "um guardião",
        "elder_guardian" => "um guardião ancião",
        "pillager" => "um saqueador",
        "vindicator" => "um vingador",
        "evocation_illager" => "um evocador",
        "vex" => "um vex",
        "ravager" => "um devastador",
        "piglin" => "um piglin",
        "piglin_brute" => "um piglin bruto",
        "zombie_pigman" => "um piglin zumbi",
        "hoglin" => "um hoglin",
        "zoglin" => "um zoglin",
        "wolf" => "um lobo",
        "iron_golem" => "um golem de ferro",
        "polar_bear" => "um urso polar",
        "bee" => "uma abelha",
        "llama" | "trader_llama" => "uma lhama",
        "goat" => "uma cabra",
        "panda" => "um panda",
        "dolphin" => "um golfinho",
        "pufferfish" => "um baiacu",
        "warden" => "o warden",
        "wither" => "o wither",
        "ender_dragon" => "o dragão do fim",
        "shulker" => "um shulker",
        "breeze" => "uma breeze",
        "creaking" => "um rangedor",
        "fox" => "uma raposa",
        "cat" => "um gato",
        "ocelot" => "uma jaguatirica",
        _ => "",
    };
    if pt.is_empty() { format!("um(a) {}", n.replace('_', " ")) } else { pt.to_string() }
}

impl Server {
    // ---------------------------------------------------------------- lookups

    pub(crate) fn subj(&self, e: Ent) -> Option<Subj<'_>> {
        match e {
            Ent::Player(sid) => self.players.get(&sid).filter(|p| p.state == State::Spawned && !p.dead).map(|p| Subj::P(sid, p)),
            Ent::Mob(eid) => self.mob_ref(eid).map(Subj::M),
        }
    }

    pub(crate) fn mob_ref(&self, eid: i64) -> Option<&Mob> {
        let i = *self.mobsys.index.get(&eid)?;
        self.mobs.get(i).filter(|m| m.entity_id == eid && m.alive())
    }

    fn mob_mut(&mut self, eid: i64) -> Option<&mut Mob> {
        let i = *self.mobsys.index.get(&eid)?;
        self.mobs.get_mut(i).filter(|m| m.entity_id == eid && m.alive())
    }

    fn reindex(&mut self) {
        self.mobsys.index.clear();
        for (i, m) in self.mobs.iter().enumerate() {
            if m.entity_id != 0 {
                self.mobsys.index.insert(m.entity_id, i);
            }
        }
    }

    /// Runs `f` on the mob with that id, taken out of the list (so `f` may use the whole server).
    pub(crate) fn with_mob<R>(&mut self, eid: i64, f: impl FnOnce(&mut Server, &mut Mob) -> R) -> Option<R> {
        let i = *self.mobsys.index.get(&eid)?;
        if self.mobs.get(i).is_none_or(|m| m.entity_id != eid) {
            return None;
        }
        let mut m = std::mem::take(&mut self.mobs[i]);
        let r = f(self, &mut m);
        self.mobs[i] = m;
        Some(r)
    }

    pub(crate) fn is_day(&self, world: usize) -> bool {
        let t = self.worlds.worlds.get(world).map(|w| w.time.rem_euclid(24000)).unwrap_or(0);
        !(12542..23460).contains(&t)
    }

    pub(crate) fn raining(&self, world: usize) -> bool {
        self.worlds.worlds.get(world).is_some_and(|w| w.weather != ceres_world::Weather::Clear)
    }

    pub(crate) fn nearest_player_dist(&self, world: usize, at: [f32; 3]) -> f32 {
        self.players
            .values()
            .filter(|p| p.world == world && p.state == State::Spawned && !p.dead)
            .map(|p| filters::dist(at, [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]))
            .fold(f32::MAX, f32::min)
    }

    pub(crate) fn online_named(&self, name: &str) -> Option<(SessionId, &Player)> {
        self.players.iter().find(|(_, p)| p.state == State::Spawned && p.name.eq_ignore_ascii_case(name)).map(|(s, p)| (*s, p))
    }

    pub(crate) fn stack_name(&self, it: &ItemDesc) -> String {
        if it.id == 0 || it.count == 0 {
            return String::new();
        }
        self.gd.items_by_id.get(&it.id).map(|i| i.name.clone()).unwrap_or_default()
    }

    /// Names /summon accepts.
    pub(crate) fn summonable_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .mobsys
            .reg
            .entities
            .values()
            .filter(|d| d.summonable && d.is_mob())
            .map(|d| json::short(&d.id).to_string())
            .collect();
        v.sort();
        v
    }

    // ---------------------------------------------------------------- creating

    /// Loads the entity definitions (once, at boot).
    pub(crate) fn load_mob_registry(&mut self) {
        self.mobsys.reg = Rc::new(Registry::load());
    }

    /// Creates an entity from its definition; `event` replaces `minecraft:entity_spawned`.
    pub(crate) fn spawn_entity(&mut self, world: usize, id: &str, pos: [f32; 3], event: Option<&str>) -> Option<i64> {
        let alive = self.mobs.iter().filter(|m| m.alive()).count();
        if alive >= MAX_MOBS || !pos.iter().all(|v| v.is_finite()) {
            return None;
        }
        let def = self.mobsys.reg.entity(id)?.clone();
        if !def.is_mob() {
            return None;
        }
        let eid = self.next_entity_id;
        self.next_entity_id += 1;
        let mut m = Mob { entity_id: eid, world, pos, yaw: (rand::random::<u32>() % 360) as f32, def: def.clone(), ..Default::default() };
        m.head_yaw = m.yaw;
        m.sent_pos = pos;
        for p in &def.properties {
            m.props.insert(p.name.clone(), p.default.clone());
        }
        self.rebuild_mob(&mut m);
        m.health = m.t.start_health;
        m.air = m.t.air_ticks;
        m.equip.drop_chance = [0.085; 6];
        m.ambient_due = self.ticks + 80 + rand::random::<u64>() % 160;
        m.env = self.env_at(world, pos, m.t.height);
        let ev = event.unwrap_or("minecraft:entity_spawned").to_string();
        // the event may name a group directly (summon cat <variant>) or be a real event
        if m.def.events.contains_key(&ev) {
            self.mob_event(&mut m, &ev, None);
        }
        // health as the groups added by the spawn event say (slime sizes, tamed wolves...)
        m.health = m.t.start_health;
        self.roll_equipment(&mut m);
        let pks = self.spawn_packets(&m);
        m.sent_meta = self.mob_meta(&m);
        for pk in pks {
            self.broadcast(&pk, |_, q| q.world == world);
        }
        self.mobsys.index.insert(eid, self.mobs.len());
        self.mobs.push(m);
        Some(eid)
    }

    fn roll_equipment(&self, m: &mut Mob) {
        let Some(eq) = m.comps.get("minecraft:equipment").cloned() else { return };
        if let Some(chances) = eq.get("slot_drop_chance").and_then(|c| c.as_array()) {
            for c in chances {
                let slot = json::string(c, "slot").unwrap_or("");
                let ch = json::num(c, "drop_chance", 0.085);
                let i = match slot {
                    "slot.weapon.mainhand" => 0,
                    "slot.weapon.offhand" => 1,
                    "slot.armor.head" => 2,
                    "slot.armor.chest" => 3,
                    "slot.armor.legs" => 4,
                    "slot.armor.feet" => 5,
                    _ => continue,
                };
                m.equip.drop_chance[i] = ch;
            }
        }
        let Some(table) = json::string(&eq, "table") else { return };
        let cx = loot::LootCx { difficulty: self.cfg.difficulty, ..Default::default() };
        for it in self.roll_loot(table, &cx) {
            let name = self.stack_name(&it);
            self.equip_item(m, it, &name);
        }
    }

    /// Puts an item in the right slot (hand, offhand or armour); returns what was there.
    pub(crate) fn equip_item(&self, m: &mut Mob, it: ItemDesc, name: &str) -> Option<ItemDesc> {
        let slot = items::slot_of(name);
        let place = match slot {
            items::Slot::Head => &mut m.equip.armor[0],
            items::Slot::Chest => &mut m.equip.armor[1],
            items::Slot::Legs => &mut m.equip.armor[2],
            items::Slot::Feet => &mut m.equip.armor[3],
            items::Slot::Offhand => &mut m.equip.offhand,
            items::Slot::Hand => &mut m.equip.hand,
        };
        let old = std::mem::replace(place, it);
        (!old.is_empty()).then_some(old)
    }

    // ---------------------------------------------------------------- network

    pub(crate) fn mob_meta(&self, m: &Mob) -> Vec<Meta> {
        let t = &m.t;
        let mut f: u128 = (1 << 35) | (1 << 47) | (1 << 48) | (1 << 14);
        if t.gravity {
            f |= 1 << 49;
        }
        let mut set = |bit: u32, on: bool| {
            if on {
                f |= 1u128 << bit;
            }
        };
        set(0, m.fire_ticks > 0 && !t.fire_immune);
        set(6, false);
        set(7, m.love > 0);
        set(8, t.saddled);
        set(9, t.charged);
        set(10, m.fuse.is_some() || t.ignited);
        set(11, t.baby);
        set(18, m.climbing);
        set(19, m.comps.contains_key("minecraft:can_climb"));
        set(20, t.nav == traits::NavKind::Swim);
        set(21, t.can_fly);
        set(22, t.nav == traits::NavKind::Walk);
        set(24, m.sitting);
        set(25, m.angry_until.is_some());
        set(26, m.interested);
        set(27, m.charging);
        set(28, t.tamed);
        set(30, m.leash.is_some()); // LEASHED
        set(31, t.sheared);
        set(33, m.id() == "minecraft:elder_guardian");
        set(34, m.walk_speed > 0.001);
        set(36, t.chested);
        set(50, t.fire_immune);
        let driven = !m.riders.is_empty() && (m.comps.contains_key("minecraft:input_ground_controlled") || m.comps.contains_key("minecraft:item_controllable"));
        set(44, driven); // WASD_CONTROLLED
        set(45, driven && m.comps.contains_key("minecraft:horse.jump_strength")); // CAN_POWER_JUMP
        set(64 + 11, m.sleeping); // SLEEPING
        set(64 + 17, t.captain); // IS_ILLAGER_CAPTAIN
        set(64 + 18, t.stunned); // STUNNED
        let mut meta = vec![
            Meta::Long(meta_key::FLAGS, f as u64 as i64),
            Meta::Long(meta_key::FLAGS2, (f >> 64) as u64 as i64),
            Meta::Float(meta_key::SCALE, t.scale),
            Meta::Float(meta_key::BOUNDING_BOX_WIDTH, t.width),
            Meta::Float(meta_key::BOUNDING_BOX_HEIGHT, t.height),
            Meta::Short(meta_key::AIR, m.air.clamp(0, i16::MAX as i32) as i16),
            Meta::Short(meta_key::MAX_AIR, t.air_ticks.clamp(0, i16::MAX as i32) as i16),
        ];
        if let Some(v) = t.variant.or(m.item_variant) {
            meta.push(Meta::Int(meta_key::VARIANT, v));
        }
        if let Some(v) = t.mark_variant {
            meta.push(Meta::Int(meta_key::MARK_VARIANT, v));
        }
        if let Some(c) = t.color {
            meta.push(Meta::Byte(meta_key::COLOR, c));
        }
        if let Some(c) = t.color2 {
            meta.push(Meta::Byte(meta_key::COLOR_2, c));
        }
        if let Some(s) = t.skin_id {
            meta.push(Meta::Int(meta_key::SKIN_ID, s));
        }
        if m.pose != 0 {
            meta.push(Meta::Int(meta_key::POSE_INDEX, m.pose));
        }
        if let Some(b) = m.carried_block {
            meta.push(Meta::Int(23, b)); // carried block (enderman)
        }
        if let Some(owner) = m.owner.as_ref().and_then(|o| self.online_named(o)) {
            meta.push(Meta::Long(meta_key::OWNER_EID, owner.1.entity_id));
        }
        if m.charging {
            if let Some(eid) = m.target.and_then(|t| self.ent_eid(t)) {
                meta.push(Meta::Long(meta_key::TARGET_EID, eid));
            }
        }
        if let Some(holder) = m.leash.and_then(|s| self.players.get(&s)) {
            meta.push(Meta::Long(37, holder.entity_id)); // LeashHolder
        } else if let Some(k) = m.knot.and_then(|f| self.mobsys.knots.get(&(m.world, f))) {
            meta.push(Meta::Long(37, *k));
        }
        if let Some(n) = &m.name_tag {
            meta.push(Meta::Str(meta_key::NAMETAG, n.clone()));
            meta.push(Meta::Byte(meta_key::ALWAYS_SHOW_NAMETAG, 1));
        }
        meta
    }

    pub(crate) fn ent_eid(&self, e: Ent) -> Option<i64> {
        match e {
            Ent::Player(s) => self.players.get(&s).map(|p| p.entity_id),
            Ent::Mob(eid) => Some(eid),
        }
    }

    fn equipment_packets(&self, m: &Mob) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        if !m.equip.hand.is_empty() {
            out.push(protocol::mob_equipment(m.entity_id, &m.equip.hand, 0, 0, ceres_protocol::window::INVENTORY));
        }
        if !m.equip.offhand.is_empty() {
            out.push(protocol::mob_equipment(m.entity_id, &m.equip.offhand, 0, 0, ceres_protocol::window::OFFHAND));
        }
        if m.equip.armor.iter().any(|a| !a.is_empty()) {
            out.push(protocol::mob_armor_equipment(m.entity_id, &m.equip.armor, &ItemDesc::default()));
        }
        out
    }

    fn spawn_packets(&self, m: &Mob) -> Vec<Vec<u8>> {
        let meta = self.mob_meta(m);
        let attrs = [("minecraft:health", 0.0, m.health.max(0.0), m.t.max_health), ("minecraft:movement", 0.0, m.t.speed, 1024.0)];
        let flip = if m.t.dragon { 180.0 } else { 0.0 };
        let mut out = vec![protocol::add_entity(m.entity_id, m.id(), m.pos, m.motion, m.pitch, m.yaw + flip, m.head_yaw + flip, &attrs, &meta)];
        out.extend(self.equipment_packets(m));
        out
    }

    /// The mobs of the player's world (on joining or changing worlds).
    pub(crate) fn send_world_mobs(&mut self, sid: SessionId) {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return };
        let mut pks: Vec<Vec<u8>> = self.knot_packets(world);
        pks.extend(self.mobs.iter().filter(|m| m.world == world && m.alive()).flat_map(|m| self.spawn_packets(m)));
        if let Some(p) = self.players.get_mut(&sid) {
            p.out.extend(pks);
        }
    }

    pub(crate) fn mob_sound(&mut self, m: &Mob, sound: &str) {
        let at = [m.pos[0], m.pos[1] + m.t.height * 0.5, m.pos[2]];
        let pk = protocol::mob_sound(sound, at, m.id(), m.t.baby);
        self.broadcast(&pk, |_, q| q.world == m.world);
    }

    fn sync_mob(&mut self, m: &mut Mob) {
        let meta = self.mob_meta(m);
        if meta != m.sent_meta {
            let pk = protocol::set_actor_metadata(m.entity_id, &meta);
            self.broadcast(&pk, |_, q| q.world == m.world);
            m.sent_meta = meta;
        }
        let equip: Vec<i16> = [&m.equip.hand, &m.equip.offhand].into_iter().chain(m.equip.armor.iter()).map(|i| i.id).collect();
        if equip != m.sent_equip {
            if !m.sent_equip.is_empty() || equip.iter().any(|i| *i != 0) {
                let mut pks = self.equipment_packets(m);
                if m.equip.hand.is_empty() {
                    pks.push(protocol::mob_equipment(m.entity_id, &ItemDesc::default(), 0, 0, ceres_protocol::window::INVENTORY));
                }
                if m.equip.armor.iter().all(|a| a.is_empty()) {
                    pks.push(protocol::mob_armor_equipment(m.entity_id, &m.equip.armor, &ItemDesc::default()));
                }
                for pk in pks {
                    self.broadcast(&pk, |_, q| q.world == m.world);
                }
            }
            m.sent_equip = equip;
        }
        let moved = (0..3).map(|k| (m.pos[k] - m.sent_pos[k]).abs()).fold(0.0, f32::max) > 0.01
            || (m.yaw - m.sent_yaw).abs() > 2.0
            || (m.head_yaw - m.sent_head).abs() > 3.0;
        if moved && (m.age % 2 == 0 || m.t.can_fly) {
            m.sent_pos = m.pos;
            m.sent_yaw = m.yaw;
            m.sent_head = m.head_yaw;
            let flip = if m.t.dragon { 180.0 } else { 0.0 };
            let pk = protocol::move_entity(m.entity_id, m.pos, m.pitch, m.yaw + flip, m.head_yaw + flip, m.on_ground);
            // the rider's client moves its vehicle itself
            self.broadcast(&pk, |s, q| q.world == m.world && !m.riders.contains(&s));
        }
    }

    // ---------------------------------------------------------------- ticking

    pub(crate) fn tick_mobs(&mut self) {
        if self.mobs.is_empty() {
            return;
        }
        self.reindex();
        if self.ticks % 20 == 0 && !self.mobsys.knots.is_empty() {
            self.prune_knots();
        }
        // where the players are standing (for filters such as in_water / has_biome_tag)
        let sids: Vec<(SessionId, usize, [f32; 3])> = self
            .players
            .iter()
            .filter(|(_, p)| p.state == State::Spawned)
            .map(|(s, p)| (*s, p.world, [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]))
            .collect();
        if self.ticks % 5 == 0 || self.mobsys.penv.len() != sids.len() {
            self.mobsys.penv.clear();
            for (sid, w, feet) in sids {
                let e = self.env_at(w, feet, 1.8);
                self.mobsys.penv.insert(sid, e);
            }
        }
        let cutoff = self.ticks.saturating_sub(200);
        self.mobsys.villager_attacks.retain(|v| v.3 > cutoff);
        self.mobsys.paths_left = 16;
        let n = self.mobs.len();
        for i in 0..n {
            if self.mobs[i].entity_id == 0 || self.mobs[i].gone {
                continue;
            }
            let mut m = std::mem::take(&mut self.mobs[i]);
            self.tick_mob(&mut m);
            self.mobs[i] = m;
        }
        // events aimed at other mobs (triggers with a target, anger spreading...)
        let mut rounds = 0;
        while !self.mobsys.queued.is_empty() && rounds < 4 {
            rounds += 1;
            for (eid, ev, other) in std::mem::take(&mut self.mobsys.queued) {
                self.with_mob(eid, |s, m| s.mob_event(m, &ev, other));
            }
        }
        if self.mobs.iter().any(|m| m.gone) {
            let mut i = 0;
            while i < self.mobs.len() {
                if self.mobs[i].gone {
                    for r in self.mobs[i].riders.clone() {
                        self.dismount(r);
                    }
                    let m = self.mobs.remove(i);
                    self.broadcast(&protocol::remove_actor(m.entity_id), |_, q| q.world == m.world);
                } else {
                    i += 1;
                }
            }
            self.reindex();
        }
    }

    fn tick_mob(&mut self, m: &mut Mob) {
        m.age += 1;
        m.hurt_cooldown = m.hurt_cooldown.saturating_sub(1);
        m.attack_cooldown = m.attack_cooldown.saturating_sub(1);
        if let Some(t) = m.death_ticks.as_mut() {
            *t = t.saturating_sub(1);
            let left = *t;
            if left == 0 {
                m.gone = true;
            }
            if m.t.dragon {
                // the dragon rises, bursting with light and experience
                m.pos[1] += 0.1;
                m.yaw += 4.0;
                if left % 10 == 0 {
                    let at = [m.pos[0] + json::rand_f(-4.0, 4.0), m.pos[1] + json::rand_f(0.0, 4.0), m.pos[2] + json::rand_f(-4.0, 4.0)];
                    self.broadcast(&protocol::level_event(2025, at, 40), |_, q| q.world == m.world);
                    self.broadcast(&protocol::level_sound_event("explode", at, -1), |_, q| q.world == m.world);
                    self.spawn_xp(m.world, [m.pos[0], m.pos[1], m.pos[2]], 25);
                }
                self.sync_mob(m);
            }
            return;
        }
        if m.pos[1] < (ceres_world::MIN_Y - 40) as f32 {
            m.gone = true;
            return;
        }
        if self.cfg.difficulty == 0 && m.hostile() && !m.persistent && m.name_tag.is_none() {
            m.gone = true;
            return;
        }
        let nearest = self.nearest_player_dist(m.world, m.pos);
        if !self.despawn_check(m, nearest) {
            dbg(|| format!("{} {} despawned (nearest player {nearest:.0})", m.def.id, m.entity_id));
            m.gone = true;
            return;
        }
        if nearest > ACTIVE_RANGE {
            return; // asleep until someone comes close
        }
        if m.age % 4 == (m.entity_id as u64 % 4) || m.env.biome == 0 && m.age < 3 {
            m.env = self.env_at(m.world, m.pos, m.t.height);
        }
        self.mob_sensors(m);
        if m.gone || m.death_ticks.is_some() {
            return;
        }
        if self.ridden_tick(m) {
            self.sync_mob(m);
            return;
        }
        if !m.t.stunned {
            self.run_goals(m);
        } else {
            m.ctl = Ctl::default();
        }
        self.leash_tick(m);
        self.mob_physics(m);
        if m.walk_speed < 0.001 && m.target.is_none() {
            m.inactive_ticks += 1;
        } else {
            m.inactive_ticks = 0;
        }
        self.sync_mob(m);
    }

    fn despawn_check(&self, m: &Mob, nearest: f32) -> bool {
        if m.age % 20 != 0 || m.persistent || m.t.persistent || m.name_tag.is_some() || m.owner.is_some() {
            return true;
        }
        let Some(d) = m.comps.get("minecraft:despawn") else { return true };
        if nearest == f32::MAX {
            return true; // nobody in this world: keep it
        }
        if let Some(f) = d.get("filters") {
            if !filters::test(Some(f), &filters::Cx::new(self, m)) {
                return true;
            }
        }
        if let Some(dist) = d.get("despawn_from_distance") {
            let max = json::num(dist, "max_distance", 128.0);
            let min = json::num(dist, "min_distance", 32.0);
            if nearest > max {
                return false;
            }
            let random = json::boolean(d, "despawn_from_chance", true);
            let one_in = json::num(d, "min_range_random_chance", 800.0).max(20.0) / 20.0;
            if random && nearest > min && rand::random::<f32>() * one_in < 1.0 {
                return false;
            }
        }
        if d.get("despawn_from_inactivity").is_some_and(|v| v.as_bool() != Some(false)) {
            let after = json::num(d, "min_range_inactivity_timer", 30.0) * 20.0;
            if nearest > 32.0 && m.inactive_ticks as f32 > after && rand::random::<u32>() % 40 == 0 {
                return false;
            }
        }
        true
    }

    // ---------------------------------------------------------------- player actions

    /// Right click on a mob. True if it was a mob.
    pub(crate) fn interact_mob(&mut self, sid: SessionId, eid: i64) -> bool {
        dbg(|| format!("interact with entity {eid} (known: {})", self.mobsys.index.contains_key(&eid)));
        if !self.mobsys.index.contains_key(&eid) {
            self.reindex();
        }
        if self.mob_ref(eid).is_none() {
            return self.mobsys.index.contains_key(&eid);
        }
        let Some(p) = self.players.get(&sid) else { return true };
        if p.gamemode == 6 || p.dead {
            return true;
        }
        let feet = [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]];
        let close = self.mob_ref(eid).is_some_and(|m| m.world == p.world && filters::dist(m.pos, feet) < 8.0);
        if !close {
            return true;
        }
        self.with_mob(eid, |s, m| s.interact(sid, m));
        true
    }

    /// A player hit something that is not a player: true if it was a mob.
    pub(crate) fn attack_mob(&mut self, attacker: SessionId, eid: i64, damage: f32) -> bool {
        if !self.mobsys.index.contains_key(&eid) {
            self.reindex();
        }
        let Some(m) = self.mob_ref(eid) else { return self.mobs.iter().any(|m| m.entity_id == eid) };
        let Some(p) = self.players.get(&attacker) else { return true };
        let feet = [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]];
        let edge = (filters::dist(m.center(), [feet[0], feet[1] + 1.0, feet[2]]) - m.t.width.max(m.t.height) / 2.0).max(0.0);
        if m.world != p.world || p.gamemode == 6 || edge > 6.0 {
            return true;
        }
        if m.comps.contains_key("minecraft:cannot_be_attacked") {
            return true;
        }
        let tick = self.ticks;
        self.mobsys.player_attacked.insert(attacker, (Ent::Mob(eid), tick));
        let fire = blocks::enchant_level(p.inventory.held_item(), 13); // fire aspect
        let knock = blocks::enchant_level(p.inventory.held_item(), 12); // knockback
        self.with_mob(eid, |s, m| {
            if s.hurt(m, damage, "entity_attack", Some(Ent::Player(attacker)), Some(feet)) {
                if fire > 0 && !m.t.fire_immune {
                    m.fire_ticks = m.fire_ticks.max(80 * fire as u32);
                }
                if knock > 0 {
                    let (dx, dz) = (m.pos[0] - feet[0], m.pos[2] - feet[2]);
                    let d = (dx * dx + dz * dz).sqrt().max(0.01);
                    m.motion[0] += dx / d * 0.5 * knock as f32;
                    m.motion[2] += dz / d * 0.5 * knock as f32;
                }
            }
        });
        if self.players.get(&attacker).is_some_and(|p| p.gamemode != 1) {
            self.wear_held_item(attacker, true);
        }
        true
    }

    // ---------------------------------------------------------------- commands

    /// `/summon <entity> [x y z] [event]`
    pub(crate) fn cmd_summon(&mut self, sender: commands::Sender, args: &[&str]) {
        let usage = "§cUso: /summon <entidade> [x y z] [evento]";
        let Some(name) = args.first() else { return self.reply(sender, usage) };
        let name = match name.to_ascii_lowercase().as_str() {
            "vaca" => "cow".to_string(),
            "porco" => "pig".to_string(),
            "ovelha" => "sheep".to_string(),
            "galinha" => "chicken".to_string(),
            "zumbi" => "zombie".to_string(),
            "esqueleto" => "skeleton".to_string(),
            "aranha" => "spider".to_string(),
            "lobo" => "wolf".to_string(),
            "gato" => "cat".to_string(),
            "aldeao" | "aldeão" => "villager_v2".to_string(),
            "villager" => "villager_v2".to_string(),
            "zombie_villager" => "zombie_villager_v2".to_string(),
            other => other.to_string(),
        };
        if name == "lightning_bolt" || name == "minecraft:lightning_bolt" {
            let commands::Sender::Player(sid) = sender else { return self.reply(sender, "§cUse /summon como jogador.") };
            let Some(p) = self.players.get(&sid) else { return };
            let (world, feet) = (p.world, [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]);
            let mut at = feet;
            if args.len() >= 4 {
                let c = |s: &str, base: f32| -> Option<f32> {
                    match s.strip_prefix('~') {
                        Some("") => Some(base),
                        Some(rest) => rest.parse::<f32>().ok().map(|v| base + v),
                        None => s.parse().ok(),
                    }
                };
                match (c(args[1], feet[0]), c(args[2], feet[1]), c(args[3], feet[2])) {
                    (Some(x), Some(y), Some(z)) if [x, y, z].iter().all(|v| v.is_finite() && v.abs() < 30_000_000.0) => at = [x, y, z],
                    _ => return self.reply(sender, "§cUso: /summon lightning_bolt [x y z]"),
                }
            }
            let block = [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32];
            if !self.worlds.worlds[world].is_loaded(block) {
                return self.reply(sender, "§cEsse lugar não está carregado.");
            }
            self.strike_lightning(world, at);
            return self.reply(sender, "§aminecraft:lightning_bolt criado.");
        }
        let known = self.mobsys.reg.entity(&name).is_some_and(|d| d.is_mob());
        if !known {
            return self.reply(sender, usage);
        }
        let commands::Sender::Player(sid) = sender else {
            return self.reply(sender, "§cUse /summon como jogador.");
        };
        let Some(p) = self.players.get(&sid) else { return };
        let (world, feet) = (p.world, [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]);
        let pos = if args.len() >= 4 {
            let c = |s: &str, base: f32| -> Option<f32> {
                match s.strip_prefix('~') {
                    Some("") => Some(base),
                    Some(rest) => rest.parse::<f32>().ok().map(|v| base + v),
                    None => s.parse().ok(),
                }
            };
            match (c(args[1], feet[0]), c(args[2], feet[1]), c(args[3], feet[2])) {
                (Some(x), Some(y), Some(z)) if [x, y, z].iter().all(|v| v.is_finite() && v.abs() < 30_000_000.0) => [x, y, z],
                _ => return self.reply(sender, usage),
            }
        } else {
            feet
        };
        let event = args.get(4).map(|e| if e.contains(':') { e.to_string() } else { format!("minecraft:{e}") });
        match self.spawn_entity(world, &name, pos, event.as_deref()) {
            Some(_) => {
                let id = self.mobsys.reg.entity(&name).map(|d| d.id.clone()).unwrap_or_default();
                self.reply(sender, &format!("§a{id} criado."))
            }
            None => self.reply(sender, "§cMobs demais no servidor."),
        }
    }

    // ---------------------------------------------------------------- saving

    /// Writes each world's mobs to `<world>/mobs.json`.
    pub(crate) fn save_mobs(&self) {
        for (w, world) in self.worlds.worlds.iter().enumerate() {
            let list: Vec<serde_json::Value> = self
                .mobs
                .iter()
                .filter(|m| m.world == w && m.alive())
                .map(|m| {
                    let props: serde_json::Map<String, serde_json::Value> = m.props.iter().map(|(k, v)| (k.clone(), v.to_json())).collect();
                    let item = |i: &ItemDesc| {
                        let n = self.stack_name(i);
                        if n.is_empty() { serde_json::Value::Null } else { serde_json::json!({"name": n, "count": i.count, "meta": i.meta}) }
                    };
                    serde_json::json!({
                        "type": m.id(),
                        "pos": m.pos,
                        "yaw": m.yaw,
                        "health": m.health,
                        "groups": m.groups,
                        "properties": props,
                        "owner": m.owner,
                        "name": m.name_tag,
                        "persistent": m.persistent,
                        "sitting": m.sitting,
                        "pose": m.pose,
                        "grow": m.grow_ticks,
                        "home": m.home,
                        "knot": m.knot,
                        "item_variant": m.item_variant,
                        "hand": item(&m.equip.hand),
                        "offhand": item(&m.equip.offhand),
                        "armor": m.equip.armor.iter().map(item).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let path = world.dir.join("mobs.json");
            if list.is_empty() && !path.exists() {
                continue;
            }
            let tmp = world.dir.join("mobs.json.tmp");
            let text = serde_json::to_string(&list).unwrap_or_default();
            if std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
                eprintln!("[World {}] could not save the mobs", world.folder);
            }
        }
    }

    /// Reads back the mobs saved by [`Self::save_mobs`] (also the older format of this server).
    pub(crate) fn load_mobs(&mut self) {
        if self.mobsys.reg.entities.is_empty() {
            self.load_mob_registry();
        }
        let mut total = 0;
        for w in 0..self.worlds.worlds.len() {
            let path = self.worlds.worlds[w].dir.join("mobs.json");
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(&text) else {
                eprintln!("[World {}] mobs.json is damaged, ignored", self.worlds.worlds[w].folder);
                continue;
            };
            for v in list {
                let Some(kind) = v["type"].as_str() else { continue };
                let pos: Vec<f32> = v["pos"].as_array().into_iter().flatten().filter_map(|n| n.as_f64()).map(|n| n as f32).collect();
                if pos.len() != 3 || !pos.iter().all(|n| n.is_finite()) {
                    continue;
                }
                let groups: Option<Vec<String>> = v["groups"].as_array().map(|a| a.iter().filter_map(|g| g.as_str().map(String::from)).collect());
                // saved mobs come back as they were: no spawn event
                let event = if groups.is_some() { Some("ceres:loaded") } else { None };
                let Some(eid) = self.spawn_entity(w, kind, [pos[0], pos[1], pos[2]], event) else { continue };
                let item = |s: &Server, v: &serde_json::Value| -> ItemDesc {
                    let Some(n) = v["name"].as_str() else { return ItemDesc::default() };
                    let mut it = s.simple_item(n, v["count"].as_u64().unwrap_or(1).clamp(1, 64) as u16).unwrap_or_default();
                    it.meta = v["meta"].as_u64().unwrap_or(0) as u32;
                    it
                };
                let hand = item(self, &v["hand"]);
                let off = item(self, &v["offhand"]);
                let armor: Vec<ItemDesc> = (0..4).map(|i| item(self, &v["armor"][i])).collect();
                self.with_mob(eid, |s, m| {
                    if let Some(g) = groups {
                        m.groups = g;
                        if let Some(props) = v["properties"].as_object() {
                            let q = |_: &str, _: &[f32]| 0.0;
                            for (k, pv) in props {
                                if let Some(val) = m.def.prop(k).and_then(|d| d.parse(pv, &q)) {
                                    m.props.insert(k.clone(), val);
                                }
                            }
                        }
                        s.rebuild_mob(m);
                        m.equip.hand = hand;
                        m.equip.offhand = off;
                        for (i, a) in armor.into_iter().enumerate() {
                            m.equip.armor[i] = a;
                        }
                    } else if v["sheared"].as_bool() == Some(true) {
                        s.mob_event(m, "minecraft:on_sheared", None);
                    }
                    m.yaw = v["yaw"].as_f64().unwrap_or(0.0) as f32;
                    m.health = (v["health"].as_f64().unwrap_or(m.t.max_health as f64) as f32).clamp(1.0, m.t.max_health);
                    m.owner = v["owner"].as_str().map(String::from);
                    m.name_tag = v["name"].as_str().map(String::from);
                    m.persistent = v["persistent"].as_bool().unwrap_or(false);
                    m.sitting = v["sitting"].as_bool().unwrap_or(false);
                    m.pose = v["pose"].as_i64().unwrap_or(0).clamp(0, 12) as i32;
                    m.grow_ticks = v["grow"].as_u64().unwrap_or(0) as u32;
                    m.home = v["home"].as_array().and_then(|a| {
                        let c: Vec<i32> = a.iter().filter_map(|n| n.as_i64()).map(|n| n as i32).collect();
                        (c.len() == 3).then(|| [c[0], c[1], c[2]])
                    });
                    m.item_variant = v["item_variant"].as_i64().map(|n| n.clamp(0, 64) as i32);
                    m.knot = v["knot"].as_array().and_then(|a| {
                        let c: Vec<i32> = a.iter().filter_map(|n| n.as_i64()).map(|n| n as i32).collect();
                        (c.len() == 3).then(|| [c[0], c[1], c[2]])
                    });
                });
                total += 1;
            }
        }
        if total > 0 {
            println!("[World] {total} mob(s) loaded");
        }
    }

    /// An item stack by name (for loot).
    pub(crate) fn simple_item(&self, name: &str, count: u16) -> Option<ItemDesc> {
        let info = self.gd.find_item(name)?;
        Some(ItemDesc { id: info.id, count, meta: 0, stack_id: 0, block_runtime_id: info.block_runtime_id, extra: ceres_world::extra_data(None) })
    }

    /// Drops an item stack at a position, with a little random push.
    pub(crate) fn drop_at(&mut self, world: usize, pos: [f32; 3], item: ItemDesc) {
        let rx = (rand::random::<f32>() - 0.5) * 0.2;
        let rz = (rand::random::<f32>() - 0.5) * 0.2;
        self.spawn_item(world, pos, [rx, 0.2, rz], item, 10);
    }
}
