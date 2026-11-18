//! Dropped item entities (AddItemActor / TakeItemActor / MoveActorAbsolute): gravity, ground
//! collision, stack merging, 5-minute despawn and automatic player pickup.

use super::*;
use ceres_protocol::{ItemDesc, level_event, window};

const DESPAWN_TICKS: u32 = 6000; // 5 minutes at 20 TPS
const GRAVITY: f32 = 0.04;

pub(crate) struct ItemEntity {
    pub(crate) entity_id: i64,
    pub(crate) world: usize,
    pub(crate) item: ItemDesc,
    pub(crate) pos: [f32; 3],
    pub(crate) motion: [f32; 3],
    pub(crate) on_ground: bool,
    pub(crate) pickup_delay: u16,
    pub(crate) age: u32,
}

impl Server {
    pub(crate) fn spawn_item(
        &mut self,
        world: usize,
        pos: [f32; 3],
        motion: [f32; 3],
        mut item: ItemDesc,
        pickup_delay: u16,
    ) {
        if item.is_empty() {
            return;
        }
        item.stack_id = 0;
        let eid = self.next_entity_id;
        self.next_entity_id += 1;
        let pk = protocol::add_item_actor(eid, &item, pos, motion);
        self.broadcast(&pk, |_, p| p.world == world);
        self.items.push(ItemEntity {
            entity_id: eid,
            world,
            item,
            pos,
            motion,
            on_ground: false,
            pickup_delay,
            age: 0,
        });
    }

    /// Throws `item` forward from `sid`'s eye position.
    pub(crate) fn drop_from_player(&mut self, sid: SessionId, item: ItemDesc) {
        let Some(p) = self.players.get(&sid) else { return };
        let (world, pos, pitch, yaw) = (p.world, p.pos, p.rot[0].to_radians(), p.rot[1].to_radians());
        let cos_p = pitch.cos();
        let motion = [-yaw.sin() * cos_p * 0.3, -pitch.sin() * 0.3 + 0.1, yaw.cos() * cos_p * 0.3];
        self.spawn_item(world, [pos[0], pos[1] - 0.3, pos[2]], motion, item, 40);
    }

    /// Sends all active dropped items in `sid`'s world to `sid`.
    pub(crate) fn send_world_items(&mut self, sid: SessionId) {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return };
        let pks: Vec<Vec<u8>> = self
            .items
            .iter()
            .filter(|it| it.world == world)
            .map(|it| protocol::add_item_actor(it.entity_id, &it.item, it.pos, it.motion))
            .collect();
        if let Some(p) = self.players.get_mut(&sid) {
            p.out.extend(pks);
        }
    }

    pub(crate) fn tick_items(&mut self) {
        if self.items.is_empty() {
            return;
        }

        // 1. Physics + despawn
        let mut expired = Vec::new();
        let mut moved_packets: Vec<(usize, Vec<u8>)> = Vec::new();
        for i in 0..self.items.len() {
            self.items[i].age += 1;
            if self.items[i].pickup_delay > 0 {
                self.items[i].pickup_delay -= 1;
            }
            if self.items[i].age >= DESPAWN_TICKS || self.items[i].pos[1] < (ceres_world::MIN_Y - 32) as f32 {
                expired.push(i);
                continue;
            }
            let world = self.items[i].world;
            let mut pos = self.items[i].pos;
            let mut mot = self.items[i].motion;

            let below = [pos[0].floor() as i32, (pos[1] - 0.05).floor() as i32, pos[2].floor() as i32];
            let solid_below = self.gd.is_solid(self.worlds.worlds[world].get_block(below, &self.gd));
            if solid_below && mot[1] <= 0.0 && (pos[1] - (below[1] as f32 + 1.0)).abs() < 0.2 {
                pos[1] = below[1] as f32 + 1.0;
                mot[1] = 0.0;
                mot[0] *= 0.6;
                mot[2] *= 0.6;
                self.items[i].on_ground = true;
            } else {
                self.items[i].on_ground = false;
                mot[1] = (mot[1] - GRAVITY).max(-1.0);
                mot[0] *= 0.98;
                mot[2] *= 0.98;
            }

            let speed2 = mot[0] * mot[0] + mot[1] * mot[1] + mot[2] * mot[2];
            if speed2 > 1e-4 {
                let nx = pos[0] + mot[0];
                let ny = pos[1] + mot[1];
                let nz = pos[2] + mot[2];
                let next_block = [nx.floor() as i32, ny.floor() as i32, nz.floor() as i32];
                if self.gd.is_solid(self.worlds.worlds[world].get_block(next_block, &self.gd)) {
                    if mot[1] < 0.0 {
                        pos[1] = (next_block[1] + 1) as f32;
                        mot[1] = 0.0;
                        self.items[i].on_ground = true;
                    }
                    mot[0] = 0.0;
                    mot[2] = 0.0;
                } else {
                    pos = [nx, ny, nz];
                }
                self.items[i].pos = pos;
                self.items[i].motion = mot;
                if self.items[i].age % 2 == 0 {
                    moved_packets.push((
                        world,
                        protocol::move_actor_absolute(self.items[i].entity_id, pos, self.items[i].on_ground, false),
                    ));
                }
            } else {
                self.items[i].motion = [0.0; 3];
            }
        }
        for i in expired.into_iter().rev() {
            let dead = self.items.swap_remove(i);
            self.broadcast(&protocol::remove_actor(dead.entity_id), |_, p| p.world == dead.world);
        }
        for (world, pk) in moved_packets {
            self.broadcast(&pk, |_, p| p.world == world);
        }

        // 2. Stack merging (every 10 ticks)
        if self.ticks % 10 == 0 && self.items.len() > 1 {
            let mut merged_away: Vec<usize> = Vec::new();
            for i in 0..self.items.len() {
                if merged_away.contains(&i) {
                    continue;
                }
                let max_stack = self.gd.max_stack(self.items[i].item.id) as u16;
                if self.items[i].item.count >= max_stack {
                    continue;
                }
                for j in (i + 1)..self.items.len() {
                    if merged_away.contains(&j) || self.items[i].world != self.items[j].world {
                        continue;
                    }
                    if !self.items[i].item.same_type(&self.items[j].item) {
                        continue;
                    }
                    let dx = self.items[i].pos[0] - self.items[j].pos[0];
                    let dy = self.items[i].pos[1] - self.items[j].pos[1];
                    let dz = self.items[i].pos[2] - self.items[j].pos[2];
                    if dx * dx + dy * dy + dz * dz <= 1.2 * 1.2
                        && self.items[i].item.count + self.items[j].item.count <= max_stack
                    {
                        self.items[i].item.count += self.items[j].item.count;
                        merged_away.push(j);
                        let (w, pos, it, eid) = (
                            self.items[i].world,
                            self.items[i].pos,
                            self.items[i].item.clone(),
                            self.items[i].entity_id,
                        );
                        let dead_eid = self.items[j].entity_id;
                        self.broadcast(&protocol::remove_actor(dead_eid), |_, p| p.world == w);
                        self.broadcast(&protocol::remove_actor(eid), |_, p| p.world == w);
                        self.broadcast(&protocol::add_item_actor(eid, &it, pos, [0.0; 3]), |_, p| p.world == w);
                        if self.items[i].item.count >= max_stack {
                            break;
                        }
                    }
                }
            }
            merged_away.sort_unstable();
            for j in merged_away.into_iter().rev() {
                self.items.swap_remove(j);
            }
        }

        // 3. Player pickup
        let collectors: Vec<(SessionId, i64, usize, [f32; 3], i32)> = self
            .players
            .iter()
            .filter(|(_, p)| p.state == State::Spawned && !p.dead && p.gamemode != 6)
            .map(|(&sid, p)| (sid, p.entity_id, p.world, [p.pos[0], p.pos[1] - 1.62, p.pos[2]], p.gamemode))
            .collect();

        let mut picked_indices: Vec<usize> = Vec::new();
        for idx in 0..self.items.len() {
            if self.items[idx].pickup_delay > 0 {
                continue;
            }
            let item_world = self.items[idx].world;
            let item_pos = self.items[idx].pos;
            let item_eid = self.items[idx].entity_id;
            let item_stack = self.items[idx].item.clone();

            for &(sid, taker_eid, world, feet, gm) in &collectors {
                if world != item_world {
                    continue;
                }
                let dx = feet[0] - item_pos[0];
                let dy = (feet[1] + 0.9) - item_pos[1];
                let dz = feet[2] - item_pos[2];
                if dx * dx + dz * dz > 1.6 * 1.6 || dy.abs() > 1.5 {
                    continue;
                }
                let Some(p) = self.players.get_mut(&sid) else { continue };
                let (changed, rem) = p.inventory.add_item(item_stack.clone(), &self.gd);
                if changed.is_empty() && gm != 1 {
                    continue; // survival inventory full
                }
                for slot in &changed {
                    let s = *slot as usize;
                    p.out.push(protocol::inventory_slot(window::INVENTORY as u32, s as u32, &p.inventory.main[s]));
                }
                let held_changed = changed.contains(&p.inventory.held_slot);
                if rem == 0 || gm == 1 {
                    picked_indices.push(idx);
                    self.broadcast(&protocol::take_item_actor(item_eid, taker_eid), |_, q| q.world == world);
                    self.broadcast(&protocol::remove_actor(item_eid), |_, q| q.world == world);
                    self.broadcast(&protocol::level_event(level_event::SOUND_POP, item_pos, 0), |_, q| {
                        q.world == world
                    });
                } else {
                    self.items[idx].item.count = rem;
                }
                if held_changed {
                    self.broadcast_equipment(sid);
                }
                break;
            }
        }
        picked_indices.sort_unstable();
        for idx in picked_indices.into_iter().rev() {
            self.items.swap_remove(idx);
        }
    }
}
