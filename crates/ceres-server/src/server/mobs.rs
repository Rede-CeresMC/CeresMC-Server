//! Mobs: cow, pig, sheep and chicken (passive) and the zombie (hostile). They have gravity and
//! collision with blocks, wander around (stepping up one block, not walking off cliffs), float in
//! water, make their sounds, and drop their loot and experience when they die. Passive mobs run
//! away when hurt; zombies chase survival players within 16 blocks, hit them, burn in sunlight
//! and are removed on peaceful.
//! Natural spawning (spawn-animals / spawn-monsters): animals on grass in daylight, zombies at
//! night under the open sky, 24 to 48 blocks from a player, with a cap per player; zombies far
//! from everyone despawn. (Without a light engine, caves do not spawn monsters yet.)
//! `/summon <mob> [x y z]` spawns one. They are saved in `mobs.json` in each world's folder
//! (not yet as vanilla LevelDB entities).

use super::*;
use ceres_protocol::{ItemDesc, actor_event};

const GRAVITY: f32 = 0.08;
const MAX_MOBS: usize = 300;
const EYE_HEIGHT: f32 = 1.62;
const FLAGS: i64 = (1 << 35) | (1 << 48) | (1 << 49); // breathing, has collision, gravity

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MobKind {
    Cow,
    Pig,
    Sheep,
    Chicken,
    Zombie,
}

impl MobKind {
    fn hostile(self) -> bool {
        self == MobKind::Zombie
    }

    fn xp(self) -> u32 {
        if self.hostile() { 5 } else { 1 + rand::random::<u32>() % 3 }
    }

    pub(crate) fn from_name(name: &str) -> Option<MobKind> {
        match name.strip_prefix("minecraft:").unwrap_or(name).to_ascii_lowercase().as_str() {
            "cow" | "vaca" => Some(MobKind::Cow),
            "pig" | "porco" => Some(MobKind::Pig),
            "sheep" | "ovelha" => Some(MobKind::Sheep),
            "chicken" | "galinha" => Some(MobKind::Chicken),
            "zombie" | "zumbi" => Some(MobKind::Zombie),
            _ => None,
        }
    }

    pub(crate) fn identifier(self) -> &'static str {
        match self {
            MobKind::Cow => "minecraft:cow",
            MobKind::Pig => "minecraft:pig",
            MobKind::Sheep => "minecraft:sheep",
            MobKind::Chicken => "minecraft:chicken",
            MobKind::Zombie => "minecraft:zombie",
        }
    }

    fn max_health(self) -> f32 {
        match self {
            MobKind::Chicken => 4.0,
            MobKind::Sheep => 8.0,
            MobKind::Zombie => 20.0,
            _ => 10.0,
        }
    }

    /// [width, height]
    fn size(self) -> [f32; 2] {
        match self {
            MobKind::Cow => [0.9, 1.4],
            MobKind::Pig => [0.9, 0.9],
            MobKind::Sheep => [0.9, 1.3],
            MobKind::Chicken => [0.4, 0.7],
            MobKind::Zombie => [0.6, 1.9],
        }
    }

    /// blocks per tick when walking
    fn speed(self) -> f32 {
        match self {
            MobKind::Chicken => 0.06,
            MobKind::Zombie => 0.05,
            _ => 0.07,
        }
    }

    /// (item, min, max) dropped on death
    fn drops(self) -> &'static [(&'static str, u16, u16)] {
        match self {
            MobKind::Cow => &[("minecraft:leather", 0, 2), ("minecraft:beef", 1, 3)],
            MobKind::Pig => &[("minecraft:porkchop", 1, 3)],
            MobKind::Sheep => &[("minecraft:mutton", 1, 2)],
            MobKind::Chicken => &[("minecraft:feather", 0, 2), ("minecraft:chicken", 1, 1)],
            MobKind::Zombie => &[("minecraft:rotten_flesh", 0, 2)],
        }
    }
}

const WOOL: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue",
    "brown", "green", "red", "black",
];

pub(crate) struct Mob {
    pub entity_id: i64,
    pub kind: MobKind,
    pub world: usize,
    pub pos: [f32; 3],
    pub motion: [f32; 3],
    pub yaw: f32,
    pub health: f32,
    pub on_ground: bool,
    /// where it is walking to
    target: Option<[f32; 3]>,
    /// running away (ticks left)
    panic: u32,
    hurt_cooldown: u32,
    /// dying: removed when it reaches 0
    death_ticks: Option<u32>,
    /// sheep wool colour (index into WOOL)
    color: u8,
    /// sheep without its wool (it grows back after eating grass)
    sheared: bool,
    /// on fire (zombies in sunlight)
    burning: bool,
    /// ticks until it may hit again
    attack_cooldown: u32,
    /// chicken: ticks until the next egg
    egg_in: u32,
    sent_pos: [f32; 3],
    sent_yaw: f32,
    age: u64,
}

fn rand_range(min: u16, max: u16) -> u16 {
    if max <= min { min } else { min + (rand::random::<u32>() % (max - min + 1) as u32) as u16 }
}

impl Server {
    fn mob_packet(&self, m: &Mob) -> Vec<u8> {
        let color = (m.kind == MobKind::Sheep).then_some(m.color);
        protocol::add_mob(m.entity_id, m.kind.identifier(), m.pos, m.yaw, m.health, m.kind.max_health(), m.kind.size(), color, Self::mob_flags(m))
    }

    fn mob_flags(m: &Mob) -> i64 {
        FLAGS | if m.sheared { 1 << 31 } else { 0 } | if m.burning { 1 } else { 0 }
    }

    /// Right click on a mob: shears on a sheep, a bucket on a cow. True if it was a mob.
    pub(crate) fn interact_mob(&mut self, sid: SessionId, eid: i64) -> bool {
        let Some(i) = self.mobs.iter().position(|m| m.entity_id == eid) else { return false };
        let Some(p) = self.players.get(&sid) else { return true };
        let feet = [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]];
        let (world, gm) = (p.world, p.gamemode);
        let held = p.inventory.held_item().clone();
        let held_name = self.gd.items_by_id.get(&held.id).map(|it| it.name.clone()).unwrap_or_default();
        let m = &self.mobs[i];
        let d2: f32 = (0..3).map(|k| (m.pos[k] - feet[k]).powi(2)).sum();
        if m.world != world || m.death_ticks.is_some() || d2 > 49.0 || gm == 6 {
            return true;
        }
        let (kind, pos, sheared, color) = (m.kind, m.pos, m.sheared, m.color);
        let sound_at = [pos[0], pos[1] + 0.5, pos[2]];
        match (kind, held_name.as_str()) {
            (MobKind::Sheep, "minecraft:shears") if !sheared => {
                self.mobs[i].sheared = true;
                let flags = Self::mob_flags(&self.mobs[i]);
                self.broadcast(&protocol::set_mob_flags(eid, flags, Some(color)), |_, q| q.world == world);
                self.broadcast(&protocol::mob_sound("shear", sound_at, kind.identifier(), false), |_, q| q.world == world);
                let wool = format!("minecraft:{}_wool", WOOL[color as usize % 16]);
                if let Some(item) = self.simple_item(&wool, rand_range(1, 3)) {
                    self.spawn_item(world, [pos[0], pos[1] + 0.8, pos[2]], [0.0, 0.2, 0.0], item, 10);
                }
                if gm != 1 {
                    self.wear_held(sid, 1);
                }
                self.swing_arm(sid);
            }
            (MobKind::Cow, "minecraft:bucket") => {
                self.broadcast(&protocol::mob_sound("milk", sound_at, kind.identifier(), false), |_, q| q.world == world);
                if gm != 1 {
                    self.consume_held(sid, gm);
                    self.give_back(sid, "minecraft:milk_bucket", 0);
                }
                self.swing_arm(sid);
            }
            _ => {}
        }
        true
    }

    /// Spawns a mob; returns its entity id (None when there are too many).
    pub(crate) fn spawn_mob(&mut self, world: usize, kind: MobKind, pos: [f32; 3]) -> Option<i64> {
        if self.mobs.len() >= MAX_MOBS {
            return None;
        }
        let entity_id = self.next_entity_id;
        self.next_entity_id += 1;
        // sheep: mostly white, sometimes grey/black/brown/pink (vanilla odds, roughly)
        let color = match rand::random::<u32>() % 1000 {
            0..=81 => 7,
            82..=163 => 8,
            164..=245 => 15,
            246..=275 => 12,
            276..=277 => 6,
            _ => 0,
        };
        let m = Mob {
            entity_id,
            kind,
            world,
            pos,
            motion: [0.0; 3],
            yaw: (rand::random::<u32>() % 360) as f32,
            health: kind.max_health(),
            on_ground: false,
            target: None,
            panic: 0,
            hurt_cooldown: 0,
            death_ticks: None,
            color,
            sheared: false,
            burning: false,
            attack_cooldown: 0,
            egg_in: 6000 + rand::random::<u32>() % 6000,
            sent_pos: pos,
            sent_yaw: 0.0,
            age: 0,
        };
        let pk = self.mob_packet(&m);
        self.broadcast(&pk, |_, q| q.world == world);
        self.mobs.push(m);
        Some(entity_id)
    }

    /// The mobs of the player's world (on joining or changing worlds).
    pub(crate) fn send_world_mobs(&mut self, sid: SessionId) {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return };
        let pks: Vec<Vec<u8>> =
            self.mobs.iter().filter(|m| m.world == world && m.death_ticks.is_none()).map(|m| self.mob_packet(m)).collect();
        if let Some(p) = self.players.get_mut(&sid) {
            p.out.extend(pks);
        }
    }

    fn solid_at(&mut self, world: usize, pos: [f32; 3]) -> bool {
        let b = [pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32];
        self.gd.is_solid(self.worlds.worlds[world].get_block(b, &self.gd))
    }

    fn water_at(&mut self, world: usize, pos: [f32; 3]) -> bool {
        let b = [pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32];
        let name = self.gd.block_name(self.worlds.worlds[world].get_block(b, &self.gd));
        name == "minecraft:water" || name == "minecraft:flowing_water"
    }

    /// Whether a body of `height` fits with its feet at `pos`.
    fn fits(&mut self, world: usize, pos: [f32; 3], height: f32) -> bool {
        let mut y = 0.0;
        while y < height {
            if self.solid_at(world, [pos[0], pos[1] + y, pos[2]]) {
                return false;
            }
            y += 1.0;
        }
        !self.solid_at(world, [pos[0], pos[1] + height - 0.01, pos[2]])
    }

    pub(crate) fn tick_mobs(&mut self) {
        if self.mobs.is_empty() {
            return;
        }
        let mut gone = Vec::new();
        let mut packets: Vec<(usize, Vec<u8>)> = Vec::new();
        let mut eggs: Vec<(usize, [f32; 3])> = Vec::new();
        // players a zombie may go after: (session, world, feet)
        let difficulty = self.cfg.difficulty;
        let prey: Vec<(SessionId, usize, [f32; 3])> = self
            .players
            .iter()
            .filter(|(_, p)| p.state == State::Spawned && !p.dead && (p.gamemode == 0 || p.gamemode == 2))
            .map(|(s, p)| (*s, p.world, [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]))
            .collect();
        let mut hits: Vec<(SessionId, usize)> = Vec::new();
        let mut burn_damage: Vec<usize> = Vec::new();
        for i in 0..self.mobs.len() {
            let (world, kind) = (self.mobs[i].world, self.mobs[i].kind);
            {
                let m = &mut self.mobs[i];
                m.age += 1;
                m.hurt_cooldown = m.hurt_cooldown.saturating_sub(1);
                if let Some(t) = m.death_ticks.as_mut() {
                    *t = t.saturating_sub(1);
                    if *t == 0 {
                        gone.push(i);
                    }
                    continue;
                }
            }
            if self.mobs[i].pos[1] < (ceres_world::MIN_Y - 40) as f32 || (kind.hostile() && difficulty == 0) {
                gone.push(i);
                continue;
            }
            if kind.hostile() {
                let m = &mut self.mobs[i];
                m.attack_cooldown = m.attack_cooldown.saturating_sub(1);
                let near = prey
                    .iter()
                    .filter(|(_, w, _)| *w == world)
                    .map(|(s, _, f)| (*s, *f, (0..3).map(|k| (f[k] - m.pos[k]).powi(2)).sum::<f32>()))
                    .filter(|(_, _, d2)| *d2 < 16.0 * 16.0)
                    .min_by(|a, b| a.2.total_cmp(&b.2));
                if let Some((sid, feet, _)) = near {
                    m.target = Some(feet);
                    m.panic = 0;
                    let horiz = ((feet[0] - m.pos[0]).powi(2) + (feet[2] - m.pos[2]).powi(2)).sqrt();
                    if horiz < 1.4 && (feet[1] - m.pos[1]).abs() < 2.0 && m.attack_cooldown == 0 {
                        m.attack_cooldown = 20;
                        hits.push((sid, i));
                    }
                }
            }
            // brain: pick where to walk
            let speed = {
                let m = &mut self.mobs[i];
                if m.panic > 0 {
                    m.panic -= 1;
                    if m.target.is_none() || m.age % 20 == 0 {
                        let a = (rand::random::<u32>() % 360) as f32 * std::f32::consts::PI / 180.0;
                        m.target = Some([m.pos[0] + a.cos() * 8.0, m.pos[1], m.pos[2] + a.sin() * 8.0]);
                    }
                    kind.speed() * 2.0
                } else if kind.hostile() && m.target.is_some() {
                    kind.speed() * 2.0 // chasing
                } else {
                    if m.target.is_none() && rand::random::<u32>() % 120 == 0 {
                        let dx = (rand::random::<u32>() % 17) as f32 - 8.0;
                        let dz = (rand::random::<u32>() % 17) as f32 - 8.0;
                        m.target = Some([m.pos[0] + dx, m.pos[1], m.pos[2] + dz]);
                    }
                    kind.speed()
                }
            };
            let (mut pos, mut mot, target, height) =
                (self.mobs[i].pos, self.mobs[i].motion, self.mobs[i].target, kind.size()[1]);
            let mut yaw = self.mobs[i].yaw;
            let mut new_target = target;
            let mut walk = [0.0f32, 0.0];
            if let Some(t) = target {
                let (dx, dz) = (t[0] - pos[0], t[2] - pos[2]);
                let dist = (dx * dx + dz * dz).sqrt();
                if dist < 0.5 {
                    new_target = None;
                } else {
                    walk = [dx / dist * speed, dz / dist * speed];
                    yaw = dz.atan2(dx).to_degrees() - 90.0;
                }
            }
            let in_water = self.water_at(world, [pos[0], pos[1] + 0.2, pos[2]]);
            // horizontal: step, jump up one block, or stop at walls and cliffs
            if walk != [0.0, 0.0] {
                let next = [pos[0] + walk[0], pos[1], pos[2] + walk[1]];
                let ahead = [pos[0] + walk[0] * 6.0, pos[1], pos[2] + walk[1] * 6.0];
                if self.fits(world, next, height) {
                    // a drop of more than 3 blocks: do not walk there
                    let below = (1..=3).any(|d| self.solid_at(world, [next[0], next[1] - d as f32 + 0.01, next[2]]));
                    if below || in_water || !self.mobs[i].on_ground {
                        pos[0] = next[0];
                        pos[2] = next[2];
                    } else {
                        new_target = None;
                    }
                } else if self.mobs[i].on_ground
                    && self.solid_at(world, ahead)
                    && self.fits(world, [next[0], next[1] + 1.0, next[2]], height)
                {
                    mot[1] = 0.42; // jump up the step
                } else {
                    new_target = None;
                }
            }
            // vertical
            let mut on_ground = false;
            if in_water {
                mot[1] = (mot[1] + 0.04).min(0.1);
            } else {
                mot[1] -= GRAVITY;
                if kind == MobKind::Chicken {
                    mot[1] = mot[1].max(-0.08); // chickens flap down
                }
                mot[1] = mot[1].max(-2.0);
            }
            let ny = pos[1] + mot[1];
            if mot[1] < 0.0 && self.solid_at(world, [pos[0], ny, pos[2]]) {
                pos[1] = ny.floor() + 1.0;
                mot[1] = 0.0;
                on_ground = true;
            } else if mot[1] > 0.0 && self.solid_at(world, [pos[0], ny + height, pos[2]]) {
                mot[1] = 0.0;
            } else {
                pos[1] = ny;
            }
            // knockback / pushes
            for axis in [0usize, 2] {
                if mot[axis].abs() > 0.001 {
                    let mut moved = pos;
                    moved[axis] += mot[axis];
                    if self.fits(world, moved, height) {
                        pos = moved;
                    }
                    mot[axis] *= if on_ground { 0.6 } else { 0.91 };
                } else {
                    mot[axis] = 0.0;
                }
            }
            let ambient = rand::random::<u32>() % 500 == 0;
            if kind == MobKind::Zombie && self.mobs[i].age % 10 == 0 {
                let tod = self.worlds.worlds[world].time.rem_euclid(24000);
                let day = !(12600..23400).contains(&tod);
                let wet = in_water || self.worlds.worlds[world].weather != ceres_world::Weather::Clear;
                let open_sky = day && !wet && {
                    let top = [pos[0], pos[1] + height, pos[2]];
                    !(0..48).any(|d| self.solid_at(world, [top[0], top[1] + d as f32, top[2]]))
                };
                if open_sky != self.mobs[i].burning {
                    self.mobs[i].burning = open_sky;
                    let flags = Self::mob_flags(&self.mobs[i]);
                    packets.push((world, protocol::set_mob_flags(self.mobs[i].entity_id, flags, None)));
                }
                if open_sky && self.mobs[i].age % 20 == 0 {
                    burn_damage.push(i);
                }
            }
            if kind == MobKind::Sheep && self.mobs[i].sheared && on_ground && rand::random::<u32>() % 1000 == 0 {
                let below = [pos[0].floor() as i32, pos[1].floor() as i32 - 1, pos[2].floor() as i32];
                let block = self.worlds.worlds[world].get_block(below, &self.gd);
                if self.gd.block_name(block) == "minecraft:grass_block" {
                    if let Some(dirt) = self.gd.default_block("minecraft:dirt") {
                        self.change_block(world, below, dirt);
                    }
                    let eid = self.mobs[i].entity_id;
                    self.mobs[i].sheared = false;
                    let flags = Self::mob_flags(&self.mobs[i]);
                    packets.push((world, protocol::actor_event(eid, 10, 0))); // eating grass
                    packets.push((world, protocol::set_mob_flags(eid, flags, Some(self.mobs[i].color))));
                }
            }
            let m = &mut self.mobs[i];
            m.pos = pos;
            m.motion = mot;
            m.yaw = yaw;
            m.on_ground = on_ground;
            m.target = new_target;
            if kind == MobKind::Chicken {
                m.egg_in = m.egg_in.saturating_sub(1);
                if m.egg_in == 0 {
                    m.egg_in = 6000 + rand::random::<u32>() % 6000;
                    eggs.push((world, pos));
                }
            }
            let moved = (0..3).map(|k| (m.pos[k] - m.sent_pos[k]).abs()).fold(0.0, f32::max) > 0.01
                || (m.yaw - m.sent_yaw).abs() > 2.0;
            if moved && m.age % 2 == 0 {
                m.sent_pos = m.pos;
                m.sent_yaw = m.yaw;
                packets.push((world, protocol::move_mob(m.entity_id, m.pos, m.yaw, m.on_ground)));
            }
            if ambient {
                packets.push((world, protocol::mob_sound("ambient", [pos[0], pos[1] + 0.5, pos[2]], kind.identifier(), false)));
            }
        }
        for (world, pk) in packets {
            self.broadcast(&pk, |_, q| q.world == world);
        }
        for (sid, i) in hits {
            let Some(m) = self.mobs.get(i) else { continue };
            let (eid, world, pos) = (m.entity_id, m.world, m.pos);
            let Some(p) = self.players.get(&sid) else { continue };
            let feet = [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]];
            let name = p.name.clone();
            let (dx, dz) = (feet[0] - pos[0], feet[2] - pos[2]);
            let d = (dx * dx + dz * dz).sqrt().max(0.01);
            let damage = match difficulty { 1 => 2.0, 3 => 4.0, _ => 3.0 };
            self.broadcast(&protocol::actor_event(eid, 4, 0), |_, q| q.world == world); // arm swing
            self.damage_player(sid, damage, &format!("§c{name} foi morto por um zumbi"), false, Some([dx / d * 0.4, 0.36, dz / d * 0.4]));
        }
        for i in burn_damage {
            if self.mobs.get(i).is_some_and(|m| m.death_ticks.is_none()) {
                self.hurt_mob(i, 1.0, None);
            }
        }
        for (world, pos) in eggs {
            if let Some(egg) = self.simple_item("minecraft:egg", 1) {
                self.spawn_item(world, pos, [0.0, 0.1, 0.0], egg, 10);
            }
        }
        for i in gone.into_iter().rev() {
            let m = self.mobs.remove(i);
            self.broadcast(&protocol::remove_actor(m.entity_id), |_, q| q.world == m.world);
        }
    }

    /// An item stack by name (for loot).
    pub(crate) fn simple_item(&self, name: &str, count: u16) -> Option<ItemDesc> {
        let info = self.gd.find_item(name)?;
        Some(ItemDesc {
            id: info.id,
            count,
            meta: 0,
            stack_id: 0,
            block_runtime_id: info.block_runtime_id,
            extra: ceres_world::extra_data(None),
        })
    }

    /// A player hit something that is not a player: true if it was a mob.
    pub(crate) fn attack_mob(&mut self, attacker: SessionId, eid: i64, damage: f32) -> bool {
        let Some(i) = self.mobs.iter().position(|m| m.entity_id == eid) else { return false };
        let Some(p) = self.players.get(&attacker) else { return true };
        let (apos, aworld, gm) = (p.pos, p.world, p.gamemode);
        let m = &self.mobs[i];
        if m.world != aworld || m.death_ticks.is_some() || m.hurt_cooldown > 0 || gm == 6 {
            return true;
        }
        let feet = [apos[0], apos[1] - EYE_HEIGHT, apos[2]];
        let d2: f32 = (0..3).map(|k| (m.pos[k] - feet[k]).powi(2)).sum();
        if d2 > 49.0 {
            return true; // out of reach
        }
        self.hurt_mob(i, damage, Some(feet));
        true
    }

    /// Damages a mob (knockback away from `from`); it dies at 0 health.
    pub(crate) fn hurt_mob(&mut self, i: usize, damage: f32, from: Option<[f32; 3]>) {
        let m = &mut self.mobs[i];
        m.health -= damage;
        m.hurt_cooldown = 10;
        m.panic = if m.kind.hostile() { 0 } else { 60 };
        m.target = None;
        if let Some(f) = from {
            let (dx, dz) = (m.pos[0] - f[0], m.pos[2] - f[2]);
            let d = (dx * dx + dz * dz).sqrt().max(0.01);
            m.motion = [dx / d * 0.4, 0.36, dz / d * 0.4];
        }
        let (eid, world, pos, kind, dead) = (m.entity_id, m.world, m.pos, m.kind, m.health <= 0.0);
        let sound_at = [pos[0], pos[1] + 0.5, pos[2]];
        self.broadcast(&protocol::actor_event(eid, actor_event::HURT_ANIMATION, 0), |_, q| q.world == world);
        if !dead {
            self.broadcast(&protocol::mob_sound("hurt", sound_at, kind.identifier(), false), |_, q| q.world == world);
            return;
        }
        self.mobs[i].death_ticks = Some(20);
        self.mobs[i].health = 0.0;
        self.broadcast(&protocol::actor_event(eid, actor_event::DEATH_ANIMATION, 0), |_, q| q.world == world);
        self.broadcast(&protocol::mob_sound("death", sound_at, kind.identifier(), false), |_, q| q.world == world);
        let color = self.mobs[i].color;
        let mut loot: Vec<(String, u16)> =
            kind.drops().iter().map(|(n, a, b)| (n.to_string(), rand_range(*a, *b))).collect();
        if kind == MobKind::Sheep {
            loot.push((format!("minecraft:{}_wool", WOOL[color as usize % 16]), 1));
        }
        for (name, count) in loot.into_iter().filter(|(_, c)| *c > 0) {
            if let Some(item) = self.simple_item(&name, count) {
                let rx = (rand::random::<f32>() - 0.5) * 0.2;
                let rz = (rand::random::<f32>() - 0.5) * 0.2;
                self.spawn_item(world, [pos[0], pos[1] + 0.5, pos[2]], [rx, 0.2, rz], item, 10);
            }
        }
        self.spawn_xp(world, [pos[0], pos[1] + 0.5, pos[2]], kind.xp());
    }

    /// `/summon <mob> [x y z]`
    pub(crate) fn cmd_summon(&mut self, sender: commands::Sender, args: &[&str]) {
        let usage = "§cUso: /summon <cow|pig|sheep|chicken|zombie> [x y z]";
        let Some(kind) = args.first().and_then(|n| MobKind::from_name(n)) else { return self.reply(sender, usage) };
        let commands::Sender::Player(sid) = sender else {
            return self.reply(sender, "§cUse /summon como jogador.");
        };
        let Some(p) = self.players.get(&sid) else { return };
        let (world, feet) = (p.world, [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]);
        let pos = if args.len() >= 4 {
            let c = |s: &str, base: f32| -> Option<f32> {
                match s.strip_prefix('~') {
                    Some("") => Some(base),
                    Some(rest) => rest.parse::<f32>().ok().map(|v| base + v),
                    None => s.parse().ok(),
                }
            };
            match (c(args[1], feet[0]), c(args[2], feet[1]), c(args[3], feet[2])) {
                (Some(x), Some(y), Some(z)) if [x, y, z].iter().all(|v| v.is_finite() && v.abs() < 30_000_000.0) => [x, y, z],
                _ => return self.reply(sender, usage),
            }
        } else {
            feet
        };
        match self.spawn_mob(world, kind, pos) {
            Some(_) => self.reply(sender, &format!("§a{} criado.", kind.identifier())),
            None => self.reply(sender, "§cMobs demais no servidor."),
        }
    }

    /// Natural spawning and despawning, a few attempts per second.
    pub(crate) fn natural_spawning(&mut self) {
        if self.ticks % 20 != 0 {
            return;
        }
        // despawn: zombies more than 128 blocks from every player, or randomly beyond 32
        let players: Vec<(usize, [f32; 3], SessionId)> = self
            .players
            .iter()
            .filter(|(_, p)| p.state == State::Spawned && !p.dead)
            .map(|(s, p)| (p.world, [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]], *s))
            .collect();
        let mut gone = Vec::new();
        for (i, m) in self.mobs.iter().enumerate() {
            if !m.kind.hostile() || m.death_ticks.is_some() {
                continue;
            }
            let nearest = players
                .iter()
                .filter(|(w, _, _)| *w == m.world)
                .map(|(_, f, _)| (0..3).map(|k| (f[k] - m.pos[k]).powi(2)).sum::<f32>())
                .fold(f32::MAX, f32::min);
            if nearest > 128.0 * 128.0 || (nearest > 32.0 * 32.0 && rand::random::<u32>() % 40 == 0) {
                gone.push(i);
            }
        }
        for i in gone.into_iter().rev() {
            let m = self.mobs.remove(i);
            self.broadcast(&protocol::remove_actor(m.entity_id), |_, q| q.world == m.world);
        }
        for (world, feet, _) in players {
            if self.worlds.worlds[world].void_world {
                continue;
            }
            let tod = self.worlds.worlds[world].time.rem_euclid(24000);
            let night = (13000..23000).contains(&tod);
            let near = |me: &Self, hostile: bool| {
                me.mobs
                    .iter()
                    .filter(|m| m.world == world && m.kind.hostile() == hostile)
                    .filter(|m| (0..3).map(|k| (feet[k] - m.pos[k]).powi(2)).sum::<f32>() < 64.0 * 64.0)
                    .count()
            };
            let monsters = self.cfg.spawn_monsters && self.cfg.difficulty > 0 && night && near(self, true) < 8;
            let animals = self.cfg.spawn_animals && !night && near(self, false) < 6 && rand::random::<u32>() % 20 == 0;
            if !monsters && !animals {
                continue;
            }
            let a = (rand::random::<u32>() % 360) as f32 * std::f32::consts::PI / 180.0;
            let dist = 24.0 + (rand::random::<u32>() % 24) as f32;
            let (x, z) = ((feet[0] + a.cos() * dist).floor() + 0.5, (feet[2] + a.sin() * dist).floor() + 0.5);
            let Some(y) = self.spawn_height(world, x, z, feet[1]) else { continue };
            let ground = self.worlds.worlds[world].get_block([x.floor() as i32, y as i32 - 1, z.floor() as i32], &self.gd);
            let open_sky = !(2..48).any(|d| self.solid_at(world, [x, y + d as f32, z]));
            if !open_sky {
                continue;
            }
            if monsters {
                self.spawn_mob(world, MobKind::Zombie, [x, y, z]);
            } else if self.gd.block_name(ground) == "minecraft:grass_block" {
                let kind = [MobKind::Cow, MobKind::Pig, MobKind::Sheep, MobKind::Chicken][rand::random::<u32>() as usize % 4];
                // animals come in small groups
                for k in 0..(2 + rand::random::<u32>() % 3) {
                    let p = [x + (k % 2) as f32, y, z + (k / 2) as f32];
                    if self.fits(world, p, kind.size()[1]) && self.solid_at(world, [p[0], p[1] - 0.5, p[2]]) {
                        self.spawn_mob(world, kind, p);
                    }
                }
            }
        }
    }

    /// The feet height of a spot with ground under it and two free blocks, near `around_y`.
    fn spawn_height(&mut self, world: usize, x: f32, z: f32, around_y: f32) -> Option<f32> {
        let start = (around_y + 16.0).min((ceres_world::MAX_Y - 2) as f32) as i32;
        let end = (around_y - 16.0).max((ceres_world::MIN_Y + 1) as f32) as i32;
        let mut y = start;
        while y > end {
            let fy = y as f32;
            if self.solid_at(world, [x, fy - 1.0, z]) && self.fits(world, [x, fy, z], 2.0) && !self.water_at(world, [x, fy, z]) {
                return Some(fy);
            }
            y -= 1;
        }
        None
    }

    /// Writes each world's mobs to `<world>/mobs.json`.
    pub(crate) fn save_mobs(&self) {
        for (w, world) in self.worlds.worlds.iter().enumerate() {
            let list: Vec<serde_json::Value> = self
                .mobs
                .iter()
                .filter(|m| m.world == w && m.death_ticks.is_none())
                .map(|m| {
                    serde_json::json!({
                        "type": m.kind.identifier(),
                        "pos": m.pos,
                        "yaw": m.yaw,
                        "health": m.health,
                        "color": m.color,
                        "sheared": m.sheared,
                    })
                })
                .collect();
            let path = world.dir.join("mobs.json");
            if list.is_empty() && !path.exists() {
                continue;
            }
            let tmp = world.dir.join("mobs.json.tmp");
            let text = serde_json::to_string(&list).unwrap_or_default();
            if std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
                eprintln!("[World {}] could not save the mobs", world.folder);
            }
        }
    }

    /// Reads back the mobs saved by [`Self::save_mobs`].
    pub(crate) fn load_mobs(&mut self) {
        let mut total = 0;
        for w in 0..self.worlds.worlds.len() {
            let path = self.worlds.worlds[w].dir.join("mobs.json");
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(&text) else {
                eprintln!("[World {}] mobs.json is damaged, ignored", self.worlds.worlds[w].folder);
                continue;
            };
            for v in list {
                let Some(kind) = v["type"].as_str().and_then(MobKind::from_name) else { continue };
                let pos: Vec<f32> = v["pos"].as_array().into_iter().flatten().filter_map(|n| n.as_f64()).map(|n| n as f32).collect();
                if pos.len() != 3 || !pos.iter().all(|n| n.is_finite()) {
                    continue;
                }
                let Some(eid) = self.spawn_mob(w, kind, [pos[0], pos[1], pos[2]]) else { break };
                if let Some(m) = self.mobs.iter_mut().find(|m| m.entity_id == eid) {
                    m.yaw = v["yaw"].as_f64().unwrap_or(0.0) as f32;
                    m.health = (v["health"].as_f64().unwrap_or(kind.max_health() as f64) as f32).clamp(1.0, kind.max_health());
                    m.color = (v["color"].as_u64().unwrap_or(0) as u8).min(15);
                    m.sheared = v["sheared"].as_bool().unwrap_or(false);
                }
                total += 1;
            }
        }
        if total > 0 {
            println!("[World] {total} mob(s) loaded");
        }
    }
}
