//! Monster spawners (`minecraft:mob_spawner`, block entity `MobSpawner`): a spawn egg used on one
//! sets its mob (the client shows it spinning inside); with a player within 16 blocks it makes up
//! to 4 of that mob within 4 blocks every 10 to 40 seconds, unless 6 are already around.

use super::*;
use ceres_codec::nbt::Tag;

fn spawner_nbt(pos: [i32; 3], entity: &str) -> Tag {
    Tag::Compound(vec![
        ("id".into(), Tag::String("MobSpawner".into())),
        ("x".into(), Tag::Int(pos[0])),
        ("y".into(), Tag::Int(pos[1])),
        ("z".into(), Tag::Int(pos[2])),
        ("isMovable".into(), Tag::Byte(1)),
        ("EntityIdentifier".into(), Tag::String(entity.into())),
        ("Delay".into(), Tag::Short(20)),
        ("MinSpawnDelay".into(), Tag::Short(200)),
        ("MaxSpawnDelay".into(), Tag::Short(800)),
        ("SpawnCount".into(), Tag::Short(4)),
        ("MaxNearbyEntities".into(), Tag::Short(6)),
        ("RequiredPlayerRange".into(), Tag::Short(16)),
        ("SpawnRange".into(), Tag::Short(4)),
        ("DisplayEntityWidth".into(), Tag::Float(0.8)),
        ("DisplayEntityHeight".into(), Tag::Float(0.8)),
        ("DisplayEntityScale".into(), Tag::Float(1.0)),
    ])
}

fn short(t: &Tag, key: &str, default: i64) -> i64 {
    t.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}

impl Server {
    /// A spawn egg used on a spawner sets what it spawns. True when it was one.
    pub(crate) fn use_egg_on_spawner(&mut self, sid: SessionId, clicked: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm) = (p.world, p.gamemode);
        let held_name = self.gd.items_by_id.get(&p.inventory.held_item().id).map(|i| i.name.clone()).unwrap_or_default();
        let Some(entity) = useitems::egg_entity(&held_name) else { return false };
        let block = self.worlds.worlds[world].get_block(clicked, &self.gd);
        if self.gd.block_name(block) != "minecraft:mob_spawner" {
            return false;
        }
        let mut tag = self.worlds.worlds[world].get_block_entity(clicked, &self.gd).unwrap_or_else(|| spawner_nbt(clicked, &entity));
        tag.set("EntityIdentifier", Tag::String(entity));
        tag.set("Delay", Tag::Short(20));
        self.worlds.worlds[world].set_block_entity(clicked, tag.clone(), &self.gd);
        self.broadcast(&protocol::block_actor_data(clicked, &tag), |_, q| q.world == world);
        self.swing_arm(sid);
        if gm != 1 {
            self.consume_held(sid, gm);
        }
        true
    }

    pub(crate) fn tick_spawners(&mut self) {
        if self.ticks % 20 != 7 {
            return;
        }
        let worlds: Vec<usize> = {
            let mut w: Vec<usize> = self.players.values().filter(|p| p.state == State::Spawned).map(|p| p.world).collect();
            w.sort();
            w.dedup();
            w
        };
        for world in worlds {
            let spawners = self.worlds.worlds[world].block_entities_with_id("MobSpawner");
            for pos in spawners {
                let Some(mut tag) = self.worlds.worlds[world].get_block_entity(pos, &self.gd) else { continue };
                let Some(entity) = tag.get("EntityIdentifier").and_then(|t| t.as_str()).map(String::from).filter(|e| !e.is_empty()) else { continue };
                let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
                let range = short(&tag, "RequiredPlayerRange", 16) as f32;
                if self.nearest_player_dist(world, center) > range {
                    continue;
                }
                let delay = short(&tag, "Delay", 20) - 20;
                if delay > 0 {
                    tag.set("Delay", Tag::Short(delay as i16));
                    self.worlds.worlds[world].set_block_entity(pos, tag, &self.gd);
                    continue;
                }
                let (lo, hi) = (short(&tag, "MinSpawnDelay", 200), short(&tag, "MaxSpawnDelay", 800));
                let next = lo + (rand::random::<u32>() as i64 % (hi - lo + 1).max(1));
                tag.set("Delay", Tag::Short(next as i16));
                self.worlds.worlds[world].set_block_entity(pos, tag.clone(), &self.gd);
                let max_near = short(&tag, "MaxNearbyEntities", 6) as usize;
                let near = self.mobs.iter().filter(|m| m.alive() && m.world == world && m.def.id == entity && mobs::filters_dist(m.pos, center) < 9.0).count();
                if near >= max_near {
                    continue;
                }
                let r = short(&tag, "SpawnRange", 4) as i32;
                let count = short(&tag, "SpawnCount", 4).min((max_near - near) as i64);
                let mut made = 0;
                for _ in 0..count * 3 {
                    if made >= count {
                        break;
                    }
                    let b = [pos[0] + (rand::random::<u32>() % (2 * r as u32 + 1)) as i32 - r, pos[1] + (rand::random::<u32>() % 3) as i32 - 1, pos[2] + (rand::random::<u32>() % (2 * r as u32 + 1)) as i32 - r];
                    let free = |s: &mut Server, p: [i32; 3]| !s.gd.is_solid(s.worlds.worlds[world].get_block(p, &s.gd));
                    let floor = self.gd.is_solid(self.worlds.worlds[world].get_block([b[0], b[1] - 1, b[2]], &self.gd));
                    if !floor || !free(self, b) || !free(self, [b[0], b[1] + 1, b[2]]) {
                        continue;
                    }
                    if self.spawn_entity(world, &entity, [b[0] as f32 + 0.5, b[1] as f32, b[2] as f32 + 0.5], None).is_some() {
                        made += 1;
                    }
                }
                if made > 0 {
                    self.broadcast(&protocol::level_event(2004, center, 0), |_, q| q.world == world); // spawn smoke
                }
            }
        }
    }
}
