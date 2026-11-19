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
    Trident,
    XpBottle,
}

impl Kind {
    fn entity(self) -> &'static str {
        match self {
            Kind::Snowball => "minecraft:snowball",
            Kind::Egg => "minecraft:egg",
            Kind::EnderPearl => "minecraft:ender_pearl",
            Kind::Arrow => "minecraft:arrow",
            Kind::SplashPotion | Kind::LingeringPotion => "minecraft:splash_potion",
            Kind::Trident => "minecraft:thrown_trident",
            Kind::XpBottle => "minecraft:xp_bottle",
        }
    }
    fn gravity(self) -> f32 {
        match self {
            Kind::Arrow | Kind::SplashPotion | Kind::LingeringPotion | Kind::Trident => 0.05,
            Kind::XpBottle => 0.1,
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
    /// arrows: ticks since it stuck in a block (0 = flying)
    stuck: u32,
    /// tridents: the item itself (enchantments, durability), given back on pickup
    item: Option<ItemDesc>,
    /// tridents: already hit something (it falls away and no longer hurts)
    dealt: bool,
    /// crossbow arrows with Piercing: entities it may still go through, and those it hit
    pierce: u8,
    pierced: Vec<i64>,
}

/// a stuck arrow stays this long (vanilla 60 s)
const STUCK_TICKS: u32 = 1200;

fn throwable(name: &str) -> Option<Kind> {
    match name {
        "minecraft:snowball" => Some(Kind::Snowball),
        "minecraft:egg" => Some(Kind::Egg),
        "minecraft:ender_pearl" => Some(Kind::EnderPearl),
        "minecraft:splash_potion" => Some(Kind::SplashPotion),
        "minecraft:lingering_potion" => Some(Kind::LingeringPotion),
        "minecraft:experience_bottle" => Some(Kind::XpBottle),
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
    pub(crate) fn may_launch(&mut self, sid: SessionId) -> bool {
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
        self.launch_rot(sid, kind, speed, 0.0, power, punch, pickup, potion, None);
    }

    /// Launches a projectile along the player's look turned by `yaw_offset` degrees.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn launch_rot(&mut self, sid: SessionId, kind: Kind, speed: f32, yaw_offset: f32, power: u8, punch: u8, pickup: bool, potion: u32, item: Option<ItemDesc>) {
        let Some(p) = self.players.get(&sid) else { return };
        let (world, pos, rot) = (p.world, p.pos, p.rot);
        let dir = look_vector(rot[0], rot[1] + yaw_offset);
        let motion = [dir[0] * speed, dir[1] * speed, dir[2] * speed];
        let start = [pos[0] + dir[0] * 0.5, pos[1] - 0.1 + dir[1] * 0.5, pos[2] + dir[2] * 0.5];
        let eid = self.next_entity_id;
        self.next_entity_id += 1;
        let aux = matches!(kind, Kind::SplashPotion | Kind::LingeringPotion).then_some(potion as i16);
        self.broadcast(&protocol::add_projectile(eid, kind.entity(), start, motion, sid_owner_eid(self, sid), aux), |_, q| {
            q.world == world
        });
        let center = start;
        let sound = match kind {
            Kind::Arrow => "bow",
            Kind::Trident => "item.trident.throw",
            _ => "throw",
        };
        let item = if kind == Kind::Trident { item } else { None };
        self.broadcast(&protocol::level_sound_event(sound, center, -1), |_, q| q.world == world);
        self.projectiles.push(Projectile { entity_id: eid, world, kind, owner: sid, pos: start, motion, age: 0, power, punch, pickup, potion, stuck: 0, item, dealt: false, pierce: 0, pierced: Vec::new() });
    }

    /// Click in the air with a throwable: throw it. Returns true when the held item was one.
    pub(crate) fn throw_held(&mut self, sid: SessionId) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let gm = p.gamemode;
        let held = p.inventory.held_item().clone();
        let Some(name) = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()) else { return false };
        let Some(kind) = throwable(&name) else { return false };
        let pearl_wait = kind == Kind::EnderPearl && self.on_cooldown(sid, &name);
        if pearl_wait || !self.may_launch(sid) {
            // tell the client its item is still there
            let slot = (self.players[&sid].inventory.held_slot as usize).min(8);
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(protocol::inventory_slot(ceres_protocol::window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
            }
            return true;
        }
        // potions are thrown a little slower and higher than snowballs
        let speed = match kind {
            Kind::SplashPotion | Kind::LingeringPotion => 0.5,
            Kind::XpBottle => 1.0,
            _ => 1.5,
        };
        self.launch(sid, kind, speed, 0, 0, false, held.meta);
        if kind == Kind::EnderPearl {
            self.start_cooldown(sid, &name, "ender_pearl", 20);
        }
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
        let mut hit_mobs: Vec<(usize, i64, [f32; 3])> = Vec::new();
        for i in 0..self.projectiles.len() {
            let (world, kind, owner, age) = {
                let pr = &mut self.projectiles[i];
                pr.age += 1;
                (pr.world, pr.kind, pr.owner, pr.age)
            };
            match self.trident_return(i) {
                Some(true) => {
                    done.push((i, None));
                    continue;
                }
                Some(false) => continue,
                None => {}
            }
            if self.projectiles[i].stuck > 0 {
                // an arrow in a block: it waits to be picked up, then goes away
                self.projectiles[i].stuck += 1;
                let (pos, pickup, eid) = (self.projectiles[i].pos, self.projectiles[i].pickup, self.projectiles[i].entity_id);
                if self.projectiles[i].stuck > STUCK_TICKS {
                    done.push((i, None));
                    continue;
                }
                if pickup && self.projectiles[i].stuck > 10 {
                    let taker = self.players.iter().find(|(_, q)| {
                        q.world == world
                            && q.state == State::Spawned
                            && !q.dead
                            && (q.gamemode == 0 || q.gamemode == 2)
                            && (pos[0] - q.pos[0]).abs() < 1.2
                            && (pos[2] - q.pos[2]).abs() < 1.2
                            && pos[1] > q.pos[1] - EYE_HEIGHT - 1.0
                            && pos[1] < q.pos[1] + 0.5
                    });
                    if let Some((&sid, q)) = taker {
                        let peid = q.entity_id;
                        self.broadcast(&protocol::take_item_actor(eid, peid), |_, q| q.world == world);
                        match self.projectiles[i].item.clone() {
                            Some(it) => self.give_stack(sid, it),
                            None => self.give_back(sid, "minecraft:arrow", 0),
                        }
                        self.broadcast(&protocol::level_sound_event("pop", pos, -1), |_, q| q.world == world);
                        done.push((i, None));
                    }
                }
                continue;
            }
            if age > MAX_AGE {
                done.push((i, None));
                continue;
            }
            let (from, motion) = (self.projectiles[i].pos, self.projectiles[i].motion);
            let to = [from[0] + motion[0], from[1] + motion[1], from[2] + motion[2]];
            // walk the path in small steps: first thing hit wins
            let steps = ((motion[0].abs() + motion[1].abs() + motion[2].abs()) * 4.0).ceil().max(1.0) as i32;
            let mut hit: Option<([f32; 3], Option<SessionId>)> = None;
            let mut mob_hit: Option<i64> = None;
            for s in 1..=steps {
                let t = s as f32 / steps as f32;
                let at = [from[0] + motion[0] * t, from[1] + motion[1] * t, from[2] + motion[2] * t];
                // players (not the shooter during the first ticks); a trident that already hit only falls
                let dealt = self.projectiles[i].dealt;
                let player = self.players.iter().find(|(psid, q)| {
                    !dealt &&
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
                if let Some(mobs::Ent::Mob(eid)) = self.entity_at(world, at, None, true).filter(|e| !dealt && matches!(e, mobs::Ent::Mob(_))) {
                    mob_hit = Some(eid);
                    hit = Some((at, None));
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
            if kind == Kind::Trident {
                let target = match (mob_hit, hit) {
                    (Some(eid), _) => Some(mobs::Ent::Mob(eid)),
                    (None, Some((_, Some(psid)))) => Some(mobs::Ent::Player(psid)),
                    _ => None,
                };
                if let Some(t) = target {
                    self.trident_hit(i, t);
                    continue;
                }
            }
            if kind == Kind::Arrow && self.projectiles[i].pierce > 0 {
                let target = match (mob_hit, hit) {
                    (Some(e), _) => Some(mobs::Ent::Mob(e)),
                    (None, Some((_, Some(s)))) => Some(mobs::Ent::Player(s)),
                    _ => None,
                };
                if let Some(t) = target {
                    let key = match t {
                        mobs::Ent::Mob(e) => e,
                        mobs::Ent::Player(s) => -(s as i64) - 1,
                    };
                    if !self.projectiles[i].pierced.contains(&key) {
                        self.projectiles[i].pierced.push(key);
                        self.projectiles[i].pierce -= 1;
                        self.arrow_pierce_damage(i, t);
                    }
                    // it goes on through
                    let pr = &mut self.projectiles[i];
                    pr.pos = to;
                    pr.motion = [motion[0] * 0.99, motion[1] * 0.99 - kind.gravity(), motion[2] * 0.99];
                    let pk = protocol::move_projectile(pr.entity_id, pr.pos, pr.motion, false);
                    self.broadcast(&pk, |_, q| q.world == world);
                    continue;
                }
            }
            if let Some(eid) = mob_hit {
                done.push((i, None));
                hit_mobs.push((i, eid, hit.map(|h| h.0).unwrap_or(to)));
                continue;
            }
            if let Some((at, None)) = hit {
                if matches!(kind, Kind::Arrow | Kind::Trident) && self.worlds.worlds[world].is_loaded([at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32]) {
                    // the arrow sticks in the block, its tip just inside
                    let len = (motion[0].powi(2) + motion[1].powi(2) + motion[2].powi(2)).sqrt().max(0.01);
                    let pr = &mut self.projectiles[i];
                    pr.pos = [at[0] - motion[0] / len * 0.05, at[1] - motion[1] / len * 0.05, at[2] - motion[2] / len * 0.05];
                    pr.stuck = 1;
                    let pk = protocol::move_projectile(pr.entity_id, pr.pos, motion, true);
                    let at2 = pr.pos;
                    self.broadcast(&pk, |_, q| q.world == world);
                    let snd = if kind == Kind::Trident { "item.trident.hit_ground" } else { "bow.hit" };
                    self.broadcast(&protocol::level_sound_event(snd, at2, -1), |_, q| q.world == world);
                    continue;
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
            let pk = protocol::move_projectile(pr.entity_id, pr.pos, pr.motion, false);
            self.broadcast(&pk, |_, q| q.world == world);
        }
        for (i, hit) in done.into_iter().rev() {
            let pr = self.projectiles.swap_remove(i);
            self.broadcast(&protocol::remove_actor(pr.entity_id), |_, q| q.world == pr.world);
            if let Some(&(_, eid, at)) = hit_mobs.iter().find(|h| h.0 == i) {
                self.projectile_hit_mob(&pr, eid, at);
                continue;
            }
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
            Kind::XpBottle => self.burst_xp_bottle(pr.world, at),
            Kind::Trident => {
                // stopped where the loaded world ends: it lands there as an item
                if let Some(it) = pr.item.clone().filter(|_| pr.pickup) {
                    self.spawn_item(pr.world, at, [0.0; 3], it, 10);
                }
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

impl Server {
    /// A player's projectile hit a mob: arrows hurt it, snowballs and eggs push it, potions burst.
    fn projectile_hit_mob(&mut self, pr: &Projectile, eid: i64, at: [f32; 3]) {
        let speed = (pr.motion[0].powi(2) + pr.motion[1].powi(2) + pr.motion[2].powi(2)).sqrt();
        let from = [at[0] - pr.motion[0] * 5.0, at[1], at[2] - pr.motion[2] * 5.0];
        let who = Some(mobs::Ent::Player(pr.owner));
        match pr.kind {
            Kind::Arrow => {
                let mut dmg = (speed * 2.0).ceil();
                if pr.power > 0 {
                    dmg *= 1.0 + 0.25 * (pr.power as f32 + 1.0);
                }
                let tick = self.ticks;
                self.mobsys.player_attacked.insert(pr.owner, (mobs::Ent::Mob(eid), tick));
                self.with_mob(eid, |s, m| s.hurt(m, dmg, "projectile", who, Some(from)));
            }
            Kind::Snowball | Kind::Egg => {
                // snowballs hurt blazes (filter in snowball.json), otherwise only a push
                let blaze = self.mob_ref(eid).is_some_and(|m| m.t.is("blaze"));
                self.with_mob(eid, |s, m| {
                    if blaze && pr.kind == Kind::Snowball {
                        s.hurt(m, 3.0, "projectile", who, Some(from));
                    } else {
                        let h = (pr.motion[0].powi(2) + pr.motion[2].powi(2)).sqrt().max(0.01);
                        m.motion = [pr.motion[0] / h * 0.3, 0.2, pr.motion[2] / h * 0.3];
                        m.last_hurt_by = who;
                    }
                });
            }
            Kind::SplashPotion | Kind::LingeringPotion => {
                self.burst_potion(pr.world, at, pr.potion, pr.kind == Kind::LingeringPotion, None, pr.owner);
            }
            Kind::Trident => {}
            Kind::XpBottle => self.burst_xp_bottle(pr.world, at),
            Kind::EnderPearl => {
                if self.players.get(&pr.owner).is_some_and(|p| p.world == pr.world && !p.dead) {
                    self.teleport(pr.owner, [at[0], at[1].floor() + 0.1, at[2]]);
                }
            }
        }
    }
}

const ENCHANT_IMPALING: i16 = 29;
const ENCHANT_LOYALTY: i16 = 31;
const ENCHANT_CHANNELING: i16 = 32;

impl Server {
    /// A thrown trident hits a player or a mob: 8 damage (+2.5 per Impaling level on creatures
    /// of the water), then it bounces off and falls.
    fn trident_hit(&mut self, i: usize, target: mobs::Ent) {
        let (owner, motion, pos, world) = {
            let pr = &self.projectiles[i];
            (pr.owner, pr.motion, pr.pos, pr.world)
        };
        let item = self.projectiles[i].item.clone().unwrap_or_default();
        let impaling = blocks::enchant_level(&item, ENCHANT_IMPALING) as f32;
        let h = (motion[0].powi(2) + motion[2].powi(2)).sqrt().max(0.01);
        let kb = [motion[0] / h * 0.4, 0.3, motion[2] / h * 0.4];
        let owner_name = self.players.get(&owner).map(|p| p.name.clone()).unwrap_or_default();
        match target {
            mobs::Ent::Player(victim) => {
                if victim != owner {
                    let name = self.players.get(&victim).map(|p| p.name.clone()).unwrap_or_default();
                    let rain = self.raining(world);
                    let wet = self.mobsys.penv.get(&victim).is_some_and(|e| e.in_water || (rain && e.sky));
                    let dmg = 8.0 + if wet { 2.5 * impaling } else { 0.0 };
                    let origin = [pos[0] - motion[0], pos[1], pos[2] - motion[2]];
                    if !self.shield_blocks(victim, origin, dmg, false) {
                        self.damage_player(victim, dmg, &format!("§c{name} foi empalado por {owner_name}"), false, Some(kb));
                    }
                }
            }
            mobs::Ent::Mob(eid) => {
                // Bedrock's Impaling: anything touching water or in the rain under the sky
                let rain = self.raining(world);
                let wet = self.mob_ref(eid).is_some_and(|m| m.env.in_water || (rain && m.env.sky));
                let dmg = 8.0 + if wet { 2.5 * impaling } else { 0.0 };
                let tick = self.ticks;
                self.mobsys.player_attacked.insert(owner, (mobs::Ent::Mob(eid), tick));
                let from = [pos[0] - motion[0] * 5.0, pos[1], pos[2] - motion[2] * 5.0];
                self.with_mob(eid, |s, m| s.hurt(m, dmg, "projectile", Some(mobs::Ent::Player(owner)), Some(from)));
            }
        }
        self.broadcast(&protocol::level_sound_event("item.trident.hit", pos, -1), |_, q| q.world == world);
        if blocks::enchant_level(&item, ENCHANT_CHANNELING) > 0 {
            if let Some(at) = self.subj(target).map(|s| s.feet()) {
                self.channeling_strike(world, at);
            }
        }
        let pr = &mut self.projectiles[i];
        pr.dealt = true;
        pr.motion = [-motion[0] * 0.01, -0.1, -motion[2] * 0.01];
    }

    /// Loyalty: after a hit (or stuck in a block) the trident flies back to its thrower.
    /// Some(true) = it arrived (remove it), Some(false) = moved this tick, None = not returning.
    fn trident_return(&mut self, i: usize) -> Option<bool> {
        let pr = &self.projectiles[i];
        if pr.kind != Kind::Trident || !(pr.dealt || pr.stuck > 0) {
            return None;
        }
        let item = pr.item.clone()?;
        let loyalty = blocks::enchant_level(&item, ENCHANT_LOYALTY);
        if loyalty == 0 || (pr.stuck > 0 && pr.stuck < 10) {
            return None;
        }
        let (owner, world, eid, pos) = (pr.owner, pr.world, pr.entity_id, pr.pos);
        let p = self.players.get(&owner).filter(|p| p.world == world && !p.dead && p.state == State::Spawned)?;
        let (target, peid) = ([p.pos[0], p.pos[1] - 0.4, p.pos[2]], p.entity_id);
        let d = [target[0] - pos[0], target[1] - pos[1], target[2] - pos[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len < 1.5 {
            self.broadcast(&protocol::take_item_actor(eid, peid), |_, q| q.world == world);
            self.broadcast(&protocol::level_sound_event("item.trident.return", target, -1), |_, q| q.world == world);
            if self.projectiles[i].pickup {
                self.give_stack(owner, item);
            }
            return Some(true);
        }
        let step = (0.4 + 0.25 * loyalty as f32).min(len);
        let motion = [d[0] / len * step, d[1] / len * step, d[2] / len * step];
        let pr = &mut self.projectiles[i];
        pr.stuck = 0;
        pr.dealt = true;
        pr.pos = [pos[0] + motion[0], pos[1] + motion[1], pos[2] + motion[2]];
        pr.motion = motion;
        let new_pos = pr.pos;
        let pk = protocol::move_projectile(eid, new_pos, [-motion[0], -motion[1], -motion[2]], false);
        self.broadcast(&pk, |_, q| q.world == world);
        Some(false)
    }
}

impl Server {
    /// Gives the arrow fired last by a crossbow its Piercing level.
    pub(crate) fn set_last_pierce(&mut self, level: u8) {
        if let Some(pr) = self.projectiles.last_mut() {
            pr.pierce = level;
        }
    }

    /// A piercing arrow hurts what it passes through, as an arrow does.
    fn arrow_pierce_damage(&mut self, i: usize, target: mobs::Ent) {
        let (motion, pos, owner) = (self.projectiles[i].motion, self.projectiles[i].pos, self.projectiles[i].owner);
        let speed = (motion[0].powi(2) + motion[1].powi(2) + motion[2].powi(2)).sqrt();
        let h = (motion[0].powi(2) + motion[2].powi(2)).sqrt().max(0.01);
        let dmg = (speed * 2.0).ceil();
        match target {
            mobs::Ent::Player(victim) => {
                if victim == owner {
                    return;
                }
                let name = self.players.get(&victim).map(|p| p.name.clone()).unwrap_or_default();
                let owner_name = self.players.get(&owner).map(|p| p.name.clone()).unwrap_or_default();
                let kb = [motion[0] / h * 0.4, 0.3, motion[2] / h * 0.4];
                self.damage_player(victim, dmg, &format!("§c{name} foi flechado por {owner_name}"), false, Some(kb));
            }
            mobs::Ent::Mob(eid) => {
                let from = [pos[0] - motion[0] * 5.0, pos[1], pos[2] - motion[2] * 5.0];
                self.with_mob(eid, |s, m| s.hurt(m, dmg, "projectile", Some(mobs::Ent::Player(owner)), Some(from)));
            }
        }
    }
}

impl Server {
    /// A bottle o' enchanting breaks: blue splash, glass sound and 3-11 experience (as
    /// PowerNukkitX's EntityXpBottle).
    fn burst_xp_bottle(&mut self, world: usize, at: [f32; 3]) {
        self.broadcast(&protocol::level_event(2002, at, 0x0038_5dc6), |_, q| q.world == world); // potion splash particles
        self.broadcast(&protocol::level_sound_event("glass", at, -1), |_, q| q.world == world);
        self.spawn_xp(world, at, 3 + rand::random::<u32>() % 9);
    }
}
