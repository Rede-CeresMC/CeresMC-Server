//! Potions: what each potion type (the item's meta) does, drinking them, and splash/lingering
//! potions bursting near players. Durations and levels are vanilla's.
//! TODO: lingering potions leave an area effect cloud; here they burst like a weaker splash.

use super::effects::effect;
use super::*;

/// (effect, amplifier, ticks; None = instant)
type PotionEffect = (u8, u8, Option<u32>);

const NIGHT_VISION: u8 = 16;
const INVISIBILITY: u8 = 14;
const JUMP_BOOST: u8 = 8;
const WATER_BREATHING: u8 = 13;
const SLOW_FALLING: u8 = 27;
const WIND_CHARGED: u8 = 32;
const WEAVING: u8 = 33;
const OOZING: u8 = 34;
const INFESTED: u8 = 35;

/// Effects of a potion by its Bedrock id (the item meta).
pub(crate) fn potion_effects(id: u32) -> Vec<PotionEffect> {
    use effect::*;
    let t = |e: u8, amp: u8, ticks: u32| (e, amp, Some(ticks));
    let i = |e: u8, amp: u8| (e, amp, None);
    match id {
        5 => vec![t(NIGHT_VISION, 0, 3600)],
        6 => vec![t(NIGHT_VISION, 0, 9600)],
        7 => vec![t(INVISIBILITY, 0, 3600)],
        8 => vec![t(INVISIBILITY, 0, 9600)],
        9 => vec![t(JUMP_BOOST, 0, 3600)],
        10 => vec![t(JUMP_BOOST, 0, 9600)],
        11 => vec![t(JUMP_BOOST, 1, 1800)],
        12 => vec![t(FIRE_RESISTANCE, 0, 3600)],
        13 => vec![t(FIRE_RESISTANCE, 0, 9600)],
        14 => vec![t(SPEED, 0, 3600)],
        15 => vec![t(SPEED, 0, 9600)],
        16 => vec![t(SPEED, 1, 1800)],
        17 => vec![t(SLOWNESS, 0, 1800)],
        18 => vec![t(SLOWNESS, 0, 4800)],
        19 => vec![t(WATER_BREATHING, 0, 3600)],
        20 => vec![t(WATER_BREATHING, 0, 9600)],
        21 => vec![i(INSTANT_HEALTH, 0)],
        22 => vec![i(INSTANT_HEALTH, 1)],
        23 => vec![i(INSTANT_DAMAGE, 0)],
        24 => vec![i(INSTANT_DAMAGE, 1)],
        25 => vec![t(POISON, 0, 900)],
        26 => vec![t(POISON, 0, 2400)],
        27 => vec![t(POISON, 1, 440)],
        28 => vec![t(REGENERATION, 0, 900)],
        29 => vec![t(REGENERATION, 0, 2400)],
        30 => vec![t(REGENERATION, 1, 440)],
        31 => vec![t(STRENGTH, 0, 3600)],
        32 => vec![t(STRENGTH, 0, 9600)],
        33 => vec![t(STRENGTH, 1, 1800)],
        34 => vec![t(WEAKNESS, 0, 1800)],
        35 => vec![t(WEAKNESS, 0, 4800)],
        36 => vec![t(WITHER, 1, 800)],
        37 => vec![t(SLOWNESS, 3, 400), t(RESISTANCE, 2, 400)],
        38 => vec![t(SLOWNESS, 3, 800), t(RESISTANCE, 2, 800)],
        39 => vec![t(SLOWNESS, 5, 400), t(RESISTANCE, 3, 400)],
        40 => vec![t(SLOW_FALLING, 0, 1800)],
        41 => vec![t(SLOW_FALLING, 0, 4800)],
        42 => vec![t(SLOWNESS, 3, 400)],
        43 => vec![t(WIND_CHARGED, 0, 3600)],
        44 => vec![t(WEAVING, 0, 3600)],
        45 => vec![t(OOZING, 0, 3600)],
        46 => vec![t(INFESTED, 0, 3600)],
        _ => vec![], // water, mundane, thick, awkward
    }
}

/// Colour of the splash particles (RGB), from the first effect.
fn potion_color(id: u32) -> i32 {
    let rgb = match potion_effects(id).first().map(|e| e.0) {
        Some(effect::SPEED) => 0x33EBFF,
        Some(effect::SLOWNESS) => 0x8BAFE0,
        Some(effect::STRENGTH) => 0xFFC700,
        Some(effect::INSTANT_HEALTH) => 0xF82423,
        Some(effect::INSTANT_DAMAGE) => 0xA9656A,
        Some(JUMP_BOOST) => 0xFDFF84,
        Some(effect::REGENERATION) => 0xCD5CAB,
        Some(effect::FIRE_RESISTANCE) => 0xFF9900,
        Some(WATER_BREATHING) => 0x98DAC0,
        Some(INVISIBILITY) => 0xF6F6F6,
        Some(NIGHT_VISION) => 0xC2FF66,
        Some(effect::WEAKNESS) => 0x484D48,
        Some(effect::POISON) => 0x87A363,
        Some(effect::WITHER) => 0x736156,
        Some(SLOW_FALLING) => 0xF3CFB9,
        Some(_) => 0x7F7F7F,
        None => 0x385DC6, // water
    };
    rgb | (0xFF << 24)
}

/// Effects that hurt or hinder: kept away from creative players, and from other players when
/// PvP is off.
pub(crate) fn is_harmful(id: u8) -> bool {
    matches!(
        id,
        effect::SLOWNESS
            | effect::MINING_FATIGUE
            | effect::INSTANT_DAMAGE
            | effect::NAUSEA
            | effect::HUNGER
            | effect::WEAKNESS
            | effect::POISON
            | effect::WITHER
            | effect::FATAL_POISON
            | 15 // blindness
            | 24 // levitation
            | 28 // bad omen
            | 30 // darkness
            | WIND_CHARGED
            | WEAVING
            | OOZING
            | INFESTED
    )
}

impl Server {
    /// Gives a potion's effects; `strength` (0..1) shortens them and weakens instant ones.
    pub(crate) fn apply_potion(&mut self, sid: SessionId, potion: u32, strength: f32) {
        self.apply_potion_filtered(sid, potion, strength, true);
    }

    fn apply_potion_filtered(&mut self, sid: SessionId, potion: u32, strength: f32, allow_harmful: bool) {
        let creative = self.players.get(&sid).is_some_and(|p| p.gamemode == 1 || p.gamemode == 6);
        for (id, amp, ticks) in potion_effects(potion) {
            if is_harmful(id) && (!allow_harmful || creative) {
                continue;
            }
            match ticks {
                Some(t) => {
                    let secs = ((t as f32 * strength) / 20.0).round() as u32;
                    if secs > 0 {
                        self.add_effect(sid, id, amp, Some(secs), true);
                    }
                }
                None => self.apply_instant_scaled(sid, id, amp, strength),
            }
        }
    }

    /// A splash (or lingering) potion bursts at `at`: everyone within 4 blocks gets the effects,
    /// stronger the closer they are (the player hit directly gets them in full).
    pub(crate) fn burst_potion(
        &mut self,
        world: usize,
        at: [f32; 3],
        potion: u32,
        lingering: bool,
        hit: Option<SessionId>,
        thrower: SessionId,
    ) {
        self.broadcast(&protocol::level_event(PARTICLE_SPLASH, at, potion_color(potion)), |_, q| q.world == world);
        self.broadcast(&protocol::level_sound_event("glass", at, -1), |_, q| q.world == world);
        let near: Vec<(SessionId, f32)> = self
            .players
            .iter()
            .filter(|(_, q)| q.world == world && q.state == State::Spawned && !q.dead && q.gamemode != 6)
            .map(|(s, q)| {
                let body = [q.pos[0], q.pos[1] - 0.8, q.pos[2]];
                (*s, ((body[0] - at[0]).powi(2) + (body[1] - at[1]).powi(2) + (body[2] - at[2]).powi(2)).sqrt())
            })
            .filter(|(_, d)| *d < 4.0)
            .collect();
        for (sid, dist) in near {
            let mut strength = if Some(sid) == hit { 1.0 } else { 1.0 - dist / 4.0 };
            if lingering {
                strength *= 0.25;
            }
            // with PvP off, a potion thrown by someone else only helps
            let allow_harmful = self.cfg.pvp || sid == thrower;
            self.apply_potion_filtered(sid, potion, strength, allow_harmful);
        }
    }
}

const PARTICLE_SPLASH: i32 = 2002;
