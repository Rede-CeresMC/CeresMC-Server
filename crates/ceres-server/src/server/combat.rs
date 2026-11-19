//! Health, PvP combat (weapon damage, armor reduction, knockback, hurt flash), fall/void damage,
//! death and respawn.

use super::*;
use ceres_protocol::{actor_event, respawn_state};

const EYE_HEIGHT: f32 = 1.62;

/// Damage of a weapon in the hand (the fist included).
pub(crate) fn item_damage_of(name: &str) -> f32 {
    item_attack_damage(name)
}

fn item_attack_damage(name: &str) -> f32 {
    match name.strip_prefix("minecraft:").unwrap_or(name) {
        "wooden_sword" | "golden_sword" => 5.0,
        "stone_sword" | "copper_sword" => 6.0,
        "iron_sword" => 7.0,
        "diamond_sword" => 8.0,
        "netherite_sword" => 9.0,
        "trident" => 9.0,
        "mace" => 7.0,
        "wooden_axe" | "golden_axe" => 4.0,
        "stone_axe" | "copper_axe" => 5.0,
        "iron_axe" => 6.0,
        "diamond_axe" => 7.0,
        "netherite_axe" => 8.0,
        "wooden_pickaxe" | "golden_pickaxe" => 3.0,
        "stone_pickaxe" | "copper_pickaxe" => 4.0,
        "iron_pickaxe" => 5.0,
        "diamond_pickaxe" => 6.0,
        "netherite_pickaxe" => 7.0,
        "wooden_shovel" | "golden_shovel" => 2.0,
        "stone_shovel" | "copper_shovel" => 3.0,
        "iron_shovel" => 4.0,
        "diamond_shovel" => 5.0,
        "netherite_shovel" => 6.0,
        "wooden_hoe" | "golden_hoe" => 3.0,
        "stone_hoe" | "copper_hoe" => 4.0,
        "iron_hoe" => 5.0,
        "diamond_hoe" => 6.0,
        "netherite_hoe" => 7.0,
        // item_components.nbt: minecraft:damage 1/2/2/3/1/4/5, plus the fist
        "wooden_spear" | "golden_spear" => 2.0,
        "stone_spear" | "copper_spear" => 3.0,
        "iron_spear" => 4.0,
        "diamond_spear" => 5.0,
        "netherite_spear" => 6.0,
        _ => 1.0,
    }
}

const ENCHANT_PROTECTION: i16 = 0;
const ENCHANT_FEATHER_FALLING: i16 = 2;
const ENCHANT_SHARPNESS: i16 = 9;
const ENCHANT_KNOCKBACK: i16 = 12;

/// Enchantment protection points of the armor worn: sum of level x weight, at most 20.
fn armor_enchant_points(p: &Player, weights: &[(i16, u32)]) -> u32 {
    let mut total = 0;
    for piece in p.inventory.armor.iter().filter(|a| !a.is_empty()) {
        for (id, weight) in weights {
            total += blocks::enchant_level(piece, *id) as u32 * weight;
        }
    }
    total.min(20)
}

fn armor_defense_points(name: &str) -> i32 {
    match name.strip_prefix("minecraft:").unwrap_or(name) {
        "leather_helmet" | "leather_boots" | "golden_boots" | "chainmail_boots" => 1,
        "leather_leggings" | "golden_helmet" | "chainmail_helmet" | "iron_helmet" | "iron_boots" | "turtle_helmet" => 2,
        "leather_chestplate" | "golden_leggings" | "diamond_helmet" | "diamond_boots" | "netherite_helmet"
        | "netherite_boots" => 3,
        "chainmail_leggings" => 4,
        "golden_chestplate" | "chainmail_chestplate" | "iron_leggings" => 5,
        "iron_chestplate" | "diamond_leggings" | "netherite_leggings" => 6,
        "diamond_chestplate" | "netherite_chestplate" => 8,
        _ => 0,
    }
}

impl Server {
    fn armor_reduction(&self, p: &Player) -> f32 {
        let mut pts = 0;
        for a in &p.inventory.armor {
            if !a.is_empty() {
                if let Some(info) = self.gd.items_by_id.get(&a.id) {
                    pts += armor_defense_points(&info.name);
                }
            }
        }
        (pts.min(20) as f32) * 0.04 // up to 80% damage reduction
    }

    pub(crate) fn respawn_destination(&mut self, sid: SessionId) -> (usize, [f32; 3]) {
        if let Some((wi, sp)) = self.players.get(&sid).and_then(|p| p.spawn_point) {
            if wi < self.worlds.worlds.len() {
                return (wi, [sp[0] as f32 + 0.5, sp[1] as f32, sp[2] as f32 + 0.5]);
            }
        }
        let wi = self.players.get(&sid).map(|p| p.world).unwrap_or(self.worlds.default);
        let sp = self.worlds.worlds[wi].safe_spawn(&self.gd);
        (wi, [sp[0] as f32 + 0.5, sp[1] as f32, sp[2] as f32 + 0.5])
    }

    /// Deals `raw_damage` to `sid`. Returns true if the damage landed.
    pub(crate) fn damage_player(
        &mut self,
        sid: SessionId,
        raw_damage: f32,
        death_msg: &str,
        bypass_armor: bool,
        knockback: Option<[f32; 3]>,
    ) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        if p.state != State::Spawned || p.dead || p.hurt_cooldown > 0 {
            return false;
        }
        let name = p.name.clone();
        if !self.plugins_allow(serde_json::json!({"event": "damage", "player": name, "amount": raw_damage})) {
            return false;
        }
        let Some(p) = self.players.get(&sid) else { return false };
        // peaceful only removes monsters and hunger: players still hurt each other (vanilla)
        if !bypass_armor && (p.gamemode == 1 || p.gamemode == 6) {
            return false;
        }
        let reduction = if bypass_armor { 0.0 } else { self.armor_reduction(p) };
        // Protection enchantments: 1 point per level, 4% each, at most 20 points
        let epf = if bypass_armor { 0 } else { armor_enchant_points(p, &[(ENCHANT_PROTECTION, 1)]) };
        let mut dmg = raw_damage * (1.0 - reduction) * (1.0 - epf as f32 * 0.04) * (1.0 - p.resistance());
        if raw_damage > 0.0 {
            dmg = dmg.max(0.5); // a snowball hurts nothing, anything else at least half a heart
        }

        let p = self.players.get_mut(&sid).unwrap();
        // a hit that does no damage (snowball) only pushes: no invulnerability frames, no hunger
        if raw_damage <= 0.0 {
            let (world, eid) = (p.world, p.entity_id);
            if let Some(kb) = knockback {
                let mot_pk = protocol::set_actor_motion(eid, kb);
                p.out.push(mot_pk.clone());
                self.broadcast(&mot_pk, |o, q| o != sid && q.world == world);
            }
            return true;
        }
        // absorption hearts go first
        if p.absorption > 0.0 {
            let taken = p.absorption.min(dmg);
            p.absorption -= taken;
            dmg -= taken;
        }
        p.hurt_cooldown = 10; // 0.5s invulnerability frames
        let lethal = p.health - dmg <= 0.0;
        // a totem acts before the health ever reaches 0 (the client shows death at 0)
        if lethal && self.try_totem(sid) {
            return true;
        }
        let Some(p) = self.players.get_mut(&sid) else { return false };
        p.health = (p.health - dmg).max(0.0);
        if (p.gamemode == 0 || p.gamemode == 2) && !bypass_armor {
            p.exhaustion = (p.exhaustion + food::exhaustion::HURT).min(40.0);
        }
        let (world, eid, health) = (p.world, p.entity_id, p.health);
        let pk = p.vitals_packet();
        p.out.push(pk);

        self.broadcast(&protocol::actor_event(eid, actor_event::HURT_ANIMATION, 0), |_, q| q.world == world);
        if let Some(kb) = knockback {
            let mot_pk = protocol::set_actor_motion(eid, kb);
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(mot_pk.clone());
            }
            self.broadcast(&mot_pk, |o, q| o != sid && q.world == world);
        }

        if !bypass_armor {
            self.wear_armor(sid, raw_damage);
        }
        if health <= 0.0 {
            self.kill_player(sid, death_msg);
        }
        true
    }

    pub(crate) fn kill_player(&mut self, sid: SessionId, death_msg: &str) {
        self.dismount(sid);
        self.stop_sleeping(sid);
        self.close_menu(sid, true, true);
        self.close_container(sid, true);
        let Some(p) = self.players.get_mut(&sid) else { return };
        if p.dead {
            return;
        }
        p.dead = true;
        p.health = 0.0;
        p.fall_start_y = None;
        let (world, eid, pos) = (p.world, p.entity_id, p.pos);
        p.eating = None;
        p.absorption = 0.0;
        let pk = p.vitals_packet();
        p.out.push(pk);
        self.clear_effects(sid);
        let Some(p) = self.players.get_mut(&sid) else { return };

        // dying costs the experience; up to 7 points per level (max 100) drop as orbs
        let xp_drop = if self.cfg.keep_inventory || p.gamemode == 1 || p.gamemode == 6 {
            0
        } else {
            let drop = (p.xp_level * 7).min(100);
            p.xp_level = 0;
            p.xp_points = 0;
            drop
        };
        let dropped = if self.cfg.keep_inventory { Vec::new() } else { p.inventory.drain_all() };
        if !self.cfg.keep_inventory {
            self.sync_full_inventory(sid);
            self.broadcast_equipment(sid);
        }
        if xp_drop > 0 {
            self.spawn_xp(world, [pos[0], pos[1] - 1.0, pos[2]], xp_drop);
        }
        for item in dropped {
            let rx = (rand::random::<f32>() - 0.5) * 0.25;
            let rz = (rand::random::<f32>() - 0.5) * 0.25;
            self.spawn_item(world, [pos[0], pos[1] - 0.8, pos[2]], [rx, 0.2, rz], item, 20);
        }

        self.broadcast(&protocol::actor_event(eid, actor_event::DEATH_ANIMATION, 0), |_, q| q.world == world);
        self.broadcast(&protocol::system_message(death_msg), |_, _| true);

        let (_, dest_feet) = self.respawn_destination(sid);
        let dest_eyes = [dest_feet[0], dest_feet[1] + EYE_HEIGHT, dest_feet[2]];
        if let Some(p) = self.players.get_mut(&sid) {
            // only "searching": the client then shows the death screen, and READY_TO_SPAWN goes
            // out when it presses respawn (sending it now would skip the screen)
            p.out.push(protocol::respawn(dest_eyes, respawn_state::SEARCHING_FOR_SPAWN, eid));
            p.out.push(protocol::death_info(death_msg));
        }
    }

    pub(crate) fn on_respawn(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let (_pos, state, _eid) = protocol::read_respawn(r)?;
        if state != respawn_state::CLIENT_READY_TO_SPAWN {
            return Ok(());
        }
        let Some(dead) = self.players.get(&sid).map(|p| p.dead) else { return Ok(()) };
        if !dead {
            return Ok(());
        }
        let (dest_world, dest_feet) = self.respawn_destination(sid);
        let cur_world = self.players[&sid].world;
        if cur_world != dest_world {
            self.switch_world(sid, dest_world);
        }
        if let Some(p) = self.players.get_mut(&sid) {
            p.dead = false;
            p.health = 20.0;
            p.reset_food();
            p.fire_ticks = 0;
            p.hurt_cooldown = 20;
            p.fall_start_y = None;
            p.actor_state = Default::default();
            let eid = p.entity_id;
            let pk = p.vitals_packet();
            p.out.push(pk);
            let dest_eyes = [dest_feet[0], dest_feet[1] + EYE_HEIGHT, dest_feet[2]];
            p.out.push(protocol::respawn(dest_eyes, respawn_state::READY_TO_SPAWN, eid));
        }
        let world = self.players[&sid].world;
        let eid = self.players[&sid].entity_id;
        self.broadcast(&protocol::actor_event(eid, actor_event::RESPAWN, 0), |_, q| q.world == world);
        self.teleport(sid, dest_feet);
        self.sync_actor_state(sid);
        // the others' clients drop the body after the death animation: add the player again
        self.respawn_for_others(sid);
        Ok(())
    }

    /// Damage of a melee hit by this player: weapon, Strength/Weakness, Sharpness, critical.
    pub(crate) fn melee_damage(&self, sid: SessionId) -> f32 {
        let Some(att) = self.players.get(&sid) else { return 0.0 };
        let held = att.inventory.held_item();
        let mut dmg = if held.is_empty() {
            1.0
        } else {
            self.gd.items_by_id.get(&held.id).map(|i| item_attack_damage(&i.name)).unwrap_or(1.0)
        };
        dmg = (dmg + att.attack_bonus()).max(0.0);
        dmg += 1.25 * blocks::enchant_level(held, ENCHANT_SHARPNESS) as f32;
        if let Some((fall, density)) = self.mace_fall(sid) {
            dmg += weapons::mace_bonus(fall, density);
        }
        if att.fall_start_y.is_some_and(|fy| fy - (att.pos[1] - EYE_HEIGHT) > 0.2) && !att.flying {
            dmg *= 1.5; // critical hit
        }
        dmg
    }

    pub(crate) fn on_attack_entity(&mut self, attacker_sid: SessionId, target_eid: i64) {
        if !self.spear_ready(attacker_sid) {
            return;
        }
        // why an attack on a player did not count goes to server.log (to find PvP problems)
        let attacker_name = self.players.get(&attacker_sid).map(|p| p.name.clone()).unwrap_or_default();
        let refuse = |why: &str| crate::logfile::log(&format!("[Combat] attack by {attacker_name} on entity {target_eid} ignored: {why}"));
        if self.mobs.iter().any(|m| m.entity_id == target_eid) {
            let dmg = self.melee_damage(attacker_sid);
            let smash = self.mace_fall(attacker_sid);
            self.attack_mob(attacker_sid, target_eid, dmg);
            if let (Some((fall, _)), Some(pos)) = (smash, self.mobs.iter().find(|m| m.entity_id == target_eid).map(|m| m.pos)) {
                self.mace_smash_effects(attacker_sid, pos, Some(target_eid), fall);
            }
            return;
        }
        if !self.cfg.pvp {
            return refuse("pvp is off");
        }
        let Some((&victim_sid, _)) =
            self.players.iter().find(|(_, p)| p.state == State::Spawned && p.entity_id == target_eid)
        else {
            let known: Vec<i64> = self.players.values().map(|p| p.entity_id).collect();
            return refuse(&format!("no spawned player with that id (players: {known:?})")); // no mobs yet
        };
        if victim_sid == attacker_sid {
            return;
        }
        let Some(att) = self.players.get(&attacker_sid) else { return };
        let Some(vic) = self.players.get(&victim_sid) else { return };
        if att.dead || vic.dead || att.world != vic.world || att.gamemode == 6 || vic.gamemode == 1 || vic.gamemode == 6
        {
            let why = format!(
                "attacker dead={} gm={}, victim dead={} gm={}, same world={}",
                att.dead, att.gamemode, vic.dead, vic.gamemode, att.world == vic.world
            );
            return refuse(&why);
        }
        let d2: f32 = (0..3).map(|i| (att.pos[i] - vic.pos[i]).powi(2)).sum();
        if d2 > 36.0 {
            return refuse(&format!("out of reach ({:.1} blocks)", d2.sqrt())); // melee reach: 6 blocks
        }
        let ev = serde_json::json!({"event": "attack", "player": att.name, "target": vic.name});
        if !self.plugins_allow(ev) {
            return;
        }
        let (Some(att), Some(vic)) = (self.players.get(&attacker_sid), self.players.get(&victim_sid)) else { return };
        let held = att.inventory.held_item();
        let mut dmg = if held.is_empty() {
            1.0
        } else {
            self.gd.items_by_id.get(&held.id).map(|i| item_attack_damage(&i.name)).unwrap_or(1.0)
        };
        dmg = (dmg + att.attack_bonus()).max(0.0);
        dmg += 1.25 * blocks::enchant_level(held, ENCHANT_SHARPNESS) as f32;
        let knockback_level = blocks::enchant_level(held, ENCHANT_KNOCKBACK) as f32;
        let smash = self.mace_fall(attacker_sid);
        if let Some((fall, density)) = smash {
            dmg += weapons::mace_bonus(fall, density);
        }
        if att.fall_start_y.is_some_and(|fy| fy - (att.pos[1] - EYE_HEIGHT) > 0.2) && !att.flying {
            dmg *= 1.5; // critical hit
        }
        let dx = vic.pos[0] - att.pos[0];
        let dz = vic.pos[2] - att.pos[2];
        let len = (dx * dx + dz * dz).sqrt().max(0.01);
        let push = 0.4 + 0.5 * knockback_level;
        let kb = [dx / len * push, 0.36, dz / len * push];
        let msg = format!("§c{} foi morto por {}", vic.name, att.name);
        let attacker_pos = att.pos;
        let (victim_pos, world) = (vic.pos, vic.world);
        let axe = self.gd.tool_info(held.id).is_some_and(|t| t.tool == ceres_world::Tool::Axe);
        self.swing_arm(attacker_sid);
        if self.shield_blocks(victim_sid, attacker_pos, dmg, axe) {
            return;
        }
        if self.damage_player(victim_sid, dmg, &msg, false, Some(kb)) {
            // only hits that count make the hit sound
            self.broadcast(&protocol::level_sound_event("attack.strong", victim_pos, -1), |_, q| q.world == world);
            if let Some((fall, _)) = smash {
                let feet = [victim_pos[0], victim_pos[1] - EYE_HEIGHT, victim_pos[2]];
                self.mace_smash_effects(attacker_sid, feet, None, fall);
            }
            self.wear_held_item(attacker_sid, true);
            self.exhaust(attacker_sid, food::exhaustion::ATTACK);
        }
    }

    pub(crate) fn tick_players_survival(&mut self) {
        let sids: Vec<SessionId> = self.players.keys().copied().collect();
        for sid in sids {
            let Some(p) = self.players.get_mut(&sid) else { continue };
            if p.state != State::Spawned || p.dead {
                continue;
            }
            if p.hurt_cooldown > 0 {
                p.hurt_cooldown -= 1;
            }
            self.tick_food(sid);
            self.tick_shield(sid);
            if self.players.get(&sid).is_some_and(|p| p.gamemode != 1 && p.gamemode != 6) {
                self.tick_effects(sid);
                self.tick_environment(sid);
            }
            let Some(p) = self.players.get_mut(&sid) else { continue };
            if p.dead {
                continue;
            }

            let feet_y = p.pos[1] - EYE_HEIGHT;
            let name = p.name.clone();
            let gm = p.gamemode;
            let world = p.world;
            // gliding with an elytra is not a fall either
            let flying = p.flying || p.actor_state.gliding;
            let feet_block = [p.pos[0].floor() as i32, (feet_y - 0.05).floor() as i32, p.pos[2].floor() as i32];

            // Void damage (< MIN_Y - 16)
            if feet_y < (ceres_world::MIN_Y - 16) as f32 && gm != 6 {
                let msg = format!("§c{name} caiu no vazio");
                self.damage_player(sid, 4.0, &msg, gm != 1, None);
                continue;
            }

            // Fall tracking in every mode but spectator (the mace smash needs it in creative too);
            // fall damage only in survival / adventure
            if gm != 6 {
                let on_ground = self.gd.is_solid(self.worlds.worlds[world].get_block(feet_block, &self.gd));
                let in_liquid = {
                    let b = self.worlds.worlds[world]
                        .get_block([p.pos[0].floor() as i32, feet_y.floor() as i32, p.pos[2].floor() as i32], &self.gd);
                    self.gd.block_name(b).contains("water")
                };
                if in_liquid || flying {
                    if let Some(p) = self.players.get_mut(&sid) {
                        p.fall_start_y = None;
                    }
                } else if on_ground {
                    let fall = self.players.get_mut(&sid).and_then(|p| p.fall_start_y.take());
                    if let Some(start_y) = fall {
                        let dist = start_y - feet_y;
                        if dist > 3.5 && (gm == 0 || gm == 2) {
                            let mut dmg = (dist - 3.0).floor();
                            // Feather Falling (3 per level) and Protection soften falls
                            if let Some(p) = self.players.get(&sid) {
                                let epf = armor_enchant_points(p, &[(ENCHANT_PROTECTION, 1), (ENCHANT_FEATHER_FALLING, 3)]);
                                dmg = (dmg * (1.0 - epf as f32 * 0.04)).floor();
                            }
                            if dmg >= 1.0 {
                                let msg = format!("§c{name} caiu de um lugar alto");
                                // armor does not soften falls
                                self.damage_player(sid, dmg, &msg, true, None);
                            }
                        }
                    }
                } else if let Some(p) = self.players.get_mut(&sid) {
                    p.fall_start_y = Some(p.fall_start_y.map_or(feet_y, |prev| prev.max(feet_y)));
                }
            } else if let Some(p) = self.players.get_mut(&sid) {
                p.fall_start_y = None;
            }
        }
    }
}

/// Attack cooldown of a spear in ticks (item_components.nbt, minecraft:cooldown, category spear).
pub(crate) fn spear_cooldown(name: &str) -> Option<u32> {
    Some(match name.strip_prefix("minecraft:")? {
        "wooden_spear" => 13,
        "stone_spear" => 15,
        "copper_spear" => 17,
        "iron_spear" | "golden_spear" => 19,
        "diamond_spear" => 21,
        "netherite_spear" => 23,
        _ => return None,
    })
}

impl Server {
    /// A spear that is still cooling down does not hit; one that hits starts its cooldown.
    /// False when the attack must be ignored.
    pub(crate) fn spear_ready(&mut self, sid: SessionId) -> bool {
        let Some(p) = self.players.get(&sid) else { return true };
        let name = self.gd.items_by_id.get(&p.inventory.held_item().id).map(|i| i.name.clone()).unwrap_or_default();
        let Some(ticks) = spear_cooldown(&name) else { return true };
        if self.on_cooldown(sid, &name) {
            return false;
        }
        self.start_cooldown(sid, &name, "spear", ticks);
        true
    }
}
