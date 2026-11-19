//! Lightning: a bolt (`minecraft:lightning_bolt`) hurts what stands within 3 blocks for 5 and
//! sets it on fire, may light the ground, and hurts mobs with the cause `lightning`, so the
//! behaviour pack's damage sensors do the rest (charged creepers, pigs into zombified piglins,
//! villagers into witches, red mooshrooms into brown...).
//! It strikes now and then near players during thunderstorms, and where a trident with
//! Channeling hits something under the open sky in a thunderstorm.

use super::*;

const BOLT_TICKS: u32 = 20;

pub(crate) struct Bolt {
    entity_id: i64,
    world: usize,
    age: u32,
}

impl Server {
    /// A bolt of lightning at `pos` (the block it hits is below it).
    pub(crate) fn strike_lightning(&mut self, world: usize, pos: [f32; 3]) {
        let eid = self.next_entity_id;
        self.next_entity_id += 1;
        self.broadcast(&protocol::add_entity(eid, "minecraft:lightning_bolt", pos, [0.0; 3], 0.0, 0.0, 0.0, &[], &[]), |_, q| q.world == world);
        self.broadcast(&protocol::level_sound_event("thunder", pos, -1), |_, q| q.world == world);
        self.bolts.push(Bolt { entity_id: eid, world, age: 0 });
        for e in self.entities_near(world, pos, 3.0, None) {
            match e {
                mobs::Ent::Player(sid) => {
                    let Some(p) = self.players.get(&sid) else { continue };
                    if p.gamemode == 1 || p.gamemode == 6 {
                        continue;
                    }
                    let name = p.name.clone();
                    if self.damage_player(sid, 5.0, &format!("§c{name} foi atingido por um raio"), false, None) {
                        if let Some(p) = self.players.get_mut(&sid) {
                            p.fire_ticks = p.fire_ticks.max(160);
                        }
                    }
                }
                mobs::Ent::Mob(m) => {
                    self.with_mob(m, |s, mob| {
                        // the hurt frames must not swallow the strike that charges a creeper
                        mob.hurt_cooldown = 0;
                        if s.hurt(mob, 5.0, "lightning", None, None) && !mob.t.fire_immune {
                            mob.fire_ticks = mob.fire_ticks.max(160);
                        }
                    });
                }
            }
        }
        // fire on the block it hit
        if self.cfg.mob_griefing {
            let b = [pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32];
            let here = self.worlds.worlds[world].get_block(b, &self.gd);
            let below = self.worlds.worlds[world].get_block([b[0], b[1] - 1, b[2]], &self.gd);
            if here == self.gd.air_id && self.gd.is_solid(below) {
                if let Some(fire) = self.gd.default_block("minecraft:fire") {
                    self.change_block(world, b, fire);
                }
            }
        }
    }

    /// The highest open spot of a column (where lightning lands).
    fn top_of_column(&mut self, world: usize, x: i32, z: i32, from_y: i32) -> Option<f32> {
        let top = (from_y + 48).min(ceres_world::MAX_Y);
        for y in (ceres_world::MIN_Y + 1..=top).rev() {
            if !self.worlds.worlds[world].is_loaded([x, y, z]) {
                return None;
            }
            let id = self.worlds.worlds[world].get_block([x, y, z], &self.gd);
            if id != self.gd.air_id {
                return Some(y as f32 + 1.0);
            }
        }
        None
    }

    /// Thunderstorms: a bolt now and then near each player; old bolts go away.
    pub(crate) fn tick_lightning(&mut self) {
        let mut gone = Vec::new();
        for (i, b) in self.bolts.iter_mut().enumerate() {
            b.age += 1;
            if b.age >= BOLT_TICKS {
                gone.push(i);
            }
        }
        for i in gone.into_iter().rev() {
            let b = self.bolts.swap_remove(i);
            self.broadcast(&protocol::remove_actor(b.entity_id), |_, q| q.world == b.world);
        }
        let near: Vec<(usize, [f32; 3])> = self
            .players
            .values()
            .filter(|p| p.state == State::Spawned && !p.dead)
            .filter(|p| self.worlds.worlds.get(p.world).is_some_and(|w| w.weather == ceres_world::Weather::Thunder))
            .map(|p| (p.world, p.pos))
            .collect();
        for (world, pos) in near {
            if rand::random::<u32>() % 1200 != 0 {
                continue;
            }
            let x = pos[0].floor() as i32 + (rand::random::<u32>() % 65) as i32 - 32;
            let z = pos[2].floor() as i32 + (rand::random::<u32>() % 65) as i32 - 32;
            if let Some(y) = self.top_of_column(world, x, z, pos[1] as i32) {
                self.strike_lightning(world, [x as f32 + 0.5, y, z as f32 + 0.5]);
            }
        }
    }

    /// Channeling: a trident hit at `pos` calls lightning in a thunderstorm under the open sky.
    pub(crate) fn channeling_strike(&mut self, world: usize, pos: [f32; 3]) {
        if self.worlds.worlds.get(world).is_none_or(|w| w.weather != ceres_world::Weather::Thunder) {
            return;
        }
        let b = [pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32];
        let covered = (b[1] + 1..(b[1] + 64).min(ceres_world::MAX_Y)).any(|y| {
            let id = self.worlds.worlds[world].get_block([b[0], y, b[2]], &self.gd);
            self.gd.is_solid(id)
        });
        if !covered {
            self.broadcast(&protocol::level_sound_event("item.trident.thunder", pos, -1), |_, q| q.world == world);
            self.strike_lightning(world, pos);
        }
    }
}
