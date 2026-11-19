//! Firework rockets and elytra flight. A rocket used on a block flies up and bursts with the
//! colours stored in the item; used in the air while gliding it boosts the player. Gliding wears
//! the elytra (1 durability per second) and does not count as falling.

use super::*;
use ceres_codec::nbt::{self, Flavor, Tag};

const ACTOR_EVENT_FIREWORK_PARTICLES: u8 = 25;
const ELYTRA_WEAR_TICKS: u64 = 20;

pub(crate) struct Firework {
    entity_id: i64,
    world: usize,
    pos: [f32; 3],
    motion: [f32; 3],
    life: u32,
    age: u32,
    /// the rocket's "Fireworks" compound (explosions and flight)
    data: Tag,
    /// player boosted by this rocket (elytra), who it follows
    rider: Option<SessionId>,
    /// who launched it
    owner: SessionId,
    /// shot from a crossbow: flies straight and bursts on what it hits
    shot: bool,
}

/// The "Fireworks" compound of a rocket item (empty when the item has none).
fn fireworks_tag(item: &ceres_protocol::ItemDesc) -> Tag {
    if item.extra.len() > 3 && item.extra[0] == 0xff && item.extra[1] == 0xff {
        if let Ok((_, tag)) = nbt::read_root(&mut R::new(&item.extra[3..]), Flavor::LittleEndian) {
            if let Some(f @ Tag::Compound(_)) = tag.get("Fireworks") {
                return f.clone();
            }
        }
    }
    Tag::Compound(vec![("Explosions".into(), Tag::List(10, Vec::new())), ("Flight".into(), Tag::Byte(1))])
}

fn explosion_count(data: &Tag) -> usize {
    data.get("Explosions").and_then(|l| l.as_list()).map_or(0, |l| l.len())
}

impl Server {
    fn spawn_firework(&mut self, owner: SessionId, world: usize, pos: [f32; 3], motion: [f32; 3], data: Tag, rider: Option<SessionId>) {
        let flight = data.get("Flight").and_then(|v| v.as_i64()).unwrap_or(1).clamp(0, 3) as u32;
        let life = 10 * (flight + 1) + rand::random::<u32>() % 6 + rand::random::<u32>() % 7;
        let eid = self.next_entity_id;
        self.next_entity_id += 1;
        let item = Tag::Compound(vec![("Fireworks".into(), data.clone())]);
        let pk = protocol::add_firework(eid, pos, motion, &nbt::to_network(&item));
        self.broadcast(&pk, |_, q| q.world == world);
        let center = pos;
        self.broadcast(&protocol::level_sound_event("launch", center, -1), |_, q| q.world == world);
        self.fireworks.push(Firework { entity_id: eid, world, pos, motion, life, age: 0, data, rider, owner, shot: false });
    }

    /// A rocket used on a block: it takes off from the clicked face.
    pub(crate) fn launch_firework_at(&mut self, sid: SessionId, pos: [f32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let held = p.inventory.held_item().clone();
        if self.gd.items_by_id.get(&held.id).is_none_or(|i| i.name != "minecraft:firework_rocket") {
            return false;
        }
        let (world, gm) = (p.world, p.gamemode);
        if !self.may_launch(sid) {
            return true; // cooldown and limits shared with projectiles
        }
        let jitter = || (rand::random::<f32>() - 0.5) * 0.004;
        self.spawn_firework(sid, world, pos, [jitter(), 0.05, jitter()], fireworks_tag(&held), None);
        self.consume_held(sid, gm);
        true
    }

    /// A rocket used in the air: boosts a gliding player. Returns true when the held item is a rocket.
    pub(crate) fn use_firework_in_air(&mut self, sid: SessionId) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let held = p.inventory.held_item().clone();
        if self.gd.items_by_id.get(&held.id).is_none_or(|i| i.name != "minecraft:firework_rocket") {
            return false;
        }
        if !p.actor_state.gliding || p.dead {
            return true; // vanilla: a rocket in the air only does something while gliding
        }
        let (world, gm, pos, rot) = (p.world, p.gamemode, p.pos, p.rot);
        if !self.may_launch(sid) {
            return true;
        }
        let Some(p) = self.players.get(&sid) else { return true };
        let (pitch, yaw) = (rot[0].to_radians(), rot[1].to_radians());
        let look = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
        let boost = [look[0] * 1.5, look[1] * 1.5, look[2] * 1.5];
        let eid = p.entity_id;
        let mot = protocol::set_actor_motion(eid, boost);
        if let Some(p) = self.players.get_mut(&sid) {
            p.out.push(mot.clone());
        }
        self.broadcast(&mot, |o, q| o != sid && q.world == world);
        self.spawn_firework(sid, world, pos, boost, fireworks_tag(&held), Some(sid));
        self.consume_held(sid, gm);
        true
    }

    fn explode_firework(&mut self, f: &Firework) {
        let world = f.world;
        self.broadcast(&protocol::actor_event(f.entity_id, ACTOR_EVENT_FIREWORK_PARTICLES, 0), |_, q| q.world == world);
        let count = explosion_count(&f.data);
        if count == 0 {
            return;
        }
        let sound = if f.data.get("Explosions").and_then(|l| l.as_list()).is_some_and(|l| {
            l.iter().any(|e| e.get("FireworkType").and_then(|v| v.as_i64()) == Some(1))
        }) {
            "large_blast"
        } else {
            "blast"
        };
        self.broadcast(&protocol::level_sound_event(sound, f.pos, -1), |_, q| q.world == world);
        // rockets with stars hurt whoever is close: 5 + 2 per star, less with distance
        let force = (count * 2 + 5) as f32;
        let near: Vec<(SessionId, f32)> = self
            .players
            .iter()
            // with PvP off a rocket only hurts the player who launched it
            .filter(|(s, q)| q.world == world && q.state == State::Spawned && !q.dead && (self.cfg.pvp || **s == f.owner))
            .map(|(s, q)| (*s, ((q.pos[0] - f.pos[0]).powi(2) + (q.pos[1] - 0.8 - f.pos[1]).powi(2) + (q.pos[2] - f.pos[2]).powi(2)).sqrt()))
            .filter(|(_, d)| *d <= 5.0)
            .collect();
        for (sid, dist) in near {
            let dmg = force * (5.0 - dist).max(0.0) / 5.0;
            if dmg > 0.0 {
                let name = self.players.get(&sid).map(|p| p.name.clone()).unwrap_or_default();
                self.damage_player(sid, dmg, &format!("§c{name} explodiu com fogos de artifício"), false, None);
            }
        }
    }

    pub(crate) fn tick_fireworks(&mut self) {
        // elytra: gliding wears it and is not a fall
        let ticks = self.ticks;
        let gliders: Vec<SessionId> = self.players.iter().filter(|(_, p)| p.actor_state.gliding && !p.dead).map(|(s, _)| *s).collect();
        for sid in gliders {
            if let Some(p) = self.players.get_mut(&sid) {
                p.fall_start_y = None;
            }
            if ticks % ELYTRA_WEAR_TICKS == 0 {
                self.wear_elytra(sid);
            }
        }

        if self.fireworks.is_empty() {
            return;
        }
        let mut done = Vec::new();
        for i in 0..self.fireworks.len() {
            let rider_pos = self.fireworks[i].rider.and_then(|s| self.players.get(&s)).map(|p| p.pos);
            let f = &mut self.fireworks[i];
            f.age += 1;
            match rider_pos {
                // a boosting rocket stays with its player
                Some(pos) => f.pos = pos,
                None if f.shot => {
                    f.pos = [f.pos[0] + f.motion[0], f.pos[1] + f.motion[1], f.pos[2] + f.motion[2]];
                }
                None => {
                    f.motion = [f.motion[0] * 1.15, f.motion[1] + 0.04, f.motion[2] * 1.15];
                    f.pos = [f.pos[0] + f.motion[0], f.pos[1] + f.motion[1], f.pos[2] + f.motion[2]];
                }
            }
            let (eid, world, pos, shot, owner) = (f.entity_id, f.world, f.pos, f.shot, f.owner);
            if f.age >= f.life {
                done.push(i);
                continue;
            }
            if shot && f.age > 2 {
                let owner_eid = self.players.get(&owner).map(|p| p.entity_id);
                let hit_entity = self.entity_at(world, pos, None, true).is_some_and(|e| match e {
                    mobs::Ent::Player(s) => s != owner,
                    mobs::Ent::Mob(m) => Some(m) != owner_eid,
                });
                let b = [pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32];
                let hit_block = !self.worlds.worlds[world].is_loaded(b) || self.gd.is_solid(self.worlds.worlds[world].get_block(b, &self.gd));
                if hit_entity || hit_block {
                    done.push(i);
                    continue;
                }
            }
            self.broadcast(&protocol::move_actor_absolute(eid, pos, false, false), |_, q| q.world == world);
        }
        for i in done.into_iter().rev() {
            let f = self.fireworks.swap_remove(i);
            self.explode_firework(&f);
            self.broadcast(&protocol::remove_actor(f.entity_id), |_, q| q.world == f.world);
        }
    }

    fn wear_elytra(&mut self, sid: SessionId) {
        let Some(p) = self.players.get(&sid) else { return };
        if p.gamemode == 1 || p.gamemode == 6 {
            return;
        }
        let chest = p.inventory.armor[1].clone();
        if self.gd.items_by_id.get(&chest.id).is_none_or(|i| i.name != "minecraft:elytra") {
            return;
        }
        // an elytra with 1 durability left stops working instead of breaking
        let max = durability::max_durability("minecraft:elytra").unwrap_or(432);
        if durability::item_damage(&chest) + 1 >= max {
            return;
        }
        let mut piece = chest;
        durability::wear_item(&mut piece, 1, &self.gd);
        let p = self.players.get_mut(&sid).unwrap();
        p.inventory.armor[1] = piece;
        p.inventory.dirty = true;
        p.out.push(protocol::inventory_content(ceres_protocol::window::ARMOR as u32, &p.inventory.armor));
    }
}

impl Server {
    /// A crossbow fires a loaded rocket along the player's look (turned by `yaw_offset`).
    pub(crate) fn shoot_firework(&mut self, sid: SessionId, data: Tag, yaw_offset: f32) {
        let Some(p) = self.players.get(&sid) else { return };
        let (world, pos, rot) = (p.world, p.pos, p.rot);
        let (pitch, yaw) = (rot[0].to_radians(), (rot[1] + yaw_offset).to_radians());
        let dir = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
        let start = [pos[0] + dir[0] * 0.6, pos[1] - 0.3 + dir[1] * 0.6, pos[2] + dir[2] * 0.6];
        let motion = [dir[0] * 1.6, dir[1] * 1.6, dir[2] * 1.6];
        self.spawn_firework(sid, world, start, motion, data, None);
        if let Some(f) = self.fireworks.last_mut() {
            f.shot = true;
        }
    }
}
