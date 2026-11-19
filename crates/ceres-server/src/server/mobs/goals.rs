//! Goals (`minecraft:behavior.*`): each tick the running goals continue or stop, then the
//! others are tried from the highest priority (lowest number) down; a goal starts when no running
//! goal with a better or equal priority uses one of its controls (move, look, jump, target), and
//! it stops the worse ones it conflicts with.

use super::filters::{self, Cx, Subj, dist};
use super::json::*;
use super::nav::{SOLID, WATER, LAVA};
use super::traits::{G, NavKind};
use super::*;
use serde_json::Value;

/// The `entity_types` entries of a goal: filters plus distance / speed settings.
fn entity_types(p: &Value) -> Vec<Value> {
    match p.get("entity_types") {
        Some(Value::Array(a)) => a.clone(),
        Some(o @ Value::Object(_)) => vec![o.clone()],
        _ => Vec::new(),
    }
}

impl Server {
    pub(crate) fn run_goals(&mut self, m: &mut Mob) {
        m.ctl = Ctl::default();
        m.panicking = false;
        m.avoiding = false;
        m.interested = false;
        m.charging = false;
        let n = m.goals.len();
        // stop the goals that cannot go on
        for gi in 0..n {
            if m.goals[gi].st.running && !self.goal_ok(m, gi) {
                self.goal_stop(m, gi);
            }
        }
        // start new ones
        for gi in 0..n {
            if m.goals[gi].st.running {
                continue;
            }
            let (kind, prio) = (m.goals[gi].def.kind, m.goals[gi].def.priority);
            let c = kind.controls();
            if c == 0 && kind != G::TimerFlag {
                continue;
            }
            if m.goals[gi].st.next > self.ticks {
                continue;
            }
            let blocked = m.goals.iter().any(|o| o.st.running && o.def.kind.controls() & c != 0 && o.def.priority <= prio);
            if blocked {
                continue;
            }
            if self.goal_start(m, gi) {
                m.goals[gi].st.running = true;
                for o in 0..n {
                    if o != gi && m.goals[o].st.running && m.goals[o].def.kind.controls() & c != 0 && m.goals[o].def.priority > prio {
                        self.goal_stop(m, o);
                    }
                }
            }
        }
        // run them
        for gi in 0..n {
            if m.goals[gi].st.running && !self.goal_tick(m, gi) {
                self.goal_stop(m, gi);
            }
        }
        // a target that cannot be attacked any more is dropped
        if let Some(t) = m.target {
            if !self.valid_target(m, t, m.t.follow_range.max(16.0) * 2.0) {
                m.target = None;
            }
        }
        // target changes fire on_target_acquired / on_target_escape
        if m.target != m.last_target {
            let (new, old) = (m.target, m.last_target);
            dbg(|| format!("{} {} target {:?} -> {:?}", m.def.id, m.entity_id, old, new));
            m.last_target = m.target;
            if new.is_some() && old.is_none() {
                let ev = m.comps.get("minecraft:on_target_acquired").cloned();
                self.fire_ref(m, ev.as_ref(), new);
                self.broadcast_anger(m);
            } else if new.is_none() {
                let ev = m.comps.get("minecraft:on_target_escape").cloned();
                self.fire_ref(m, ev.as_ref(), old);
            }
        }
    }

    fn goal_stop(&mut self, m: &mut Mob, gi: usize) {
        let g = &mut m.goals[gi];
        let kind = g.def.kind;
        g.st.running = false;
        g.st.t = 0;
        g.st.phase = 0;
        let cooldown = match kind {
            G::Stroll | G::Swim | G::Fly | G::Hover => 0,
            G::Melee | G::Ranged => 0,
            _ => 10,
        };
        g.st.next = self.ticks + cooldown;
        if kind == G::Sleep {
            m.sleeping = false;
        }
        if kind == G::TimerFlag {
            let ev = m.goals[gi].def.p.get("on_end").cloned();
            self.fire_ref(m, ev.as_ref(), None);
            let (lo, hi) = range(m.goals[gi].def.p.get("cooldown_range"), (10.0, 10.0));
            m.goals[gi].st.next = self.ticks + (rand_f(lo, hi) * 20.0) as u64;
        }
        if matches!(kind, G::HurtByTarget | G::NearestAttackable | G::OwnerHurtByTarget | G::OwnerHurtTarget | G::DefendTrusted | G::DefendVillage | G::TargetHighestDamage)
            && m.goals.iter().all(|o| !o.st.running || o.def.kind.controls() & traits::TARGET == 0)
        {
            m.target = None;
        }
    }

    /// Whether a running goal may keep going (cheap checks; the tick does the rest).
    fn goal_ok(&mut self, m: &Mob, gi: usize) -> bool {
        let g = &m.goals[gi];
        if !filters::test(g.def.p.get("filters"), &Cx::new(self, m)) {
            return false;
        }
        match g.def.kind {
            G::HurtByTarget | G::NearestAttackable | G::OwnerHurtByTarget | G::OwnerHurtTarget | G::DefendTrusted | G::DefendVillage | G::TargetHighestDamage => {
                m.target.is_some_and(|t| self.valid_target(m, t, m.t.follow_range.max(16.0) * 1.5))
            }
            G::Melee | G::Ranged | G::Leap | G::Ram | G::Swell => m.target.is_some(),
            G::StayWhileSitting => m.sitting,
            _ => true,
        }
    }

    /// May this mob go after `t`?
    pub(crate) fn valid_target(&self, m: &Mob, t: Ent, range: f32) -> bool {
        match self.subj(t) {
            Some(Subj::P(_, p)) => {
                (p.gamemode == 0 || p.gamemode == 2)
                    && p.world == m.world
                    && dist(m.pos, [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]) <= range
                    && !(self.cfg.difficulty == 0 && m.hostile())
                    && m.owner.as_deref() != Some(p.name.as_str())
            }
            Some(Subj::M(o)) => o.world == m.world && o.entity_id != m.entity_id && dist(m.pos, o.pos) <= range,
            None => false,
        }
    }

    fn target_pos(&self, m: &Mob) -> Option<[f32; 3]> {
        m.target.and_then(|t| self.subj(t)).map(|s| s.feet())
    }

    fn goal_start(&mut self, m: &mut Mob, gi: usize) -> bool {
        let g = m.goals[gi].def.clone();
        let p = &g.p;
        if !filters::test(p.get("filters"), &Cx::new(self, m)) {
            return false;
        }
        let now = self.ticks;
        let stagger = (now + m.entity_id as u64) % 10 == 0;
        let st = GoalSt { next: m.goals[gi].st.next, ..Default::default() };
        m.goals[gi].st = st;
        match g.kind {
            // ------------------------------------------------ targets
            G::HurtByTarget | G::TargetHighestDamage => {
                let Some(by) = m.last_hurt_by else { return false };
                if now > m.last_hurt_tick + 100 || m.target == Some(by) {
                    return false;
                }
                let Some(o) = self.subj(by) else { return false };
                let types = entity_types(p);
                let ok = types.is_empty() || types.iter().any(|t| filters::test(t.get("filters"), &Cx::new(self, m).with_other_subj(o)));
                if !ok || !self.valid_target(m, by, 64.0) {
                    return false;
                }
                m.target = Some(by);
                if boolean(p, "alert_same_type", false) {
                    let id = m.def.id.clone();
                    for o in self.mobs.iter_mut() {
                        if o.alive() && o.def.id == id && o.world == m.world && o.target.is_none() && dist(o.pos, m.pos) < 16.0 {
                            o.target = Some(by);
                        }
                    }
                }
                true
            }
            G::NearestAttackable => {
                let interval = num(p, "scan_interval", 10.0).max(1.0) as u64;
                if (now + m.entity_id as u64) % interval != 0 {
                    return false;
                }
                let chance_ = num(p, "attack_interval", 0.0);
                if chance_ > 0.0 && rand::random::<f32>() * chance_ * 20.0 > 10.0 {
                    return false;
                }
                let Some(t) = self.find_attack_target(m, p) else { return false };
                m.target = Some(t);
                true
            }
            G::OwnerHurtByTarget | G::OwnerHurtTarget | G::DefendTrusted => {
                if !m.t.tamed && g.kind != G::DefendTrusted {
                    return false;
                }
                let names: Vec<String> = if g.kind == G::DefendTrusted { m.trusted.clone() } else { m.owner.iter().cloned().collect() };
                for name in names {
                    let Some((sid, _)) = self.online_named(&name) else { continue };
                    let table = if g.kind == G::OwnerHurtTarget { &self.mobsys.player_attacked } else { &self.mobsys.player_hurt_by };
                    if let Some((e, at)) = table.get(&sid).copied() {
                        if now < at + 100 && Some(e) != m.target && e != Ent::Mob(m.entity_id) && self.valid_target(m, e, 48.0) {
                            let ok = match self.subj(e) {
                                Some(Subj::M(o)) => !(o.t.tamed && o.owner == m.owner) && !o.t.is("creeper"),
                                _ => true,
                            };
                            if ok {
                                m.target = Some(e);
                                return true;
                            }
                        }
                    }
                }
                false
            }
            G::DefendVillage => {
                let found = self.mobsys.villager_attacks.iter().rev().find(|(w, at, _, _)| *w == m.world && dist(*at, m.pos) < 32.0).map(|v| v.2);
                match found {
                    Some(e) if self.valid_target(m, e, 48.0) => {
                        m.target = Some(e);
                        true
                    }
                    _ => false,
                }
            }
            G::TargetWhenPushed => false,
            // ------------------------------------------------ staying alive
            G::Float => m.env.in_water || m.env.in_lava,
            G::Panic => {
                let recent = now < m.last_hurt_tick + 60 && m.last_hurt_tick > 0;
                let sources = str_list(p.get("damage_sources"));
                let source_ok = sources.is_empty() || sources.iter().any(|s| s == "all" || *s == m.last_cause);
                let force = boolean(p, "force", false);
                let on_fire = m.fire_ticks > 0 && !m.t.fire_immune;
                if !(force || (recent && source_ok) || on_fire) {
                    return false;
                }
                let away = m.last_hurt_by.and_then(|e| self.subj(e)).map(|s| s.feet());
                let dest = self.flee_spot(m, away, 7);
                m.goals[gi].st.dest = dest;
                dest.is_some()
            }
            G::AvoidMob => {
                if !stagger && m.age % 5 != 0 {
                    return false;
                }
                let types = entity_types(p);
                let default_max = num(p, "max_dist", 6.0);
                let mut best: Option<(Ent, f32, [f32; 3], usize)> = None;
                for e in self.entities_near(m.world, m.pos, 16.0, Some(m.entity_id)) {
                    let Some(o) = self.subj(e) else { continue };
                    if let Subj::P(_, pl) = o {
                        if pl.gamemode == 1 || pl.gamemode == 6 {
                            continue;
                        }
                    }
                    let d = dist(o.feet(), m.pos);
                    for (ti, t) in types.iter().enumerate() {
                        let maxd = num(t, "max_dist", default_max);
                        if d <= maxd && best.is_none_or(|b| d < b.1) && filters::test(t.get("filters"), &Cx::new(self, m).with_other_subj(o)) {
                            best = Some((e, d, o.feet(), ti));
                        }
                    }
                }
                let Some((e, _, at, ti)) = best else { return false };
                let flee = num(&types[ti], "max_flee", num(p, "max_flee", 10.0)) as i32;
                let Some(dest) = self.flee_spot(m, Some(at), flee.max(4)) else { return false };
                let st = &mut m.goals[gi].st;
                st.dest = Some(dest);
                st.ent = Some(e);
                st.n = ti as u32;
                true
            }
            G::AvoidBlock => {
                if (now + m.entity_id as u64) % (num(p, "tick_interval", 5.0).max(1.0) as u64 * 4) != 0 {
                    return false;
                }
                let blocks = item_list(p.get("target_blocks"));
                let r = num(p, "search_range", 8.0) as i32;
                let Some(b) = self.find_block(m.world, m.pos, r, num(p, "search_height", 4.0) as i32, &blocks, 48) else { return false };
                let at = [b[0] as f32 + 0.5, b[1] as f32, b[2] as f32 + 0.5];
                let dest = self.flee_spot(m, Some(at), r.max(4));
                m.goals[gi].st.dest = dest;
                dest.is_some()
            }
            G::FleeSun => {
                let name = g.name.as_str();
                let wants = if name.ends_with("flee_sun") || name.ends_with("find_cover") {
                    self.is_day(m.world) && m.env.sky && (m.fire_ticks > 0 || name.ends_with("find_cover")) && !self.raining(m.world)
                } else {
                    !self.is_day(m.world) && m.env.sky
                };
                if !wants || rand::random::<u32>() % 10 != 0 {
                    return false;
                }
                let dest = self.shade_spot(m, 10);
                m.goals[gi].st.dest = dest;
                dest.is_some()
            }
            G::Breathe => m.env.underwater && m.air < m.t.air_ticks / 3,
            G::MoveToWater | G::MoveToLand | G::MoveToLava => {
                let water = g.kind == G::MoveToWater;
                let lava = g.kind == G::MoveToLava;
                let in_it = if lava { m.env.in_lava } else { m.env.in_water };
                if (water || lava) == in_it || (now + m.entity_id as u64) % (num(p, "search_count", 10.0).max(1.0) as u64 * 2) != 0 {
                    return false;
                }
                let r = num(p, "search_range", 16.0) as i32;
                let hgt = num(p, "search_height", 4.0) as i32;
                let found = if water || lava {
                    let want = if lava { LAVA } else { WATER };
                    self.find_class(m.world, m.pos, r, hgt, want, 64)
                } else {
                    self.random_ground_spot(m, r, hgt, m.pos).map(|p| [p[0] as i32, p[1] as i32, p[2] as i32])
                };
                let Some(b) = found else { return false };
                m.goals[gi].st.dest = Some([b[0] as f32 + 0.5, b[1] as f32, b[2] as f32 + 0.5]);
                true
            }
            // ------------------------------------------------ animals
            G::Tempt => {
                if !stagger && m.age % 3 != 0 {
                    return false;
                }
                let items = item_list(p.get("items"));
                let r = num(p, "within_radius", 10.0).max(2.0);
                let found = self.players.iter().filter(|(_, q)| q.world == m.world && q.state == State::Spawned && !q.dead && q.gamemode != 6).find_map(|(sid, q)| {
                    let feet = [q.pos[0], q.pos[1] - EYE_HEIGHT, q.pos[2]];
                    let held = self.stack_name(q.inventory.held_item());
                    (dist(feet, m.pos) <= r && items.contains(&held)).then_some(*sid)
                });
                let Some(sid) = found else { return false };
                m.goals[gi].st.ent = Some(Ent::Player(sid));
                true
            }
            G::Breed => {
                if m.love == 0 || m.t.baby {
                    return false;
                }
                let Some(partner) = self.find_partner(m) else { return false };
                m.goals[gi].st.ent = Some(Ent::Mob(partner));
                true
            }
            G::FollowParent => {
                if !m.t.baby || m.age % 20 != 0 {
                    return false;
                }
                let parent = self
                    .mobs
                    .iter()
                    .filter(|o| o.alive() && o.world == m.world && o.def.id == m.def.id && !o.t.baby)
                    .map(|o| (o.entity_id, dist(o.pos, m.pos)))
                    .filter(|(_, d)| *d < 8.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                match parent {
                    Some((eid, d)) if d > 3.0 => {
                        m.goals[gi].st.ent = Some(Ent::Mob(eid));
                        true
                    }
                    _ => false,
                }
            }
            G::FollowOwner | G::TeleportToOwner => {
                if !m.t.tamed || m.sitting {
                    return false;
                }
                let Some((_, o)) = m.owner.as_ref().and_then(|o| self.online_named(o)) else { return false };
                if o.world != m.world || o.gamemode == 6 {
                    return false;
                }
                let d = dist([o.pos[0], o.pos[1] - EYE_HEIGHT, o.pos[2]], m.pos);
                if g.kind == G::TeleportToOwner { d > 12.0 } else { d > num(p, "start_distance", 10.0) }
            }
            G::StayWhileSitting => m.sitting,
            G::FollowMob => {
                if m.age % 20 != m.entity_id as u64 % 20 {
                    return false;
                }
                let r = num(p, "search_range", 10.0);
                let name = g.name.as_str();
                let found = if name.ends_with("swim_with_entity") {
                    self.players
                        .iter()
                        .filter(|(_, q)| q.world == m.world && q.actor_state.swimming && q.gamemode != 6)
                        .find(|(_, q)| dist([q.pos[0], q.pos[1] - EYE_HEIGHT, q.pos[2]], m.pos) < r)
                        .map(|(s, _)| Ent::Player(*s))
                } else {
                    let same = name.ends_with("follow_caravan");
                    let captain = name.ends_with("follow_target_captain");
                    self.mobs
                        .iter()
                        .filter(|o| o.alive() && o.world == m.world && dist(o.pos, m.pos) < r.max(8.0))
                        .filter(|o| if captain { o.t.captain } else if same { o.def.id == m.def.id && o.entity_id < m.entity_id } else { o.def.id != m.def.id && o.t.nav == m.t.nav })
                        .min_by(|a, b| dist(a.pos, m.pos).total_cmp(&dist(b.pos, m.pos)))
                        .map(|o| Ent::Mob(o.entity_id))
                };
                let Some(e) = found else { return false };
                m.goals[gi].st.ent = Some(e);
                true
            }
            G::Beg => {
                let items = item_list(p.get("items"));
                let wants_emerald = g.name.ends_with("trade_interest") || g.name.ends_with("look_at_trading_player");
                let r = num(p, "look_distance", 8.0);
                let found = self.players.iter().filter(|(_, q)| q.world == m.world && q.state == State::Spawned && !q.dead).find_map(|(sid, q)| {
                    let feet = [q.pos[0], q.pos[1] - EYE_HEIGHT, q.pos[2]];
                    let held = self.stack_name(q.inventory.held_item());
                    let likes = if wants_emerald { held == "minecraft:emerald" } else { items.contains(&held) };
                    (dist(feet, m.pos) <= r && likes).then_some(*sid)
                });
                let Some(sid) = found else { return false };
                let (lo, hi) = range(p.get("look_time"), (2.0, 4.0));
                let st = &mut m.goals[gi].st;
                st.ent = Some(Ent::Player(sid));
                st.n = (rand_f(lo, hi) * 20.0) as u32;
                true
            }
            // ------------------------------------------------ wandering
            G::Stroll => {
                let interval = num(p, "interval", 120.0).max(1.0);
                if m.env.in_water && m.t.nav == NavKind::Swim {
                    return false;
                }
                if rand::random::<f32>() * interval > 1.0 {
                    return false;
                }
                let xz = num(p, "xz_dist", 10.0) as i32;
                let y = num(p, "y_dist", 7.0) as i32;
                let around = self.home_center(m).unwrap_or(m.pos);
                let dest = if m.t.can_fly && !m.t.gravity {
                    self.random_open_spot(m, xz, y, false, around)
                } else {
                    self.random_ground_spot(m, xz, y, around)
                };
                m.goals[gi].st.dest = dest;
                dest.is_some()
            }
            G::Swim => {
                if !m.env.in_water {
                    return false;
                }
                let interval = num(p, "interval", 120.0).max(1.0);
                if g.name.ends_with("swim_idle") {
                    if rand::random::<u32>() % 200 == 0 {
                        m.goals[gi].st.n = (num(p, "idle_time", 5.0) * 20.0) as u32;
                        return true;
                    }
                    return false;
                }
                if rand::random::<f32>() * interval.min(80.0) > 1.0 {
                    return false;
                }
                let xz = num(p, "xz_dist", 10.0) as i32;
                let y = num(p, "y_dist", 7.0) as i32;
                let dest = self.random_open_spot(m, xz, y, true, m.pos);
                m.goals[gi].st.dest = dest;
                dest.is_some()
            }
            G::Fly => {
                if !m.t.can_fly {
                    return false;
                }
                if rand::random::<u32>() % 20 != 0 {
                    return false;
                }
                let xz = num(p, "xz_dist", 10.0) as i32;
                let y = num(p, "y_dist", 7.0) as i32;
                let mut around = m.pos;
                if let Some(off) = p.get("y_offset") {
                    around[1] += as_num(Some(off)).unwrap_or(0.0);
                }
                if g.name.ends_with("circle_around_anchor") {
                    // circle above where it is (phantoms)
                    let st = &mut m.goals[gi].st;
                    st.dest = Some([m.pos[0], m.pos[1], m.pos[2]]);
                    st.f = (rand::random::<u32>() % 360) as f32;
                    st.phase = 1;
                    return true;
                }
                let dest = self.random_open_spot(m, xz, y, false, around);
                m.goals[gi].st.dest = dest;
                dest.is_some()
            }
            G::Hover => {
                if rand::random::<f32>() * num(p, "interval", 120.0).max(1.0) > 1.0 {
                    return false;
                }
                let xz = num(p, "xz_dist", 8.0) as i32;
                let y = num(p, "y_dist", 8.0) as i32;
                let around = self.home_center(m).unwrap_or(m.pos);
                let mut dest = self.random_open_spot(m, xz, y, false, around);
                if let Some(d) = dest.as_mut() {
                    // stay a little above the ground
                    let (lo, hi) = range(p.get("hover_height"), (1.0, 4.0));
                    let want = rand_f(lo, hi);
                    let b = [d[0].floor() as i32, d[1].floor() as i32, d[2].floor() as i32];
                    let mut ground = b[1];
                    for dy in 0..16 {
                        if self.class_at(m.world, [b[0], b[1] - dy, b[2]]) & SOLID != 0 {
                            ground = b[1] - dy + 1;
                            break;
                        }
                    }
                    d[1] = d[1].min(ground as f32 + want);
                }
                m.goals[gi].st.dest = dest;
                dest.is_some()
            }
            G::StayNearHome => {
                let Some(home) = self.home_center(m) else { return false };
                let radius = m.comps.get("minecraft:home").map(|h| num(h, "restriction_radius", 16.0)).unwrap_or(16.0);
                dist(home, m.pos) > radius && {
                    m.goals[gi].st.dest = Some(home);
                    true
                }
            }
            G::Breach => m.env.in_water && !m.env.underwater && rand::random::<f32>() * num(p, "interval", 10.0).max(1.0) * 20.0 < 1.0,
            G::SlimeHop => m.target.is_none() && (m.on_ground || m.env.in_water),
            // ------------------------------------------------ fighting
            G::Melee => {
                let Some(tp) = self.target_pos(m) else { return false };
                let name = g.name.as_str();
                // breezes and piglins holding ground only keep their distance
                let _ = name;
                dist(tp, m.pos) < m.t.follow_range.max(16.0) * 1.5
            }
            G::Leap => {
                let Some(tp) = self.target_pos(m) else { return false };
                let d = dist(tp, m.pos);
                m.on_ground && (2.0..=4.0).contains(&d) && rand::random::<u32>() % 5 == 0
            }
            G::Ranged => {
                let Some(tp) = self.target_pos(m) else { return false };
                let r = ranged_radius(p);
                dist(tp, m.pos) <= r.max(16.0) * 1.5
            }
            G::Swell => {
                let Some(tp) = self.target_pos(m) else { return m.fuse.is_some() };
                m.fuse.is_some() || dist(tp, m.pos) < num(p, "start_distance", 2.5)
            }
            G::Ram => {
                let Some(tp) = self.target_pos(m) else { return false };
                let d = dist(tp, m.pos);
                m.on_ground && d >= num(p, "min_ram_distance", 4.0) && d <= num(p, "ram_distance", 7.0)
            }
            // ------------------------------------------------ looking
            G::LookAtPlayer | G::LookAtEntity => {
                let prob = num(p, "probability", 0.02);
                if !chance(prob) {
                    return false;
                }
                let r = num(p, "look_distance", 8.0);
                let types = entity_types(p);
                let found = if g.kind == G::LookAtPlayer {
                    self.players
                        .iter()
                        .filter(|(_, q)| q.world == m.world && q.state == State::Spawned && !q.dead && q.gamemode != 6)
                        .map(|(s, q)| (Ent::Player(*s), dist([q.pos[0], q.pos[1] - EYE_HEIGHT, q.pos[2]], m.pos)))
                        .filter(|(_, d)| *d <= r)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|x| x.0)
                } else {
                    self.entities_near(m.world, m.pos, r, Some(m.entity_id))
                        .into_iter()
                        .find(|e| self.subj(*e).is_some_and(|o| types.iter().any(|t| filters::test(t.get("filters"), &Cx::new(self, m).with_other_subj(o)))))
                };
                let Some(e) = found else { return false };
                let (lo, hi) = range(p.get("look_time"), (2.0, 4.0));
                let st = &mut m.goals[gi].st;
                st.ent = Some(e);
                st.n = (rand_f(lo, hi) * 20.0) as u32;
                true
            }
            G::LookAtTarget => m.target.is_some(),
            G::LookAround => {
                if rand::random::<u32>() % 50 != 0 {
                    return false;
                }
                let (lo, hi) = range(p.get("look_time"), (2.0, 4.0));
                let a = (m.yaw + rand_f(-120.0, 120.0) + 90.0).to_radians();
                let st = &mut m.goals[gi].st;
                st.dest = Some([m.pos[0] + a.cos() * 5.0, m.pos[1] + m.t.height * 0.85, m.pos[2] + a.sin() * 5.0]);
                st.n = (rand_f(lo, hi) * 20.0) as u32;
                true
            }
            // ------------------------------------------------ blocks and items
            G::MoveToBlock | G::Harvest | G::StompEgg | G::RaidGarden | G::LayEgg => {
                if m.target.is_some() && g.kind != G::LayEgg {
                    return false;
                }
                let interval = num(p, "tick_interval", num(p, "interval", 20.0)).max(1.0) as u64;
                if (now + m.entity_id as u64) % (interval * 2) != 0 {
                    return false;
                }
                if !chance(num(p, "start_chance", 1.0)) {
                    return false;
                }
                let blocks = self.goal_blocks(m, &g);
                if blocks.is_empty() {
                    return false;
                }
                let r = num(p, "search_range", 16.0).min(24.0) as i32;
                let hgt = num(p, "search_height", 4.0).min(8.0) as i32;
                let Some(b) = self.find_block(m.world, m.pos, r, hgt, &blocks, 96) else { return false };
                let st = &mut m.goals[gi].st;
                st.block = Some(b);
                let above = g.kind == G::LayEgg || g.kind == G::StompEgg || g.kind == G::RaidGarden || g.kind == G::Harvest;
                st.dest = Some([b[0] as f32 + 0.5, b[1] as f32 + if above { 0.0 } else { 1.0 }, b[2] as f32 + 0.5]);
                true
            }
            G::EatBlock => {
                if !m.on_ground || m.target.is_some() {
                    return false;
                }
                let q = Self::molang_of(m);
                let ch = molang::value(p.get("success_chance"), 0.02, &q);
                if !chance(ch) {
                    return false;
                }
                let Some(b) = self.edible_block(m, p) else { return false };
                let st = &mut m.goals[gi].st;
                st.block = Some(b);
                st.n = (num(p, "time_until_eat", 1.8) * 20.0) as u32;
                let pk = protocol::actor_event(m.entity_id, 10, 0); // eating grass
                self.broadcast(&pk, |_, q| q.world == m.world);
                true
            }
            G::PickupItems => {
                if m.age % 10 != m.entity_id as u64 % 10 || m.target.is_some() {
                    return false;
                }
                if g.name.ends_with("take_block") {
                    return m.carried_block.is_none() && self.cfg.mob_griefing && rand::random::<u32>() % 20 == 0;
                }
                if g.name.ends_with("share_items") || g.name.ends_with("transport_items") {
                    return false;
                }
                let maxd = num(p, "max_dist", 3.0).max(1.0);
                let Some(i) = self.wanted_item(m, p, maxd) else { return false };
                let st = &mut m.goals[gi].st;
                st.n = self.items[i].entity_id as u32;
                st.dest = Some(self.items[i].pos);
                st.f = self.items[i].entity_id as f32;
                true
            }
            G::GiveToOwner => {
                if g.name.ends_with("drop_item_for") {
                    // cats bring a gift in the morning
                    let t = self.worlds.worlds[m.world].time.rem_euclid(24000);
                    return m.t.tamed && (0..200).contains(&t) && m.owner.is_some() && m.goals[gi].st.next < now && rand::random::<u32>() % 4 == 0;
                }
                !m.equip.hand.is_empty() && m.owner.is_some()
            }
            G::Sleep => {
                let name = g.name.as_str();
                let night = !self.is_day(m.world);
                if name.ends_with("nap") {
                    return self.is_day(m.world) && m.target.is_none() && rand::random::<u32>() % 400 == 0;
                }
                night && m.target.is_none() && rand::random::<u32>() % 40 == 0
            }
            // ------------------------------------------------ events and timers
            G::TimerFlag => {
                let ev = p.get("on_start").cloned();
                let (lo, hi) = range(p.get("duration_range"), (num(p, "duration", 5.0), num(p, "duration", 5.0)));
                m.goals[gi].st.n = (rand_f(lo, hi) * 20.0) as u32;
                self.fire_ref(m, ev.as_ref(), None);
                true
            }
            G::SendEvent => {
                let name = g.name.as_str();
                if name.ends_with("drink_potion") {
                    let need = (m.health < m.t.max_health * 0.5 && m.target.is_some()) || m.fire_ticks > 0 || m.env.underwater;
                    if need && rand::random::<u32>() % 10 == 0 {
                        m.goals[gi].st.n = 32;
                        return true;
                    }
                    return false;
                }
                if !name.ends_with("send_event") {
                    return false;
                }
                let Some(tp) = self.target_pos(m) else { return false };
                let d = dist(tp, m.pos);
                let choices = p.get("event_choices").and_then(|c| c.as_array()).cloned().unwrap_or_default();
                let valid: Vec<usize> = choices
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| d >= num(c, "min_activation_range", 0.0) && d <= num(c, "max_activation_range", 16.0))
                    .filter(|(_, c)| filters::test(c.get("filters"), &Cx::new(self, m)))
                    .map(|(i, _)| i)
                    .collect();
                if valid.is_empty() || rand::random::<u32>() % 20 != 0 {
                    return false;
                }
                let pick = valid[rand::random::<u32>() as usize % valid.len()];
                let st = &mut m.goals[gi].st;
                st.n = pick as u32;
                st.phase = 0;
                true
            }
            G::Summon => {
                if !g.name.ends_with("summon_entity") {
                    return false;
                }
                let Some(tp) = self.target_pos(m) else { return false };
                let d = dist(tp, m.pos);
                let choices = p.get("summon_choices").and_then(|c| c.as_array()).cloned().unwrap_or_default();
                let valid: Vec<usize> = choices
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| d >= num(c, "min_activation_range", 0.0) && d <= num(c, "max_activation_range", 32.0))
                    .filter(|(_, c)| filters::test(c.get("filters"), &Cx::new(self, m)))
                    .map(|(i, _)| i)
                    .collect();
                if valid.is_empty() || rand::random::<u32>() % 10 != 0 {
                    return false;
                }
                let pick = valid[rand::random::<u32>() as usize % valid.len()];
                m.goals[gi].st.n = pick as u32;
                true
            }
            G::Idle => {
                let name = g.name.as_str();
                let random = ["croak", "roll", "sneeze", "snacking", "lay_down", "random_sitting", "sniff"].iter().any(|s| name.ends_with(s));
                let play_dead = name.ends_with("play_dead") && m.env.in_water && now < m.last_hurt_tick + 20 && m.health < m.t.max_health * 0.5;
                if !(random && m.target.is_none() && rand::random::<u32>() % 600 == 0) && !play_dead {
                    return false;
                }
                m.goals[gi].st.n = if play_dead { 200 } else { json::rand_i(40, 120) as u32 };
                true
            }
            G::Dragon => g.name.ends_with("holdingpattern") || g.name.ends_with("strafeplayer") || g.name.ends_with("chargeplayer"),
            G::Passive => false,
        }
    }

    fn goal_tick(&mut self, m: &mut Mob, gi: usize) -> bool {
        let g = m.goals[gi].def.clone();
        let p = &g.p;
        let now = self.ticks;
        m.goals[gi].st.t += 1;
        let t = m.goals[gi].st.t;
        let speed = num(p, "speed_multiplier", 1.0);
        match g.kind {
            G::HurtByTarget | G::NearestAttackable | G::OwnerHurtByTarget | G::OwnerHurtTarget | G::DefendTrusted | G::DefendVillage | G::TargetHighestDamage => {
                if m.target.is_none() {
                    return false;
                }
                // look for a closer prey now and then
                if g.kind == G::NearestAttackable && boolean(p, "reselect_targets", false) && t % 20 == 0 {
                    if let Some(n) = self.find_attack_target(m, p) {
                        m.target = Some(n);
                    }
                }
                // lose a target it cannot see for too long
                if g.kind == G::NearestAttackable && boolean(p, "must_see", false) && t % 10 == 0 {
                    if let Some(tp) = self.target_pos(m) {
                        let seen = self.can_see(m.world, m.eye(), [tp[0], tp[1] + 1.5, tp[2]]);
                        if seen {
                            m.goals[gi].st.n = 0;
                        } else {
                            m.goals[gi].st.n += 10;
                            if m.goals[gi].st.n as f32 > num(p, "must_see_forget_duration", 3.0) * 20.0 {
                                m.target = None;
                                return false;
                            }
                        }
                    }
                }
                let persist = num(p, "persist_time", 0.0);
                let _ = persist;
                true
            }
            G::Float => {
                m.ctl.jump = true;
                m.env.in_water || m.env.in_lava
            }
            G::Panic => {
                m.panicking = true;
                let Some(dest) = m.goals[gi].st.dest else { return false };
                m.ctl.dest = Some(dest);
                m.ctl.speed = speed.max(1.0);
                if dist(dest, m.pos) < 1.2 || m.stuck > 20 {
                    let still = now < m.last_hurt_tick + 60 || (m.fire_ticks > 0 && !m.t.fire_immune);
                    if !still {
                        return false;
                    }
                    let away = m.last_hurt_by.and_then(|e| self.subj(e)).map(|s| s.feet());
                    m.goals[gi].st.dest = self.flee_spot(m, away, 7);
                }
                if t % 40 == 1 {
                    if let Some(s) = string(p, "panic_sound").map(String::from) {
                        self.mob_sound(m, &s);
                    }
                }
                t < 200
            }
            G::AvoidMob | G::AvoidBlock => {
                m.avoiding = true;
                let Some(dest) = m.goals[gi].st.dest else { return false };
                let types = entity_types(p);
                let ty = types.get(m.goals[gi].st.n as usize);
                let threat = m.goals[gi].st.ent.and_then(|e| self.subj(e)).map(|s| s.feet());
                let close = threat.is_some_and(|tp| dist(tp, m.pos) < 7.0);
                let walk = ty.map(|t| num(t, "walk_speed_multiplier", 1.0)).unwrap_or(num(p, "walk_speed_modifier", 1.0));
                let sprint = ty.map(|t| num(t, "sprint_speed_multiplier", 1.0)).unwrap_or(num(p, "sprint_speed_modifier", 1.0));
                m.ctl.dest = Some(dest);
                m.ctl.speed = if close { sprint } else { walk };
                dist(dest, m.pos) > 1.0 && t < 200 && m.stuck < 40
            }
            G::FleeSun | G::MoveToWater | G::MoveToLand | G::MoveToLava | G::StayNearHome => {
                let Some(dest) = m.goals[gi].st.dest else { return false };
                m.ctl.dest = Some(dest);
                m.ctl.speed = speed;
                let arrived = match g.kind {
                    G::MoveToWater => m.env.in_water,
                    G::MoveToLand => !m.env.in_water,
                    G::MoveToLava => m.env.in_lava,
                    _ => dist(dest, m.pos) < num(p, "goal_radius", 1.0).max(1.0),
                };
                !arrived && t < 400 && m.stuck < 60
            }
            G::Breathe => {
                m.ctl.dest = Some([m.pos[0], m.pos[1] + 8.0, m.pos[2]]);
                m.ctl.speed = speed.max(1.0);
                m.env.underwater
            }
            G::Tempt => {
                let Some(Ent::Player(sid)) = m.goals[gi].st.ent else { return false };
                let items = item_list(p.get("items"));
                let Some(q) = self.players.get(&sid) else { return false };
                let feet = [q.pos[0], q.pos[1] - EYE_HEIGHT, q.pos[2]];
                let held = self.stack_name(q.inventory.held_item());
                if !items.contains(&held) || q.world != m.world || dist(feet, m.pos) > num(p, "within_radius", 10.0).max(2.0) + 2.0 {
                    return false;
                }
                m.ctl.look = Some(q.pos);
                if dist(feet, m.pos) > 2.5 {
                    m.ctl.dest = Some(feet);
                    m.ctl.speed = speed;
                }
                true
            }
            G::Breed => {
                let Some(Ent::Mob(peid)) = m.goals[gi].st.ent else { return false };
                let Some(partner) = self.mob_ref(peid) else { return false };
                if partner.love == 0 && t > 5 {
                    return false;
                }
                let pp = partner.pos;
                m.ctl.look = Some([pp[0], pp[1] + 0.5, pp[2]]);
                m.ctl.dest = Some(pp);
                m.ctl.speed = speed;
                if dist(pp, m.pos) < 2.5 && t >= 60 {
                    if m.entity_id < peid {
                        self.breed(m, peid);
                    }
                    return false;
                }
                t < 300
            }
            G::FollowParent | G::FollowMob => {
                let Some(e) = m.goals[gi].st.ent else { return false };
                let Some(tp) = self.subj(e).map(|s| s.feet()) else { return false };
                let d = dist(tp, m.pos);
                let stop = num(p, "stop_distance", if g.kind == G::FollowParent { 3.0 } else { 2.0 });
                if d > 16.0 {
                    return false;
                }
                if d > stop {
                    m.ctl.dest = Some(tp);
                    m.ctl.speed = speed;
                }
                m.ctl.look = Some([tp[0], tp[1] + 1.0, tp[2]]);
                d > stop || t < 40
            }
            G::FollowOwner => {
                let Some((_, o)) = m.owner.as_ref().and_then(|o| self.online_named(o)) else { return false };
                let feet = [o.pos[0], o.pos[1] - EYE_HEIGHT, o.pos[2]];
                let (oworld, opos) = (o.world, o.pos);
                if oworld != m.world || m.sitting {
                    return false;
                }
                let d = dist(feet, m.pos);
                if d > 12.0 && m.on_ground && !g.name.ends_with("pet_sleep_with_owner") {
                    self.teleport_near(m, feet);
                    return true;
                }
                m.ctl.look = Some(opos);
                if d > num(p, "stop_distance", 2.0) {
                    m.ctl.dest = Some(feet);
                    m.ctl.speed = speed;
                    true
                } else {
                    false
                }
            }
            G::TeleportToOwner => {
                if let Some((_, o)) = m.owner.as_ref().and_then(|o| self.online_named(o)) {
                    let feet = [o.pos[0], o.pos[1] - EYE_HEIGHT, o.pos[2]];
                    self.teleport_near(m, feet);
                }
                false
            }
            G::StayWhileSitting => {
                m.ctl.hold = true;
                m.sitting
            }
            G::Beg => {
                let Some(e) = m.goals[gi].st.ent else { return false };
                let Some(Subj::P(_, q)) = self.subj(e) else { return false };
                m.ctl.look = Some(q.pos);
                m.interested = true;
                t < m.goals[gi].st.n.max(20)
            }
            G::Stroll | G::Swim | G::Fly | G::Hover => {
                if g.name.ends_with("swim_idle") {
                    return t < m.goals[gi].st.n;
                }
                if m.goals[gi].st.phase == 1 {
                    // circling (phantoms)
                    let st = &mut m.goals[gi].st;
                    st.f += 2.5;
                    let c = st.dest.unwrap_or(m.pos);
                    let a = st.f.to_radians();
                    let r = num(p, "radius_range", 10.0).max(5.0);
                    m.ctl.dest = Some([c[0] + a.cos() * r, c[1], c[2] + a.sin() * r]);
                    m.ctl.speed = speed;
                    return t < 400;
                }
                let Some(dest) = m.goals[gi].st.dest else { return false };
                m.ctl.dest = Some(dest);
                m.ctl.speed = speed;
                let reach = if m.t.can_fly || g.kind == G::Swim { 1.0 } else { 0.8 };
                let near = ((dest[0] - m.pos[0]).powi(2) + (dest[2] - m.pos[2]).powi(2)).sqrt() < reach && (dest[1] - m.pos[1]).abs() < 2.0;
                !near && t < 300 && m.stuck < 40
            }
            G::Breach => {
                if t == 1 {
                    let a = (m.yaw + 90.0).to_radians();
                    m.motion = [a.cos() * 0.4, 0.9, a.sin() * 0.4];
                }
                t < 20
            }
            G::SlimeHop => {
                if m.target.is_some() {
                    return false;
                }
                if m.on_ground && t % 30 == 1 {
                    if rand::random::<u32>() % 3 == 0 {
                        m.yaw = (rand::random::<u32>() % 360) as f32;
                    }
                    let a = (m.yaw + 90.0).to_radians();
                    let size = m.t.scale.max(0.5);
                    m.motion = [a.cos() * 0.25 * size.min(1.5), 0.42, a.sin() * 0.25 * size.min(1.5)];
                    self.mob_sound(m, "jump");
                }
                t < 200
            }
            G::Melee => self.melee_tick(m, gi),
            G::Leap => {
                if t == 1 {
                    if let Some(tp) = self.target_pos(m) {
                        let (dx, dz) = (tp[0] - m.pos[0], tp[2] - m.pos[2]);
                        let d = (dx * dx + dz * dz).sqrt().max(0.01);
                        let yd = num(p, "yd", 0.4);
                        m.motion = [dx / d * 0.4 + m.motion[0] * 0.2, yd, dz / d * 0.4 + m.motion[2] * 0.2];
                    }
                }
                t < 10
            }
            G::Ranged => self.ranged_tick(m, gi),
            G::Swell => {
                m.ctl.hold = true;
                if let Some(tp) = self.target_pos(m) {
                    m.ctl.look = Some([tp[0], tp[1] + 1.5, tp[2]]);
                    if dist(tp, m.pos) > num(p, "stop_distance", 7.0) {
                        return false;
                    }
                }
                m.fuse.is_some() || m.target.is_some()
            }
            G::Ram => {
                let Some(tp) = self.target_pos(m) else { return false };
                let d = dist(tp, m.pos);
                m.ctl.look = Some([tp[0], tp[1] + 1.0, tp[2]]);
                m.ctl.dest = Some(tp);
                m.ctl.speed = num(p, "ram_speed", 1.8) / m.t.speed.max(0.05) * 0.25;
                if d < 1.5 {
                    if let Some(target) = m.target {
                        let force = num(p, "knockback_force", 2.5);
                        let dmg = rand_f(m.t.damage.0, m.t.damage.1).max(1.0);
                        self.mob_hit(m, target, dmg, Some(force * 0.3));
                    }
                    let (lo, hi) = range(p.get("cooldown_range"), (10.0, 30.0));
                    m.goals[gi].st.next = now + (rand_f(lo, hi) * 20.0) as u64;
                    m.target = None;
                    return false;
                }
                t < 100
            }
            G::LookAtPlayer | G::LookAtEntity => {
                let Some(e) = m.goals[gi].st.ent else { return false };
                let Some(o) = self.subj(e) else { return false };
                let f = o.feet();
                let h = match o {
                    Subj::P(..) => EYE_HEIGHT,
                    Subj::M(x) => x.t.height * 0.85,
                };
                m.ctl.look = Some([f[0], f[1] + h, f[2]]);
                t < m.goals[gi].st.n && dist(f, m.pos) < num(p, "look_distance", 8.0) + 2.0
            }
            G::LookAtTarget => {
                let Some(tp) = self.target_pos(m) else { return false };
                m.ctl.look = Some([tp[0], tp[1] + 1.5, tp[2]]);
                true
            }
            G::LookAround => {
                m.ctl.look = m.goals[gi].st.dest;
                t < m.goals[gi].st.n
            }
            G::MoveToBlock | G::Harvest | G::StompEgg | G::RaidGarden | G::LayEgg => self.block_goal_tick(m, gi),
            G::EatBlock => {
                m.ctl.hold = true;
                if t >= m.goals[gi].st.n {
                    if let Some(b) = m.goals[gi].st.block {
                        self.eat_block(m, b, p);
                    }
                    return false;
                }
                true
            }
            G::PickupItems => self.pickup_tick(m, gi),
            G::GiveToOwner => {
                let Some((_, o)) = m.owner.as_ref().and_then(|o| self.online_named(o)) else { return false };
                let feet = [o.pos[0], o.pos[1] - EYE_HEIGHT, o.pos[2]];
                if o.world != m.world {
                    return false;
                }
                m.ctl.dest = Some(feet);
                m.ctl.speed = speed;
                if dist(feet, m.pos) < 2.5 {
                    if g.name.ends_with("drop_item_for") {
                        if let Some(table) = string(p, "loot_table").map(String::from) {
                            let cx = loot::LootCx { difficulty: self.cfg.difficulty, ..Default::default() };
                            for it in self.roll_loot(&table, &cx) {
                                self.drop_at(m.world, m.pos, it);
                            }
                        }
                        m.goals[gi].st.next = now + 24000;
                    } else {
                        let it = std::mem::take(&mut m.equip.hand);
                        if !it.is_empty() {
                            self.drop_at(m.world, [m.pos[0], m.pos[1] + 0.5, m.pos[2]], it);
                        }
                    }
                    return false;
                }
                t < 400
            }
            G::Sleep => {
                m.ctl.hold = true;
                m.sleeping = true;
                let wake = if g.name.ends_with("nap") { !self.is_day(m.world) || t > 1200 } else { self.is_day(m.world) };
                !wake && now < m.last_hurt_tick.max(1) + u64::MAX / 2 && m.target.is_none() && now > m.last_hurt_tick + 20
            }
            G::TimerFlag => t < m.goals[gi].st.n,
            G::SendEvent => {
                let name = g.name.as_str();
                if name.ends_with("drink_potion") {
                    m.ctl.hold = true;
                    if t == m.goals[gi].st.n {
                        self.mob_sound(m, "drink");
                        if m.fire_ticks > 0 {
                            m.fire_ticks = 0;
                            m.effects.push(("fire_resistance".into(), 3600));
                        } else if m.env.underwater {
                            m.effects.push(("water_breathing".into(), 3600));
                            m.air = m.t.air_ticks;
                        } else {
                            m.health = (m.health + 4.0).min(m.t.max_health);
                        }
                        return false;
                    }
                    return true;
                }
                let choices = p.get("event_choices").and_then(|c| c.as_array()).cloned().unwrap_or_default();
                let Some(choice) = choices.get(m.goals[gi].st.n as usize) else { return false };
                m.ctl.hold = true;
                if let Some(tp) = self.target_pos(m) {
                    m.ctl.look = Some([tp[0], tp[1] + 1.5, tp[2]]);
                }
                if t == 1 {
                    if let Some(s) = string(choice, "start_sound_event").map(String::from) {
                        self.mob_sound(m, &s);
                    }
                }
                let steps = choice.get("sequence").and_then(|s| s.as_array()).cloned().unwrap_or_default();
                for s in &steps {
                    let at = (num(s, "base_delay", 0.0) * 20.0) as u32 + 1;
                    if at == t {
                        if let Some(e) = string(s, "event").map(String::from) {
                            let other = m.target;
                            self.fire(m, &e, "self", other, 0);
                        }
                        if let Some(snd) = string(s, "sound_event").map(String::from) {
                            self.mob_sound(m, &snd);
                        }
                    }
                }
                let cast = (num(choice, "cast_duration", 2.0) * 20.0) as u32;
                if t >= cast {
                    m.goals[gi].st.next = now + (num(choice, "cooldown_time", 5.0) * 20.0) as u64;
                    return false;
                }
                true
            }
            G::Summon => {
                let choices = p.get("summon_choices").and_then(|c| c.as_array()).cloned().unwrap_or_default();
                let Some(choice) = choices.get(m.goals[gi].st.n as usize).cloned() else { return false };
                m.ctl.hold = true;
                m.charging = true;
                let tp = self.target_pos(m);
                if let Some(tp) = tp {
                    m.ctl.look = Some([tp[0], tp[1] + 1.5, tp[2]]);
                }
                if t == 1 {
                    if let Some(s) = string(&choice, "start_sound_event").map(String::from) {
                        self.mob_sound(m, &s);
                    }
                }
                for s in choice.get("sequence").and_then(|s| s.as_array()).cloned().unwrap_or_default() {
                    let at = (num(&s, "base_delay", 0.0) * 20.0) as u32 + 1;
                    if at == t {
                        self.summon_step(m, &s, tp);
                    }
                }
                let cast = (num(&choice, "cast_duration", 2.0) * 20.0) as u32;
                if t >= cast.max(2) {
                    m.goals[gi].st.next = now + (num(&choice, "cooldown_time", 5.0) * 20.0) as u64;
                    return false;
                }
                true
            }
            G::Idle => {
                m.ctl.hold = true;
                t < m.goals[gi].st.n
            }
            G::Dragon => self.dragon_tick(m, gi),
            G::TargetWhenPushed | G::Passive => false,
        }
    }

    /// Best prey for nearest_attackable_target.
    pub(crate) fn find_attack_target(&mut self, m: &Mob, p: &Value) -> Option<Ent> {
        let types = entity_types(p);
        if types.is_empty() {
            return None;
        }
        let must_see = boolean(p, "must_see", false);
        let within = num(p, "within_radius", 0.0);
        let default_max = num(p, "max_dist", m.t.follow_range.max(16.0));
        let reach = types.iter().map(|t| num(t, "max_dist", default_max)).fold(0.0, f32::max).max(within);
        let mut best: Option<(Ent, f32, i32)> = None;
        let cands = self.entities_near(m.world, m.pos, reach, Some(m.entity_id));
        for e in cands {
            let Some(o) = self.subj(e) else { continue };
            if !self.valid_target(m, e, reach + 1.0) {
                continue;
            }
            let d = dist(o.feet(), m.pos);
            if within > 0.0 && d > within {
                continue;
            }
            let mut eyes = o.feet();
            eyes[1] += 1.2;
            let mut chosen: Option<(i32, bool)> = None;
            for t in &types {
                let mut maxd = num(t, "max_dist", default_max);
                if let Subj::P(_, pl) = o {
                    if pl.actor_state.sneaking {
                        maxd *= num(p, "target_sneak_visibility_multiplier", 0.8);
                    }
                }
                if d <= maxd && filters::test(t.get("filters"), &Cx::new(self, m).with_other_subj(o)) {
                    chosen = Some((num(t, "priority", 0.0) as i32, boolean(t, "must_see", must_see)));
                    break;
                }
            }
            let Some((prio, see)) = chosen else { continue };
            if see && !self.can_see(m.world, m.eye(), eyes) {
                continue;
            }
            if best.is_none_or(|b| prio > b.2 || (prio == b.2 && d < b.1)) {
                best = Some((e, d, prio));
            }
        }
        best.map(|b| b.0)
    }

    fn find_partner(&self, m: &Mob) -> Option<i64> {
        let b = m.comps.get("minecraft:breedable")?;
        let mates = mate_types(b, &m.def.id);
        self.mobs
            .iter()
            .filter(|o| o.alive() && o.entity_id != m.entity_id && o.world == m.world && o.love > 0 && !o.t.baby)
            .filter(|o| mates.iter().any(|(mt, _)| *mt == o.def.id))
            .filter(|o| dist(o.pos, m.pos) < 8.0)
            .min_by(|a, b| dist(a.pos, m.pos).total_cmp(&dist(b.pos, m.pos)))
            .map(|o| o.entity_id)
    }

    /// Two animals in love make a baby.
    fn breed(&mut self, m: &mut Mob, partner: i64) {
        let Some(b) = m.comps.get("minecraft:breedable").cloned() else { return };
        let pid = self.mob_ref(partner).map(|p| p.def.id.clone()).unwrap_or_default();
        let mates = mate_types(&b, &m.def.id);
        let (_, baby_from_breed) = mates.iter().find(|(mt, _)| *mt == pid).cloned().unwrap_or((pid.clone(), None));
        let baby_type = baby_from_breed
            .or_else(|| {
                m.comps
                    .get("minecraft:offspring")
                    .and_then(|o| o.get("offspring_pairs"))
                    .and_then(|pairs| pairs.get(&pid).or(pairs.get(&m.def.id)))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .unwrap_or(m.def.id.clone());
        m.love = 0;
        m.love_cooldown = 6000;
        if let Some(o) = self.mob_mut(partner) {
            o.love = 0;
            o.love_cooldown = 6000;
        }
        let at = [(m.pos[0]), m.pos[1], m.pos[2]];
        let Some(baby) = self.spawn_entity(m.world, &baby_type, at, Some("minecraft:entity_born")) else { return };
        // inherited properties (climate variants...)
        let inherit: Vec<String> = m
            .comps
            .get("minecraft:offspring")
            .and_then(|o| o.get("property_inheritance"))
            .and_then(|p| p.as_object())
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        let props: Vec<(String, PropVal)> = inherit.iter().filter_map(|k| m.props.get(k).map(|v| (k.clone(), v.clone()))).collect();
        let tamed_owner = if boolean(&b, "inherit_tamed", true) && m.t.tamed { m.owner.clone() } else { None };
        if let Some(bm) = self.mob_mut(baby) {
            for (k, v) in props {
                bm.props.insert(k, v);
            }
            bm.persistent = true;
            if let Some(o) = tamed_owner {
                bm.owner = Some(o);
            }
        }
        if let Some(ev) = b.get("breed_event").cloned() {
            if let Some((e, target)) = json::event_ref(Some(&ev)) {
                if target == "baby" {
                    self.mobsys.queued.push((baby, e, Some(Ent::Mob(m.entity_id))));
                } else {
                    self.fire(m, &e, &target, Some(Ent::Mob(baby)), 0);
                }
            }
        }
        let xp = m.comps.get("minecraft:experience_reward").map(|x| molang::value(x.get("on_bred"), 0.0, &Self::molang_of(m))).unwrap_or(1.0 + rand::random::<u32>() as f32 % 7.0);
        if xp >= 1.0 {
            self.spawn_xp(m.world, at, xp as u32);
        }
    }

    fn teleport_near(&mut self, m: &mut Mob, feet: [f32; 3]) {
        let h = m.t.height.ceil() as i32;
        for _ in 0..10 {
            let b = [feet[0].floor() as i32 + json::rand_i(-3, 3), feet[1].floor() as i32 + json::rand_i(-1, 1), feet[2].floor() as i32 + json::rand_i(-3, 3)];
            if (b[0] - feet[0].floor() as i32).abs() < 2 && (b[2] - feet[2].floor() as i32).abs() < 2 {
                continue;
            }
            let c = self.class_at(m.world, [b[0], b[1] - 1, b[2]]);
            if c & SOLID != 0 && c & nav::DAMAGE == 0 && !self.box_blocked(m.world, [b[0] as f32 + 0.5, b[1] as f32, b[2] as f32 + 0.5], m.t.width, h as f32) {
                m.pos = [b[0] as f32 + 0.5, b[1] as f32, b[2] as f32 + 0.5];
                m.motion = [0.0; 3];
                m.path.clear();
                return;
            }
        }
    }

    /// A spot about `r` blocks away from `away` (or anywhere when None).
    fn flee_spot(&mut self, m: &Mob, away: Option<[f32; 3]>, r: i32) -> Option<[f32; 3]> {
        for _ in 0..4 {
            let around = match away {
                Some(a) => {
                    let (dx, dz) = (m.pos[0] - a[0], m.pos[2] - a[2]);
                    let d = (dx * dx + dz * dz).sqrt().max(0.01);
                    [m.pos[0] + dx / d * r as f32 * 0.8, m.pos[1], m.pos[2] + dz / d * r as f32 * 0.8]
                }
                None => m.pos,
            };
            let spot = if m.t.can_fly && !m.t.gravity {
                self.random_open_spot(m, (r / 2).max(2), 3, false, around)
            } else if m.env.in_water && m.t.nav == NavKind::Swim {
                self.random_open_spot(m, (r / 2).max(2), 3, true, around)
            } else {
                self.random_ground_spot(m, (r / 2).max(2), 4, around)
            };
            if spot.is_some() {
                return spot;
            }
        }
        None
    }

    /// A covered spot nearby (out of the sun / rain).
    fn shade_spot(&mut self, m: &Mob, r: i32) -> Option<[f32; 3]> {
        for _ in 0..10 {
            let Some(s) = self.random_ground_spot(m, r, 3, m.pos) else { continue };
            let b = [s[0].floor() as i32, s[1].floor() as i32, s[2].floor() as i32];
            let covered = (2..24).any(|dy| self.class_at(m.world, [b[0], b[1] + dy, b[2]]) & SOLID != 0);
            if covered {
                return Some(s);
            }
        }
        None
    }

    fn home_center(&self, m: &Mob) -> Option<[f32; 3]> {
        let h = m.home?;
        m.comps.get("minecraft:home")?;
        Some([h[0] as f32 + 0.5, h[1] as f32, h[2] as f32 + 0.5])
    }

    /// Looks for a block among `names` by sampling around `at`.
    pub(crate) fn find_block(&mut self, world: usize, at: [f32; 3], r: i32, h: i32, names: &[String], samples: usize) -> Option<[i32; 3]> {
        let base = [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32];
        let mut best: Option<([i32; 3], i32)> = None;
        // the close blocks first, then random ones farther out
        for i in 0..samples {
            let b = if i < 27 {
                [base[0] + (i as i32 % 3) - 1, base[1] + (i as i32 / 3 % 3) - 1, base[2] + (i as i32 / 9) - 1]
            } else {
                [base[0] + json::rand_i(-r, r), base[1] + json::rand_i(-h, h), base[2] + json::rand_i(-r, r)]
            };
            if !self.worlds.worlds[world].is_loaded(b) {
                continue;
            }
            let id = self.worlds.worlds[world].get_block(b, &self.gd);
            let name = self.gd.block_name(id);
            if names.iter().any(|n| n == name) {
                let d = (b[0] - base[0]).abs() + (b[1] - base[1]).abs() + (b[2] - base[2]).abs();
                if best.is_none_or(|x| d < x.1) {
                    best = Some((b, d));
                }
            }
        }
        best.map(|b| b.0)
    }

    fn find_class(&mut self, world: usize, at: [f32; 3], r: i32, h: i32, class: u8, samples: usize) -> Option<[i32; 3]> {
        let base = [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32];
        let mut best: Option<([i32; 3], i32)> = None;
        for _ in 0..samples {
            let b = [base[0] + json::rand_i(-r, r), base[1] + json::rand_i(-h, h), base[2] + json::rand_i(-r, r)];
            if self.worlds.worlds[world].is_loaded(b) && self.class_at(world, b) & class != 0 {
                let d = (b[0] - base[0]).abs() + (b[1] - base[1]).abs() + (b[2] - base[2]).abs();
                if best.is_none_or(|x| d < x.1) {
                    best = Some((b, d));
                }
            }
        }
        best.map(|b| b.0)
    }

    /// The blocks a block goal goes to.
    fn goal_blocks(&self, m: &Mob, g: &traits::GoalDef) -> Vec<String> {
        let p = &g.p;
        let mut v = item_list(p.get("target_blocks"));
        if v.is_empty() {
            v = item_list(p.get("blocks"));
        }
        if !v.is_empty() {
            return v;
        }
        let list: &[&str] = match g.name.trim_start_matches("minecraft:behavior.") {
            "inspect_bookshelf" => &["bookshelf"],
            "work_composter" => &["composter"],
            "ocelot_sit_on_block" => &["chest", "furnace", "lit_furnace", "bed"],
            "go_and_give_items_to_noteblock" => &["noteblock"],
            "stomp_turtle_egg" => &["turtle_egg"],
            "harvest_farm_block" | "fertilize_farm_block" => &["wheat", "carrots", "potatoes", "beetroot"],
            "lay_egg" => {
                if m.def.id.contains("frog") {
                    &["water"]
                } else {
                    &["sand"]
                }
            }
            "go_home" => &["bee_nest", "beehive"],
            _ => &[],
        };
        list.iter().map(|s| format!("minecraft:{s}")).collect()
    }

    fn block_goal_tick(&mut self, m: &mut Mob, gi: usize) -> bool {
        let g = m.goals[gi].def.clone();
        let p = &g.p;
        let t = m.goals[gi].st.t;
        let (Some(dest), Some(b)) = (m.goals[gi].st.dest, m.goals[gi].st.block) else { return false };
        let reach = num(p, "goal_radius", 1.5).max(1.0);
        let close = dist(dest, m.pos) <= reach + 0.5;
        if !close {
            m.ctl.dest = Some(dest);
            m.ctl.speed = num(p, "speed_multiplier", 1.0);
            return t < 600 && m.stuck < 60;
        }
        m.ctl.look = Some([b[0] as f32 + 0.5, b[1] as f32 + 0.5, b[2] as f32 + 0.5]);
        m.ctl.hold = true;
        let phase = m.goals[gi].st.phase;
        if phase == 0 {
            m.goals[gi].st.phase = 1;
            m.goals[gi].st.n = t;
            match g.kind {
                G::StompEgg => {
                    if self.cfg.mob_griefing {
                        let air = self.gd.air_id;
                        self.change_block(m.world, b, air);
                        self.broadcast(&protocol::level_sound_event("break", [b[0] as f32, b[1] as f32, b[2] as f32], -1), |_, q| q.world == m.world);
                    }
                }
                G::RaidGarden | G::Harvest => {
                    let id = self.worlds.worlds[m.world].get_block(b, &self.gd);
                    let name = self.gd.block_name(id).to_string();
                    let air = self.gd.air_id;
                    if g.kind == G::Harvest {
                        if let Some(seed) = super::items::crop_drop(&name) {
                            if let Some(it) = self.simple_item(seed, 1 + rand::random::<u16>() % 2) {
                                self.drop_at(m.world, [b[0] as f32 + 0.5, b[1] as f32 + 0.5, b[2] as f32 + 0.5], it);
                            }
                        }
                    }
                    if self.cfg.mob_griefing || g.kind == G::Harvest {
                        self.change_block(m.world, b, air);
                    }
                }
                G::LayEgg => {
                    let egg = if m.def.id.contains("frog") { "minecraft:frog_spawn" } else { "minecraft:turtle_egg" };
                    let above = [b[0], b[1] + 1, b[2]];
                    if let Some(eid) = self.gd.default_block(egg) {
                        if self.class_at(m.world, above) & SOLID == 0 {
                            self.change_block(m.world, above, eid);
                        }
                    }
                    let ev = p.get("on_lay").cloned();
                    self.fire_ref(m, ev.as_ref(), None);
                }
                _ => {
                    let evs: Vec<Value> = match p.get("on_reach") {
                        Some(Value::Array(a)) => a.clone(),
                        Some(o) => vec![o.clone()],
                        None => Vec::new(),
                    };
                    for e in evs {
                        self.fire_ref(m, Some(&e), None);
                    }
                    if g.name.ends_with("ocelot_sit_on_block") {
                        m.sitting = true;
                    }
                }
            }
        }
        let stay = (num(p, "stay_duration", 3.0) * 20.0) as u32;
        let done = t >= m.goals[gi].st.n + stay;
        if done && g.name.ends_with("ocelot_sit_on_block") {
            m.sitting = false;
        }
        !done
    }

    fn edible_block(&mut self, m: &Mob, p: &Value) -> Option<[i32; 3]> {
        let pairs = p.get("eat_and_replace_block_pairs").and_then(|v| v.as_array())?;
        let feet = [m.pos[0].floor() as i32, m.pos[1].floor() as i32, m.pos[2].floor() as i32];
        for b in [feet, [feet[0], feet[1] - 1, feet[2]]] {
            let id = self.worlds.worlds[m.world].get_block(b, &self.gd);
            let name = self.gd.block_name(id);
            for pr in pairs {
                let (want, _) = item_ref(string(pr, "eat_block").unwrap_or(""));
                if name == want || (want == "minecraft:grass" && name == "minecraft:grass_block") || (want == "minecraft:tallgrass" && name == "minecraft:short_grass") {
                    return Some(b);
                }
            }
        }
        None
    }

    fn eat_block(&mut self, m: &mut Mob, b: [i32; 3], p: &Value) {
        let id = self.worlds.worlds[m.world].get_block(b, &self.gd);
        let name = self.gd.block_name(id).to_string();
        let Some(pairs) = p.get("eat_and_replace_block_pairs").and_then(|v| v.as_array()).cloned() else { return };
        for pr in pairs {
            let (want, _) = item_ref(string(&pr, "eat_block").unwrap_or(""));
            let matches = name == want || (want == "minecraft:grass" && name == "minecraft:grass_block") || (want == "minecraft:tallgrass" && name == "minecraft:short_grass");
            if !matches {
                continue;
            }
            let (repl, _) = item_ref(string(&pr, "replace_block").unwrap_or("air"));
            let new = if repl == "minecraft:air" { Some(self.gd.air_id) } else { self.gd.default_block(&repl) };
            if let (Some(n), true) = (new, self.cfg.mob_griefing) {
                self.change_block(m.world, b, n);
            }
            let ev = p.get("on_eat").cloned();
            self.fire_ref(m, ev.as_ref(), None);
            return;
        }
    }

    fn wanted_item(&self, m: &Mob, p: &Value, maxd: f32) -> Option<usize> {
        let any = boolean(p, "can_pickup_any_item", false);
        let same = boolean(p, "pickup_same_items_as_in_hand", false);
        let excluded = item_list(p.get("excluded_items"));
        let equip = m.comps.get("minecraft:equip_item").is_some() || boolean(p, "can_pickup_to_hand_or_equipment", true);
        let shareables: Vec<String> = m.comps.get("minecraft:shareables").map(|s| json::item_list(s.get("items"))).unwrap_or_default();
        let hand = self.stack_name(&m.equip.hand);
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.world == m.world && it.pickup_delay == 0 && dist(it.pos, m.pos) <= maxd + 1.0)
            .filter(|(_, it)| {
                let n = self.stack_name(&it.item);
                if excluded.contains(&n) {
                    return false;
                }
                if same {
                    return !hand.is_empty() && n == hand;
                }
                if shareables.contains(&n) {
                    return self.better_equipment(m, &n);
                }
                any && equip && self.better_equipment(m, &n)
            })
            .min_by(|a, b| dist(a.1.pos, m.pos).total_cmp(&dist(b.1.pos, m.pos)))
            .map(|(i, _)| i)
    }

    fn better_equipment(&self, m: &Mob, name: &str) -> bool {
        let slot = items::slot_of(name);
        let cur = match slot {
            items::Slot::Head => &m.equip.armor[0],
            items::Slot::Chest => &m.equip.armor[1],
            items::Slot::Legs => &m.equip.armor[2],
            items::Slot::Feet => &m.equip.armor[3],
            items::Slot::Offhand => &m.equip.offhand,
            items::Slot::Hand => &m.equip.hand,
        };
        if cur.is_empty() {
            return true;
        }
        items::tier(name) > items::tier(&self.stack_name(cur))
    }

    fn pickup_tick(&mut self, m: &mut Mob, gi: usize) -> bool {
        let g = m.goals[gi].def.clone();
        if g.name.ends_with("take_block") {
            // endermen lift a block next to them
            let blocks = item_list(g.p.get("blocks"));
            let names: Vec<String> = if blocks.is_empty() {
                ["grass_block", "dirt", "sand", "gravel", "clay", "pumpkin", "melon_block", "cactus", "red_flower", "yellow_flower", "mycelium", "netherrack", "crimson_nylium", "warped_nylium"].iter().map(|s| format!("minecraft:{s}")).collect()
            } else {
                blocks
            };
            if let Some(b) = self.find_block(m.world, m.pos, 2, 2, &names, 40) {
                let id = self.worlds.worlds[m.world].get_block(b, &self.gd);
                let air = self.gd.air_id;
                self.change_block(m.world, b, air);
                m.carried_block = Some(id);
            }
            return false;
        }
        let eid = m.goals[gi].st.f as i64;
        let Some(i) = self.items.iter().position(|it| it.entity_id == eid) else { return false };
        let pos = self.items[i].pos;
        m.ctl.dest = Some(pos);
        m.ctl.speed = num(&g.p, "speed_multiplier", 1.0);
        if dist(pos, m.pos) < num(&g.p, "goal_radius", 1.5).max(1.2) {
            let it = self.items.swap_remove(i);
            self.broadcast(&protocol::take_item_actor(it.entity_id, m.entity_id), |_, q| q.world == m.world);
            self.broadcast(&protocol::remove_actor(it.entity_id), |_, q| q.world == m.world);
            let name = self.stack_name(&it.item);
            if let Some(old) = self.equip_item(m, it.item, &name) {
                self.drop_at(m.world, [m.pos[0], m.pos[1] + 0.5, m.pos[2]], old);
            }
            m.persistent = true;
            let slot = match items::slot_of(&name) {
                items::Slot::Hand => 0,
                items::Slot::Offhand => 1,
                items::Slot::Head => 2,
                items::Slot::Chest => 3,
                items::Slot::Legs => 4,
                items::Slot::Feet => 5,
            };
            m.equip.drop_chance[slot] = 1.0;
            return false;
        }
        m.goals[gi].st.t < 200
    }

    fn summon_step(&mut self, m: &mut Mob, s: &Value, target: Option<[f32; 3]>) {
        let id = string(s, "entity_type").unwrap_or("").to_string();
        let count = num(s, "num_entities_spawned", 1.0).max(1.0) as i32;
        let cap = num(s, "summon_cap", 0.0) as usize;
        if cap > 0 {
            let r = num(s, "summon_cap_radius", 16.0);
            let have = self.mobs.iter().filter(|o| o.alive() && o.def.id == id && dist(o.pos, m.pos) < r).count();
            if have >= cap {
                return;
            }
        }
        if let Some(snd) = string(s, "sound_event").map(String::from) {
            self.mob_sound(m, &snd);
        }
        let shape = string(s, "shape").unwrap_or("circle");
        let center = if string(s, "target").unwrap_or("self") == "target" { target.unwrap_or(m.pos) } else { m.pos };
        let size = num(s, "size", 1.0);
        let known = self.mobsys.reg.entity(&id).is_some_and(|d| d.is_mob());
        for i in 0..count {
            let pos = if shape == "line" {
                let tp = target.unwrap_or(m.pos);
                let (dx, dz) = (tp[0] - m.pos[0], tp[2] - m.pos[2]);
                let d = (dx * dx + dz * dz).sqrt().max(0.01);
                let k = 1.25 * (i + 1) as f32;
                [m.pos[0] + dx / d * k, m.pos[1], m.pos[2] + dz / d * k]
            } else {
                let a = (i as f32 / count as f32) * std::f32::consts::TAU;
                [center[0] + a.cos() * size, center[1], center[2] + a.sin() * size]
            };
            if known {
                if let Some(new) = self.spawn_entity(m.world, &id, [pos[0], pos[1] + if id.contains("vex") { 1.0 } else { 0.0 }, pos[2]], None) {
                    if let Some(n) = self.mob_mut(new) {
                        n.target = m.target;
                        n.owner = None;
                        n.persistent = false;
                    }
                }
            } else {
                // evoker fangs and the like: bite whoever stands there
                let victims = self.entities_near(m.world, pos, 0.9, Some(m.entity_id));
                for v in victims {
                    self.mob_hit(m, v, 6.0, None);
                }
                self.broadcast(&protocol::level_sound_event("attack.strong", pos, -1), |_, q| q.world == m.world);
            }
        }
    }

    /// Ender dragon: circles around where it appeared (its "portal"), now and then strafes a
    /// player spitting a dragon fireball or charges at them; anything touching its body is hit
    /// and thrown back by the wings.
    fn dragon_tick(&mut self, m: &mut Mob, gi: usize) -> bool {
        let t = m.goals[gi].st.t;
        let home = match m.home {
            Some(h) => [h[0] as f32 + 0.5, h[1] as f32, h[2] as f32 + 0.5],
            None => {
                m.home = Some([m.pos[0].floor() as i32, m.pos[1].floor() as i32, m.pos[2].floor() as i32]);
                m.pos
            }
        };
        if t == 1 {
            m.goals[gi].st.f = (m.pos[2] - home[2]).atan2(m.pos[0] - home[0]).to_degrees();
        }
        if t % 40 == 1 {
            let p = serde_json::json!({"entity_types": [{"filters": {"test": "is_family", "subject": "other", "value": "player"}, "max_dist": 150}]});
            m.target = self.find_attack_target(m, &p);
        }
        // wings and head
        if t % 10 == 0 {
            let c = m.center();
            for e in self.entities_near(m.world, c, 6.0, Some(m.entity_id)) {
                if let Some(Subj::P(..)) = self.subj(e) {
                    let near = self.subj(e).map(|s| dist(s.feet(), c) < 5.0).unwrap_or(false);
                    if near {
                        self.mob_hit(m, e, 10.0, Some(1.5));
                    }
                }
            }
        }
        let tp = self.target_pos(m);
        let phase = m.goals[gi].st.phase;
        match (phase, tp) {
            (1, Some(tp)) => {
                // strafe: fly at the player from above, spit a fireball, turn away
                let aim = [tp[0], tp[1] + 12.0, tp[2]];
                m.ctl.dest = Some(aim);
                m.ctl.speed = 1.0;
                m.ctl.look = Some(tp);
                if dist(aim, m.pos) < 30.0 || t > m.goals[gi].st.n + 200 {
                    if let Some(target) = m.target {
                        self.mob_shoot(m, target, Some("minecraft:dragon_fireball"));
                    }
                    m.goals[gi].st.phase = 0;
                    m.goals[gi].st.n = t;
                }
            }
            (2, Some(tp)) => {
                // charge straight at the player
                m.ctl.dest = Some([tp[0], tp[1] + 1.0, tp[2]]);
                m.ctl.speed = 1.6;
                m.ctl.look = Some(tp);
                if dist(tp, m.center()) < 5.0 || t > m.goals[gi].st.n + 160 {
                    m.goals[gi].st.phase = 0;
                    m.goals[gi].st.n = t;
                }
            }
            _ => {
                // holding pattern
                let st = &mut m.goals[gi].st;
                st.f += 0.9;
                st.phase = 0;
                let a = st.f.to_radians();
                m.ctl.dest = Some([home[0] + a.cos() * 32.0, home[1] + 18.0 + (a * 2.0).sin() * 6.0, home[2] + a.sin() * 32.0]);
                m.ctl.speed = 1.0;
                m.ctl.look = None;
                let since = t.saturating_sub(st.n);
                if tp.is_some() && since > 160 && rand::random::<u32>() % 60 == 0 {
                    st.phase = if rand::random::<u32>() % 3 == 0 { 2 } else { 1 };
                    st.n = t;
                }
            }
        }
        true
    }
}

/// (mate type, baby type) pairs of a breedable component in any of its shapes.
pub(crate) fn mate_types(b: &Value, me: &str) -> Vec<(String, Option<String>)> {
    match b.get("breeds_with") {
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| (string(x, "mate_type").unwrap_or(me).to_string(), string(x, "baby_type").map(String::from)))
            .collect(),
        Some(Value::Object(o)) if o.contains_key("mate_type") => {
            vec![(o["mate_type"].as_str().unwrap_or(me).to_string(), o.get("baby_type").and_then(|v| v.as_str()).map(String::from))]
        }
        Some(Value::Object(o)) => o.iter().map(|(k, v)| (k.clone(), string(v, "baby_type").map(String::from))).collect(),
        _ => vec![(me.to_string(), None)],
    }
}

pub(crate) fn ranged_radius(p: &Value) -> f32 {
    if let Some(r) = p.get("attack_radius") {
        return as_num(Some(r)).unwrap_or(15.0);
    }
    range(p.get("attack_range"), (0.0, 15.0)).1
}
