//! Riding: a player mounts a mob whose `minecraft:rideable` lets players on (a saddled pig, a
//! horse, a saddled strider...). SetActorLink (41) ties them; the client then drives (Bedrock
//! moves the vehicle on the rider's side with server-authoritative movement) and the server
//! keeps the mob under the rider's seat. Leaving: Interact StopRiding (3), death, logout.
//! Also `minecraft:equippable` (saddles and horse armour) and `minecraft:tamemount` (riding a
//! wild horse until it accepts the rider or throws them off).

use super::json::*;
use super::*;
use serde_json::Value;

const LINK_NONE: u8 = 0;
const LINK_RIDER: u8 = 1;

impl Server {
    fn seat_of(m: &Mob) -> [f32; 3] {
        let Some(r) = m.comps.get("minecraft:rideable") else { return [0.0, m.t.height, 0.0] };
        let seat = match r.get("seats") {
            Some(Value::Array(a)) => a.first().cloned(),
            Some(o) => Some(o.clone()),
            None => None,
        };
        let p = seat.as_ref().and_then(|s| s.get("position"));
        let v = |i: usize| p.and_then(|p| p.get(i)).and_then(|v| as_num(Some(v))).unwrap_or(0.0);
        [v(0), if p.is_some() { v(1) } else { m.t.height }, v(2)]
    }

    /// The player rides `m` if its rideable component lets players on. True when they did.
    pub(crate) fn try_mount(&mut self, sid: SessionId, m: &mut Mob, sneaking: bool) -> bool {
        let Some(r) = m.comps.get("minecraft:rideable").cloned() else { return false };
        let families = str_list(r.get("family_types"));
        if !families.is_empty() && !families.iter().any(|f| f == "player") {
            return false;
        }
        if (sneaking && boolean(&r, "crouching_skip_interact", true)) || m.t.baby || !m.riders.is_empty() {
            return false;
        }
        let Some(p) = self.players.get_mut(&sid) else { return false };
        if p.riding.is_some() || p.dead {
            return false;
        }
        p.riding = Some(m.entity_id);
        let peid = p.entity_id;
        m.riders.push(sid);
        m.ctl = Ctl::default();
        m.path.clear();
        let seat = Self::seat_of(m);
        let pk = protocol::set_actor_link(m.entity_id, peid, LINK_RIDER, true, false);
        self.broadcast(&pk, |_, q| q.world == m.world);
        let seat_meta = protocol::set_actor_metadata(peid, &[Meta::Vec3(56, [seat[0], seat[1] + EYE_HEIGHT * 0.0, seat[2]])]);
        self.broadcast(&seat_meta, |_, q| q.world == m.world);
        true
    }

    /// The player gets off whatever they ride.
    pub(crate) fn dismount(&mut self, sid: SessionId) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(veid) = p.riding.take() else { return };
        let (peid, world) = (p.entity_id, p.world);
        let pk = protocol::set_actor_link(veid, peid, LINK_NONE, true, false);
        self.broadcast(&pk, |_, q| q.world == world);
        if let Some(i) = self.mobsys.index.get(&veid).copied() {
            if let Some(m) = self.mobs.get_mut(i).filter(|m| m.entity_id == veid) {
                m.riders.retain(|r| *r != sid);
            }
        } else if let Some(m) = self.mobs.iter_mut().find(|m| m.entity_id == veid) {
            m.riders.retain(|r| *r != sid);
        }
    }

    /// Each tick of a mob carrying a player: it stays under the seat, and a wild horse decides
    /// whether it accepts its rider. True when the mob is ridden (its own AI is off).
    pub(crate) fn ridden_tick(&mut self, m: &mut Mob) -> bool {
        m.riders.retain(|sid| self.players.get(sid).is_some_and(|p| p.riding == Some(m.entity_id) && !p.dead && p.world == m.world));
        let Some(&sid) = m.riders.first() else { return false };
        let Some(p) = self.players.get(&sid) else { return false };
        let seat = Self::seat_of(m);
        let (feet, yaw) = ([p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]], p.rot[1]);
        m.pos = [feet[0], feet[1] - seat[1], feet[2]];
        m.yaw = yaw;
        m.head_yaw = yaw;
        m.motion = [0.0; 3];
        m.fall = 0.0;
        m.on_ground = true;
        // a wild horse: tamed after a while, or it throws the rider off
        if !m.t.tamed && m.age % 40 == 0 {
            if let Some(tm) = m.comps.get("minecraft:tamemount").cloned() {
                if json::chance(0.25) {
                    let name = self.players.get(&sid).map(|p| p.name.clone());
                    m.owner = name;
                    m.persistent = true;
                    self.broadcast(&protocol::actor_event(m.entity_id, 7, 0), |_, q| q.world == m.world);
                    self.fire_ref(m, tm.get("tame_event"), Some(Ent::Player(sid)));
                } else if json::chance(0.4) {
                    self.broadcast(&protocol::actor_event(m.entity_id, 6, 0), |_, q| q.world == m.world);
                    self.dismount(sid);
                    m.riders.retain(|r| *r != sid);
                    return false;
                }
            }
        }
        true
    }

    /// `minecraft:equippable`: a saddle or horse armour from the hand onto the mob.
    pub(crate) fn try_equip_mob(&mut self, sid: SessionId, m: &mut Mob, held_name: &str, gm: i32) -> bool {
        let Some(eq) = m.comps.get("minecraft:equippable").cloned() else { return false };
        let slots = eq.get("slots").and_then(|s| s.as_array()).cloned().unwrap_or_default();
        for slot in slots {
            let accepted = item_list(slot.get("accepted_items"));
            if !accepted.iter().any(|a| a == held_name || a.replace('_', "") == held_name.replace('_', "")) {
                continue;
            }
            let index = num(&slot, "slot", 0.0) as i32;
            if index == 0 && m.t.saddled {
                return false;
            }
            if index == 1 {
                if let Some(it) = self.players.get(&sid).map(|p| p.inventory.held_item().clone()) {
                    let mut one = it;
                    one.count = 1;
                    let old = std::mem::replace(&mut m.equip.armor[1], one);
                    if !old.is_empty() {
                        self.give_stack(sid, old);
                    }
                }
            }
            self.fire_ref(m, slot.get("on_equip"), Some(Ent::Player(sid)));
            if gm != 1 {
                self.consume_held(sid, gm);
            }
            self.mob_sound(m, if index == 0 { "saddle" } else { "armor.equip_generic" });
            self.swing_arm(sid);
            return true;
        }
        false
    }
}

impl Server {
    /// Lead: used on a leashable mob it ties the mob to the player (LeashHolder metadata 37);
    /// used again by the holder it unties it and gives the lead back. True when handled.
    pub(crate) fn use_lead(&mut self, sid: SessionId, m: &mut Mob, held_name: &str, gm: i32) -> bool {
        // tied to a fence: anyone unties it and gets the lead
        if m.knot.take().is_some() {
            if gm != 1 {
                if let Some(it) = self.simple_item("minecraft:lead", 1) {
                    self.give_stack(sid, it);
                }
            }
            self.mob_sound(m, "leashknot.break");
            self.prune_knots();
            return true;
        }
        if m.leash.is_some() {
            if m.leash == Some(sid) {
                m.leash = None;
                if gm != 1 {
                    if let Some(it) = self.simple_item("minecraft:lead", 1) {
                        self.give_stack(sid, it);
                    }
                }
                self.mob_sound(m, "leashknot.break");
                return true;
            }
            return false;
        }
        if held_name != "minecraft:lead" || !m.comps.contains_key("minecraft:leashable") {
            return false;
        }
        m.leash = Some(sid);
        m.persistent = true;
        if gm != 1 {
            self.consume_held(sid, gm);
        }
        self.mob_sound(m, "leashknot.place");
        self.swing_arm(sid);
        true
    }

    /// A leashed mob is pulled towards its holder; too far, the lead snaps and drops.
    pub(crate) fn leash_tick(&mut self, m: &mut Mob) {
        if let Some(f) = m.knot {
            if !self.is_fence(m.world, f) {
                m.knot = None;
                self.snap_leash(m);
                self.prune_knots();
                return;
            }
            self.knot_eid(m.world, f);
            self.pull_to([f[0] as f32 + 0.5, f[1] as f32, f[2] as f32 + 0.5], m);
            return;
        }
        let Some(sid) = m.leash else { return };
        let holder = self.players.get(&sid).filter(|p| p.world == m.world && !p.dead && p.state == State::Spawned).map(|p| [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]);
        let Some(h) = holder else {
            self.snap_leash(m);
            return;
        };
        if !self.pull_to(h, m) {
            self.snap_leash(m);
        }
    }

    /// Pulls a leashed mob towards `h`; false when it is too far and the lead snaps.
    fn pull_to(&mut self, h: [f32; 3], m: &mut Mob) -> bool {
        let d = filters::dist(h, m.pos);
        if d > 10.0 {
            return false;
        }
        if d > 6.0 {
            let k = 0.05 * (d - 6.0);
            for i in 0..3 {
                m.motion[i] += (h[i] - m.pos[i]) / d * k;
            }
            m.ctl.dest = Some(h);
            m.ctl.speed = 1.2;
        }
        true
    }

    pub(crate) fn snap_leash(&mut self, m: &mut Mob) {
        m.leash = None;
        if m.knot.take().is_some() {
            self.prune_knots();
        }
        if let Some(it) = self.simple_item("minecraft:lead", 1) {
            self.drop_at(m.world, [m.pos[0], m.pos[1] + 0.5, m.pos[2]], it);
        }
        self.mob_sound(m, "leashknot.break");
    }
}

/// Leash knots: a player holding mobs on leads uses a fence and ties them to it (a
/// `minecraft:leash_knot` entity on the fence, shared by every mob tied there). Using the fence
/// again with nothing tied to the player takes the leads back; breaking the fence drops them.
impl Server {
    fn is_fence(&mut self, world: usize, pos: [i32; 3]) -> bool {
        let Some(w) = self.worlds.worlds.get_mut(world) else { return false };
        if !w.is_loaded(pos) {
            return true; // unloaded: keep it as it is
        }
        let id = w.get_block(pos, &self.gd);
        self.gd.block_name(id).ends_with("_fence")
    }

    fn knot_pos(f: [i32; 3]) -> [f32; 3] {
        [f[0] as f32 + 0.5, f[1] as f32 + 0.25, f[2] as f32 + 0.5]
    }

    /// The knot on a fence, made (and shown) when missing.
    fn knot_eid(&mut self, world: usize, f: [i32; 3]) -> i64 {
        if let Some(e) = self.mobsys.knots.get(&(world, f)) {
            return *e;
        }
        let eid = self.next_entity_id;
        self.next_entity_id += 1;
        self.mobsys.knots.insert((world, f), eid);
        let pk = protocol::add_entity(eid, "minecraft:leash_knot", Self::knot_pos(f), [0.0; 3], 0.0, 0.0, 0.0, &[], &[]);
        self.broadcast(&pk, |_, q| q.world == world);
        self.broadcast(&protocol::level_sound_event("leashknot.place", Self::knot_pos(f), -1), |_, q| q.world == world);
        eid
    }

    /// Knots that no mob is tied to any more go away.
    pub(crate) fn prune_knots(&mut self) {
        let used: std::collections::HashSet<(usize, [i32; 3])> = self.mobs.iter().filter(|m| m.alive()).filter_map(|m| m.knot.map(|f| (m.world, f))).collect();
        let gone: Vec<((usize, [i32; 3]), i64)> = self.mobsys.knots.iter().filter(|(k, _)| !used.contains(k)).map(|(k, e)| (*k, *e)).collect();
        for ((world, f), eid) in gone {
            self.mobsys.knots.remove(&(world, f));
            self.broadcast(&protocol::remove_actor(eid), |_, q| q.world == world);
        }
    }

    /// The knots of a world (for a player joining it).
    pub(crate) fn knot_packets(&self, world: usize) -> Vec<Vec<u8>> {
        self.mobsys
            .knots
            .iter()
            .filter(|((w, _), _)| *w == world)
            .map(|((_, f), eid)| protocol::add_entity(*eid, "minecraft:leash_knot", Self::knot_pos(*f), [0.0; 3], 0.0, 0.0, 0.0, &[], &[]))
            .collect()
    }

    /// A player uses a fence: ties the mobs they lead to it, or takes back the ones tied there.
    /// True when something happened.
    pub(crate) fn use_fence_leash(&mut self, sid: SessionId, f: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, feet) = (p.world, [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]);
        if !self.is_fence(world, f) {
            return false;
        }
        let at = Self::knot_pos(f);
        let led: Vec<usize> = (0..self.mobs.len()).filter(|&i| {
            let m = &self.mobs[i];
            m.alive() && m.world == world && m.leash == Some(sid) && filters::dist(m.pos, at) <= 7.0
        }).collect();
        if !led.is_empty() {
            for i in led {
                self.mobs[i].leash = None;
                self.mobs[i].knot = Some(f);
            }
            self.knot_eid(world, f);
            self.swing_arm(sid);
            return true;
        }
        let tied: Vec<usize> = (0..self.mobs.len()).filter(|&i| {
            let m = &self.mobs[i];
            m.alive() && m.world == world && m.knot == Some(f) && filters::dist(m.pos, feet) <= 10.0
        }).collect();
        if tied.is_empty() {
            return false;
        }
        for i in tied {
            self.mobs[i].knot = None;
            self.mobs[i].leash = Some(sid);
        }
        self.prune_knots();
        self.swing_arm(sid);
        true
    }
}
