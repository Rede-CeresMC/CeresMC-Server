//! Status effects (MobEffect): speed, haste, strength, regeneration, poison and the rest, with
//! amplifier and duration, applied every tick. Vanilla rules; ids are Bedrock's.

use super::*;

pub(crate) mod effect {
    pub const SPEED: u8 = 1;
    pub const SLOWNESS: u8 = 2;
    pub const HASTE: u8 = 3;
    pub const MINING_FATIGUE: u8 = 4;
    pub const STRENGTH: u8 = 5;
    pub const INSTANT_HEALTH: u8 = 6;
    pub const INSTANT_DAMAGE: u8 = 7;
    pub const NAUSEA: u8 = 9;
    pub const REGENERATION: u8 = 10;
    pub const RESISTANCE: u8 = 11;
    pub const FIRE_RESISTANCE: u8 = 12;
    pub const HUNGER: u8 = 17;
    pub const WEAKNESS: u8 = 18;
    pub const POISON: u8 = 19;
    pub const WITHER: u8 = 20;
    pub const ABSORPTION: u8 = 22;
    pub const SATURATION: u8 = 23;
    pub const FATAL_POISON: u8 = 25;
}

/// Command names of the effects, by id.
pub(crate) const EFFECT_NAMES: [(&str, u8); 36] = [
    ("speed", 1),
    ("slowness", 2),
    ("haste", 3),
    ("mining_fatigue", 4),
    ("strength", 5),
    ("instant_health", 6),
    ("instant_damage", 7),
    ("jump_boost", 8),
    ("nausea", 9),
    ("regeneration", 10),
    ("resistance", 11),
    ("fire_resistance", 12),
    ("water_breathing", 13),
    ("invisibility", 14),
    ("blindness", 15),
    ("night_vision", 16),
    ("hunger", 17),
    ("weakness", 18),
    ("poison", 19),
    ("wither", 20),
    ("health_boost", 21),
    ("absorption", 22),
    ("saturation", 23),
    ("levitation", 24),
    ("fatal_poison", 25),
    ("conduit_power", 26),
    ("slow_falling", 27),
    ("bad_omen", 28),
    ("village_hero", 29),
    ("darkness", 30),
    ("trial_omen", 31),
    ("wind_charged", 32),
    ("weaving", 33),
    ("oozing", 34),
    ("infested", 35),
    ("raid_omen", 36),
];

pub(crate) fn effect_id(name: &str) -> Option<u8> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name).to_ascii_lowercase();
    EFFECT_NAMES.iter().find(|(e, _)| *e == n).map(|(_, id)| *id).or_else(|| n.parse().ok().filter(|v| (1..=36).contains(v)))
}

pub(crate) fn effect_name(id: u8) -> &'static str {
    EFFECT_NAMES.iter().find(|(_, i)| *i == id).map(|(n, _)| *n).unwrap_or("?")
}

fn instant(id: u8) -> bool {
    matches!(id, effect::INSTANT_HEALTH | effect::INSTANT_DAMAGE | effect::SATURATION)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Effect {
    pub amplifier: u8,
    /// ticks left; i32::MAX = infinite
    pub ticks: i32,
}

pub(crate) const MOB_EFFECT_ADD: u8 = 1;
pub(crate) const MOB_EFFECT_MODIFY: u8 = 2;
pub(crate) const MOB_EFFECT_REMOVE: u8 = 3;

impl Player {
    pub(crate) fn effect_level(&self, id: u8) -> Option<u8> {
        self.effects.get(&id).map(|e| e.amplifier)
    }

    /// Movement attribute with Speed and Slowness applied.
    pub(crate) fn movement_speed(&self) -> f32 {
        let mut m = 0.1;
        if let Some(a) = self.effect_level(effect::SPEED) {
            m *= 1.0 + 0.2 * (a as f32 + 1.0);
        }
        if let Some(a) = self.effect_level(effect::SLOWNESS) {
            m *= (1.0 - 0.15 * (a as f32 + 1.0)).max(0.0);
        }
        m
    }

    /// Mining speed multiplier from Haste and Mining Fatigue.
    pub(crate) fn mining_multiplier(&self) -> f32 {
        let mut m = 1.0;
        if let Some(a) = self.effect_level(effect::HASTE) {
            m *= 1.0 + 0.2 * (a as f32 + 1.0);
        }
        if let Some(a) = self.effect_level(effect::MINING_FATIGUE) {
            m *= 0.3f32.powi(a.min(3) as i32 + 1);
        }
        m
    }

    /// Melee damage bonus from Strength and Weakness.
    pub(crate) fn attack_bonus(&self) -> f32 {
        let s = self.effect_level(effect::STRENGTH).map_or(0.0, |a| 3.0 * (a as f32 + 1.0));
        let w = self.effect_level(effect::WEAKNESS).map_or(0.0, |a| 4.0 * (a as f32 + 1.0));
        s - w
    }

    /// Fraction of damage Resistance takes away.
    pub(crate) fn resistance(&self) -> f32 {
        self.effect_level(effect::RESISTANCE).map_or(0.0, |a| (0.2 * (a as f32 + 1.0)).min(1.0))
    }
}

impl Server {
    /// Gives an effect (replacing a weaker or shorter one, like vanilla). `seconds` None = infinite.
    pub(crate) fn add_effect(&mut self, sid: SessionId, id: u8, amplifier: u8, seconds: Option<u32>, particles: bool) {
        if instant(id) {
            self.apply_instant(sid, id, amplifier);
            return;
        }
        let ticks = seconds.map_or(i32::MAX, |s| (s.min(1_000_000) * 20) as i32);
        let Some(p) = self.players.get_mut(&sid) else { return };
        let event = match p.effects.get(&id) {
            Some(old) if old.amplifier > amplifier || (old.amplifier == amplifier && old.ticks >= ticks) => return,
            Some(_) => MOB_EFFECT_MODIFY,
            None => MOB_EFFECT_ADD,
        };
        p.effects.insert(id, Effect { amplifier, ticks });
        let duration = if ticks == i32::MAX { -1 } else { ticks };
        p.out.push(protocol::mob_effect(p.entity_id, event, id as i32, amplifier as i32, particles, duration));
        if id == effect::ABSORPTION {
            p.absorption = p.absorption.max(4.0 * (amplifier as f32 + 1.0));
        }
        let pk = if id == effect::SPEED || id == effect::SLOWNESS { p.vitals_with_movement() } else { p.vitals_packet() };
        p.out.push(pk);
    }

    pub(crate) fn remove_effect(&mut self, sid: SessionId, id: u8) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        if p.effects.remove(&id).is_none() {
            return;
        }
        p.out.push(protocol::mob_effect(p.entity_id, MOB_EFFECT_REMOVE, id as i32, 0, false, 0));
        if id == effect::ABSORPTION {
            p.absorption = 0.0;
        }
        let pk = if id == effect::SPEED || id == effect::SLOWNESS { p.vitals_with_movement() } else { p.vitals_packet() };
        p.out.push(pk);
    }

    pub(crate) fn clear_effects(&mut self, sid: SessionId) {
        let ids: Vec<u8> = self.players.get(&sid).map(|p| p.effects.keys().copied().collect()).unwrap_or_default();
        for id in ids {
            self.remove_effect(sid, id);
        }
    }

    fn apply_instant(&mut self, sid: SessionId, id: u8, amplifier: u8) {
        self.apply_instant_scaled(sid, id, amplifier, 1.0);
    }

    /// Instant effects at a fraction of their strength (splash potions further away).
    pub(crate) fn apply_instant_scaled(&mut self, sid: SessionId, id: u8, amplifier: u8, strength: f32) {
        let amount = 4.0 * 2f32.powi(amplifier.min(10) as i32) * strength.clamp(0.0, 1.0);
        if amount <= 0.0 {
            return;
        }
        match id {
            effect::INSTANT_HEALTH => {
                let Some(p) = self.players.get_mut(&sid) else { return };
                p.health = (p.health + amount).min(20.0);
                let pk = p.vitals_packet();
                p.out.push(pk);
            }
            effect::INSTANT_DAMAGE => {
                if self.players.get(&sid).is_some_and(|p| p.gamemode == 1 || p.gamemode == 6) {
                    return; // creative and spectator players cannot be hurt
                }
                let name = self.players.get(&sid).map(|p| p.name.clone()).unwrap_or_default();
                self.damage_player(sid, amount * 1.5, &format!("§c{name} foi morto por magia"), true, None);
            }
            effect::SATURATION => {
                let Some(p) = self.players.get_mut(&sid) else { return };
                p.hunger = (p.hunger + amplifier as f32 + 1.0).min(20.0);
                p.saturation = (p.saturation + 2.0 * (amplifier as f32 + 1.0)).min(p.hunger);
                let pk = p.vitals_packet();
                p.out.push(pk);
            }
            _ => {}
        }
    }

    /// Per-tick effect work: periodic healing/damage and expiry.
    pub(crate) fn tick_effects(&mut self, sid: SessionId) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        if p.effects.is_empty() || p.dead {
            return;
        }
        let mut expired = Vec::new();
        let mut heal: f32 = 0.0;
        let mut hurt: f32 = 0.0;
        let mut wither: f32 = 0.0;
        let mut exhaust: f32 = 0.0;
        for (&id, e) in p.effects.iter_mut() {
            if e.ticks != i32::MAX {
                e.ticks -= 1;
                if e.ticks <= 0 {
                    expired.push(id);
                    continue;
                }
            }
            let every = |base: i32| {
                let interval = (base >> e.amplifier.min(5)).max(1);
                e.ticks % interval == 0
            };
            match id {
                effect::REGENERATION if every(50) => heal += 1.0,
                effect::POISON | effect::FATAL_POISON if every(25) => hurt += 1.0,
                effect::WITHER if every(40) => wither += 1.0,
                effect::HUNGER => exhaust += 0.005 * (e.amplifier as f32 + 1.0),
                _ => {}
            }
        }
        let fatal = p.effects.contains_key(&effect::FATAL_POISON);
        let name = p.name.clone();
        if heal > 0.0 && p.health < 20.0 {
            p.health = (p.health + heal).min(20.0);
            let pk = p.vitals_packet();
            p.out.push(pk);
        }
        if exhaust > 0.0 && (p.gamemode == 0 || p.gamemode == 2) {
            p.exhaustion = (p.exhaustion + exhaust).min(40.0);
        }
        // plain poison never takes the last heart
        let health = p.health;
        if hurt > 0.0 && (fatal || health > 1.0) {
            let dmg = if fatal { hurt } else { hurt.min(health - 1.0) };
            if p.hurt_cooldown > 0 {
                p.hurt_cooldown = 0;
            }
            self.damage_player(sid, dmg, &format!("§c{name} morreu envenenado"), true, None);
        }
        if wither > 0.0 {
            if let Some(p) = self.players.get_mut(&sid) {
                p.hurt_cooldown = 0;
            }
            self.damage_player(sid, wither, &format!("§c{name} definhou até a morte"), true, None);
        }
        for id in expired {
            self.remove_effect(sid, id);
        }
    }
}
