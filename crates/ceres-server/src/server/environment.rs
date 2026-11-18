//! Damage from the world around a player: lava and fire (and burning afterwards), drowning when
//! the head is under water too long, and suffocating inside a block. Survival and adventure only.

use super::*;

const EYE_HEIGHT: f32 = 1.62;
/// ticks of air a player has under water (15 s)
pub(crate) const MAX_AIR: i16 = 300;
const LAVA_BURN_TICKS: u32 = 300;
const FIRE_BURN_TICKS: u32 = 160;

impl Server {
    pub(crate) fn tick_environment(&mut self, sid: SessionId) {
        let ticks = self.ticks;
        let Some(p) = self.players.get(&sid) else { return };
        if p.dead || !(p.gamemode == 0 || p.gamemode == 2) {
            return;
        }
        let world = p.world;
        let eye = [p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32];
        let feet = [eye[0], (p.pos[1] - EYE_HEIGHT + 0.1).floor() as i32, eye[2]];
        let fire_resistant = p.effect_level(effects::effect::FIRE_RESISTANCE).is_some();
        let water_breathing = p.effect_level(13).is_some() || p.effect_level(26).is_some();
        let name = p.name.clone();
        let w = &mut self.worlds.worlds[world];
        let head_block = w.get_block(eye, &self.gd);
        let feet_block = w.get_block(feet, &self.gd);
        let head_name = self.gd.block_name(head_block).to_string();
        let feet_name = self.gd.block_name(feet_block).to_string();
        let is = |n: &str, what: &str| n == format!("minecraft:{what}") || n == format!("minecraft:flowing_{what}");
        let in_lava = is(&head_name, "lava") || is(&feet_name, "lava");
        let in_water = is(&head_name, "water") || is(&feet_name, "water");
        let head_in_water = is(&head_name, "water");
        let in_fire = matches!(feet_name.as_str(), "minecraft:fire" | "minecraft:soul_fire");
        let in_cauldron_water = self.is_water_cauldron(feet_block);
        let noclip = self.players.get(&sid).is_some_and(|p| p.noclip);
        let suffocating = self.gd.is_solid(head_block) && !in_lava && !noclip;

        let mut hurts: Vec<(f32, String, bool)> = Vec::new(); // damage, message, bypasses armor
        let Some(p) = self.players.get_mut(&sid) else { return };
        let before = p.actor_state;

        // fire and lava
        if in_lava && !fire_resistant {
            hurts.push((4.0, format!("§c{name} tentou nadar na lava"), false));
            p.fire_ticks = p.fire_ticks.max(LAVA_BURN_TICKS);
        } else if in_fire && !fire_resistant {
            hurts.push((1.0, format!("§c{name} morreu queimado"), false));
            p.fire_ticks = p.fire_ticks.max(FIRE_BURN_TICKS);
        }
        if in_water || in_cauldron_water {
            p.fire_ticks = 0;
        }
        if p.fire_ticks > 0 {
            p.fire_ticks -= 1;
            if fire_resistant {
                p.fire_ticks = 0;
            } else if p.fire_ticks % 20 == 0 && !in_lava {
                hurts.push((1.0, format!("§c{name} morreu queimado"), true));
            }
        }
        p.actor_state.on_fire = p.fire_ticks > 0;

        // air
        if head_in_water && !water_breathing {
            p.actor_state.air_lost = (p.actor_state.air_lost + 1).min(MAX_AIR);
            if p.actor_state.air_lost >= MAX_AIR && ticks % 20 == 0 {
                hurts.push((2.0, format!("§c{name} se afogou"), true));
            }
        } else if p.actor_state.air_lost > 0 {
            p.actor_state.air_lost = (p.actor_state.air_lost - 5).max(0);
        }

        if suffocating && ticks % 10 == 0 {
            hurts.push((1.0, format!("§c{name} sufocou dentro de uma parede"), true));
        }

        // air changes every tick under water; the bar only needs a few updates a second
        let changed = before.on_fire != p.actor_state.on_fire
            || (before.air_lost != p.actor_state.air_lost
                && (p.actor_state.air_lost % 10 == 0 || p.actor_state.air_lost == 0 || p.actor_state.air_lost == MAX_AIR));
        if changed {
            self.sync_actor_state(sid);
        }
        for (dmg, msg, bypass) in hurts {
            self.damage_player(sid, dmg, &msg, bypass, None);
        }
    }
}
