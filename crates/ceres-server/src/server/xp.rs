//! Experience: levels and points (vanilla curve), orbs that fly to nearby players, XP from ores
//! and from dying, and /xp.

use super::*;

const ORB_PICKUP_DISTANCE: f32 = 1.5;
const ORB_ATTRACT_DISTANCE: f32 = 8.0;
const ORB_DESPAWN_TICKS: u32 = 6000;
const SOUND_ORB: i32 = 1051;
const EYE_HEIGHT: f32 = 1.62;

pub(crate) struct XpOrb {
    entity_id: i64,
    world: usize,
    pos: [f32; 3],
    value: u32,
    age: u32,
}

/// Points needed to go from `level` to the next one.
pub(crate) fn points_for_level(level: u32) -> u32 {
    match level {
        0..=15 => 2 * level + 7,
        16..=30 => 5 * level - 38,
        _ => 9 * level - 158,
    }
}

/// Progress bar (0..1) for `points` collected inside `level`.
pub(crate) fn progress(level: u32, points: u32) -> f32 {
    points as f32 / points_for_level(level) as f32
}

/// Total points a player at `level` with `points` has.
pub(crate) fn total_points(level: u32, points: u32) -> u32 {
    (0..level.min(24791)).map(points_for_level).sum::<u32>() + points
}

/// XP a broken block gives (ores, with a tool that harvests them), random within vanilla's range.
pub(crate) fn block_xp(name: &str) -> u32 {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let n = n.strip_prefix("deepslate_").unwrap_or(n);
    let (min, max) = match n {
        "coal_ore" => (0, 2),
        "diamond_ore" | "emerald_ore" => (3, 7),
        "lapis_ore" | "nether_quartz_ore" | "quartz_ore" => (2, 5),
        "redstone_ore" | "lit_redstone_ore" => (1, 5),
        "nether_gold_ore" => (0, 1),
        "sculk" => (1, 1),
        "sculk_sensor" | "sculk_catalyst" | "sculk_shrieker" => (5, 5),
        "spawner" | "mob_spawner" => (15, 43),
        _ => return 0,
    };
    min + rand::random::<u32>() % (max - min + 1)
}

/// Splits XP into orb sizes like vanilla does.
fn orb_sizes(mut amount: u32) -> Vec<u32> {
    const SIZES: [u32; 11] = [2477, 1237, 617, 307, 149, 73, 37, 17, 7, 3, 1];
    let mut out = Vec::new();
    while amount > 0 && out.len() < 64 {
        let size = SIZES.iter().copied().find(|s| *s <= amount).unwrap_or(1);
        out.push(size);
        amount -= size;
    }
    out
}

impl Server {
    pub(crate) fn spawn_xp(&mut self, world: usize, pos: [f32; 3], amount: u32) {
        for value in orb_sizes(amount) {
            let eid = self.next_entity_id;
            self.next_entity_id += 1;
            let motion = [(rand::random::<f32>() - 0.5) * 0.2, 0.2, (rand::random::<f32>() - 0.5) * 0.2];
            self.broadcast(&protocol::add_xp_orb(eid, pos, motion, value as i32), |_, q| q.world == world);
            self.xp_orbs.push(XpOrb { entity_id: eid, world, pos, value, age: 0 });
        }
    }

    /// Adds (or with a negative amount removes) experience points.
    pub(crate) fn add_xp(&mut self, sid: SessionId, amount: i64) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let old_level = p.xp_level;
        let total = (total_points(p.xp_level, p.xp_points) as i64 + amount).clamp(0, i32::MAX as i64) as u32;
        let (mut level, mut left) = (0u32, total);
        while level < 24791 && left >= points_for_level(level) {
            left -= points_for_level(level);
            level += 1;
        }
        p.xp_level = level;
        p.xp_points = left;
        let pk = p.vitals_packet();
        p.out.push(pk);
        if level > old_level && level % 5 == 0 {
            let pos = p.pos;
            let volume = 0x1000_0000 * (level.min(30) as i32 / 5);
            p.out.push(protocol::level_sound_event("levelup", pos, volume));
        }
    }

    pub(crate) fn add_xp_levels(&mut self, sid: SessionId, levels: i64) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        p.xp_level = (p.xp_level as i64 + levels).clamp(0, 24791) as u32;
        p.xp_points = p.xp_points.min(points_for_level(p.xp_level).saturating_sub(1));
        let pk = p.vitals_packet();
        p.out.push(pk);
    }

    /// Orbs drift towards the nearest player and are collected when they touch one.
    pub(crate) fn tick_xp_orbs(&mut self) {
        if self.xp_orbs.is_empty() {
            return;
        }
        let mut collected: Vec<(usize, SessionId)> = Vec::new();
        let mut expired = Vec::new();
        let mut moves: Vec<(usize, Vec<u8>)> = Vec::new();
        for (i, orb) in self.xp_orbs.iter_mut().enumerate() {
            orb.age += 1;
            if orb.age > ORB_DESPAWN_TICKS {
                expired.push(i);
                continue;
            }
            let nearest = self
                .players
                .iter()
                .filter(|(_, q)| q.world == orb.world && q.state == State::Spawned && !q.dead && q.gamemode != 6)
                .map(|(s, q)| {
                    let feet = [q.pos[0], q.pos[1] - EYE_HEIGHT + 0.5, q.pos[2]];
                    let d = [feet[0] - orb.pos[0], feet[1] - orb.pos[1], feet[2] - orb.pos[2]];
                    (*s, d, (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt())
                })
                .min_by(|a, b| a.2.total_cmp(&b.2));
            let Some((sid, d, dist)) = nearest else { continue };
            if orb.age > 10 && dist < ORB_PICKUP_DISTANCE {
                collected.push((i, sid));
            } else if dist < ORB_ATTRACT_DISTANCE && orb.age > 5 {
                let speed = 0.25f32.min(dist);
                for k in 0..3 {
                    orb.pos[k] += d[k] / dist.max(0.01) * speed;
                }
                if orb.age % 2 == 0 {
                    moves.push((orb.world, protocol::move_actor_absolute(orb.entity_id, orb.pos, false, false)));
                }
            }
        }
        for (world, pk) in moves {
            self.broadcast(&pk, |_, q| q.world == world);
        }
        let mut gone: Vec<(usize, Option<SessionId>)> =
            collected.into_iter().map(|(i, s)| (i, Some(s))).chain(expired.into_iter().map(|i| (i, None))).collect();
        gone.sort_by_key(|(i, _)| std::cmp::Reverse(*i));
        for (i, sid) in gone {
            let orb = self.xp_orbs.swap_remove(i);
            self.broadcast(&protocol::remove_actor(orb.entity_id), |_, q| q.world == orb.world);
            if let Some(sid) = sid {
                if let Some(p) = self.players.get_mut(&sid) {
                    let pos = p.pos;
                    p.out.push(protocol::level_event(SOUND_ORB, pos, 0));
                }
                self.add_xp(sid, orb.value as i64);
            }
        }
    }

    /// Sends the orbs of the player's world (joining or changing world).
    pub(crate) fn send_world_orbs(&mut self, sid: SessionId) {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return };
        let pks: Vec<Vec<u8>> = self
            .xp_orbs
            .iter()
            .filter(|o| o.world == world)
            .map(|o| protocol::add_xp_orb(o.entity_id, o.pos, [0.0; 3], o.value as i32))
            .collect();
        if let Some(p) = self.players.get_mut(&sid) {
            p.out.extend(pks);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_curve() {
        assert_eq!(points_for_level(0), 7);
        assert_eq!(points_for_level(16), 42);
        assert_eq!(points_for_level(31), 121);
        assert_eq!(total_points(30, 0), 1395);
        assert_eq!(orb_sizes(10), vec![7, 3]);
    }
}
