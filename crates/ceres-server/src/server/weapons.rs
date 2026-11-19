//! Weapons used with the use button and special hits:
//! - crossbow: hold to charge (1.25 s, less with Quick Charge), it stays loaded (the arrow is kept
//!   in the item's `chargedItem`, so everyone sees it loaded), the next use fires it; Multishot
//!   fires three arrows.
//! - trident: hold and let go to throw it (it sticks, can be picked up, comes back with Loyalty);
//!   with Riptide in water or rain the player is launched instead.
//! - mace: a hit while falling more than 1.5 blocks is a smash: extra damage by the height
//!   (Density adds more), no fall damage, nearby entities pushed away, Wind Burst bounces the
//!   attacker up.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::{ItemDesc, window};

const EYE_HEIGHT: f32 = 1.62;
const ENCHANT_RIPTIDE: i16 = 30;
const ENCHANT_MULTISHOT: i16 = 33;
const ENCHANT_PIERCING: i16 = 34;
const ENCHANT_QUICK_CHARGE: i16 = 35;
const ENCHANT_WIND_BURST: i16 = 38;
const ENCHANT_DENSITY: i16 = 39;

/// Ticks to load a crossbow.
fn crossbow_charge_ticks(item: &ItemDesc) -> u64 {
    let qc = blocks::enchant_level(item, ENCHANT_QUICK_CHARGE) as u64;
    25u64.saturating_sub(5 * qc).max(1)
}

/// Extra damage of a mace hit after falling `fall` blocks (vanilla curve), plus Density.
pub(crate) fn mace_bonus(fall: f32, density: u8) -> f32 {
    if fall <= 1.5 {
        return 0.0;
    }
    let first = fall.min(3.0) * 4.0;
    let second = (fall - 3.0).clamp(0.0, 5.0) * 2.0;
    let rest = (fall - 8.0).max(0.0);
    first + second + rest + 0.5 * density as f32 * fall
}

impl Server {
    /// Puts an exact stack (enchantments, damage...) in the player's inventory, dropping what does
    /// not fit.
    pub(crate) fn give_stack(&mut self, sid: SessionId, item: ItemDesc) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let (changed, leftover) = p.inventory.add_item(item.clone(), &self.gd);
        for s in &changed {
            let idx = *s as usize;
            p.out.push(protocol::inventory_slot(window::INVENTORY as u32, idx as u32, &p.inventory.main[idx]));
        }
        if changed.contains(&p.inventory.held_slot) {
            self.broadcast_equipment(sid);
        }
        if leftover > 0 {
            let mut rest = item;
            rest.count = leftover;
            self.drop_from_player(sid, rest);
        }
    }

    fn held_name(&self, sid: SessionId) -> String {
        self.players.get(&sid).and_then(|p| self.gd.items_by_id.get(&p.inventory.held_item().id)).map(|i| i.name.clone()).unwrap_or_default()
    }

    /// Replaces the held item and tells the player and everyone around.
    fn set_held(&mut self, sid: SessionId, item: ItemDesc) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let slot = (p.inventory.held_slot as usize).min(8);
        p.inventory.set_main(slot, item);
        p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
        self.broadcast_equipment(sid);
    }

    /// Use button in the air with a weapon. True when it was one.
    pub(crate) fn weapon_click_air(&mut self, sid: SessionId) -> bool {
        let name = self.held_name(sid);
        let ticks = self.ticks;
        match name.as_str() {
            "minecraft:crossbow" => {
                let Some(p) = self.players.get(&sid) else { return true };
                if p.dead || p.gamemode == 6 {
                    return true;
                }
                let held = p.inventory.held_item().clone();
                if durability::item_field(&held, "chargedItem").is_some() {
                    self.fire_crossbow(sid, held);
                } else if let Some(p) = self.players.get_mut(&sid) {
                    if p.bow_since.is_none() {
                        p.bow_since = Some(ticks);
                        let at = p.pos;
                        let world = p.world;
                        self.broadcast(&protocol::level_sound_event("crossbow.loading.start", at, -1), |_, q| q.world == world);
                    }
                }
                true
            }
            n if n.ends_with("_spear") => {
                // held: a charge (see tick_spear_charges)
                if let Some(p) = self.players.get_mut(&sid) {
                    if !p.dead && p.gamemode != 6 && p.bow_since.is_none() {
                        p.bow_since = Some(ticks);
                    }
                }
                true
            }
            "minecraft:trident" => {
                if let Some(p) = self.players.get_mut(&sid) {
                    if !p.dead && p.gamemode != 6 && p.bow_since.is_none() {
                        p.bow_since = Some(ticks);
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// The use button was let go. True when a weapon handled it.
    pub(crate) fn weapon_release(&mut self, sid: SessionId) -> bool {
        let name = self.held_name(sid);
        if name.ends_with("_spear") {
            if let Some(p) = self.players.get_mut(&sid) {
                p.bow_since = None;
            }
            return true;
        }
        if name != "minecraft:crossbow" && name != "minecraft:trident" {
            return false;
        }
        let ticks = self.ticks;
        let Some(p) = self.players.get_mut(&sid) else { return true };
        let Some(since) = p.bow_since.take() else { return true };
        let held = p.inventory.held_item().clone();
        let held_for = ticks.saturating_sub(since);
        if name == "minecraft:crossbow" {
            if held_for >= crossbow_charge_ticks(&held) {
                self.load_crossbow(sid, held);
            }
        } else if held_for >= 5 {
            self.throw_trident(sid, held, held_for);
        }
        true
    }

    fn load_crossbow(&mut self, sid: SessionId, mut crossbow: ItemDesc) {
        let Some(p) = self.players.get(&sid) else { return };
        let creative = p.gamemode == 1;
        // a rocket in the other hand is loaded before arrows
        let off_rocket = self.gd.items_by_id.get(&p.inventory.offhand.id).is_some_and(|i| i.name == "minecraft:firework_rocket") && !p.inventory.offhand.is_empty();
        if off_rocket {
            let rocket = p.inventory.offhand.clone();
            let (world, at) = (p.world, p.pos);
            let mut fields = vec![
                ("Name".into(), Tag::String("minecraft:firework_rocket".into())),
                ("Count".into(), Tag::Byte(1)),
                ("Damage".into(), Tag::Short(0)),
                ("WasPickedUp".into(), Tag::Byte(0)),
            ];
            if let Some(fw) = durability::item_field(&rocket, "Fireworks") {
                fields.push(("tag".into(), Tag::Compound(vec![("Fireworks".into(), fw)])));
            }
            if !creative {
                let Some(p) = self.players.get_mut(&sid) else { return };
                p.inventory.offhand.count -= 1;
                if p.inventory.offhand.count == 0 {
                    p.inventory.offhand = ItemDesc::default();
                }
                self.broadcast_equipment(sid);
            }
            durability::set_item_field(&mut crossbow, "chargedItem", Some(Tag::Compound(fields)));
            self.set_held(sid, crossbow);
            self.broadcast(&protocol::level_sound_event("crossbow.loading.end", at, -1), |_, q| q.world == world);
            return;
        }
        let arrow_slot = p.inventory.main.iter().position(|it| !it.is_empty() && self.gd.items_by_id.get(&it.id).is_some_and(|i| i.name == "minecraft:arrow"));
        let off_arrow = self.gd.items_by_id.get(&p.inventory.offhand.id).is_some_and(|i| i.name == "minecraft:arrow") && !p.inventory.offhand.is_empty();
        if !creative && arrow_slot.is_none() && !off_arrow {
            return;
        }
        let (world, at) = (p.world, p.pos);
        if !creative {
            let Some(p) = self.players.get_mut(&sid) else { return };
            if off_arrow {
                p.inventory.offhand.count -= 1;
                if p.inventory.offhand.count == 0 {
                    p.inventory.offhand = ItemDesc::default();
                }
                self.broadcast_equipment(sid);
            } else if let Some(slot) = arrow_slot {
                let mut rest = p.inventory.main[slot].clone();
                rest.count -= 1;
                p.inventory.set_main(slot, if rest.count == 0 { ItemDesc::default() } else { rest });
                p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
            }
        }
        let charged = Tag::Compound(vec![
            ("Name".into(), Tag::String("minecraft:arrow".into())),
            ("Count".into(), Tag::Byte(1)),
            ("Damage".into(), Tag::Short(0)),
            ("WasPickedUp".into(), Tag::Byte(0)),
        ]);
        durability::set_item_field(&mut crossbow, "chargedItem", Some(charged));
        self.set_held(sid, crossbow);
        self.broadcast(&protocol::level_sound_event("crossbow.loading.end", at, -1), |_, q| q.world == world);
    }

    fn fire_crossbow(&mut self, sid: SessionId, mut crossbow: ItemDesc) {
        if !self.may_launch(sid) {
            return;
        }
        let Some(p) = self.players.get(&sid) else { return };
        let (world, at, creative) = (p.world, p.pos, p.gamemode == 1);
        let multishot = blocks::enchant_level(&crossbow, ENCHANT_MULTISHOT) > 0;
        let charged = durability::item_field(&crossbow, "chargedItem");
        let rocket = charged.as_ref().and_then(|c| c.get("Name")).and_then(|n| n.as_str()) == Some("minecraft:firework_rocket");
        if rocket {
            let data = charged
                .as_ref()
                .and_then(|c| c.get("tag"))
                .and_then(|t| t.get("Fireworks"))
                .cloned()
                .unwrap_or_else(|| Tag::Compound(vec![("Explosions".into(), Tag::List(10, Vec::new())), ("Flight".into(), Tag::Byte(1))]));
            self.shoot_firework(sid, data.clone(), 0.0);
            if multishot {
                self.shoot_firework(sid, data.clone(), -10.0);
                self.shoot_firework(sid, data, 10.0);
            }
        } else {
            let pierce = blocks::enchant_level(&crossbow, ENCHANT_PIERCING);
            self.launch_rot(sid, projectiles::Kind::Arrow, 3.15, 0.0, 0, 0, !creative, 0, None);
            self.set_last_pierce(pierce);
            if multishot {
                self.launch_rot(sid, projectiles::Kind::Arrow, 3.15, -10.0, 0, 0, false, 0, None);
                self.set_last_pierce(pierce);
                self.launch_rot(sid, projectiles::Kind::Arrow, 3.15, 10.0, 0, 0, false, 0, None);
                self.set_last_pierce(pierce);
            }
        }
        durability::set_item_field(&mut crossbow, "chargedItem", None);
        self.set_held(sid, crossbow);
        self.broadcast(&protocol::level_sound_event("crossbow.shoot", at, -1), |_, q| q.world == world);
        if !creative {
            self.wear_held(sid, if rocket { 3 } else if multishot { 3 } else { 1 });
        }
    }

    /// Throw force grows with the time held, as PowerNukkitX's ItemTrident: min((p^2+2p)/3, 1) * 2.5.
    fn throw_trident(&mut self, sid: SessionId, trident: ItemDesc, held_for: u64) {
        let secs = held_for as f32 / 20.0;
        let force = ((secs * secs + secs * 2.0) / 3.0).min(1.0) * 2.5;
        if force < 0.1 {
            return;
        }
        let Some(p) = self.players.get(&sid) else { return };
        let (world, pos, rot, creative, eid) = (p.world, p.pos, p.rot, p.gamemode == 1, p.entity_id);
        let riptide = blocks::enchant_level(&trident, ENCHANT_RIPTIDE);
        if riptide > 0 {
            // only in water or in the rain under the open sky
            let feet = [pos[0].floor() as i32, (pos[1] - EYE_HEIGHT).floor() as i32, pos[2].floor() as i32];
            let block = self.worlds.worlds[world].get_block(feet, &self.gd);
            let in_water = self.gd.block_name(block).contains("water");
            let rain_on = self.raining(world) && (feet[1] + 2..feet[1] + 64).all(|y| !self.gd.is_solid(self.worlds.worlds[world].get_block([feet[0], y, feet[2]], &self.gd)));
            if !(in_water || rain_on) {
                return;
            }
            let (pitch, yaw) = (rot[0].to_radians(), rot[1].to_radians());
            let dir = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
            let f = 3.0 * (1.0 + riptide as f32) / 4.0;
            let mot = protocol::set_actor_motion(eid, [dir[0] * f, dir[1] * f + 0.2, dir[2] * f]);
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(mot.clone());
                p.fall_start_y = None;
            }
            self.broadcast(&mot, |o, q| o != sid && q.world == world);
            let sound = format!("item.trident.riptide_{}", riptide.clamp(1, 3));
            self.broadcast(&protocol::level_sound_event(&sound, pos, -1), |_, q| q.world == world);
            if !creative {
                self.wear_held(sid, 1);
            }
            return;
        }
        if !self.may_launch(sid) {
            return;
        }
        let mut thrown = trident.clone();
        thrown.count = 1;
        thrown.stack_id = 0;
        if !creative {
            durability::wear_item(&mut thrown, 1, &self.gd);
            self.set_held(sid, ItemDesc::default());
        }
        self.launch_rot(sid, projectiles::Kind::Trident, force, 0.0, 0, 0, !creative, 0, Some(thrown));
    }

    /// Mace smash after a hit landed: the fall is used up, nearby entities are pushed away,
    /// Wind Burst bounces the attacker up. `fall` is how far the attacker had fallen.
    pub(crate) fn mace_smash_effects(&mut self, sid: SessionId, victim_pos: [f32; 3], victim: Option<i64>, fall: f32) {
        let Some(p) = self.players.get(&sid) else { return };
        let held = p.inventory.held_item().clone();
        let (world, eid, feet_y) = (p.world, p.entity_id, p.pos[1] - EYE_HEIGHT);
        let wind = blocks::enchant_level(&held, ENCHANT_WIND_BURST);
        if let Some(p) = self.players.get_mut(&sid) {
            p.fall_start_y = Some(feet_y); // the fall that made the smash does not hurt
        }
        // as PowerNukkitX's ItemMace.onPostAttack: ground dust, and the heavy sound for big smashes
        let sound = if weapons_smash_damage(fall) >= 16.0 { "mace.heavy_smash_ground" } else { "mace.smash_ground" };
        self.broadcast(&protocol::level_sound_event(sound, victim_pos, -1), |_, q| q.world == world);
        self.broadcast(&protocol::level_event(9815, victim_pos, 0), |_, q| q.world == world); // ParticleSmashAttackGroundDust
        // push away what stands around the victim
        let near = self.entities_near(world, victim_pos, 3.5, victim);
        for e in near {
            match e {
                mobs::Ent::Player(o) if o != sid => {
                    let Some(q) = self.players.get(&o) else { continue };
                    let (dx, dz) = (q.pos[0] - victim_pos[0], q.pos[2] - victim_pos[2]);
                    let d = (dx * dx + dz * dz).sqrt().max(0.3);
                    let pk = protocol::set_actor_motion(q.entity_id, [dx / d * 0.7, 0.5, dz / d * 0.7]);
                    if let Some(q) = self.players.get_mut(&o) {
                        q.out.push(pk.clone());
                    }
                    self.broadcast(&pk, |x, q| x != o && q.world == world);
                }
                mobs::Ent::Mob(m) => {
                    if let Some(mm) = self.mobs.iter_mut().find(|x| x.entity_id == m) {
                        let (dx, dz) = (mm.pos[0] - victim_pos[0], mm.pos[2] - victim_pos[2]);
                        let d = (dx * dx + dz * dz).sqrt().max(0.3);
                        mm.motion = [dx / d * 0.7, 0.5, dz / d * 0.7];
                    }
                }
                _ => {}
            }
        }
        if wind > 0 {
            let pk = protocol::set_actor_motion(eid, [0.0, 0.6 + 0.35 * wind as f32, 0.0]);
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(pk.clone());
            }
            self.broadcast(&pk, |o, q| o != sid && q.world == world);
            self.broadcast(&protocol::level_sound_event("wind_charge.burst", victim_pos, -1), |_, q| q.world == world);
        }
    }

    /// How far the player has fallen and the mace's Density level, when holding a mace.
    pub(crate) fn mace_fall(&self, sid: SessionId) -> Option<(f32, u8)> {
        let p = self.players.get(&sid)?;
        let held = p.inventory.held_item();
        if self.gd.items_by_id.get(&held.id)?.name != "minecraft:mace" || p.flying {
            return None;
        }
        let fall = p.fall_start_y? - (p.pos[1] - EYE_HEIGHT);
        (fall > 1.5).then_some((fall, blocks::enchant_level(held, ENCHANT_DENSITY)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mace_curve() {
        assert_eq!(mace_bonus(1.0, 0), 0.0);
        assert_eq!(mace_bonus(3.0, 0), 12.0);
        assert_eq!(mace_bonus(8.0, 0), 22.0);
        assert_eq!(mace_bonus(10.0, 0), 24.0);
        assert_eq!(mace_bonus(4.0, 2), 14.0 + 4.0);
    }
}

impl Server {
    /// Spear charge (as PowerNukkitX's ItemSpear.whileUsing, LGPL-3.0): while the use button
    /// is held and the player moves at least 0.13 blocks a tick, every 5 ticks the spear hits
    /// the closest living thing in front for 1.5 times its damage plus 3 per block of speed.
    pub(crate) fn tick_spear_charges(&mut self) {
        let users: Vec<(SessionId, [f32; 3], [f32; 3], usize, String)> = self
            .players
            .iter()
            .filter(|(_, p)| p.bow_since.is_some() && !p.dead)
            .filter_map(|(s, p)| {
                let name = self.gd.items_by_id.get(&p.inventory.held_item().id)?.name.clone();
                name.ends_with("_spear").then(|| (*s, p.pos, p.rot, p.world, name))
            })
            .collect();
        self.spear_prev.retain(|s, _| users.iter().any(|u| u.0 == *s));
        for (sid, pos, rot, world, name) in users {
            let prev = self.spear_prev.insert(sid, pos).unwrap_or(pos);
            let speed = ((pos[0] - prev[0]).powi(2) + (pos[1] - prev[1]).powi(2) + (pos[2] - prev[2]).powi(2)).sqrt();
            if self.ticks % 5 != 0 || speed < 0.13 {
                continue;
            }
            let (pitch, yaw) = (rot[0].to_radians(), rot[1].to_radians());
            let dir = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
            let feet = [pos[0], pos[1] - EYE_HEIGHT, pos[2]];
            let center = [feet[0] + dir[0] * 1.5, feet[1] + 0.9 + dir[1] * 1.5, feet[2] + dir[2] * 1.5];
            let target = self
                .entities_near(world, center, 2.0, None)
                .into_iter()
                .filter(|e| *e != mobs::Ent::Player(sid))
                .filter_map(|e| self.subj(e).map(|s| (e, mobs::filters_dist(s.feet(), feet))))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((target, _)) = target else { continue };
            let base = combat::item_damage_of(&name);
            let dmg = base * 1.5 + speed * 3.0;
            let kb = [dir[0] * 0.5, 0.3, dir[2] * 0.5];
            match target {
                mobs::Ent::Player(o) => {
                    if !self.cfg.pvp {
                        continue;
                    }
                    let victim = self.players.get(&o).map(|p| p.name.clone()).unwrap_or_default();
                    let attacker = self.players.get(&sid).map(|p| p.name.clone()).unwrap_or_default();
                    self.damage_player(o, dmg, &format!("§c{victim} foi atravessado por {attacker}"), false, Some(kb));
                }
                mobs::Ent::Mob(eid) => {
                    self.with_mob(eid, |s, m| s.hurt(m, dmg, "entity_attack", Some(mobs::Ent::Player(sid)), Some(feet)));
                }
            }
            let snd = format!("{}.attack_hit", name.trim_start_matches("minecraft:"));
            self.broadcast(&protocol::level_sound_event(&snd, pos, -1), |_, q| q.world == world);
        }
    }
}

fn weapons_smash_damage(fall: f32) -> f32 {
    7.0 + mace_bonus(fall, 0)
}

/// Lunge enchantment id (minecraft:lunge).
const LUNGE: i16 = 41;

impl Server {
    /// A spear stab (UseItem "use as attack", logic of PowerNukkitX's ItemSpear.onSpearStab,
    /// LGPL-3.0): the Lunge enchantment first pushes the player forward (0.5 + 0.4 per level,
    /// costing 4 exhaustion per level; not in water, and not under 6 hunger in survival); then,
    /// sprinting or riding, the spear hits the best target within 5 blocks in a narrow cone in
    /// front for its damage + 1.5 per Lunge level.
    pub(crate) fn spear_stab(&mut self, sid: SessionId) {
        let Some(p) = self.players.get(&sid) else { return };
        if p.dead || p.gamemode == 6 {
            return;
        }
        let held = p.inventory.held_item().clone();
        let Some(name) = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()) else { return };
        if !name.ends_with("_spear") {
            return;
        }
        let (pos, rot, world, gm, hunger, eid) = (p.pos, p.rot, p.world, p.gamemode, p.hunger, p.entity_id);
        let moving = p.actor_state.sprinting || p.riding.is_some();
        let riding = p.riding;
        if self.on_cooldown(sid, &name) {
            return;
        }
        self.start_cooldown(sid, &name, "spear", 20);
        let level = super::blocks::enchant_level(&held, LUNGE) as f32;
        let feet = [pos[0].floor() as i32, (pos[1] - EYE_HEIGHT).floor() as i32, pos[2].floor() as i32];
        let wet = self.liquid_at(world, feet).is_some() || self.liquid_at(world, [feet[0], feet[1] + 1, feet[2]]).is_some();
        let yaw = rot[1].to_radians();
        if level > 0.0 && !wet && !((gm == 0 || gm == 2) && hunger < 6.0) {
            let k = 0.5 + level * 0.4;
            let push = [-yaw.sin() * k, 0.0, yaw.cos() * k];
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(protocol::set_actor_motion(eid, push));
            }
            self.exhaust(sid, 4.0 * level);
            let snd = format!("item.enchant.lunge{}", (level as u32).clamp(1, 3));
            self.broadcast(&protocol::level_sound_event(&snd, pos, -1), |_, q| q.world == world);
        }
        let short = name.trim_start_matches("minecraft:");
        if !moving {
            self.broadcast(&protocol::level_sound_event(&format!("item.{short}.attack_miss"), pos, -1), |_, q| q.world == world);
            return;
        }
        let pitch = rot[0].to_radians();
        let dir = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
        let min_dot = if riding.is_some() { 0.75 } else { 0.866 };
        let mut best: Option<(mobs::Ent, f32, [f32; 3])> = None;
        for e in self.entities_near(world, pos, 6.0, None) {
            if e == mobs::Ent::Player(sid) || riding.is_some_and(|v| e == mobs::Ent::Mob(v)) {
                continue;
            }
            let Some(s) = self.subj(e) else { continue };
            let f = s.feet();
            let h = match e {
                mobs::Ent::Player(_) => 1.8,
                mobs::Ent::Mob(m) => self.mobs.iter().find(|x| x.entity_id == m).map_or(1.0, |x| x.t.height),
            };
            let t = [f[0], f[1] + h * 0.5, f[2]];
            let d = [t[0] - pos[0], t[1] - pos[1], t[2] - pos[2]];
            let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if dist > 5.0 || dist < 1e-3 {
                continue;
            }
            let dot = (d[0] * dir[0] + d[1] * dir[1] + d[2] * dir[2]) / dist;
            if dot < min_dot {
                continue;
            }
            let score = dot - dist / 5.0 * 0.1;
            if best.as_ref().is_none_or(|b| score > b.1) {
                best = Some((e, score, f));
            }
        }
        let Some((target, _, _)) = best else {
            self.broadcast(&protocol::level_sound_event(&format!("item.{short}.attack_miss"), pos, -1), |_, q| q.world == world);
            return;
        };
        let dmg = combat::item_damage_of(&name) + level * 1.5;
        let kb = [dir[0] * 0.4, 0.3, dir[2] * 0.4];
        let attacker_feet = [pos[0], pos[1] - EYE_HEIGHT, pos[2]];
        match target {
            mobs::Ent::Player(o) => {
                if !self.cfg.pvp {
                    return;
                }
                let victim = self.players.get(&o).map(|p| p.name.clone()).unwrap_or_default();
                let attacker = self.players.get(&sid).map(|p| p.name.clone()).unwrap_or_default();
                self.damage_player(o, dmg, &format!("§c{victim} foi atravessado por {attacker}"), false, Some(kb));
            }
            mobs::Ent::Mob(m) => {
                self.with_mob(m, |s, mob| s.hurt(mob, dmg, "entity_attack", Some(mobs::Ent::Player(sid)), Some(attacker_feet)));
            }
        }
        if gm != 1 {
            self.wear_held(sid, 1);
        }
        self.broadcast(&protocol::level_sound_event(&format!("item.{short}.attack_hit"), pos, -1), |_, q| q.world == world);
    }
}
