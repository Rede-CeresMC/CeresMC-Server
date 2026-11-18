//! Projectiles: snowballs, eggs, ender pearls (thrown with a click in the air) and arrows (bow:
//! hold to draw, let go to shoot). Simulated on the server: gravity, drag, block and player hits.

use super::*;
use ceres_protocol::ItemDesc;

const EYE_HEIGHT: f32 = 1.62;
const MAX_AGE: u32 = 1200;
const ENCHANT_POWER: i16 = 19;
const ENCHANT_PUNCH: i16 = 20;
const ENCHANT_INFINITY: i16 = 22;
/// ticks between two throws (vanilla 4)
const THROW_COOLDOWN: u64 = 4;
const MAX_PER_PLAYER: usize = 64;
const MAX_TOTAL: usize = 2000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    Snowball,
    Egg,
    EnderPearl,
    Arrow,
    SplashPotion,
    LingeringPotion,
}

impl Kind {
    fn entity(self) -> &'static str {
        match self {
            Kind::Snowball => "minecraft:snowball",
            Kind::Egg => "minecraft:egg",
            Kind::EnderPearl => "minecraft:ender_pearl",
            Kind::Arrow => "minecraft:arrow",
            Kind::SplashPotion | Kind::LingeringPotion => "minecraft:splash_potion",
        }
    }
    fn gravity(self) -> f32 {
        match self {
            Kind::Arrow | Kind::SplashPotion | Kind::LingeringPotion => 0.05,
            _ => 0.03,
        }
    }
}

pub(crate) struct Projectile {
    entity_id: i64,
    world: usize,
    kind: Kind,
    owner: SessionId,
    pos: [f32; 3],
    motion: [f32; 3],
    age: u32,
    /// arrows: extra damage from Power, knockback from Punch, dropped as an item on a block hit
    power: u8,
    punch: u8,
    pickup: bool,
    /// potion type of a thrown potion
    potion: u32,
}

fn throwable(name: &str) -> Option<Kind> {
    match name {
        "minecraft:snowball" => Some(Kind::Snowball),
        "minecraft:egg" => Some(Kind::Egg),
        "minecraft:ender_pearl" => Some(Kind::EnderPearl),
        "minecraft:splash_potion" => Some(Kind::SplashPotion),
        "minecraft:lingering_potion" => Some(Kind::LingeringPotion),
        _ => None,
    }
}

/// Unit vector of where a player looks.
fn look_vector(pitch: f32, yaw: f32) -> [f32; 3] {
    let (p, y) = (pitch.to_radians(), yaw.to_radians());
    [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}

impl Server {
    /// Whether `sid` may launch a projectile now (cooldown and limits). Records the launch.
    fn may_launch(&mut self, sid: SessionId) -> bool {
        let ticks = self.ticks;
        let mine = self.projectiles.iter().filter(|p| p.owner == sid).count();
        if mine >= MAX_PER_PLAYER || self.projectiles.len() >= MAX_TOTAL {
            return false;
        }
        let Some(p) = self.players.get_mut(&sid) else { return false };
        if p.dead || p.gamemode == 6 || p.state != State::Spawned || ticks < p.last_throw + THROW_COOLDOWN {
            return false;
        }
        p.last_throw = ticks;
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn launch(&mut self, sid: SessionId, kind: Kind, speed: f32, power: u8, punch: u8, pickup: bool, potion: u32) {
        let Some(p) = self.players.get(&sid) else { return };
        let (world, pos, rot) = (p.world, p.pos, p.rot);
        let dir = look_vector(rot[0], rot[1]);
        let motion = [dir[0] * speed, dir[1] * speed, dir[2] * speed];
        let start = [pos[0] + dir[0] * 0.5, pos[1] - 0.1 + dir[1] * 0.5, pos[2] + dir[2] * 0.5];
        let eid = self.next_entity_id;
        self.next_entity_id += 1;
        let aux = matches!(kind, Kind::SplashPotion | Kind::LingeringPotion).then_some(potion as i16);
        self.broadcast(&protocol::add_projectile(eid, kind.entity(), start, motion, sid_owner_eid(self, sid), aux), |_, q| {
            q.world == world
        });
        let center = start;
        let sound = if kind == Kind::Arrow { "bow" } else { "throw" };
        self.broadcast(&protocol::level_sound_event(sound, center, -1), |_, q| q.world == world);
        self.projectiles.push(Projectile { entity_id: eid, world, kind, owner: sid, pos: start, motion, age: 0, power, punch, pickup, potion });
    }

    /// Click in the air with a throwable: throw it. Returns true when the held item was one.
    pub(crate) fn throw_held(&mut self, sid: SessionId) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let gm = p.gamemode;
        let held = p.inventory.held_item().clone();
        let Some(name) = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()) else { return false };
        let Some(kind) = throwable(&name) else { return false };
        if !self.may_launch(sid) {
            // tell the client its item is still there
            let slot = (self.players[&sid].inventory.held_slot as usize).min(8);
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(protocol::inventory_slot(ceres_protocol::window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
            }
            return true;
        }
        // potions are thrown a little slower and higher than snowballs
        let speed = if matches!(kind, Kind::SplashPotion | Kind::LingeringPotion) { 0.5 } else { 1.5 };
        self.launch(sid, kind, speed, 0, 0, false, held.meta);
        self.consume_held(sid, gm);
        self.swing_arm(sid);
        true
    }

    /// Click in the air with a bow: start drawing it (if there are arrows).
    pub(crate) fn start_bow(&mut self, sid: SessionId) -> bool {
        let ticks = self.ticks;
        let Some(p) = self.players.get_mut(&sid) else { return false };
        let held = p.inventory.held_item();
        if self.gd.items_by_id.get(&held.id).is_none_or(|i| i.name != "minecraft:bow") {
            return false;
        }
        if p.dead || p.gamemode == 6 {
            return true;
        }
        if p.bow_since.is_none() {
            p.bow_since = Some(ticks);
        }
        true
    }

    /// Let go of the bow: shoot with a force that depends on how long it was drawn.
    pub(crate) fn release_bow(&mut self, sid: SessionId) {
        let ticks = self.ticks;
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(since) = p.bow_since.take() else { return };
        let gm = p.gamemode;
        let bow = p.inventory.held_item().clone();
        if self.gd.items_by_id.get(&bow.id).is_none_or(|i| i.name != "minecraft:bow") {
            return;
        }
        let charge = (ticks.saturating_sub(since)) as f32 / 20.0;
        let force = ((charge * charge + charge * 2.0) / 3.0).min(1.0);
        if force < 0.1 {
            return;
        }
        let infinity = blocks::enchant_level(&bow, ENCHANT_INFINITY) > 0;
        let arrow_slot = p.inventory.main.iter().position(|it| {
            self.gd.items_by_id.get(&it.id).is_some_and(|i| i.name == "minecraft:arrow") && !it.is_empty()
        });
        if gm != 1 && arrow_slot.is_none() {
            return;
        }
        if !self.may_launch(sid) {
            return;
        }
        let Some(p) = self.players.get_mut(&sid) else { return };
        let pickup = gm != 1 && !infinity;
        if pickup {
            let slot = arrow_slot.unwrap();
            let mut rest = p.inventory.main[slot].clone();
            rest.count -= 1;
            p.inventory.set_main(slot, if rest.count == 0 { ItemDesc::default() } else { rest });
            p.out.push(protocol::inventory_slot(
                ceres_protocol::window::INVENTORY as u32,
                slot as u32,
                &p.inventory.main[slot],
            ));
        }
        let power = blocks::enchant_level(&bow, ENCHANT_POWER);
        let punch = blocks::enchant_level(&bow, ENCHANT_PUNCH);
        self.launch(sid, Kind::Arrow, force * 3.0, power, punch, pickup, 0);
        self.wear_held(sid, 1);
    }

    pub(crate) fn tick_projectiles(&mut self) {
        let mut done = Vec::new();
        for i in 0..self.projectiles.len() {
            let (world, kind, owner, age) = {
                let pr = &mut self.projectiles[i];
                pr.age += 1;
                (pr.world, pr.kind, pr.owner, pr.age)
            };
            if age > MAX_AGE {
                done.push((i, None));
                continue;
            }
            let (from, motion) = (self.projectiles[i].pos, self.projectiles[i].motion);
            let to = [from[0] + motion[0], from[1] + motion[1], from[2] + motion[2]];
            // walk the path in small steps: first thing hit wins
            let steps = ((motion[0].abs() + motion[1].abs() + motion[2].abs()) * 4.0).ceil().max(1.0) as i32;
            let mut hit: Option<([f32; 3], Option<SessionId>)> = None;
            for s in 1..=steps {
                let t = s as f32 / steps as f32;
                let at = [from[0] + motion[0] * t, from[1] + motion[1] * t, from[2] + motion[2] * t];
                // players (not the shooter during the first ticks)
                let player = self.players.iter().find(|(psid, q)| {
                    q.world == world
                        && q.state == State::Spawned
                        && !q.dead
                        && q.gamemode != 6
                        && (**psid != owner || age > 5)
                        && {
                            let feet_y = q.pos[1] - EYE_HEIGHT;
                            (at[0] - q.pos[0]).abs() < 0.4 && (at[2] - q.pos[2]).abs() < 0.4 && at[1] >= feet_y && at[1] <= feet_y + 1.9
                        }
                });
                if let Some((psid, _)) = player {
                    hit = Some((at, Some(*psid)));
                    break;
                }
                let bpos = [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32];
                // never load chunks for a projectile: it stops where the loaded world ends
                if !self.worlds.worlds[world].is_loaded(bpos) {
                    hit = Some((at, None));
                    break;
                }
                let b = self.worlds.worlds[world].get_block(bpos, &self.gd);
                if self.gd.is_solid(b) {
                    hit = Some((at, None));
                    break;
                }
            }
            if let Some(h) = hit {
                done.push((i, Some(h)));
                continue;
            }
            let pr = &mut self.projectiles[i];
            pr.pos = to;
            let g = kind.gravity();
            pr.motion = [motion[0] * 0.99, motion[1] * 0.99 - g, motion[2] * 0.99];
            let pk = protocol::move_actor_absolute(pr.entity_id, pr.pos, false, false);
            self.broadcast(&pk, |_, q| q.world == world);
        }
        for (i, hit) in done.into_iter().rev() {
            let pr = self.projectiles.swap_remove(i);
            self.broadcast(&protocol::remove_actor(pr.entity_id), |_, q| q.world == pr.world);
            if let Some((at, target)) = hit {
                self.projectile_hit(pr, at, target);
            }
        }
    }

    fn projectile_hit(&mut self, pr: Projectile, at: [f32; 3], target: Option<SessionId>) {
        let speed = (pr.motion[0].powi(2) + pr.motion[1].powi(2) + pr.motion[2].powi(2)).sqrt();
        let horizontal = (pr.motion[0].powi(2) + pr.motion[2].powi(2)).sqrt().max(0.01);
        let owner_name = self.players.get(&pr.owner).map(|p| p.name.clone()).unwrap_or_default();
        match pr.kind {
            Kind::EnderPearl => {
                // the thrower lands where the pearl did, and the landing hurts
                if self.players.get(&pr.owner).is_some_and(|p| p.world == pr.world && !p.dead) {
                    self.teleport(pr.owner, [at[0], at[1].floor() + 0.1, at[2]]);
                    let name = owner_name.clone();
                    self.damage_player(pr.owner, 5.0, &format!("§c{name} caiu"), true, None);
                }
            }
            Kind::Arrow => {
                if let Some(victim) = target {
                    let mut dmg = (speed * 2.0).ceil();
                    let origin = [at[0] - pr.motion[0], at[1], at[2] - pr.motion[2]];
                    if self.shield_blocks(victim, origin, dmg, false) {
                        return;
                    }
                    if pr.power > 0 {
                        dmg *= 1.0 + 0.25 * (pr.power as f32 + 1.0);
                    }
                    let push = 0.4 + 0.5 * pr.punch as f32;
                    let kb = [pr.motion[0] / horizontal * push, 0.3, pr.motion[2] / horizontal * push];
                    let victim_name = self.players.get(&victim).map(|p| p.name.clone()).unwrap_or_default();
                    self.damage_player(victim, dmg, &format!("§c{victim_name} foi flechado por {owner_name}"), false, Some(kb));
                } else if pr.pickup {
                    // the arrow lands as an item that can be picked up again
                    if let Some(info) = self.gd.items_by_name.get("minecraft:arrow") {
                        let item = ItemDesc {
                            id: info.id,
                            count: 1,
                            meta: 0,
                            stack_id: 0,
                            block_runtime_id: 0,
                            extra: ceres_world::extra_data(None),
                        };
                        let back = [at[0] - pr.motion[0] * 0.1, at[1] - pr.motion[1] * 0.1, at[2] - pr.motion[2] * 0.1];
                        self.spawn_item(pr.world, back, [0.0; 3], item, 10);
                    }
                }
            }
            Kind::SplashPotion | Kind::LingeringPotion => {
                self.burst_potion(pr.world, at, pr.potion, pr.kind == Kind::LingeringPotion, target, pr.owner);
            }
            Kind::Snowball | Kind::Egg => {
                if let Some(victim) = target {
                    // no damage to players, only a push (and the hurt flash)
                    let kb = [pr.motion[0] / horizontal * 0.4, 0.3, pr.motion[2] / horizontal * 0.4];
                    let victim_name = self.players.get(&victim).map(|p| p.name.clone()).unwrap_or_default();
                    self.damage_player(victim, 0.0, &victim_name, false, Some(kb));
                }
            }
        }
    }
}

fn sid_owner_eid(server: &Server, sid: SessionId) -> i64 {
    server.players.get(&sid).map(|p| p.entity_id).unwrap_or(-1)
}
