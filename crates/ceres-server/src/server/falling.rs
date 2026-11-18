//! Blocks with gravity (sand, gravel, concrete powder, anvils): when nothing holds them up they
//! turn into a falling block entity, fall with gravity and become a block again where they land
//! (or drop as an item when that spot is taken).

use super::*;
use ceres_protocol::ItemDesc;

const GRAVITY: f32 = 0.04;
const DRAG: f32 = 0.98;

pub(crate) struct FallingBlock {
    entity_id: i64,
    world: usize,
    block: i32,
    pos: [f32; 3],
    speed: f32,
    age: u32,
}

pub(crate) fn has_gravity(name: &str) -> bool {
    matches!(
        name,
        "minecraft:sand"
            | "minecraft:red_sand"
            | "minecraft:gravel"
            | "minecraft:suspicious_sand"
            | "minecraft:suspicious_gravel"
            | "minecraft:anvil"
            | "minecraft:chipped_anvil"
            | "minecraft:damaged_anvil"
            | "minecraft:dragon_egg"
    ) || name.ends_with("_concrete_powder")
}

impl Server {
    /// A block changed at `pos`: the block there and the one above may have lost their support.
    pub(crate) fn queue_gravity(&mut self, world: usize, pos: [i32; 3]) {
        self.gravity_queue.push((world, pos));
        self.gravity_queue.push((world, [pos[0], pos[1] + 1, pos[2]]));
    }

    fn start_falling(&mut self, world: usize, pos: [i32; 3]) {
        let w = &mut self.worlds.worlds[world];
        let block = w.get_block(pos, &self.gd);
        if !has_gravity(self.gd.block_name(block)) {
            return;
        }
        let below = w.get_block([pos[0], pos[1] - 1, pos[2]], &self.gd);
        if !self.gd.is_replaceable(below) || pos[1] - 1 < ceres_world::MIN_Y {
            return;
        }
        let air = self.gd.air_id;
        if !self.change_block(world, pos, air) {
            return;
        }
        let eid = self.next_entity_id;
        self.next_entity_id += 1;
        let at = [pos[0] as f32 + 0.5, pos[1] as f32, pos[2] as f32 + 0.5];
        self.broadcast(&protocol::add_falling_block(eid, at, block), |_, q| q.world == world);
        self.falling.push(FallingBlock { entity_id: eid, world, block, pos: at, speed: 0.0, age: 0 });
    }

    /// Gravity checks queued by block changes, then the falling blocks' movement.
    pub(crate) fn tick_falling(&mut self) {
        // blocks that start falling free the spot above them, which queues more checks
        let mut rounds = 0;
        while !self.gravity_queue.is_empty() && rounds < 64 {
            rounds += 1;
            let queue = std::mem::take(&mut self.gravity_queue);
            for (world, pos) in queue {
                self.start_falling(world, pos);
            }
        }
        self.gravity_queue.clear();

        let mut landed = Vec::new();
        for i in 0..self.falling.len() {
            let f = &mut self.falling[i];
            f.age += 1;
            f.speed = (f.speed + GRAVITY) * DRAG;
            let next_y = f.pos[1] - f.speed;
            let (x, z) = (f.pos[0].floor() as i32, f.pos[2].floor() as i32);
            let (world, eid) = (f.world, f.entity_id);
            // every block the falling block passes through this tick, top to bottom
            let mut stop = None;
            let (top, bottom) = (f.pos[1].ceil() as i32 - 1, next_y.floor() as i32);
            for y in (bottom.max(ceres_world::MIN_Y)..=top).rev() {
                let b = self.worlds.worlds[world].get_block([x, y, z], &self.gd);
                if !self.gd.is_replaceable(b) {
                    stop = Some(y + 1);
                    break;
                }
            }
            let f = &mut self.falling[i];
            if let Some(land_y) = stop {
                f.pos[1] = land_y as f32;
                landed.push(i);
            } else if next_y < (ceres_world::MIN_Y - 16) as f32 || f.age > 600 {
                f.pos[1] = next_y;
                landed.push(i);
            } else {
                f.pos[1] = next_y;
                let pk = protocol::move_actor_absolute(eid, f.pos, false, false);
                self.broadcast(&pk, |_, q| q.world == world);
            }
        }
        for i in landed.into_iter().rev() {
            let f = self.falling.swap_remove(i);
            self.broadcast(&protocol::remove_actor(f.entity_id), |_, q| q.world == f.world);
            if f.pos[1] < ceres_world::MIN_Y as f32 {
                continue; // fell out of the world
            }
            let target = [f.pos[0].floor() as i32, f.pos[1].floor() as i32, f.pos[2].floor() as i32];
            let here = self.worlds.worlds[f.world].get_block(target, &self.gd);
            if self.gd.is_replaceable(here) && self.change_block(f.world, target, f.block) {
                continue;
            }
            // the spot is taken (a torch, a slab...): the block drops as an item
            if let Some(ce) = self.gd.block_to_item.get(&f.block).cloned() {
                let item = ItemDesc {
                    id: ce.id,
                    count: 1,
                    meta: ce.meta,
                    stack_id: 0,
                    block_runtime_id: ce.block_runtime_id,
                    extra: ce.extra,
                };
                self.spawn_item(f.world, [f.pos[0], f.pos[1] + 0.5, f.pos[2]], [0.0, 0.1, 0.0], item, 10);
            }
        }
    }
}
