//! Boats and minecarts: the behaviour pack defines them (`minecraft:rideable`, `minecraft:buoyant`
//! ...) without health, so they load as entities with 4 health (as PowerNukkitX's boat). Placing
//! (logic of PowerNukkitX's ItemBoat): a boat item used on the top of a block or on water puts
//! a boat of its wood there, facing the player; a minecart item goes on a rail. Riding is the
//! rideable code (ride.rs). Broken, a vehicle drops its item, except to a creative player.

use super::*;

/// Boat variants (wood) by item, as the client draws them.
const WOODS: [&str; 10] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "bamboo", "cherry", "pale_oak"];

/// (entity, variant) of a vehicle item.
pub(crate) fn vehicle_of(item: &str) -> Option<(&'static str, Option<i32>)> {
    let n = item.strip_prefix("minecraft:")?;
    match n {
        "minecart" => return Some(("minecraft:minecart", None)),
        "chest_minecart" => return Some(("minecraft:chest_minecart", None)),
        "hopper_minecart" => return Some(("minecraft:hopper_minecart", None)),
        "tnt_minecart" => return Some(("minecraft:tnt_minecart", None)),
        "bamboo_raft" => return Some(("minecraft:boat", Some(7))),
        "bamboo_chest_raft" => return Some(("minecraft:chest_boat", Some(7))),
        _ => {}
    }
    let (wood, entity) = if let Some(w) = n.strip_suffix("_chest_boat") {
        (w, "minecraft:chest_boat")
    } else {
        (n.strip_suffix("_boat")?, "minecraft:boat")
    };
    let v = WOODS.iter().position(|w| *w == wood)?;
    Some((entity, Some(v as i32)))
}

/// The item a vehicle drops.
fn item_of(entity: &str, variant: Option<i32>) -> Option<String> {
    let wood = WOODS.get(variant.unwrap_or(0).max(0) as usize).copied().unwrap_or("oak");
    Some(match entity {
        "minecraft:boat" if wood == "bamboo" => "minecraft:bamboo_raft".into(),
        "minecraft:chest_boat" if wood == "bamboo" => "minecraft:bamboo_chest_raft".into(),
        "minecraft:boat" => format!("minecraft:{wood}_boat"),
        "minecraft:chest_boat" => format!("minecraft:{wood}_chest_boat"),
        "minecraft:minecart" | "minecraft:chest_minecart" | "minecraft:hopper_minecart" | "minecraft:tnt_minecart" => entity.to_string(),
        _ => return None,
    })
}

impl Server {
    /// A boat or minecart item used on a block. True when it was placed.
    pub(crate) fn place_vehicle(&mut self, sid: SessionId, clicked: [i32; 3], face: u8) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm, yaw) = (p.world, p.gamemode, p.rot[1]);
        let held = p.inventory.held_item().clone();
        let Some(name) = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()) else { return false };
        let Some((entity, variant)) = vehicle_of(&name) else { return false };
        let block = self.worlds.worlds[world].get_block(clicked, &self.gd);
        let bname = self.gd.block_name(block).to_string();
        let pos = if entity.ends_with("minecart") {
            if !bname.contains("rail") {
                return false;
            }
            [clicked[0] as f32 + 0.5, clicked[1] as f32, clicked[2] as f32 + 0.5]
        } else if bname.contains("water") {
            [clicked[0] as f32 + 0.5, clicked[1] as f32 + 0.625, clicked[2] as f32 + 0.5]
        } else if face == 1 {
            [clicked[0] as f32 + 0.5, clicked[1] as f32 + 1.0, clicked[2] as f32 + 0.5]
        } else {
            return false;
        };
        let Some(eid) = self.spawn_entity(world, entity, pos, None) else { return false };
        self.with_mob(eid, |s, m| {
            m.item_variant = variant;
            m.persistent = true;
            m.yaw = (yaw + 90.0).rem_euclid(360.0);
            m.head_yaw = m.yaw;
            s.sync_mob(m);
        });
        if gm != 1 {
            self.consume_held(sid, gm);
        }
        self.swing_arm(sid);
        true
    }

    /// A vehicle broken: its item, unless a creative player broke it.
    pub(crate) fn vehicle_drop(&mut self, m: &Mob, attacker: Option<Ent>) {
        if let Some(Ent::Player(s)) = attacker {
            if self.players.get(&s).is_some_and(|p| p.gamemode == 1) {
                return;
            }
        }
        let Some(name) = item_of(m.id(), m.item_variant) else { return };
        if let Some(it) = self.simple_item(&name, 1) {
            self.drop_at(m.world, [m.pos[0], m.pos[1] + 0.3, m.pos[2]], it);
        }
    }
}

/// Rail shapes by `rail_direction` (0-9): the two ends of the rail (x, y, z offsets), as
/// PowerNukkitX's minecart matrix (LGPL-3.0).
const RAIL_ENDS: [[[i32; 3]; 2]; 10] = [
    [[0, 0, -1], [0, 0, 1]],
    [[-1, 0, 0], [1, 0, 0]],
    [[-1, -1, 0], [1, 0, 0]],
    [[-1, 0, 0], [1, -1, 0]],
    [[0, 0, -1], [0, -1, 1]],
    [[0, -1, -1], [0, 0, 1]],
    [[0, 0, 1], [1, 0, 0]],
    [[0, 0, 1], [-1, 0, 0]],
    [[0, 0, -1], [-1, 0, 0]],
    [[0, 0, -1], [1, 0, 0]],
];
const MAX_SPEED: f32 = 0.4;

impl Server {
    /// (rail_direction, golden rail, powered) of a rail block.
    fn rail_at(&mut self, world: usize, pos: [i32; 3]) -> Option<(usize, bool, bool)> {
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        let name = self.gd.block_name(id);
        if !(name == "minecraft:rail" || name.ends_with("_rail")) {
            return None;
        }
        let golden = name == "minecraft:golden_rail";
        let st = self.gd.block_states.get(&id)?;
        let dir = st.states.get("rail_direction").and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 9) as usize;
        let on = st.states.get("rail_data_bit").and_then(|v| v.as_i64()).unwrap_or(0) != 0;
        Some((dir, golden, on))
    }

    fn solid_at(&mut self, world: usize, pos: [i32; 3]) -> bool {
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        self.gd.is_solid(id)
    }

    /// Where on the rail a cart at `p` is (for slopes), as PowerNukkitX's getNextRail.
    fn rail_point(&mut self, world: usize, p: [f32; 3]) -> Option<[f32; 3]> {
        let (cx, mut cy, cz) = (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
        if self.rail_at(world, [cx, cy - 1, cz]).is_some() {
            cy -= 1;
        }
        let (dir, _, _) = self.rail_at(world, [cx, cy, cz])?;
        let f = RAIL_ENDS[dir];
        let a = [cx as f32 + 0.5 + f[0][0] as f32 * 0.5, cy as f32 + 0.5 + f[0][1] as f32 * 0.5, cz as f32 + 0.5 + f[0][2] as f32 * 0.5];
        let b = [cx as f32 + 0.5 + f[1][0] as f32 * 0.5, cy as f32 + 0.5 + f[1][1] as f32 * 0.5, cz as f32 + 0.5 + f[1][2] as f32 * 0.5];
        let (sx, sy, sz) = (b[0] - a[0], (b[1] - a[1]) * 2.0, b[2] - a[2]);
        let t = if sx == 0.0 {
            p[2] - cz as f32
        } else if sz == 0.0 {
            p[0] - cx as f32
        } else {
            ((p[0] - a[0]) * sx + (p[2] - a[2]) * sz) * 2.0
        };
        let mut y = a[1] + sy * t;
        if sy < 0.0 {
            y += 1.0;
        }
        if sy > 0.0 {
            y += 0.5;
        }
        Some([a[0] + sx * t, y, a[2] + sz * t])
    }

    /// A cart tick (`minecraft:rail_movement`), logic of PowerNukkitX's EntityMinecartAbstract
    /// (LGPL-3.0): along the rail it follows, slopes pull it down, a powered golden rail speeds
    /// it up and an unpowered one brakes it, the rider's forward input starts it, drag slows it
    /// (more when empty); off rails it falls and slides to a stop. Players walking into it push
    /// it. The server moves it (its riders see it move too).
    pub(crate) fn rail_tick(&mut self, m: &mut Mob) {
        let world = m.world;
        let near: Vec<[f32; 3]> = self
            .players
            .values()
            .filter(|p| p.world == world && !p.dead && p.gamemode != 6 && p.riding != Some(m.entity_id))
            .map(|p| [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]])
            .filter(|f| (f[1] - m.pos[1]).abs() < 1.0 && (f[0] - m.pos[0]).powi(2) + (f[2] - m.pos[2]).powi(2) < 0.64)
            .collect();
        for f in near {
            let (dx, dz) = (m.pos[0] - f[0], m.pos[2] - f[2]);
            let d = (dx * dx + dz * dz).sqrt().max(0.01);
            m.motion[0] += dx / d * 0.05;
            m.motion[2] += dz / d * 0.05;
        }
        m.motion[1] -= 0.04;
        let (bx, mut by, bz) = (m.pos[0].floor() as i32, m.pos[1].floor() as i32, m.pos[2].floor() as i32);
        if self.rail_at(world, [bx, by - 1, bz]).is_some() {
            by -= 1;
        }
        let Some((dir, golden, powered)) = self.rail_at(world, [bx, by, bz]) else {
            // off rails: falls, and slides to a stop on the ground
            let below = [m.pos[0].floor() as i32, (m.pos[1] - 0.05).floor() as i32, m.pos[2].floor() as i32];
            if self.solid_at(world, below) && m.motion[1] <= 0.0 {
                m.motion[1] = 0.0;
                m.pos[1] = below[1] as f32 + 1.0;
                m.motion[0] *= 0.5;
                m.motion[2] *= 0.5;
            }
            let next = [m.pos[0] + m.motion[0], m.pos[1], m.pos[2] + m.motion[2]];
            if self.solid_at(world, [next[0].floor() as i32, next[1].floor() as i32, next[2].floor() as i32]) {
                m.motion[0] = 0.0;
                m.motion[2] = 0.0;
            }
            for i in 0..3 {
                m.pos[i] += m.motion[i];
            }
            m.on_ground = m.motion[1] == 0.0;
            return;
        };
        m.fall = 0.0;
        let before = self.rail_point(world, m.pos);
        m.pos[1] = by as f32;
        let mut slowed = golden && !powered;
        match dir {
            2 => m.motion[0] -= 0.0078125,
            3 => m.motion[0] += 0.0078125,
            4 => m.motion[2] += 0.0078125,
            5 => m.motion[2] -= 0.0078125,
            _ => {}
        }
        if (2..=5).contains(&dir) {
            m.pos[1] += 1.0;
        }
        let f = RAIL_ENDS[dir];
        let (mut fx, mut fz) = ((f[1][0] - f[0][0]) as f32, (f[1][2] - f[0][2]) as f32);
        let turn = (fx * fx + fz * fz).sqrt();
        if m.motion[0] * fx + m.motion[2] * fz < 0.0 {
            fx = -fx;
            fz = -fz;
        }
        let speed = (m.motion[0].powi(2) + m.motion[2].powi(2)).sqrt().min(2.0);
        m.motion[0] = speed * fx / turn;
        m.motion[2] = speed * fz / turn;
        // the rider pressing forward starts a cart that stands still
        if let Some(p) = m.riders.first().and_then(|s| self.players.get(s)) {
            if p.move_forward > 0.0 && m.motion[0].powi(2) + m.motion[2].powi(2) < 0.01 {
                let yaw = p.rot[1].to_radians();
                m.motion[0] += -yaw.sin() * 0.1;
                m.motion[2] += yaw.cos() * 0.1;
                slowed = false;
            }
        }
        if slowed {
            let s = (m.motion[0].powi(2) + m.motion[2].powi(2)).sqrt();
            let k = if s < 0.03 { 0.0 } else { 0.5 };
            m.motion[0] *= k;
            m.motion[2] *= k;
            m.motion[1] = 0.0;
        }
        // keep it on the rail line
        let a = [bx as f32 + 0.5 + f[0][0] as f32 * 0.5, bz as f32 + 0.5 + f[0][2] as f32 * 0.5];
        let b = [bx as f32 + 0.5 + f[1][0] as f32 * 0.5, bz as f32 + 0.5 + f[1][2] as f32 * 0.5];
        let (lx, lz) = (b[0] - a[0], b[1] - a[1]);
        let t = if lx == 0.0 {
            m.pos[0] = bx as f32 + 0.5;
            m.pos[2] - bz as f32
        } else if lz == 0.0 {
            m.pos[2] = bz as f32 + 0.5;
            m.pos[0] - bx as f32
        } else {
            ((m.pos[0] - a[0]) * lx + (m.pos[2] - a[1]) * lz) * 2.0
        };
        m.pos[0] = a[0] + lx * t;
        m.pos[2] = a[1] + lz * t;
        let k = if m.riders.is_empty() { 1.0 } else { 0.75 };
        let mx = (m.motion[0] * k).clamp(-MAX_SPEED, MAX_SPEED);
        let mz = (m.motion[2] * k).clamp(-MAX_SPEED, MAX_SPEED);
        let next = [m.pos[0] + mx, m.pos[1], m.pos[2] + mz];
        let cell = [next[0].floor() as i32, next[1].floor() as i32, next[2].floor() as i32];
        if self.solid_at(world, cell) && self.rail_at(world, cell).is_none() {
            m.motion[0] = 0.0;
            m.motion[2] = 0.0;
        } else {
            m.pos[0] = next[0];
            m.pos[2] = next[2];
        }
        let (nx, nz) = (m.pos[0].floor() as i32 - bx, m.pos[2].floor() as i32 - bz);
        if f[0][1] != 0 && nx == f[0][0] && nz == f[0][2] {
            m.pos[1] += f[0][1] as f32;
        } else if f[1][1] != 0 && nx == f[1][0] && nz == f[1][2] {
            m.pos[1] += f[1][1] as f32;
        }
        let drag = if m.riders.is_empty() { 0.96 } else { 0.997 };
        m.motion[0] *= drag;
        m.motion[2] *= drag;
        m.motion[1] = 0.0;
        if let (Some(v0), Some(v1)) = (before, self.rail_point(world, m.pos)) {
            let d = (v0[1] - v1[1]) * 0.05;
            let s = (m.motion[0].powi(2) + m.motion[2].powi(2)).sqrt();
            if s > 0.0 {
                m.motion[0] = m.motion[0] / s * (s + d);
                m.motion[2] = m.motion[2] / s * (s + d);
            }
            m.pos[1] = v1[1];
        }
        let (fx2, fz2) = (m.pos[0].floor() as i32, m.pos[2].floor() as i32);
        if fx2 != bx || fz2 != bz {
            let s = (m.motion[0].powi(2) + m.motion[2].powi(2)).sqrt();
            m.motion[0] = s * (fx2 - bx) as f32;
            m.motion[2] = s * (fz2 - bz) as f32;
        }
        if golden && powered {
            let s = (m.motion[0].powi(2) + m.motion[2].powi(2)).sqrt();
            if s > 0.01 {
                m.motion[0] += m.motion[0] / s * 0.06;
                m.motion[2] += m.motion[2] / s * 0.06;
            } else if dir == 0 {
                if self.solid_at(world, [bx - 1, by, bz]) {
                    m.motion[0] = 0.02;
                } else if self.solid_at(world, [bx + 1, by, bz]) {
                    m.motion[0] = -0.02;
                }
            } else if dir == 1 {
                if self.solid_at(world, [bx, by, bz - 1]) {
                    m.motion[2] = 0.02;
                } else if self.solid_at(world, [bx, by, bz + 1]) {
                    m.motion[2] = -0.02;
                }
            }
        }
        m.on_ground = true;
        if mx * mx + mz * mz > 0.001 {
            m.yaw = mz.atan2(mx).to_degrees();
            m.head_yaw = m.yaw;
        }
    }
}
