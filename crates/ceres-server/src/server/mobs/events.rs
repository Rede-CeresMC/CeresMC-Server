//! Entity events (add/remove component groups, randomize, sequence, first_valid, trigger,
//! set_property, play_sound...) and the components that fire them every tick: environment,
//! entity and target-nearby sensors, timers, ageing, love, spawn_entity, transformation,
//! explode fuses, anger, burning in daylight, hurt_on_condition, breathing and ambient sounds.

use super::filters::{self, Cx};
use super::json::*;
use super::*;
use serde_json::Value;

impl Server {
    /// Recomputes a mob's components and goals after its groups changed.
    pub(crate) fn rebuild_mob(&self, m: &mut Mob) {
        let comps = traits::merge(&m.def, &m.groups);
        let t = Traits::read(&comps);
        let first = m.t.max_health == 0.0;
        if !first && (t.max_health - m.t.max_health).abs() > 0.01 {
            // a bigger health bar (a tamed wolf) fills up; a smaller one clips
            m.health = if t.max_health > m.t.max_health { m.health + (t.max_health - m.t.max_health) } else { m.health.min(t.max_health) };
        }
        let goals = t
            .goals
            .iter()
            .map(|gd| {
                let st = m.goals.iter().find(|g| g.def.name == gd.name && g.def.p == gd.p).map(|g| g.st.clone()).unwrap_or_default();
                GoalRt { def: gd.clone(), st }
            })
            .collect();
        // components that start a clock when they appear
        let now = self.ticks;
        let changed = |k: &str| comps.get(k).is_some() && (first || m.comps.get(k) != comps.get(k));
        if changed("minecraft:timer") {
            let tm = &comps["minecraft:timer"];
            m.timers.insert("minecraft:timer".into(), now + timer_ticks(tm));
        }
        if comps.get("minecraft:timer").is_none() {
            m.timers.remove("minecraft:timer");
        }
        if changed("minecraft:spawn_entity") {
            m.spawn_due = spawn_entries(&comps["minecraft:spawn_entity"])
                .iter()
                .map(|e| {
                    let lo = num(e, "min_wait_time", 300.0);
                    let hi = num(e, "max_wait_time", 600.0);
                    now + (rand_f(lo, hi) * 20.0) as u64
                })
                .collect();
        }
        if comps.get("minecraft:spawn_entity").is_none() {
            m.spawn_due.clear();
        }
        if changed("minecraft:transformation") {
            let tr = &comps["minecraft:transformation"];
            let delay = match tr.get("delay") {
                Some(Value::Object(o)) => as_num(o.get("value")).unwrap_or(0.0),
                other => as_num(other).unwrap_or(0.0),
            };
            m.transform_due = Some(now + (delay * 20.0) as u64);
        }
        if comps.get("minecraft:transformation").is_none() {
            m.transform_due = None;
        }
        if changed("minecraft:angry") {
            let d = num(&comps["minecraft:angry"], "duration", 25.0);
            m.angry_until = Some(if d < 0.0 { u64::MAX } else { now + (d * 20.0) as u64 });
        }
        if comps.get("minecraft:angry").is_none() {
            m.angry_until = None;
        }
        match comps.get("minecraft:explode") {
            Some(e) if boolean(e, "fuse_lit", false) => {
                if m.fuse.is_none() {
                    m.fuse = Some(0);
                }
                let _ = e;
            }
            None => m.fuse = None,
            _ => {}
        }
        if let Some(h) = comps.get("minecraft:home") {
            if m.home.is_none() && boolean(h, "home_block_list", true) {
                m.home = Some([m.pos[0].floor() as i32, m.pos[1].floor() as i32, m.pos[2].floor() as i32]);
            }
        }
        if !t.sittable {
            m.sitting = false;
        }
        m.comps = Rc::new(comps);
        m.t = Rc::new(t);
        m.goals = goals;
    }

    /// Runs an event of this mob's definition (`other` is who caused it).
    pub(crate) fn mob_event(&mut self, m: &mut Mob, name: &str, other: Option<Ent>) {
        self.mob_event_depth(m, name, other, 0);
    }

    fn mob_event_depth(&mut self, m: &mut Mob, name: &str, other: Option<Ent>, depth: u32) {
        if depth > 8 {
            return;
        }
        let Some(ev) = m.def.events.get(name).cloned() else { return };
        let mut changed = false;
        self.event_node(m, &ev, other, &mut changed, depth);
        if changed {
            self.rebuild_mob(m);
        }
    }

    fn event_node(&mut self, m: &mut Mob, node: &Value, other: Option<Ent>, changed: &mut bool, depth: u32) {
        if let Some(f) = node.get("filters") {
            let ok = filters::test(Some(f), &Cx::new(self, m).with_other(other));
            if !ok {
                return;
            }
        }
        if let Some(r) = node.get("remove").and_then(|r| r.get("component_groups")) {
            for g in str_list(Some(r)) {
                if let Some(i) = m.groups.iter().position(|x| *x == g) {
                    m.groups.remove(i);
                    *changed = true;
                }
            }
        }
        if let Some(a) = node.get("add").and_then(|a| a.get("component_groups")) {
            for g in str_list(Some(a)) {
                if m.def.groups.contains_key(&g) && !m.groups.contains(&g) {
                    m.groups.push(g);
                    *changed = true;
                }
            }
        }
        if let Some(Value::Object(props)) = node.get("set_property") {
            let q = Self::molang_of(m);
            for (k, v) in props {
                if let Some(val) = m.def.prop(k).and_then(|d| d.parse(v, &q)) {
                    m.props.insert(k.clone(), val);
                }
            }
        }
        if node.get("reset_target").is_some() {
            m.target = None;
        }
        if node.get("stop_movement").is_some() {
            m.motion = [0.0; 3];
            m.ctl = Ctl::default();
        }
        if node.get("set_home_position").is_some() {
            m.home = Some([m.pos[0].floor() as i32, m.pos[1].floor() as i32, m.pos[2].floor() as i32]);
        }
        if let Some(s) = node.get("play_sound").and_then(|p| string(p, "sound")) {
            let s = s.to_string();
            self.mob_sound(m, &s);
        }
        if *changed && (node.get("randomize").is_some() || node.get("sequence").is_some() || node.get("trigger").is_some() || node.get("first_valid").is_some()) {
            // later steps see the groups added so far
            self.rebuild_mob(m);
            *changed = false;
        }
        if let Some(Value::Array(opts)) = node.get("randomize") {
            let total: f32 = opts.iter().map(|o| num(o, "weight", 1.0).max(0.0)).sum();
            let mut pick = rand::random::<f32>() * total;
            for o in opts {
                let w = num(o, "weight", 1.0).max(0.0);
                if pick < w {
                    self.event_node(m, o, other, changed, depth + 1);
                    break;
                }
                pick -= w;
            }
        }
        if let Some(Value::Array(steps)) = node.get("sequence") {
            for s in steps {
                self.event_node(m, s, other, changed, depth + 1);
                if *changed {
                    self.rebuild_mob(m);
                    *changed = false;
                }
            }
        }
        if let Some(Value::Array(steps)) = node.get("first_valid") {
            for s in steps {
                let ok = filters::test(s.get("filters"), &Cx::new(self, m).with_other(other));
                if ok {
                    let mut s = s.clone();
                    if let Some(o) = s.as_object_mut() {
                        o.remove("filters");
                    }
                    self.event_node(m, &s, other, changed, depth + 1);
                    break;
                }
            }
        }
        if let Some((ev, target)) = event_ref(node.get("trigger")) {
            if *changed {
                self.rebuild_mob(m);
                *changed = false;
            }
            self.fire(m, &ev, &target, other, depth + 1);
        }
    }

    /// Sends an `{event, target}` to self, other, target, parent...
    pub(crate) fn fire(&mut self, m: &mut Mob, ev: &str, target: &str, other: Option<Ent>, depth: u32) {
        match target {
            "self" | "" => self.mob_event_depth(m, ev, other, depth),
            "other" | "target" | "player" | "baby" | "parent" | "damager" => {
                let who = match target {
                    "target" => m.target,
                    _ => other,
                };
                if let Some(Ent::Mob(eid)) = who {
                    self.mobsys.queued.push((eid, ev.to_string(), Some(Ent::Mob(m.entity_id))));
                }
            }
            _ => {}
        }
    }

    /// An `{event, target}` value from a component.
    pub(crate) fn fire_ref(&mut self, m: &mut Mob, v: Option<&Value>, other: Option<Ent>) {
        if let Some((ev, target)) = event_ref(v) {
            self.fire(m, &ev, &target, other, 0);
        }
    }

    pub(crate) fn molang_of(m: &Mob) -> impl Fn(&str, &[f32]) -> f32 + use<> {
        let (baby, var, mark, skin, hp, max, tamed, sheared, saddled, charged, player) = (
            m.t.baby, m.t.variant.unwrap_or(0), m.t.mark_variant.unwrap_or(0), m.t.skin_id.unwrap_or(0), m.health, m.t.max_health,
            m.t.tamed, m.t.sheared, m.t.saddled, m.t.charged, m.last_hit_by_player > 0,
        );
        move |name: &str, _args: &[f32]| -> f32 {
            match name.trim_start_matches("query.").trim_start_matches("q.") {
                "is_baby" => baby as i32 as f32,
                "variant" => var as f32,
                "mark_variant" => mark as f32,
                "skin_id" => skin as f32,
                "health" => hp,
                "max_health" => max,
                "is_tamed" => tamed as i32 as f32,
                "is_sheared" => sheared as i32 as f32,
                "is_saddled" => saddled as i32 as f32,
                "is_charged" | "is_powered" => charged as i32 as f32,
                "last_hit_by_player" => player as i32 as f32,
                _ => 0.0,
            }
        }
    }

    // ---------------------------------------------------------------- every tick

    pub(crate) fn mob_sensors(&mut self, m: &mut Mob) {
        let now = self.ticks;
        let stagger = (m.entity_id as u64) % 10;
        // environment sensors
        if m.age % 2 == 0 {
            if let Some(es) = m.comps.get("minecraft:environment_sensor").cloned() {
                let triggers = match es.get("triggers") {
                    Some(Value::Array(a)) => a.clone(),
                    Some(o @ Value::Object(_)) => vec![o.clone()],
                    _ => match es.get("on_environment") {
                        Some(o) => vec![o.clone()],
                        None => Vec::new(),
                    },
                };
                for tr in triggers {
                    let ok = filters::test(tr.get("filters"), &Cx::new(self, m));
                    if ok {
                        let ev = tr.get("event").cloned();
                        let target = string(&tr, "target").unwrap_or("self").to_string();
                        if let Some(Value::String(e)) = ev {
                            self.fire(m, &e, &target, None, 0);
                        }
                    }
                }
            }
        }
        // damage from the surroundings (lava, sunlight...)
        if let Some(h) = m.comps.get("minecraft:hurt_on_condition").cloned() {
            for c in h.get("damage_conditions").and_then(|d| d.as_array()).into_iter().flatten() {
                let ok = filters::test(c.get("filters"), &Cx::new(self, m));
                if ok {
                    let cause = string(c, "cause").unwrap_or("none").to_string();
                    let dmg = num(c, "damage_per_tick", 1.0);
                    if cause == "lava" && !m.t.fire_immune {
                        m.fire_ticks = m.fire_ticks.max(300);
                    }
                    self.hurt(m, dmg, &cause, None, None);
                }
            }
        }
        if m.death_ticks.is_some() {
            return;
        }
        // fire
        if m.env.in_water && m.fire_ticks > 0 {
            m.fire_ticks = 0;
        }
        if m.fire_ticks > 0 {
            m.fire_ticks -= 1;
            if m.fire_ticks % 20 == 0 && !m.t.fire_immune {
                self.hurt(m, 1.0, "fire_tick", None, None);
            }
        }
        if m.env.in_lava && !m.t.fire_immune && !m.comps.contains_key("minecraft:hurt_on_condition") {
            m.fire_ticks = m.fire_ticks.max(300);
            self.hurt(m, 4.0, "lava", None, None);
        }
        if m.t.burns_in_daylight && m.age % 20 == stagger * 2 % 20 {
            let helmet = !m.equip.armor[0].is_empty();
            if self.is_day(m.world) && m.env.sky && !m.env.in_water && !self.raining(m.world) && !helmet {
                m.fire_ticks = m.fire_ticks.max(160);
            }
        }
        // mob effects
        let mut regen = 0.0f32;
        let mut poison = 0.0f32;
        m.effects.retain_mut(|(name, left)| {
            *left = left.saturating_sub(1);
            if *left % 25 == 0 {
                match name.as_str() {
                    "regeneration" => regen += 1.0,
                    "poison" | "wither" => poison += 1.0,
                    _ => {}
                }
            }
            *left > 0
        });
        if regen > 0.0 {
            m.health = (m.health + regen).min(m.t.max_health);
        }
        if poison > 0.0 && !m.t.is("undead") && m.health > 1.0 {
            self.hurt(m, poison.min(m.health - 1.0), "magic", None, None);
        }
        // breathing
        let breathes_here = m.comps.get("minecraft:breathable").is_none() || if m.env.underwater { m.t.breathes_water } else { m.t.breathes_air };
        if breathes_here {
            m.air = (m.air + 5).min(m.t.air_ticks);
        } else {
            m.air -= 1;
            if m.air <= -20 {
                m.air = 0;
                self.hurt(m, 2.0, "drowning", None, None);
            }
        }
        if m.death_ticks.is_some() {
            return;
        }
        // timers
        if let Some(due) = m.timers.get("minecraft:timer").copied() {
            if now >= due {
                let tm = m.comps.get("minecraft:timer").cloned().unwrap_or(Value::Null);
                if boolean(&tm, "looping", true) {
                    m.timers.insert("minecraft:timer".into(), now + timer_ticks(&tm));
                } else {
                    m.timers.insert("minecraft:timer".into(), u64::MAX);
                }
                self.fire_ref(m, tm.get("time_down_event"), None);
            }
        }
        // babies growing up
        if let Some(a) = m.comps.get("minecraft:ageable").cloned() {
            m.grow_ticks += 1;
            if m.grow_ticks as f32 >= num(&a, "duration", 1200.0) * 20.0 {
                m.grow_ticks = 0;
                self.fire_ref(m, a.get("grow_up"), None);
            }
        }
        // love
        if m.love > 0 {
            m.love -= 1;
            if m.age % 10 == 0 {
                let pk = protocol::actor_event(m.entity_id, 21, 0); // hearts
                self.broadcast(&pk, |_, q| q.world == m.world);
            }
        }
        m.love_cooldown = m.love_cooldown.saturating_sub(1);
        // laying eggs and the like
        if !m.spawn_due.is_empty() {
            if let Some(se) = m.comps.get("minecraft:spawn_entity").cloned() {
                let entries = spawn_entries(&se);
                for (i, e) in entries.iter().enumerate() {
                    if m.spawn_due.get(i).is_some_and(|d| now >= *d) {
                        let lo = num(e, "min_wait_time", 300.0);
                        let hi = num(e, "max_wait_time", 600.0);
                        m.spawn_due[i] = now + (rand_f(lo, hi) * 20.0) as u64;
                        let ok = filters::test(e.get("filters"), &Cx::new(self, m));
                        if !ok {
                            continue;
                        }
                        let count = num(e, "num_to_spawn", 1.0).max(1.0) as u32;
                        if let Some(item) = string(e, "spawn_item") {
                            let (name, _) = item_ref(item);
                            if let Some(it) = self.simple_item(&name, count as u16) {
                                self.drop_at(m.world, m.pos, it);
                            }
                        } else if let Some(ent) = string(e, "spawn_entity") {
                            let ev = string(e, "spawn_event").map(String::from);
                            for _ in 0..count {
                                self.spawn_entity(m.world, ent, m.pos, ev.as_deref());
                            }
                        }
                        if let Some(s) = string(e, "spawn_sound") {
                            let s = s.to_string();
                            self.mob_sound(m, &s);
                        }
                        if boolean(e, "single_use", false) {
                            m.spawn_due[i] = u64::MAX;
                        }
                    }
                }
            }
        }
        // turning into another entity
        if m.transform_due.is_some_and(|d| now >= d) {
            m.transform_due = None;
            self.transform(m);
            return;
        }
        // anger wears off
        if let Some(until) = m.angry_until {
            if now >= until {
                m.angry_until = None;
                let calm = m.comps.get("minecraft:angry").and_then(|a| a.get("calm_event")).cloned();
                m.target = None;
                self.fire_ref(m, calm.as_ref(), None);
            } else if m.age % 20 == stagger * 2 {
                if let Some(a) = m.comps.get("minecraft:angry").and_then(|a| string(a, "angry_sound")).map(String::from) {
                    if rand::random::<u32>() % 3 == 0 {
                        self.mob_sound(m, &a);
                    }
                }
            }
        }
        // explosion fuse (creepers, TNT-like mobs)
        if let Some(fuse) = m.fuse {
            if let Some(ex) = m.comps.get("minecraft:explode").cloned() {
                if fuse == 0 {
                    self.mob_sound(m, "fuse");
                }
                let (lo, hi) = range(ex.get("fuse_length"), (1.5, 1.5));
                let len = (rand_f(lo, hi).max(0.0) * 20.0) as u32;
                m.fuse = Some(fuse + 1);
                if fuse + 1 >= len {
                    let power = num(&ex, "power", 3.0);
                    let fire = boolean(&ex, "causes_fire", false);
                    let breaks = boolean(&ex, "breaks_blocks", true) && (self.cfg.mob_griefing || !boolean(&ex, "destroy_affected_by_griefing", false));
                    let at = m.center();
                    m.gone = true;
                    self.explode(m.world, at, power, fire, breaks, Some(Ent::Mob(m.entity_id)));
                    return;
                }
            } else {
                m.fuse = None;
            }
        }
        // sensors that look at other entities
        if m.age % 5 == stagger % 5 {
            self.target_nearby_sensor(m);
            self.entity_sensor(m);
        }
        // ambient sounds
        if now >= m.ambient_due {
            let a = m.comps.get("minecraft:ambient_sound_interval").cloned();
            let (value, range_s, name) = match &a {
                Some(a) => (num(a, "value", 8.0), num(a, "range", 16.0), string(a, "event_name").unwrap_or("ambient").to_string()),
                None => (8.0, 16.0, "ambient".to_string()),
            };
            m.ambient_due = now + ((value + rand::random::<f32>() * range_s) * 20.0) as u64;
            if a.is_some() || m.t.is("mob") {
                let name = if m.target.is_some() && m.def.id.contains("zombie") { name } else { name };
                self.mob_sound(m, &name);
            }
        }
    }

    fn target_nearby_sensor(&mut self, m: &mut Mob) {
        let Some(s) = m.comps.get("minecraft:target_nearby_sensor").cloned() else { return };
        if s.as_object().is_some_and(|o| o.is_empty()) {
            return;
        }
        let Some(t) = m.target else {
            m.nearby_state = 0;
            return;
        };
        let Some(pos) = self.subj(t).map(|x| x.feet()) else { return };
        let d = filters::dist(m.pos, pos);
        let inside = num(&s, "inside_range", 1.0);
        let outside = num(&s, "outside_range", 5.0);
        let visible = !boolean(&s, "must_see", false) || self.can_see(m.world, m.eye(), [pos[0], pos[1] + 1.0, pos[2]]);
        if d <= inside && m.nearby_state != 1 {
            if visible {
                m.nearby_state = 1;
                self.fire_ref(m, s.get("on_inside_range"), Some(t));
            }
        } else if d > outside && m.nearby_state != 2 {
            m.nearby_state = 2;
            self.fire_ref(m, s.get("on_outside_range"), Some(t));
        } else if m.nearby_state == 1 && !visible && s.get("on_vision_lost_inside_range").is_some() {
            m.nearby_state = 0;
            self.fire_ref(m, s.get("on_vision_lost_inside_range"), Some(t));
        }
    }

    fn entity_sensor(&mut self, m: &mut Mob) {
        let Some(s) = m.comps.get("minecraft:entity_sensor").cloned() else { return };
        let subs: Vec<Value> = match s.get("subsensors") {
            Some(Value::Array(a)) => a.clone(),
            _ => vec![s.clone()],
        };
        if m.sensor_state.len() != subs.len() {
            m.sensor_state = vec![false; subs.len()];
        }
        for (i, sub) in subs.iter().enumerate() {
            let (rx, ry) = match sub.get("range") {
                Some(Value::Array(a)) => (as_num(a.first()).unwrap_or(10.0), as_num(a.get(1)).unwrap_or(10.0)),
                other => {
                    let r = as_num(other).unwrap_or(num(sub, "sensor_range", num(&s, "sensor_range", 10.0)));
                    (r, r)
                }
            };
            let min = num(sub, "minimum_count", 1.0) as i32;
            let max = num(sub, "maximum_count", -1.0) as i32;
            let filt = sub.get("event_filters");
            let ents = self.entities_near(m.world, m.pos, rx.max(ry), Some(m.entity_id));
            let count = ents
                .iter()
                .filter(|e| {
                    let Some(o) = self.subj(**e) else { return false };
                    let p = o.feet();
                    (p[0] - m.pos[0]).abs() <= rx && (p[2] - m.pos[2]).abs() <= rx && (p[1] - m.pos[1]).abs() <= ry
                        && filters::test(filt, &Cx::new(self, m).with_other_subj(o))
                })
                .count() as i32;
            let hit = count >= min && (max < 0 || count <= max);
            if hit && !m.sensor_state[i] {
                let ev = sub.get("event").cloned();
                if let Some(Value::String(e)) = ev {
                    self.fire(m, &e, "self", None, 0);
                }
            }
            if let Some(st) = m.sensor_state.get_mut(i) {
                *st = hit;
            }
        }
    }

    /// Players and mobs of a world within `r` blocks (cube) of `at`, except the mob `skip`.
    pub(crate) fn entities_near(&self, world: usize, at: [f32; 3], r: f32, skip: Option<i64>) -> Vec<Ent> {
        let mut out = Vec::new();
        for (sid, p) in &self.players {
            if p.world == world && p.state == State::Spawned && !p.dead && p.gamemode != 6 {
                let f = [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]];
                if (0..3).all(|k| (f[k] - at[k]).abs() <= r) {
                    out.push(Ent::Player(*sid));
                }
            }
        }
        for o in &self.mobs {
            if o.world == world && o.alive() && Some(o.entity_id) != skip && (0..3).all(|k| (o.pos[k] - at[k]).abs() <= r) {
                out.push(Ent::Mob(o.entity_id));
            }
        }
        out
    }

    /// Replaces the mob with what `minecraft:transformation` names.
    fn transform(&mut self, m: &mut Mob) {
        let Some(tr) = m.comps.get("minecraft:transformation").cloned() else { return };
        let Some(into) = string(&tr, "into") else { return };
        let (id, event) = match into.split_once('<') {
            Some((a, b)) => (a.to_string(), Some(b.trim_end_matches('>').to_string())),
            None => (into.to_string(), None),
        };
        if let Some(s) = string(&tr, "transformation_sound") {
            let s = s.to_string();
            self.mob_sound(m, &s);
        }
        let Some(eid) = self.spawn_entity(m.world, &id, m.pos, event.as_deref().filter(|e| !e.is_empty())) else { return };
        let (yaw, name, persistent, owner) = (m.yaw, m.name_tag.clone(), m.persistent, m.owner.clone());
        let keep_owner = boolean(&tr, "keep_owner", false);
        let equip = if boolean(&tr, "preserve_equipment", false) { Some(m.equip.clone()) } else { None };
        let drop = boolean(&tr, "drop_equipment", false);
        if let Some(n) = self.mob_mut(eid) {
            n.yaw = yaw;
            n.name_tag = name;
            n.persistent = persistent;
            if keep_owner {
                n.owner = owner;
            }
            if let Some(e) = equip {
                n.equip = e;
            }
        }
        if drop {
            for it in [m.equip.hand.clone(), m.equip.offhand.clone()].into_iter().chain(m.equip.armor.iter().cloned()) {
                if !it.is_empty() {
                    self.drop_at(m.world, m.pos, it);
                }
            }
        }
        m.gone = true;
    }
}

fn timer_ticks(tm: &Value) -> u64 {
    if let Some(Value::Array(choices)) = tm.get("random_time_choices") {
        let total: f32 = choices.iter().map(|c| num(c, "weight", 1.0)).sum();
        let mut pick = rand::random::<f32>() * total;
        for c in choices {
            let w = num(c, "weight", 1.0);
            if pick < w {
                return (num(c, "value", 1.0) * 20.0) as u64;
            }
            pick -= w;
        }
    }
    let (lo, hi) = range(tm.get("time"), (1.0, 1.0));
    let secs = if boolean(tm, "randomInterval", true) { rand_f(lo, hi) } else { lo };
    (secs.max(0.05) * 20.0) as u64
}

fn spawn_entries(v: &Value) -> Vec<Value> {
    match v.get("entities") {
        Some(Value::Array(a)) => a.clone(),
        Some(o @ Value::Object(_)) => vec![o.clone()],
        _ => vec![v.clone()],
    }
}
