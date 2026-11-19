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
        let Some(sid) = m.leash else { return };
        let holder = self.players.get(&sid).filter(|p| p.world == m.world && !p.dead && p.state == State::Spawned).map(|p| [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]);
        let Some(h) = holder else {
            self.snap_leash(m);
            return;
        };
        let d = filters::dist(h, m.pos);
        if d > 10.0 {
            self.snap_leash(m);
        } else if d > 6.0 {
            let k = 0.05 * (d - 6.0);
            for i in 0..3 {
                m.motion[i] += (h[i] - m.pos[i]) / d * k;
            }
            m.ctl.dest = Some(h);
            m.ctl.speed = 1.2;
        }
    }

    fn snap_leash(&mut self, m: &mut Mob) {
        m.leash = None;
        if let Some(it) = self.simple_item("minecraft:lead", 1) {
            self.drop_at(m.world, [m.pos[0], m.pos[1] + 0.5, m.pos[2]], it);
        }
        self.mob_sound(m, "leashknot.break");
    }
}
