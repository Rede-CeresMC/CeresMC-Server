//! Hurting and killing mobs (damage sensors, knockback, on_hurt/on_death events, loot,
//! experience, equipment drops), mob attacks (melee, ranged with data-driven projectiles,
//! guardian beams, sonic booms), explosions and the player's right click on a mob (breeding,
//! taming, sitting, healing, growing babies, name tags, bartering and `minecraft:interact`).

use super::filters::{self, Cx, Subj, dist};
use super::json::*;
use super::*;
use serde_json::Value;

/// What a projectile shot by a mob does, read from its entity file.
#[derive(Clone, Debug, Default)]
pub(crate) struct ProjData {
    pub speed: f32,
    pub gravity: f32,
    pub inertia: f32,
    pub uncertainty: f32,
    pub damage: (f32, f32),
    pub power_multiplier: f32,
    pub knockback: bool,
    pub catch_fire: bool,
    pub explode: Option<(f32, bool)>,
    pub effects: Vec<(String, u8, [u32; 3])>,
    pub homing: bool,
    pub potion: bool,
    pub aoe: bool,
    pub wind: bool,
    /// arrows and tridents stay stuck in the block they hit
    pub sticks: bool,
}

pub(crate) struct MobProjectile {
    pub entity_id: i64,
    pub world: usize,
    pub data: Rc<ProjData>,
    pub owner: i64,
    pub owner_name: String,
    pub pos: [f32; 3],
    pub motion: [f32; 3],
    pub age: u32,
    pub target: Option<Ent>,
    pub aux: Option<u32>,
    /// ticks since it stuck in a block (0 = flying)
    pub stuck: u32,
}

pub(crate) fn parse_proj(def: &defs::EntityDef) -> ProjData {
    let mut c = def.components.get("minecraft:projectile").cloned().unwrap_or(Value::Null);
    if let Some(g) = def.groups.get("minecraft:mob_arrow").and_then(|g| g.get("minecraft:projectile")) {
        c = g.clone();
    }
    let hit = c.get("on_hit").cloned().unwrap_or(Value::Null);
    let imp = hit.get("impact_damage").cloned().unwrap_or(Value::Null);
    let mut explode = None;
    if hit.get("definition_event").is_some() || hit.get("explode").is_some() {
        let src = hit.get("explode").cloned().or_else(|| {
            def.components.get("minecraft:explode").cloned().or_else(|| def.groups.values().find_map(|g| g.get("minecraft:explode").cloned()))
        });
        if let Some(e) = src {
            explode = Some((num(&e, "power", 1.0), boolean(&e, "causes_fire", false)));
        }
    }
    let effects = hit
        .get("mob_effect")
        .and_then(|m| m.get("effects"))
        .and_then(|e| e.as_array())
        .map(|a| {
            a.iter()
                .map(|e| {
                    (
                        string(e, "effect").unwrap_or("").to_string(),
                        num(e, "amplifier", 0.0) as u8,
                        [num(e, "durationeasy", 0.0) as u32, num(e, "durationnormal", 200.0) as u32, num(e, "durationhard", 200.0) as u32],
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    ProjData {
        speed: num(&c, "power", 1.5),
        gravity: num(&c, "gravity", 0.05),
        inertia: num(&c, "inertia", 0.99),
        uncertainty: num(&c, "uncertainty_base", 0.0),
        damage: range(imp.get("damage"), (0.0, 0.0)),
        power_multiplier: num(&imp, "power_multiplier", 0.0),
        knockback: boolean(&imp, "knockback", true),
        catch_fire: boolean(&imp, "catch_fire", false) || hit.get("catch_fire").is_some(),
        explode,
        effects,
        homing: boolean(&c, "homing", false),
        potion: hit.get("thrown_potion_effect").is_some(),
        aoe: hit.get("spawn_aoe_cloud").is_some(),
        wind: hit.get("wind_burst_on_hit").is_some(),
        sticks: hit.get("stick_in_ground").is_some(),
    }
}

impl Server {
    // ---------------------------------------------------------------- taking damage

    /// Hurts a mob; returns true if the damage landed.
    pub(crate) fn hurt(&mut self, m: &mut Mob, amount: f32, cause: &str, attacker: Option<Ent>, from: Option<[f32; 3]>) -> bool {
        if !m.alive() || amount <= 0.0 {
            return false;
        }
        if m.hurt_cooldown > 0 && cause != "fire_tick" && cause != "drowning" && cause != "magic" {
            return false;
        }
        if m.t.fire_immune && matches!(cause, "fire" | "fire_tick" | "lava" | "magma") {
            return false;
        }
        if attacker == Some(Ent::Mob(m.entity_id)) {
            return false;
        }
        let mut amount = amount;
        // damage sensors may cancel, scale or react
        if let Some(ds) = m.comps.get("minecraft:damage_sensor").cloned() {
            let triggers = match ds.get("triggers") {
                Some(Value::Array(a)) => a.clone(),
                Some(o @ Value::Object(_)) => vec![o.clone()],
                _ => vec![ds.clone()],
            };
            for tr in triggers {
                let tc = string(&tr, "cause").unwrap_or("all");
                if tc != "all" && tc != cause && !(tc == "fire" && cause == "fire_tick") && !(tc == "entity_attack" && cause == "projectile" && false) {
                    continue;
                }
                let on = tr.get("on_damage").cloned().unwrap_or(Value::Null);
                let ok = {
                    let mut cx = Cx::new(self, m).with_other(attacker);
                    cx.cause = Some(cause);
                    filters::test(on.get("filters"), &cx)
                };
                if !ok {
                    continue;
                }
                if let Some(ev) = on.get("event").and_then(|e| e.as_str()).map(String::from) {
                    let target = string(&on, "target").unwrap_or("self").to_string();
                    self.fire(m, &ev, &target, attacker, 0);
                }
                let deals = match tr.get("deals_damage") {
                    Some(Value::Bool(b)) => *b,
                    Some(Value::String(s)) => s != "no",
                    _ => true,
                };
                if !deals {
                    return false;
                }
                amount *= num(&tr, "damage_multiplier", 1.0);
                amount += num(&tr, "damage_modifier", 0.0);
            }
        }
        if amount <= 0.0 || !m.alive() {
            return false;
        }
        // armour on mobs
        let points: i32 = m.equip.armor.iter().map(|a| items::armor_points(&self.stack_name(a))).sum();
        if points > 0 && matches!(cause, "entity_attack" | "projectile" | "entity_explosion" | "block_explosion") {
            amount *= 1.0 - (points.min(20) as f32 * 0.04);
        }
        let now = self.ticks;
        m.health -= amount;
        m.hurt_cooldown = 10;
        m.last_cause = cause.to_string();
        if let Some(a) = attacker {
            m.last_hurt_by = Some(a);
            m.last_hurt_tick = now;
            if let Ent::Player(_) = a {
                m.last_hit_by_player = now.max(1);
            }
            if m.t.is("villager") || m.t.is("wandering_trader") {
                self.mobsys.villager_attacks.push((m.world, m.pos, a, now));
            }
        } else {
            m.last_hurt_tick = now;
        }
        m.sitting = m.sitting && attacker.is_none();
        m.sleeping = false;
        if let Some(f) = from {
            let (dx, dz) = (m.pos[0] - f[0], m.pos[2] - f[2]);
            let d = (dx * dx + dz * dz).sqrt().max(0.01);
            let k = 0.4 * (1.0 - m.t.knockback_resistance.clamp(0.0, 1.0));
            m.motion = [dx / d * k, if k > 0.0 { 0.36 } else { m.motion[1] }, dz / d * k];
        }
        self.broadcast(&protocol::actor_event(m.entity_id, actor_event::HURT_ANIMATION, 0), |_, q| q.world == m.world);
        if m.health > 0.0 {
            self.mob_sound(m, "hurt");
            let ev = m.comps.get("minecraft:on_hurt").cloned();
            self.fire_ref(m, ev.as_ref(), attacker);
            if matches!(attacker, Some(Ent::Player(_))) {
                let ev = m.comps.get("minecraft:on_hurt_by_player").cloned();
                self.fire_ref(m, ev.as_ref(), attacker);
            }
            // friends of the same kind get angry too (pigmen, bees, wolves...)
            if let Some(Value::Object(_)) = m.comps.get("minecraft:angry").map(|v| v.clone()).as_ref() {
                if m.target.is_none() {
                    m.target = attacker;
                }
            }
            return true;
        }
        self.die(m, cause, attacker);
        true
    }

    fn die(&mut self, m: &mut Mob, cause: &str, attacker: Option<Ent>) {
        for r in std::mem::take(&mut m.riders) {
            if let Some(p) = self.players.get_mut(&r) {
                p.riding = None;
            }
            let peid = self.players.get(&r).map(|p| p.entity_id).unwrap_or(0);
            self.broadcast(&protocol::set_actor_link(m.entity_id, peid, 0, true, false), |_, q| q.world == m.world);
        }
        // a mob on a lead drops it
        if m.leash.is_some() || m.knot.is_some() {
            self.snap_leash(m);
        }
        dbg(|| format!("{} {} died ({cause}, by {attacker:?})", m.def.id, m.entity_id));
        m.health = 0.0;
        m.death_ticks = Some(if m.t.dragon { 200 } else { 20 });
        m.fuse = None;
        let ev = m.comps.get("minecraft:on_death").cloned();
        self.fire_ref(m, ev.as_ref(), attacker);
        let anim = if m.t.dragon { 37 } else { actor_event::DEATH_ANIMATION }; // DRAGON_START_DEATH_ANIM
        self.broadcast(&protocol::actor_event(m.entity_id, anim, 0), |_, q| q.world == m.world);
        self.mob_sound(m, "death");
        let by_player = self.ticks < m.last_hit_by_player + 100 && m.last_hit_by_player > 0;
        let killer_player = match attacker {
            Some(Ent::Player(s)) => Some(s),
            _ => None,
        };
        let pet = matches!(attacker, Some(Ent::Mob(e)) if self.mob_ref(e).is_some_and(|o| o.t.tamed));
        let looting = killer_player.and_then(|s| self.players.get(&s)).map(|p| blocks::enchant_level(p.inventory.held_item(), 14) as u32).unwrap_or(0);
        let killer = attacker.and_then(|a| match a {
            Ent::Mob(e) => self.mob_ref(e).map(|o| o.def.id.clone()),
            Ent::Player(_) => Some("minecraft:player".into()),
        });
        let center = [m.pos[0], m.pos[1] + 0.5, m.pos[2]];
        let exploded_self = cause == "self_explosion";
        if !exploded_self {
            if let Some(table) = m.comps.get("minecraft:loot").and_then(|l| string(l, "table")).map(String::from) {
                let cx = loot::LootCx {
                    killed_by_player: by_player,
                    killed_by_pet: pet,
                    killer,
                    looting,
                    on_fire: m.fire_ticks > 0,
                    baby: m.t.baby,
                    variant: m.t.variant.unwrap_or(0),
                    mark_variant: m.t.mark_variant.unwrap_or(0),
                    color: m.t.color.unwrap_or(0),
                    props: Some(&m.props),
                    difficulty: self.cfg.difficulty,
                    luck: 0.0,
                };
                let drops = self.roll_loot(&table, &cx);
                for it in drops {
                    self.drop_at(m.world, center, it);
                }
            }
            // equipment: picked-up items always, spawn gear sometimes (more with looting)
            let gear: Vec<(ItemDesc, f32)> = [(m.equip.hand.clone(), m.equip.drop_chance[0]), (m.equip.offhand.clone(), m.equip.drop_chance[1])]
                .into_iter()
                .chain(m.equip.armor.iter().cloned().zip(m.equip.drop_chance[2..].iter().copied()))
                .collect();
            for (it, ch) in gear {
                if !it.is_empty() && by_player && chance(ch + 0.01 * looting as f32) {
                    self.drop_at(m.world, center, it);
                }
            }
            if let Some(b) = m.carried_block.take() {
                if let Some(ce) = self.gd.block_to_item.get(&b).cloned() {
                    if let Some(it) = self.simple_item(&ce.name, 1) {
                        self.drop_at(m.world, center, it);
                    }
                }
            }
        }
        let q = Self::molang_of(m);
        let xp = match m.comps.get("minecraft:experience_reward") {
            Some(x) => molang::value(x.get("on_death"), 0.0, &q),
            None => 0.0,
        };
        if xp >= 1.0 && (by_player || pet) {
            self.spawn_xp(m.world, center, xp as u32);
        }
    }

    // ---------------------------------------------------------------- attacking

    /// A mob's hit on something (difficulty, knockback, attack effects, fire).
    pub(crate) fn mob_hit(&mut self, m: &mut Mob, target: Ent, dmg: f32, knock: Option<f32>) {
        let eid = m.entity_id;
        self.broadcast(&protocol::actor_event(eid, 4, 0), |_, q| q.world == m.world); // arm swing
        let (effect, fire) = (m.t.attack_effect.clone(), m.fire_ticks > 0);
        match target {
            Ent::Player(sid) => {
                let Some(p) = self.players.get(&sid) else { return };
                let feet = [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]];
                let name = p.name.clone();
                let dmg = match self.cfg.difficulty {
                    0 if m.hostile() => 0.0,
                    1 => (dmg / 2.0 + 1.0).min(dmg),
                    3 => dmg * 1.5,
                    _ => dmg,
                };
                if dmg <= 0.0 {
                    return;
                }
                let (dx, dz) = (feet[0] - m.pos[0], feet[2] - m.pos[2]);
                let d = (dx * dx + dz * dz).sqrt().max(0.01);
                let k = 0.4 * (1.0 + knock.unwrap_or(0.0));
                let msg = format!("§c{name} foi morto por {}", display_name(m.id()));
                if self.damage_player(sid, dmg, &msg, false, Some([dx / d * k, 0.36, dz / d * k])) {
                    let tick = self.ticks;
                    self.mobsys.player_hurt_by.insert(sid, (Ent::Mob(eid), tick));
                    if let Some((name, secs)) = effect {
                        if let Some(id) = super::super::effects::effect_id(&name) {
                            let secs = if self.cfg.difficulty >= 3 { secs * 2.0 } else { secs };
                            if secs > 0.0 {
                                self.add_effect(sid, id, 0, Some(secs as u32), true);
                            }
                        }
                    }
                    if fire {
                        if let Some(p) = self.players.get_mut(&sid) {
                            p.fire_ticks = p.fire_ticks.max(80);
                        }
                    }
                }
            }
            Ent::Mob(o) => {
                let from = m.pos;
                let k = knock.unwrap_or(0.0);
                self.with_mob(o, |s, om| {
                    if s.hurt(om, dmg, "entity_attack", Some(Ent::Mob(eid)), Some(from)) {
                        if k > 0.0 {
                            om.motion[0] *= 1.0 + k;
                            om.motion[2] *= 1.0 + k;
                        }
                        if let Some((name, secs)) = effect {
                            om.effects.push((name, (secs * 20.0) as u32));
                        }
                        if fire && !om.t.fire_immune {
                            om.fire_ticks = om.fire_ticks.max(80);
                        }
                    }
                });
            }
        }
    }

    pub(crate) fn melee_tick(&mut self, m: &mut Mob, gi: usize) -> bool {
        let g = m.goals[gi].def.clone();
        let p = &g.p;
        let name = g.name.trim_start_matches("minecraft:behavior.");
        let Some(target) = m.target else { return false };
        let Some(o) = self.subj(target) else { return false };
        let tp = o.feet();
        let (tw, th) = match o {
            Subj::P(..) => (0.6, 1.8),
            Subj::M(x) => (x.t.width, x.t.height),
        };
        let speed = num(p, "speed_multiplier", 1.0);
        let hd = ((tp[0] - m.pos[0]).powi(2) + (tp[2] - m.pos[2]).powi(2)).sqrt();
        let dy = tp[1] - m.pos[1];
        m.ctl.look = Some([tp[0], tp[1] + th * 0.85, tp[2]]);
        let st_t = m.goals[gi].st.t;
        match name {
            "hold_ground" => {
                m.ctl.hold = true;
                return hd < num(p, "within_radius", 10.0) + 4.0;
            }
            "move_towards_target" => {
                if hd > num(p, "within_radius", 2.0) {
                    m.ctl.dest = Some(tp);
                    m.ctl.speed = speed;
                }
                return true;
            }
            "move_around_target" | "jump_around_target" => {
                let (near, far) = (3.0, 7.0);
                if hd < near || hd > far {
                    let (dx, dz) = (m.pos[0] - tp[0], m.pos[2] - tp[2]);
                    let d = (dx * dx + dz * dz).sqrt().max(0.01);
                    let want = (near + far) / 2.0;
                    m.ctl.dest = Some([tp[0] + dx / d * want, m.pos[1], tp[2] + dz / d * want]);
                    m.ctl.speed = speed;
                }
                if name == "jump_around_target" && m.on_ground && st_t % 40 == 0 {
                    m.motion[1] = 0.6;
                }
                return true;
            }
            "guardian_attack" => {
                m.ctl.hold = true;
                m.charging = true;
                let seen = self.can_see(m.world, m.eye(), [tp[0], tp[1] + 1.0, tp[2]]);
                if !seen || hd > 15.0 {
                    m.goals[gi].st.n = 0;
                    return true;
                }
                m.goals[gi].st.n += 1;
                if m.goals[gi].st.n >= 80 {
                    m.goals[gi].st.n = 0;
                    let mut dmg = if m.id() == "minecraft:elder_guardian" { 8.0 } else { 6.0 };
                    if self.cfg.difficulty >= 3 {
                        dmg += 2.0;
                    }
                    self.mob_hit(m, target, dmg, None);
                }
                return true;
            }
            "knockback_roar" => {
                m.ctl.hold = true;
                if st_t % 80 == 20 {
                    let range_ = num(p, "knockback_range", 4.0);
                    let dmg = num(p, "knockback_damage", 6.0);
                    self.mob_sound(m, "roar");
                    for e in self.entities_near(m.world, m.pos, range_, Some(m.entity_id)) {
                        let same = self.subj(e).is_some_and(|s| s.is_family("illager") || s.is_family("ravager"));
                        if !same {
                            self.mob_hit(m, e, dmg, Some(2.0));
                        }
                    }
                }
                return st_t < 60 || hd < 8.0;
            }
            "swoop_attack" | "charge_attack" | "aquatic_charge_attack" => {
                m.ctl.dest = Some([tp[0], tp[1] + th * 0.5, tp[2]]);
                m.ctl.speed = speed.max(1.5);
            }
            "stalk_and_pounce_on_target" => {
                if hd > 4.0 {
                    m.ctl.dest = Some(tp);
                    m.ctl.speed = num(p, "stalk_speed", 1.2);
                } else if m.on_ground && m.goals[gi].st.phase == 0 {
                    let d = hd.max(0.01);
                    m.motion = [(tp[0] - m.pos[0]) / d * 0.5, 0.6, (tp[2] - m.pos[2]) / d * 0.5];
                    m.goals[gi].st.phase = 1;
                }
            }
            "slime_attack" => {
                if m.on_ground && st_t % 20 == 1 {
                    let d = hd.max(0.01);
                    let s = 0.3 * m.t.scale.clamp(0.5, 1.5);
                    m.motion = [(tp[0] - m.pos[0]) / d * s, 0.42, (tp[2] - m.pos[2]) / d * s];
                    m.yaw = (tp[2] - m.pos[2]).atan2(tp[0] - m.pos[0]).to_degrees() - 90.0;
                    self.mob_sound(m, "jump");
                }
            }
            _ => {
                m.ctl.dest = Some(tp);
                m.ctl.speed = speed;
            }
        }
        // in reach?
        let horizontal_reach = num(p, "horizontal_reach", 0.8);
        let reach = if name == "melee_attack" {
            ((m.t.width * 2.0).powi(2) + tw).sqrt().max(1.0) * num(p, "reach_multiplier", 1.0).max(1.0)
        } else if name == "use_kinetic_weapon" {
            3.0
        } else {
            m.t.width / 2.0 + tw / 2.0 + horizontal_reach
        };
        let damage = rand_f(m.t.damage.0, m.t.damage.1);
        let deals = damage > 0.0 || name == "slime_attack" || name == "eat_mob";
        if deals && hd <= reach && dy > -m.t.height - 0.5 && dy < m.t.height + 0.5 && m.attack_cooldown == 0 {
            let cd = num(p, "cooldown_time", num(p, "attack_interval", 1.0)).max(0.5);
            m.attack_cooldown = (cd * 20.0) as u32;
            let dmg = if name == "eat_mob" { 1000.0 } else { damage.max(1.0) };
            self.mob_hit(m, target, dmg, None);
            if name == "swoop_attack" {
                m.motion[1] = 0.6;
            }
            if boolean(p, "attack_once", false) {
                return false;
            }
        }
        true
    }

    pub(crate) fn ranged_tick(&mut self, m: &mut Mob, gi: usize) -> bool {
        let g = m.goals[gi].def.clone();
        let p = &g.p;
        let name = g.name.trim_start_matches("minecraft:behavior.");
        if name == "charge_held_item" {
            return false;
        }
        let Some(target) = m.target else { return false };
        let Some(tp) = self.subj(target).map(|s| s.feet()) else { return false };
        let d = dist(tp, m.pos);
        let r = goals::ranged_radius(p).max(4.0);
        m.ctl.look = Some([tp[0], tp[1] + 1.4, tp[2]]);
        let t = m.goals[gi].st.t;
        if t % 5 == 1 {
            let seen = self.can_see(m.world, m.eye(), [tp[0], tp[1] + 1.4, tp[2]]);
            m.goals[gi].st.f = if seen { 1.0 } else { 0.0 };
        }
        let visible = m.goals[gi].st.f > 0.0;
        if d > r * 0.8 || !visible {
            m.ctl.dest = Some(tp);
            m.ctl.speed = num(p, "speed_multiplier", 1.0);
        } else if !m.t.can_fly {
            m.ctl.hold = true;
        }
        if name == "sonic_boom" {
            if m.goals[gi].st.n == 0 {
                m.goals[gi].st.n = 34;
            }
            m.goals[gi].st.n -= 1;
            if m.goals[gi].st.n == 0 && d < 15.0 {
                self.mob_sound(m, "sonic_boom");
                if let Ent::Player(sid) = target {
                    let name = self.players.get(&sid).map(|p| p.name.clone()).unwrap_or_default();
                    let (dx, dz) = (tp[0] - m.pos[0], tp[2] - m.pos[2]);
                    let dd = (dx * dx + dz * dz).sqrt().max(0.01);
                    self.damage_player(sid, 10.0, &format!("§c{name} foi obliterado por um grito sônico"), true, Some([dx / dd * 2.5, 0.5, dz / dd * 2.5]));
                } else {
                    self.mob_hit(m, target, 10.0, Some(2.0));
                }
                m.goals[gi].st.next = self.ticks + 40;
                return false;
            }
            return true;
        }
        // shooting clock
        let (lo, hi) = if p.get("attack_interval_min").is_some() {
            (num(p, "attack_interval_min", 1.0), num(p, "attack_interval_max", 3.0))
        } else {
            range(p.get("attack_interval").or(p.get("attack_cooldown")), (1.0, 3.0))
        };
        if t == 1 {
            m.goals[gi].st.n = (rand_f(lo, hi) * 20.0).max(10.0) as u32;
        }
        let charge = (num(p, "charge_shoot_trigger", 0.0) * 20.0) as u32;
        let left = m.goals[gi].st.n.saturating_sub(1);
        m.goals[gi].st.n = left;
        m.charging = charge > 0 && left <= charge && visible;
        if left == 0 {
            if visible && d <= r {
                let shots = num(p, "burst_shots", 1.0).max(1.0) as u32;
                let def = if name == "fire_at_target" { string(p, "projectile_def").map(String::from) } else { None };
                for _ in 0..shots.min(5) {
                    self.mob_shoot(m, target, def.as_deref());
                }
                if boolean(p, "swing", false) {
                    self.broadcast(&protocol::actor_event(m.entity_id, 4, 0), |_, q| q.world == m.world);
                }
            }
            m.goals[gi].st.n = (rand_f(lo, hi) * 20.0).max(10.0) as u32;
        }
        true
    }

    /// Shoots at `target` what `minecraft:shooter` names (or `def`).
    pub(crate) fn mob_shoot(&mut self, m: &mut Mob, target: Ent, def: Option<&str>) {
        let Some(subj) = self.subj(target) else { return };
        let tp = subj.feet();
        let th = match subj {
            Subj::P(..) => 1.8,
            Subj::M(o) => o.t.height,
        };
        let shooter = m.comps.get("minecraft:shooter").cloned().unwrap_or(Value::Null);
        let (mut id, mut aux) = (def.map(String::from).or(string(&shooter, "def").map(String::from)), as_num(shooter.get("aux_val")).map(|v| v as u32));
        if def.is_none() {
            if let Some(Value::Array(list)) = shooter.get("projectiles") {
                for pr in list {
                    let ok = filters::test(pr.get("filters"), &Cx::new(self, m).with_other(Some(target)));
                    if ok {
                        id = string(pr, "def").map(String::from).or(id);
                        aux = as_num(pr.get("aux_val")).map(|v| v as u32).or(aux);
                        break;
                    }
                }
            }
        }
        let id = id.unwrap_or_else(|| "minecraft:arrow".into());
        let Some(pdef) = self.mobsys.reg.entity(&id).cloned() else { return };
        let mut data = parse_proj(&pdef);
        if let Some(pw) = as_num(shooter.get("power")) {
            data.speed = pw;
        }
        let data = Rc::new(data);
        let eye = m.eye();
        // vanilla aims at a third of the target's height; the arc lifts it from there
        let aim = [tp[0], tp[1] + th / 3.0, tp[2]];
        let (dx, dy, dz) = (aim[0] - eye[0], aim[1] - eye[1], aim[2] - eye[2]);
        let hd = (dx * dx + dz * dz).sqrt();
        // aim a little high for gravity
        let dy = dy + hd * 0.2 * (data.gravity / 0.05).min(2.0);
        let len = (dx * dx + dy * dy + dz * dz).sqrt().max(0.01);
        let spread = data.uncertainty * 0.0075 * (4 - self.cfg.difficulty.clamp(0, 3)) as f32 / 4.0;
        let mut dir = [dx / len, dy / len, dz / len];
        for v in dir.iter_mut() {
            *v += (rand::random::<f32>() - 0.5) * spread;
        }
        let motion = [dir[0] * data.speed, dir[1] * data.speed, dir[2] * data.speed];
        let start = [eye[0] + dir[0] * (m.t.width * 0.5 + 0.3), eye[1] - 0.1, eye[2] + dir[2] * (m.t.width * 0.5 + 0.3)];
        let peid = self.next_entity_id;
        self.next_entity_id += 1;
        let pot = if data.potion || id == "minecraft:arrow" { aux.map(|a| a as i16) } else { None };
        self.broadcast(&protocol::add_projectile(peid, &id, start, motion, m.entity_id, pot), |_, q| q.world == m.world);
        let snd = string(&shooter, "sound").unwrap_or(if id.contains("arrow") { "bow" } else { "shoot" }).to_string();
        self.broadcast(&protocol::level_sound_event(&snd, start, -1), |_, q| q.world == m.world);
        self.mob_projectiles.push(MobProjectile {
            entity_id: peid,
            world: m.world,
            data,
            owner: m.entity_id,
            owner_name: display_name(m.id()),
            pos: start,
            motion,
            age: 0,
            target: Some(target),
            aux,
            stuck: 0,
        });
    }

    pub(crate) fn tick_mob_projectiles(&mut self) {
        let mut done: Vec<(usize, Option<([f32; 3], Option<Ent>)>)> = Vec::new();
        for i in 0..self.mob_projectiles.len() {
            let (world, from, mut motion, owner, age) = {
                let pr = &mut self.mob_projectiles[i];
                pr.age += 1;
                (pr.world, pr.pos, pr.motion, pr.owner, pr.age)
            };
            if self.mob_projectiles[i].stuck > 0 {
                self.mob_projectiles[i].stuck += 1;
                if self.mob_projectiles[i].stuck > 1200 {
                    done.push((i, None));
                }
                continue; // mobs' arrows cannot be picked up (vanilla)
            }
            if age > 600 {
                done.push((i, None));
                continue;
            }
            let data = self.mob_projectiles[i].data.clone();
            if data.homing {
                if let Some(tp) = self.mob_projectiles[i].target.and_then(|t| self.subj(t)).map(|s| s.feet()) {
                    let d = [tp[0] - from[0], tp[1] + 1.0 - from[1], tp[2] - from[2]];
                    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(0.01);
                    for k in 0..3 {
                        motion[k] = motion[k] * 0.8 + d[k] / l * 0.12;
                    }
                }
            }
            let steps = ((motion[0].abs() + motion[1].abs() + motion[2].abs()) * 4.0).ceil().max(1.0) as i32;
            let mut hit = None;
            for s in 1..=steps {
                let t = s as f32 / steps as f32;
                let at = [from[0] + motion[0] * t, from[1] + motion[1] * t, from[2] + motion[2] * t];
                let body = self.entity_at(world, at, Some(owner), age > 3);
                if let Some(e) = body {
                    hit = Some((at, Some(e)));
                    break;
                }
                let b = [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32];
                if !self.worlds.worlds[world].is_loaded(b) || self.class_at(world, b) & nav::SOLID != 0 {
                    hit = Some((at, None));
                    break;
                }
            }
            if let Some((at, None)) = hit {
                let b = [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32];
                if data.sticks && self.worlds.worlds[world].is_loaded(b) {
                    let len = (motion[0].powi(2) + motion[1].powi(2) + motion[2].powi(2)).sqrt().max(0.01);
                    let pr = &mut self.mob_projectiles[i];
                    pr.pos = [at[0] - motion[0] / len * 0.05, at[1] - motion[1] / len * 0.05, at[2] - motion[2] / len * 0.05];
                    pr.stuck = 1;
                    let (eid, pos) = (pr.entity_id, pr.pos);
                    self.broadcast(&protocol::move_projectile(eid, pos, motion, true), |_, q| q.world == world);
                    self.broadcast(&protocol::level_sound_event("bow.hit", pos, -1), |_, q| q.world == world);
                    continue;
                }
            }
            if let Some(h) = hit {
                done.push((i, Some(h)));
                continue;
            }
            let pr = &mut self.mob_projectiles[i];
            pr.pos = [from[0] + motion[0], from[1] + motion[1], from[2] + motion[2]];
            let inertia = if data.inertia > 0.0 { data.inertia } else { 0.99 };
            pr.motion = [motion[0] * inertia, motion[1] * inertia - data.gravity, motion[2] * inertia];
            let pk = protocol::move_projectile(pr.entity_id, pr.pos, pr.motion, false);
            self.broadcast(&pk, |_, q| q.world == world);
        }
        done.sort_by_key(|d| std::cmp::Reverse(d.0));
        for (i, hit) in done {
            let pr = self.mob_projectiles.swap_remove(i);
            self.broadcast(&protocol::remove_actor(pr.entity_id), |_, q| q.world == pr.world);
            if let Some((at, target)) = hit {
                self.mob_projectile_hit(&pr, at, target);
            }
        }
    }

    /// The player or mob whose body contains `at` (`skip` is the shooter, ignored at first).
    pub(crate) fn entity_at(&self, world: usize, at: [f32; 3], skip: Option<i64>, skip_ok: bool) -> Option<Ent> {
        for (sid, q) in &self.players {
            if q.world == world && q.state == State::Spawned && !q.dead && q.gamemode != 6 {
                let feet = q.pos[1] - EYE_HEIGHT;
                if (at[0] - q.pos[0]).abs() < 0.4 && (at[2] - q.pos[2]).abs() < 0.4 && at[1] >= feet && at[1] <= feet + 1.9 {
                    return Some(Ent::Player(*sid));
                }
            }
        }
        for o in &self.mobs {
            if o.world != world || !o.alive() || (Some(o.entity_id) == skip && !skip_ok) || Some(o.entity_id) == skip {
                continue;
            }
            let r = o.t.width / 2.0 + 0.1;
            if (at[0] - o.pos[0]).abs() < r && (at[2] - o.pos[2]).abs() < r && at[1] >= o.pos[1] && at[1] <= o.pos[1] + o.t.height {
                return Some(Ent::Mob(o.entity_id));
            }
        }
        None
    }

    fn mob_projectile_hit(&mut self, pr: &MobProjectile, at: [f32; 3], target: Option<Ent>) {
        let d = &pr.data;
        dbg(|| format!("projectile from {} hit {:?} at {:?}", pr.owner, target, at));
        let speed = (pr.motion[0].powi(2) + pr.motion[1].powi(2) + pr.motion[2].powi(2)).sqrt();
        let hz = (pr.motion[0].powi(2) + pr.motion[2].powi(2)).sqrt().max(0.01);
        let world = pr.world;
        if d.potion {
            let target_sid = match target {
                Some(Ent::Player(s)) => Some(s),
                _ => None,
            };
            self.burst_potion(world, at, pr.aux.unwrap_or(0), false, target_sid, u64::MAX);
            return;
        }
        if let Some(t) = target {
            let mut dmg = rand_f(d.damage.0, d.damage.1);
            if dmg <= 0.0 && d.power_multiplier > 0.0 {
                dmg = (speed * d.power_multiplier).ceil();
            }
            let kb = if d.knockback { Some([pr.motion[0] / hz * 0.4, 0.3, pr.motion[2] / hz * 0.4]) } else { None };
            match t {
                Ent::Player(sid) => {
                    let name = self.players.get(&sid).map(|p| p.name.clone()).unwrap_or_default();
                    let origin = [at[0] - pr.motion[0], at[1], at[2] - pr.motion[2]];
                    if dmg > 0.0 && self.shield_blocks(sid, origin, dmg, false) {
                        return;
                    }
                    let dmg = match self.cfg.difficulty {
                        1 => (dmg / 2.0 + 1.0).min(dmg),
                        3 => dmg * 1.5,
                        _ => dmg,
                    };
                    let landed = self.damage_player(sid, dmg, &format!("§c{name} foi alvejado por {}", pr.owner_name), false, kb);
                    if landed {
                        let tick = self.ticks;
                        self.mobsys.player_hurt_by.insert(sid, (Ent::Mob(pr.owner), tick));
                        let diff = self.cfg.difficulty.clamp(1, 3) as usize - 1;
                        for (name, amp, dur) in &d.effects {
                            if let Some(id) = super::super::effects::effect_id(name) {
                                if dur[diff] > 0 {
                                    self.add_effect(sid, id, *amp, Some(dur[diff] / 20), true);
                                }
                            }
                        }
                        // tipped arrows of strays (slowness) and bogged (poison)
                        let tip = match pr.aux {
                            Some(19) => Some(("slowness", 30)),
                            Some(26) => Some(("poison", 4)),
                            _ => None,
                        };
                        if let Some((e, secs)) = tip {
                            if let Some(id) = super::super::effects::effect_id(e) {
                                self.add_effect(sid, id, 0, Some(secs), true);
                            }
                        }
                        if d.catch_fire {
                            if let Some(p) = self.players.get_mut(&sid) {
                                p.fire_ticks = p.fire_ticks.max(100);
                            }
                        }
                    }
                }
                Ent::Mob(eid) => {
                    let from = [at[0] - pr.motion[0] * 5.0, at[1], at[2] - pr.motion[2] * 5.0];
                    let fire = d.catch_fire;
                    let owner = pr.owner;
                    self.with_mob(eid, |s, o| {
                        if s.hurt(o, dmg, "projectile", Some(Ent::Mob(owner)), if kb.is_some() { Some(from) } else { None }) && fire && !o.t.fire_immune {
                            o.fire_ticks = o.fire_ticks.max(100);
                        }
                    });
                }
            }
        }
        if let Some((power, fire)) = d.explode {
            let breaks = self.cfg.mob_griefing;
            self.explode(world, at, power, fire, breaks, Some(Ent::Mob(pr.owner)));
        }
        if d.aoe || d.wind {
            for e in self.entities_near(world, at, if d.aoe { 4.0 } else { 2.5 }, Some(pr.owner)) {
                if let Ent::Player(sid) = e {
                    if d.aoe {
                        let name = self.players.get(&sid).map(|p| p.name.clone()).unwrap_or_default();
                        self.damage_player(sid, 3.0, &format!("§c{name} morreu pelo sopro do dragão"), true, None);
                    } else {
                        self.damage_player(sid, 0.0, "", false, Some([0.0, 0.8, 0.0]));
                    }
                }
            }
            if d.wind {
                self.broadcast(&protocol::level_sound_event("wind_charge.burst", at, -1), |_, q| q.world == world);
            }
        }
        if d.catch_fire && target.is_none() && self.cfg.mob_griefing {
            let b = [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32];
            let back = [at[0] - pr.motion[0] * 0.2, at[1] - pr.motion[1] * 0.2, at[2] - pr.motion[2] * 0.2];
            let fb = [back[0].floor() as i32, back[1].floor() as i32, back[2].floor() as i32];
            if fb != b && self.class_at(world, fb) & (nav::SOLID | nav::WATER | nav::LAVA) == 0 {
                if let Some(fire) = self.gd.default_block("minecraft:fire") {
                    self.change_block(world, fb, fire);
                }
            }
        }
    }

    /// An explosion: hurts and pushes what is near, and (when allowed) breaks blocks.
    pub(crate) fn explode(&mut self, world: usize, at: [f32; 3], power: f32, fire: bool, breaks: bool, source: Option<Ent>) {
        self.broadcast(&protocol::level_sound_event("explode", at, -1), |_, q| q.world == world);
        self.broadcast(&protocol::level_event(2025, at, (power * 10.0) as i32), |_, q| q.world == world); // ParticlesExplosion
        let reach = power * 2.0;
        let who = source.and_then(|s| match s {
            Ent::Mob(e) => self.mob_ref(e).map(|m| display_name(m.id())),
            _ => None,
        });
        for e in self.entities_near(world, at, reach, None) {
            let Some(pos) = self.subj(e).map(|s| s.feet()) else { continue };
            let body = [pos[0], pos[1] + 0.9, pos[2]];
            let d = dist(body, at);
            if d > reach {
                continue;
            }
            let exposure = if self.can_see(world, at, body) { 1.0 } else { 0.4 };
            let impact = (1.0 - d / reach) * exposure;
            let dmg = ((impact * impact + impact) / 2.0 * 7.0 * reach + 1.0).floor();
            let l = d.max(0.1);
            let kb = [(body[0] - at[0]) / l * impact, (body[1] - at[1]) / l * impact * 0.6 + 0.2 * impact, (body[2] - at[2]) / l * impact];
            match e {
                Ent::Player(sid) => {
                    let name = self.players.get(&sid).map(|p| p.name.clone()).unwrap_or_default();
                    let dmg = match self.cfg.difficulty {
                        0 => 0.0,
                        1 => (dmg / 2.0 + 1.0).min(dmg),
                        3 => dmg * 1.5,
                        _ => dmg,
                    };
                    let msg = match &who {
                        Some(w) => format!("§c{name} foi explodido por {w}"),
                        None => format!("§c{name} explodiu"),
                    };
                    self.damage_player(sid, dmg, &msg, false, Some(kb));
                }
                Ent::Mob(eid) => {
                    if Some(Ent::Mob(eid)) == source {
                        continue;
                    }
                    self.with_mob(eid, |s, m| {
                        if s.hurt(m, dmg, "entity_explosion", source, None) {
                            m.motion = [m.motion[0] + kb[0], m.motion[1] + kb[1], m.motion[2] + kb[2]];
                        }
                    });
                }
            }
        }
        if !breaks && !fire {
            return;
        }
        let r = (power * 1.3).ceil() as i32;
        let c = [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32];
        let air = self.gd.air_id;
        let mut broken = Vec::new();
        for dx in -r..=r {
            for dy in -r..=r {
                for dz in -r..=r {
                    let d = ((dx * dx + dy * dy + dz * dz) as f32).sqrt();
                    if d > power * json::rand_f(0.7, 1.3) {
                        continue;
                    }
                    let b = [c[0] + dx, c[1] + dy, c[2] + dz];
                    if !self.worlds.worlds[world].is_loaded(b) {
                        continue;
                    }
                    let id = self.worlds.worlds[world].get_block(b, &self.gd);
                    if id == air {
                        if fire && rand::random::<u32>() % 3 == 0 && self.class_at(world, [b[0], b[1] - 1, b[2]]) & nav::SOLID != 0 {
                            broken.push((b, None));
                        }
                        continue;
                    }
                    if !breaks || items::blast_proof(self.gd.block_name(id)) {
                        continue;
                    }
                    broken.push((b, Some(id)));
                }
            }
        }
        let fire_id = self.gd.default_block("minecraft:fire");
        for (b, old) in broken {
            match old {
                Some(id) => {
                    self.change_block(world, b, air);
                    if rand::random::<f32>() < 1.0 / power.max(1.0) {
                        if let Some(ce) = self.gd.block_to_item.get(&id).cloned() {
                            if let Some(it) = self.simple_item(&ce.name, 1) {
                                self.drop_at(world, [b[0] as f32 + 0.5, b[1] as f32 + 0.5, b[2] as f32 + 0.5], it);
                            }
                        }
                    }
                }
                None => {
                    if let Some(f) = fire_id {
                        self.change_block(world, b, f);
                    }
                }
            }
        }
    }

    /// Mobs of the same kind (or the families of `broadcast_targets`) join the fight.
    pub(crate) fn broadcast_anger(&mut self, m: &Mob) {
        let Some(a) = m.comps.get("minecraft:angry") else { return };
        if !boolean(a, "broadcast_anger", false) {
            return;
        }
        let Some(target) = m.target else { return };
        let r = num(a, "broadcast_range", 20.0);
        let families = str_list(a.get("broadcast_targets"));
        let id = m.def.id.clone();
        for o in self.mobs.iter_mut() {
            if !o.alive() || o.world != m.world || o.entity_id == m.entity_id || o.target.is_some() || dist(o.pos, m.pos) > r {
                continue;
            }
            let same = if families.is_empty() { o.def.id == id } else { families.iter().any(|f| o.t.is(f)) };
            if same && !(o.t.tamed && Some(target) == o.owner.as_ref().and_then(|_| None)) {
                o.target = Some(target);
            }
        }
    }

    // ---------------------------------------------------------------- right click

    pub(crate) fn interact(&mut self, sid: SessionId, m: &mut Mob) {
        let Some(p) = self.players.get(&sid) else { return };
        let gm = p.gamemode;
        let pname = p.name.clone();
        let held = p.inventory.held_item().clone();
        let held_name = self.stack_name(&held);
        let sneaking = p.actor_state.sneaking;
        let now = self.ticks;
        dbg(|| format!("{} uses {} on {}", pname, held_name, m.def.id));
        if now < m.interact_ready {
            return;
        }
        let other = Some(Ent::Player(sid));
        // armor stand: the held item goes on it (its old one comes back), an empty hand takes
        // the hand item or the highest armour piece
        if m.def.id == "minecraft:armor_stand" {
            if gm == 6 {
                return;
            }
            if sneaking {
                m.pose = (m.pose + 1) % 13; // sneak + use: next pose
                self.swing_arm(sid);
                return;
            }
            if !held.is_empty() {
                let mut one = held.clone();
                one.count = 1;
                let old = self.equip_item(m, one, &held_name);
                if gm != 1 {
                    self.consume_held(sid, gm);
                }
                if let Some(o) = old {
                    self.give_stack(sid, o);
                }
            } else {
                let taken = if !m.equip.hand.is_empty() {
                    std::mem::take(&mut m.equip.hand)
                } else if !m.equip.offhand.is_empty() {
                    std::mem::take(&mut m.equip.offhand)
                } else if let Some(slot) = m.equip.armor.iter().position(|a| !a.is_empty()) {
                    std::mem::take(&mut m.equip.armor[slot])
                } else {
                    return;
                };
                self.give_stack(sid, taken);
            }
            m.equip.drop_chance = [1.0; 6];
            self.swing_arm(sid);
            return;
        }
        if self.use_lead(sid, m, &held_name, gm) {
            return;
        }
        // a spawn egg of the same mob makes a baby
        if super::super::useitems::egg_entity(&held_name).as_deref() == Some(m.def.id.as_str()) {
            let at = [m.pos[0] + 0.3, m.pos[1], m.pos[2] + 0.3];
            if self.spawn_entity(m.world, &held_name_entity(&held_name), at, Some("minecraft:entity_born")).is_some() && gm != 1 {
                self.consume_held(sid, gm);
            }
            self.swing_arm(sid);
            return;
        }
        // dye on a dyeable mob (sheep): switch to the group with that colour
        if let Some(colour) = held_name.strip_prefix("minecraft:").and_then(|n| n.strip_suffix("_dye")) {
            let idx = super::WOOL.iter().position(|w| *w == colour).or(match colour {
                "bone_meal" | "white" => Some(0),
                "ink_sac" | "black" => Some(15),
                "cocoa_beans" | "brown" => Some(12),
                "lapis_lazuli" | "blue" => Some(11),
                _ => None,
            });
            if let (Some(idx), true) = (idx, m.comps.contains_key("minecraft:is_dyeable")) {
                if m.t.color != Some(idx as u8) {
                    let groups: Vec<(String, Option<i32>)> = m
                        .def
                        .groups
                        .iter()
                        .filter_map(|(k, g)| g.get("minecraft:color").map(|c| (k.clone(), json::as_num(c.get("value")).map(|v| v as i32))))
                        .collect();
                    if let Some((target, _)) = groups.iter().find(|(_, v)| *v == Some(idx as i32)) {
                        m.groups.retain(|g| !groups.iter().any(|(k, _)| k == g));
                        m.groups.push(target.clone());
                        self.rebuild_mob(m);
                        if gm != 1 {
                            self.consume_held(sid, gm);
                        }
                        self.swing_arm(sid);
                        return;
                    }
                }
            }
        }
        // name tags
        if held_name == "minecraft:name_tag" {
            if let Some(n) = items::custom_name(&held) {
                m.name_tag = Some(n);
                m.persistent = true;
                if gm != 1 {
                    self.consume_held(sid, gm);
                }
                self.swing_arm(sid);
            }
            return;
        }
        // minecraft:interact entries (shearing, milking, buckets, dyes...)
        if let Some(inter) = m.comps.get("minecraft:interact").cloned() {
            let list = match inter.get("interactions") {
                Some(Value::Array(a)) => a.clone(),
                Some(o @ Value::Object(_)) => vec![o.clone()],
                _ => vec![inter.clone()],
            };
            for it in list {
                let on = it.get("on_interact").cloned().unwrap_or(Value::Null);
                let ok = filters::test(on.get("filters"), &Cx::new(self, m).with_other(other));
                if !ok {
                    continue;
                }
                m.interact_ready = now + (num(&it, "cooldown", 0.0) * 20.0) as u64;
                if let Some(ev) = on.get("event").and_then(|e| e.as_str()).map(String::from) {
                    let target = string(&on, "target").unwrap_or("self").to_string();
                    self.fire(m, &ev, &target, other, 0);
                }
                if let Some(table) = it.get("spawn_items").and_then(|s| string(s, "table")).map(String::from) {
                    let cx = loot::LootCx { color: m.t.color.unwrap_or(0), props: Some(&m.props), difficulty: self.cfg.difficulty, ..Default::default() };
                    for drop in self.roll_loot(&table, &cx) {
                        self.drop_at(m.world, [m.pos[0], m.pos[1] + m.t.height * 0.7, m.pos[2]], drop);
                    }
                }
                let sounds = str_list(it.get("play_sounds"));
                for s in sounds {
                    self.mob_sound(m, &s);
                }
                if gm != 1 {
                    let hurt = num(&it, "hurt_item", 0.0) as u32;
                    if hurt > 0 {
                        self.wear_held(sid, hurt);
                    }
                    if boolean(&it, "use_item", false) {
                        self.consume_held(sid, gm);
                    }
                    if let Some(tr) = string(&it, "transform_to_item") {
                        let (name, data) = item_ref(tr);
                        let name = match (name.as_str(), data) {
                            ("minecraft:bucket", Some(1)) => "minecraft:milk_bucket".to_string(),
                            ("minecraft:bowl", _) if m.def.id.contains("mooshroom") => "minecraft:mushroom_stew".to_string(),
                            _ => name,
                        };
                        self.give_back(sid, &name, 0);
                    }
                } else if let Some(tr) = string(&it, "transform_to_item") {
                    let (name, data) = item_ref(tr);
                    if name == "minecraft:bucket" && data == Some(1) {
                        self.give_back(sid, "minecraft:milk_bucket", 0);
                    }
                }
                if let Some(add) = it.get("add_items").and_then(|a| string(a, "table")).map(String::from) {
                    let cx = loot::LootCx { difficulty: self.cfg.difficulty, ..Default::default() };
                    for drop in self.roll_loot(&add, &cx) {
                        let n = self.stack_name(&drop);
                        self.give_back(sid, &n, drop.meta);
                    }
                }
                if boolean(&it, "swing", true) {
                    self.swing_arm(sid);
                }
                return;
            }
        }
        let consume = |s: &mut Server| {
            if gm != 1 {
                s.consume_held(sid, gm);
            }
            s.swing_arm(sid);
        };
        // taming
        if !m.t.tamed {
            if let Some(tm) = m.comps.get("minecraft:tameable").cloned() {
                if item_list(tm.get("tame_items")).contains(&held_name) {
                    consume(self);
                    if chance(num(&tm, "probability", 1.0)) {
                        m.owner = Some(pname.clone());
                        m.persistent = true;
                        m.target = None;
                        self.broadcast(&protocol::actor_event(m.entity_id, 7, 0), |_, q| q.world == m.world);
                        self.fire_ref(m, tm.get("tame_event"), other);
                        m.sitting = m.t.sittable;
                    } else {
                        self.broadcast(&protocol::actor_event(m.entity_id, 6, 0), |_, q| q.world == m.world);
                    }
                    return;
                }
            }
            if let Some(tr) = m.comps.get("minecraft:trust").cloned() {
                if item_list(tr.get("trust_items")).contains(&held_name) && chance(num(&tr, "probability", 0.33)) {
                    m.trusted.push(pname.clone());
                    consume(self);
                    self.fire_ref(m, tr.get("trust_event"), other);
                    return;
                }
            }
        }
        // healing
        if let Some(h) = m.comps.get("minecraft:healable").cloned() {
            for e in h.get("items").and_then(|i| i.as_array()).into_iter().flatten() {
                let (name, _) = item_ref(string(e, "item").unwrap_or(""));
                if name == held_name && (m.health < m.t.max_health || boolean(&h, "force_use", false)) {
                    m.health = (m.health + num(e, "heal_amount", 1.0)).min(m.t.max_health);
                    consume(self);
                    return;
                }
            }
        }
        // breeding
        if let Some(b) = m.comps.get("minecraft:breedable").cloned() {
            let items = item_list(b.get("breed_items"));
            let tame_ok = !boolean(&b, "require_tame", false) || m.t.tamed;
            let filt_ok = filters::test(b.get("love_filters"), &Cx::new(self, m).with_other(other));
            if items.contains(&held_name) && !m.t.baby && m.love == 0 && m.love_cooldown == 0 && tame_ok && filt_ok {
                m.love = 600;
                consume(self);
                return;
            }
        }
        // babies grow faster when fed
        if let Some(a) = m.comps.get("minecraft:ageable").cloned() {
            let mut growth = None;
            match a.get("feed_items") {
                Some(Value::Array(list)) => {
                    for e in list {
                        match e {
                            Value::String(s) if item_ref(s).0 == held_name => growth = Some(0.1),
                            Value::Object(_) if item_ref(string(e, "item").unwrap_or("")).0 == held_name => growth = Some(num(e, "growth", 0.1)),
                            _ => {}
                        }
                    }
                }
                Some(Value::String(s)) if item_ref(s).0 == held_name => growth = Some(0.1),
                _ => {}
            }
            if let Some(g) = growth {
                m.grow_ticks += (num(&a, "duration", 1200.0) * 20.0 * g) as u32;
                consume(self);
                return;
            }
        }
        // bartering (piglins)
        if let Some(bt) = m.comps.get("minecraft:barter").cloned() {
            if held_name == "minecraft:gold_ingot" && m.barter_due.is_none() && !m.t.baby {
                consume(self);
                m.barter_due = Some(now + 120);
                let table = string(&bt, "barter_table").unwrap_or("loot_tables/entities/piglin_barter.json").to_string();
                let cx = loot::LootCx { difficulty: self.cfg.difficulty, ..Default::default() };
                for drop in self.roll_loot(&table, &cx) {
                    self.drop_at(m.world, [m.pos[0], m.pos[1] + 1.0, m.pos[2]], drop);
                }
                m.barter_due = None;
                return;
            }
        }
        // saddles and horse armour, then getting on
        if self.try_equip_mob(sid, m, &held_name, gm) {
            return;
        }
        if self.try_mount(sid, m, sneaking) {
            return;
        }
        // the owner makes a pet sit or stand
        if m.t.sittable && m.t.tamed && m.owner.as_deref() == Some(pname.as_str()) && !sneaking {
            m.sitting = !m.sitting;
            m.path.clear();
            let ev = m.comps.get("minecraft:sittable").and_then(|s| if m.sitting { s.get("sit_event") } else { s.get("stand_event") }).cloned();
            self.fire_ref(m, ev.as_ref(), other);
        }
    }
}

fn held_name_entity(egg: &str) -> String {
    super::super::useitems::egg_entity(egg).unwrap_or_default()
}

impl Server {
    /// A player throws a projectile defined in the behaviour pack (wind charge...) along their look.
    pub(crate) fn player_throw(&mut self, sid: SessionId, id: &str) -> bool {
        let Some(def) = self.mobsys.reg.entity(id).cloned() else { return false };
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, pos, rot, eid, name) = (p.world, p.pos, p.rot, p.entity_id, p.name.clone());
        let data = Rc::new(parse_proj(&def));
        let (pitch, yaw) = (rot[0].to_radians(), rot[1].to_radians());
        let dir = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
        let motion = [dir[0] * data.speed, dir[1] * data.speed, dir[2] * data.speed];
        let start = [pos[0] + dir[0] * 0.6, pos[1] - 0.1 + dir[1] * 0.6, pos[2] + dir[2] * 0.6];
        let peid = self.next_entity_id;
        self.next_entity_id += 1;
        self.broadcast(&protocol::add_projectile(peid, id, start, motion, eid, None), |_, q| q.world == world);
        self.broadcast(&protocol::level_sound_event("throw", start, -1), |_, q| q.world == world);
        self.mob_projectiles.push(MobProjectile { entity_id: peid, world, data, owner: eid, owner_name: name, pos: start, motion, age: 0, target: None, aux: None, stuck: 0 });
        true
    }
}
