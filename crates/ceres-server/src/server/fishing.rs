//! Fishing rods: using one casts a bobber (`minecraft:fishing_hook`, its owner set so the client
//! draws the line). In water it floats and, after 5 to 30 seconds (Lure takes 5 s off per level),
//! a fish bites for a second or two; reeling in then catches loot from
//! `loot_tables/gameplay/fishing.json` (Luck of the Sea favours treasure) with 1 to 6 experience.
//! A bobber that hooks a mob or a player pulls it in; one stuck in the ground just comes back.

use super::*;
use ceres_protocol::ItemDesc;

const EYE_HEIGHT: f32 = 1.62;
const ENCHANT_LUCK_OF_THE_SEA: i16 = 23;
const ENCHANT_LURE: i16 = 24;
const FISHHOOK_BUBBLE: u8 = 11;
const FISHHOOK_HOOKTIME: u8 = 13;

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Flying,
    Water,
    Ground,
    Hooked(mobs::Ent),
}

pub(crate) struct Bobber {
    entity_id: i64,
    owner: SessionId,
    world: usize,
    pos: [f32; 3],
    motion: [f32; 3],
    state: State,
    /// ticks until a fish bites
    wait: u32,
    /// ticks a fish stays on the hook
    bite: u32,
    age: u32,
}

impl Server {
    fn held_rod(&self, sid: SessionId) -> Option<ItemDesc> {
        let p = self.players.get(&sid)?;
        let held = p.inventory.held_item();
        (self.gd.items_by_id.get(&held.id)?.name == "minecraft:fishing_rod").then(|| held.clone())
    }

    /// The use button with a fishing rod: cast, or reel in the bobber already out.
    pub(crate) fn use_fishing_rod(&mut self, sid: SessionId) -> bool {
        let Some(rod) = self.held_rod(sid) else { return false };
        if let Some(i) = self.bobbers.iter().position(|b| b.owner == sid) {
            let b = self.bobbers.swap_remove(i);
            self.reel_in(sid, b, &rod);
            return true;
        }
        let Some(p) = self.players.get(&sid) else { return true };
        if p.dead || p.gamemode == 6 {
            return true;
        }
        let (world, pos, rot, eid) = (p.world, p.pos, p.rot, p.entity_id);
        let (pitch, yaw) = (rot[0].to_radians(), rot[1].to_radians());
        let dir = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
        let jitter = || (rand::random::<f32>() - 0.5) * 0.03;
        let motion = [dir[0] * 0.6 + jitter(), dir[1] * 0.6 + 0.1 + jitter(), dir[2] * 0.6 + jitter()];
        let start = [pos[0] + dir[0] * 0.4, pos[1] - 0.2, pos[2] + dir[2] * 0.4];
        let bid = self.next_entity_id;
        self.next_entity_id += 1;
        self.broadcast(&protocol::add_projectile(bid, "minecraft:fishing_hook", start, motion, eid, None), |_, q| q.world == world);
        self.broadcast(&protocol::level_sound_event("throw", start, -1), |_, q| q.world == world);
        self.swing_arm(sid);
        let lure = blocks::enchant_level(&rod, ENCHANT_LURE) as u32;
        self.bobbers.push(Bobber { entity_id: bid, owner: sid, world, pos: start, motion, state: State::Flying, wait: Self::bite_wait(lure), bite: 0, age: 0 });
        true
    }

    fn bite_wait(lure: u32) -> u32 {
        (100 + rand::random::<u32>() % 500).saturating_sub(100 * lure).max(20)
    }

    fn reel_in(&mut self, sid: SessionId, b: Bobber, rod: &ItemDesc) {
        self.broadcast(&protocol::remove_actor(b.entity_id), |_, q| q.world == b.world);
        let Some(p) = self.players.get(&sid) else { return };
        let (world, eye, survival) = (p.world, p.pos, p.gamemode != 1);
        let mut wear = 0;
        match b.state {
            State::Water if b.bite > 0 => {
                let luck = blocks::enchant_level(rod, ENCHANT_LUCK_OF_THE_SEA) as f32;
                let cx = mobs::loot::LootCx { luck, difficulty: self.cfg.difficulty, killed_by_player: true, ..Default::default() };
                // jungle waters have their own fish and junk (bamboo, cocoa beans...)
                let bp = [b.pos[0].floor() as i32, b.pos[1].floor() as i32, b.pos[2].floor() as i32];
                let biome = self.worlds.worlds[world].biome_at(bp, &self.gd);
                let jungle = self.gd.biomes.get(&biome).is_some_and(|bi| bi.tags.iter().any(|t| t == "jungle"));
                let table = if jungle { "loot_tables/gameplay/jungle_fishing.json" } else { "loot_tables/gameplay/fishing.json" };
                let catch = self.roll_loot(table, &cx);
                for it in catch {
                    // the catch flies to the player
                    let d = [eye[0] - b.pos[0], eye[1] - b.pos[1], eye[2] - b.pos[2]];
                    let h = (d[0] * d[0] + d[2] * d[2]).sqrt();
                    let motion = [d[0] * 0.1, d[1] * 0.1 + h.sqrt() * 0.08, d[2] * 0.1];
                    self.spawn_item(world, b.pos, motion, it, 0);
                }
                self.spawn_xp(world, [eye[0], eye[1] - 1.0, eye[2]], 1 + rand::random::<u32>() % 6);
                wear = 1;
            }
            State::Hooked(e) => {
                if let Some(target) = self.subj(e).map(|s| s.feet()) {
                    let d = [eye[0] - target[0], eye[1] - EYE_HEIGHT - target[1], eye[2] - target[2]];
                    let pull = [d[0] * 0.1, d[1] * 0.1 + 0.2, d[2] * 0.1];
                    match e {
                        mobs::Ent::Player(o) => {
                            if let Some(q) = self.players.get(&o) {
                                let pk = protocol::set_actor_motion(q.entity_id, pull);
                                if let Some(q) = self.players.get_mut(&o) {
                                    q.out.push(pk.clone());
                                }
                                self.broadcast(&pk, |x, q| x != o && q.world == world);
                            }
                        }
                        mobs::Ent::Mob(eid) => {
                            if let Some(m) = self.mobs.iter_mut().find(|m| m.entity_id == eid) {
                                m.motion = pull;
                            }
                        }
                    }
                }
                wear = 3;
            }
            State::Ground => wear = 2,
            _ => {}
        }
        self.broadcast(&protocol::level_sound_event("throw", eye, -1), |_, q| q.world == world);
        self.swing_arm(sid);
        if survival && wear > 0 {
            self.wear_held(sid, wear);
        }
    }

    pub(crate) fn tick_fishing(&mut self) {
        let mut gone = Vec::new();
        for i in 0..self.bobbers.len() {
            let (owner, world) = (self.bobbers[i].owner, self.bobbers[i].world);
            self.bobbers[i].age += 1;
            // the line breaks when the angler goes away or puts the rod down
            let keep = self.players.get(&owner).is_some_and(|p| {
                p.world == world && !p.dead && p.state == super::State::Spawned && {
                    let b = self.bobbers[i].pos;
                    (0..3).map(|k| (p.pos[k] - b[k]).powi(2)).sum::<f32>() < 32.0 * 32.0
                }
            }) && self.held_rod(owner).is_some();
            if !keep || self.bobbers[i].age > 6000 {
                gone.push(i);
                continue;
            }
            let b = &self.bobbers[i];
            let (pos, mut motion, state, eid) = (b.pos, b.motion, b.state, b.entity_id);
            let block_at = |s: &mut Server, p: [f32; 3]| {
                let bp = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
                if !s.worlds.worlds[world].is_loaded(bp) {
                    return (true, false);
                }
                let id = s.worlds.worlds[world].get_block(bp, &s.gd);
                let name = s.gd.block_name(id);
                (s.gd.is_solid(id), name.contains("water"))
            };
            let mut new_pos = pos;
            let mut new_state = state;
            match state {
                State::Flying => {
                    let next = [pos[0] + motion[0], pos[1] + motion[1], pos[2] + motion[2]];
                    // anything with a body on the way gets hooked
                    let caught = self.entity_at(world, next, None, true).filter(|e| *e != mobs::Ent::Player(owner));
                    if let Some(e) = caught {
                        new_state = State::Hooked(e);
                        new_pos = next;
                    } else {
                        let (solid, water) = block_at(self, next);
                        if water {
                            new_state = State::Water;
                            new_pos = next;
                            motion = [0.0; 3];
                            self.broadcast(&protocol::level_sound_event("splash", next, -1), |_, q| q.world == world);
                        } else if solid {
                            new_state = State::Ground;
                            motion = [0.0; 3];
                        } else {
                            new_pos = next;
                            motion = [motion[0] * 0.92, motion[1] * 0.92 - 0.03, motion[2] * 0.92];
                        }
                    }
                }
                State::Water => {
                    // float at the surface
                    let (_, water_above) = block_at(self, [pos[0], pos[1] + 0.3, pos[2]]);
                    let (_, water_here) = block_at(self, pos);
                    if water_above {
                        new_pos[1] += 0.05;
                    } else if !water_here {
                        new_pos[1] -= 0.05;
                    }
                    let (bite, wait) = (self.bobbers[i].bite, self.bobbers[i].wait);
                    if bite > 0 {
                        self.bobbers[i].bite -= 1;
                        if bite == 1 {
                            let lure = self.held_rod(owner).map(|r| blocks::enchant_level(&r, ENCHANT_LURE) as u32).unwrap_or(0);
                            self.bobbers[i].wait = Self::bite_wait(lure);
                        }
                    } else if wait > 0 {
                        self.bobbers[i].wait -= 1;
                        if wait == 21 {
                            self.broadcast(&protocol::actor_event(eid, FISHHOOK_BUBBLE, 0), |_, q| q.world == world);
                        }
                        if wait == 1 {
                            self.bobbers[i].bite = 20 + rand::random::<u32>() % 20;
                            new_pos[1] -= 0.2; // the bobber dips
                            self.broadcast(&protocol::actor_event(eid, FISHHOOK_HOOKTIME, 0), |_, q| q.world == world);
                            self.broadcast(&protocol::level_sound_event("splash", pos, -1), |_, q| q.world == world);
                        }
                    }
                }
                State::Hooked(e) => match self.subj(e) {
                    Some(s) => {
                        let f = s.feet();
                        new_pos = [f[0], f[1] + 1.0, f[2]];
                    }
                    None => new_state = State::Flying,
                },
                State::Ground => {}
            }
            let b = &mut self.bobbers[i];
            b.motion = motion;
            b.state = new_state;
            if new_pos != b.pos {
                b.pos = new_pos;
                let pk = protocol::move_actor_absolute(eid, new_pos, false, false);
                self.broadcast(&pk, |_, q| q.world == world);
            }
        }
        for i in gone.into_iter().rev() {
            let b = self.bobbers.swap_remove(i);
            self.broadcast(&protocol::remove_actor(b.entity_id), |_, q| q.world == b.world);
        }
    }
}
