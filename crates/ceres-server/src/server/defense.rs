//! Defensive items: the totem of undying (cheats death once) and the shield (blocks attacks and
//! arrows from the front while sneaking; axes knock it out for 5 seconds).

use super::*;
use ceres_protocol::{ItemDesc, window};

const ACTOR_EVENT_CONSUME_TOTEM: u8 = 65;
const SOUND_TOTEM: i32 = 1052;
/// ticks a shield stays down after an axe hit
const SHIELD_DISABLED_TICKS: u32 = 100;

fn named(gd: &GameData, item: &ItemDesc, name: &str) -> bool {
    !item.is_empty() && gd.items_by_id.get(&item.id).is_some_and(|i| i.name == name)
}

impl Server {
    /// A lethal hit: a totem in either hand saves the player. Returns true if it did.
    pub(crate) fn try_totem(&mut self, sid: SessionId) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        // nothing saves from the void
        if p.pos[1] < (ceres_world::MIN_Y - 16) as f32 {
            return false;
        }
        let slot = (p.inventory.held_slot as usize).min(8);
        let in_main = named(&self.gd, &p.inventory.main[slot], "minecraft:totem_of_undying");
        let in_off = named(&self.gd, &p.inventory.offhand, "minecraft:totem_of_undying");
        if !in_main && !in_off {
            return false;
        }
        let p = self.players.get_mut(&sid).unwrap();
        if in_off {
            p.inventory.offhand = ItemDesc::default();
            p.inventory.dirty = true;
            p.out.push(protocol::inventory_content(window::OFFHAND as u32, std::slice::from_ref(&p.inventory.offhand)));
        } else {
            p.inventory.set_main(slot, ItemDesc::default());
            p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
        }
        p.health = 1.0;
        let (world, eid, pos) = (p.world, p.entity_id, p.pos);
        self.clear_effects(sid);
        use effects::effect::*;
        self.add_effect(sid, REGENERATION, 1, Some(40), true);
        self.add_effect(sid, ABSORPTION, 1, Some(5), true);
        self.add_effect(sid, FIRE_RESISTANCE, 0, Some(40), true);
        self.broadcast(&protocol::actor_event(eid, ACTOR_EVENT_CONSUME_TOTEM, 0), |_, q| q.world == world);
        self.broadcast(&protocol::level_event(SOUND_TOTEM, pos, 0), |_, q| q.world == world);
        self.broadcast_equipment(sid);
        self.send_vitals_of(sid);
        true
    }

    fn send_vitals_of(&mut self, sid: SessionId) {
        if let Some(p) = self.players.get_mut(&sid) {
            let pk = p.vitals_packet();
            p.out.push(pk);
        }
    }

    fn has_shield(&self, p: &Player) -> bool {
        let slot = (p.inventory.held_slot as usize).min(8);
        named(&self.gd, &p.inventory.main[slot], "minecraft:shield") || named(&self.gd, &p.inventory.offhand, "minecraft:shield")
    }

    /// Every tick: a sneaking player holding a shield is blocking (unless an axe disabled it).
    pub(crate) fn tick_shield(&mut self, sid: SessionId) {
        let Some(p) = self.players.get(&sid) else { return };
        let blocking = p.actor_state.sneaking && p.shield_cooldown == 0 && !p.dead && self.has_shield(p);
        let changed = p.actor_state.blocking != blocking;
        let p = self.players.get_mut(&sid).unwrap();
        p.shield_cooldown = p.shield_cooldown.saturating_sub(1);
        p.shield_block_cooldown = p.shield_block_cooldown.saturating_sub(1);
        if changed {
            p.actor_state.blocking = blocking;
            self.sync_actor_state(sid);
        }
    }

    /// Whether `victim` blocks a hit coming from `from` (the attacker or the arrow's origin).
    /// A blocked hit makes the shield sound and wears the shield.
    pub(crate) fn shield_blocks(&mut self, victim: SessionId, from: [f32; 3], damage: f32, axe: bool) -> bool {
        let Some(p) = self.players.get(&victim) else { return false };
        if !p.actor_state.blocking || p.dead || p.state != State::Spawned {
            return false;
        }
        // only hits from the front half
        let yaw = p.rot[1].to_radians();
        let look = [-yaw.sin(), yaw.cos()];
        let to = [from[0] - p.pos[0], from[2] - p.pos[2]];
        if look[0] * to[0] + look[1] * to[1] <= 0.0 {
            return false;
        }
        // a shield blocks at most one hit per half second with sound and wear; the hits in
        // between are absorbed silently (its own timer: other damage is not affected)
        if p.shield_block_cooldown > 0 {
            return true;
        }
        let (world, pos) = (p.world, p.pos);
        if let Some(p) = self.players.get_mut(&victim) {
            p.shield_block_cooldown = 10;
        }
        self.broadcast(&protocol::level_sound_event("item.shield.block", pos, -1), |_, q| q.world == world);
        // vanilla: 1 + damage durability for hits of 3 or more
        if damage >= 3.0 {
            self.wear_shield(victim, 1 + damage.floor() as u32);
        }
        if axe {
            if let Some(p) = self.players.get_mut(&victim) {
                p.shield_cooldown = SHIELD_DISABLED_TICKS;
                p.actor_state.blocking = false;
            }
            self.sync_actor_state(victim);
        }
        true
    }

    fn wear_shield(&mut self, sid: SessionId, amount: u32) {
        let Some(p) = self.players.get(&sid) else { return };
        let slot = (p.inventory.held_slot as usize).min(8);
        // the shield in the off hand first (that is where it usually is)
        if named(&self.gd, &p.inventory.offhand, "minecraft:shield") {
            let mut shield = p.inventory.offhand.clone();
            let broke = durability::wear_item(&mut shield, amount, &self.gd);
            let p = self.players.get_mut(&sid).unwrap();
            p.inventory.offhand = shield;
            p.inventory.dirty = true;
            p.out.push(protocol::inventory_content(window::OFFHAND as u32, std::slice::from_ref(&p.inventory.offhand)));
            if broke {
                self.broadcast_equipment(sid);
            }
        } else if named(&self.gd, &p.inventory.main[slot], "minecraft:shield") {
            let mut shield = p.inventory.main[slot].clone();
            durability::wear_item(&mut shield, amount, &self.gd);
            let p = self.players.get_mut(&sid).unwrap();
            p.inventory.set_main(slot, shield);
            p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
            self.broadcast_equipment(sid);
        }
    }
}
