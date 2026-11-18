//! Hunger, saturation and exhaustion, natural regeneration and starvation, and eating.
//! Rules follow vanilla: exhaustion from sprinting, jumping, fighting, getting hurt and mining;
//! every 4 exhaustion costs a saturation point (or a hunger point when saturation is gone).
//! Eating: a click in the air starts it, the server finishes it after the eating time and waits for
//! the client's own "finished" click before another bite can start (same as PocketMine/Altay).

use super::*;
use ceres_protocol::ItemDesc;

/// Hunger points, saturation, eating time in ticks, edible when full, what is left in the hand.
pub(crate) struct Food {
    pub hunger: f32,
    pub saturation: f32,
    pub ticks: u64,
    pub always: bool,
    pub residue: Option<&'static str>,
}

const fn food(hunger: f32, saturation: f32) -> Food {
    Food { hunger, saturation, ticks: 32, always: false, residue: None }
}

pub(crate) fn food_of(name: &str) -> Option<Food> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    Some(match n {
        "apple" => food(4.0, 2.4),
        "baked_potato" => food(5.0, 6.0),
        "beetroot" => food(1.0, 1.2),
        "beetroot_soup" => Food { residue: Some("minecraft:bowl"), ..food(6.0, 7.2) },
        "bread" => food(5.0, 6.0),
        "carrot" => food(3.0, 3.6),
        "chorus_fruit" => Food { always: true, ..food(4.0, 2.4) },
        "cooked_beef" | "cooked_porkchop" => food(8.0, 12.8),
        "cooked_chicken" => food(6.0, 7.2),
        "cooked_cod" | "cooked_rabbit" => food(5.0, 6.0),
        "cooked_mutton" | "cooked_salmon" => food(6.0, 9.6),
        "cookie" => food(2.0, 0.4),
        "dried_kelp" => Food { ticks: 16, ..food(1.0, 0.6) },
        "enchanted_golden_apple" | "golden_apple" => Food { always: true, ..food(4.0, 9.6) },
        "golden_carrot" => food(6.0, 14.4),
        "honey_bottle" => Food { ticks: 40, residue: Some("minecraft:glass_bottle"), ..food(6.0, 1.2) },
        "melon_slice" => food(2.0, 1.2),
        "mushroom_stew" | "suspicious_stew" => Food { residue: Some("minecraft:bowl"), ..food(6.0, 7.2) },
        "poisonous_potato" => food(2.0, 1.2),
        "potato" => food(1.0, 0.6),
        "pufferfish" | "tropical_fish" => food(1.0, 0.2),
        "pumpkin_pie" => food(8.0, 4.8),
        "rabbit_stew" => Food { residue: Some("minecraft:bowl"), ..food(10.0, 12.0) },
        "beef" | "porkchop" | "rabbit" => food(3.0, 1.8),
        "chicken" | "mutton" => food(2.0, 1.2),
        "cod" | "salmon" => food(2.0, 0.4),
        "rotten_flesh" => food(4.0, 0.8),
        "potion" => Food { always: true, residue: Some("minecraft:glass_bottle"), ..food(0.0, 0.0) },
        "milk_bucket" => Food { always: true, residue: Some("minecraft:bucket"), ..food(0.0, 0.0) },
        "spider_eye" => food(2.0, 3.2),
        "sweet_berries" | "glow_berries" => food(2.0, 0.4),
        _ => return None,
    })
}

/// Effects a food gives: (effect, amplifier, seconds, chance out of 100).
fn food_effects(name: &str) -> &'static [(u8, u8, u32, u32)] {
    use super::effects::effect::*;
    match name.strip_prefix("minecraft:").unwrap_or(name) {
        "golden_apple" => &[(REGENERATION, 1, 5, 100), (ABSORPTION, 0, 120, 100)],
        "enchanted_golden_apple" => &[(REGENERATION, 1, 30, 100), (ABSORPTION, 3, 120, 100), (RESISTANCE, 0, 300, 100), (FIRE_RESISTANCE, 0, 300, 100)],
        "pufferfish" => &[(HUNGER, 2, 15, 100), (NAUSEA, 0, 15, 100), (POISON, 1, 60, 100)],
        "rotten_flesh" => &[(HUNGER, 0, 30, 80)],
        "chicken" => &[(HUNGER, 0, 30, 30)],
        "spider_eye" => &[(POISON, 0, 5, 100)],
        "poisonous_potato" => &[(POISON, 0, 5, 60)],
        _ => &[],
    }
}

const ACTOR_EVENT_EATING_ITEM: u8 = 57;

pub(crate) mod exhaustion {
    pub const SPRINT_PER_BLOCK: f32 = 0.1;
    pub const JUMP: f32 = 0.05;
    pub const SPRINT_JUMP: f32 = 0.2;
    pub const ATTACK: f32 = 0.1;
    pub const HURT: f32 = 0.1;
    pub const MINE: f32 = 0.005;
    pub const REGEN: f32 = 6.0;
}

impl Player {
    pub(crate) fn vitals_packet(&self) -> Vec<u8> {
        protocol::update_attributes_vitals(
            self.entity_id,
            &protocol::Vitals {
                health: self.health,
                hunger: self.hunger,
                saturation: self.saturation,
                exhaustion: self.exhaustion,
                absorption: self.absorption,
                movement: self.movement_speed(),
                xp_level: self.xp_level,
                xp_progress: xp::progress(self.xp_level, self.xp_points),
            },
        )
    }

    pub(crate) fn reset_food(&mut self) {
        self.hunger = 20.0;
        self.saturation = 5.0;
        self.exhaustion = 0.0;
        self.food_timer = 0;
        self.eating = None;
        self.awaiting_release = false;
    }
}

impl Server {
    /// Adds exhaustion (survival and adventure only).
    pub(crate) fn exhaust(&mut self, sid: SessionId, amount: f32) {
        if let Some(p) = self.players.get_mut(&sid) {
            if (p.gamemode == 0 || p.gamemode == 2) && !p.dead {
                p.exhaustion = (p.exhaustion + amount).min(40.0);
            }
        }
    }

    /// A click in the air with the held item (InventoryTransaction UseItem CLICK_AIR).
    pub(crate) fn on_click_air(&mut self, sid: SessionId) {
        if self.throw_held(sid) || self.start_bow(sid) || self.equip_held_armor(sid) || self.use_firework_in_air(sid) {
            return;
        }
        let ticks = self.ticks;
        let Some(p) = self.players.get_mut(&sid) else { return };
        let held = p.inventory.held_item();
        let Some(f) = self.gd.items_by_id.get(&held.id).and_then(|i| food_of(&i.name)) else { return };
        if p.awaiting_release {
            p.awaiting_release = false; // the client's own "done eating" click
            return;
        }
        if let Some(start) = p.eating {
            // the client may finish a little before the server does
            if ticks.saturating_sub(start) + 4 >= f.ticks {
                self.finish_eating(sid);
            }
            return;
        }
        if p.dead || p.gamemode == 6 || (p.hunger >= 20.0 && !f.always && p.gamemode != 1) {
            return;
        }
        p.eating = Some(ticks);
    }

    /// The client let go of the use button (InventoryTransaction ReleaseItem).
    pub(crate) fn on_release_item(&mut self, sid: SessionId) {
        self.release_bow(sid);
        if let Some(p) = self.players.get_mut(&sid) {
            p.eating = None;
            p.awaiting_release = false;
        }
    }

    fn finish_eating(&mut self, sid: SessionId) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        p.eating = None;
        p.awaiting_release = true;
        let slot = (p.inventory.held_slot as usize).min(8);
        let held = p.inventory.main[slot].clone();
        let Some(info) = self.gd.items_by_id.get(&held.id) else { return };
        let Some(f) = food_of(&info.name) else { return };
        p.hunger = (p.hunger + f.hunger).min(20.0);
        p.saturation = (p.saturation + f.saturation).min(p.hunger);
        let creative = p.gamemode == 1;
        let pk = p.vitals_packet();
        p.out.push(pk);
        let name = info.name.clone();
        if name == "minecraft:milk_bucket" {
            self.clear_effects(sid);
        }
        if name == "minecraft:potion" {
            self.apply_potion(sid, held.meta, 1.0);
        }
        if name != "minecraft:potion" && name != "minecraft:milk_bucket" && name != "minecraft:honey_bottle" {
            if let Some(p) = self.players.get(&sid) {
                let (world, pos) = (p.world, p.pos);
                self.broadcast(&protocol::level_sound_event("burp", pos, -1), |_, q| q.world == world);
            }
        }
        for &(id, amp, secs, chance) in food_effects(&name) {
            if rand::random::<u32>() % 100 < chance {
                self.add_effect(sid, id, amp, Some(secs), true);
            }
        }
        if creative {
            return;
        }
        let Some(p) = self.players.get_mut(&sid) else { return };
        let mut rest = held.clone();
        rest.count -= 1;
        p.inventory.set_main(slot, if rest.count == 0 { ItemDesc::default() } else { rest });
        let residue = f.residue.and_then(|name| self.gd.items_by_name.get(name)).map(|i| ItemDesc {
            id: i.id,
            count: 1,
            meta: 0,
            stack_id: 0,
            block_runtime_id: i.block_runtime_id,
            extra: ceres_world::extra_data(None),
        });
        let mut leftover = None;
        if let Some(r) = residue {
            if p.inventory.main[slot].is_empty() {
                p.inventory.set_main(slot, r);
            } else {
                let (_, left) = p.inventory.add_item(r.clone(), &self.gd);
                if left > 0 {
                    leftover = Some(r);
                }
            }
        }
        self.sync_full_inventory(sid);
        self.broadcast_equipment(sid);
        if let Some(r) = leftover {
            self.drop_from_player(sid, r);
        }
    }

    /// Per-tick hunger: eating, exhaustion, regeneration and starvation.
    pub(crate) fn tick_food(&mut self, sid: SessionId) {
        let (ticks, difficulty) = (self.ticks, self.cfg.difficulty);
        let Some(p) = self.players.get_mut(&sid) else { return };
        if p.dead {
            return;
        }
        if let Some(start) = p.eating {
            let need = self.gd.items_by_id.get(&p.inventory.held_item().id).and_then(|i| food_of(&i.name)).map(|f| f.ticks);
            match need {
                Some(n) if ticks.saturating_sub(start) >= n => self.finish_eating(sid),
                Some(_) => {
                    // every 4 ticks: eating particles and sound for everyone around
                    if ticks.saturating_sub(start) % 4 == 0 {
                        let held = p.inventory.held_item();
                        let data = ((held.id as i32) << 16) | (held.meta as i32 & 0xffff);
                        let (world, eid) = (p.world, p.entity_id);
                        self.broadcast(&protocol::actor_event(eid, ACTOR_EVENT_EATING_ITEM, data), |_, q| q.world == world);
                    }
                }
                None => p.eating = None, // switched to something that is not food
            }
        }
        let Some(p) = self.players.get_mut(&sid) else { return };
        let survival = p.gamemode == 0 || p.gamemode == 2;
        let before = (p.hunger, p.saturation, p.health);
        if difficulty == 0 && p.hunger < 20.0 && ticks % 20 == 0 {
            p.hunger += 1.0;
        }
        if p.exhaustion >= 4.0 {
            p.exhaustion -= 4.0;
            if p.saturation > 0.0 {
                p.saturation = (p.saturation - 1.0).max(0.0);
            } else if difficulty != 0 {
                p.hunger = (p.hunger - 1.0).max(0.0);
            }
        }
        p.food_timer += 1;
        let mut starve = false;
        if p.health < 20.0 && (p.saturation > 0.0 && p.hunger >= 20.0) {
            if p.food_timer >= 10 {
                p.food_timer = 0;
                p.health = (p.health + 1.0).min(20.0);
                if survival {
                    p.exhaustion += exhaustion::REGEN;
                }
            }
        } else if p.health < 20.0 && (p.hunger >= 18.0 || difficulty == 0) {
            if p.food_timer >= 80 {
                p.food_timer = 0;
                p.health = (p.health + 1.0).min(20.0);
                if survival {
                    p.exhaustion += exhaustion::REGEN;
                }
            }
        } else if p.hunger <= 0.0 && survival {
            if p.food_timer >= 80 {
                p.food_timer = 0;
                starve = difficulty == 3 || (difficulty == 2 && p.health > 1.0) || (difficulty == 1 && p.health > 10.0);
            }
        } else {
            p.food_timer = 0;
        }
        let changed = before != (p.hunger, p.saturation, p.health);
        let name = p.name.clone();
        if changed {
            let pk = p.vitals_packet();
            p.out.push(pk);
        }
        if starve {
            self.damage_player(sid, 1.0, &format!("§c{name} morreu de fome"), true, None);
        }
    }
}
