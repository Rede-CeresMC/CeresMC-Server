//! The body: what is around a mob, line of sight, movement with block collision (walking with
//! steps and jumps, climbing, swimming, flying, falling) and A* paths on the block grid.

use super::traits::NavKind;
use super::*;
use std::collections::BinaryHeap;

pub(crate) const SOLID: u8 = 1;
pub(crate) const WATER: u8 = 2;
pub(crate) const LAVA: u8 = 4;
pub(crate) const DAMAGE: u8 = 8;
/// fences and walls: 1.5 blocks tall
pub(crate) const TALL: u8 = 16;
pub(crate) const DOOR: u8 = 32;
pub(crate) const CLIMB: u8 = 64;
/// blocks that light things up (for the spawning light estimate)
pub(crate) const LIGHT: u8 = 128;

#[derive(PartialEq)]
struct Node {
    f: f32,
    i: usize,
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.f.total_cmp(&self.f)
    }
}

impl Server {
    /// Kind of block (cached per runtime id).
    pub(crate) fn block_class(&mut self, id: i32) -> u8 {
        if let Some(c) = self.mobsys.block_classes.get(&id) {
            return *c;
        }
        let name = self.gd.block_name(id).to_string();
        let n = name.strip_prefix("minecraft:").unwrap_or(&name);
        let mut c = 0;
        if self.gd.is_solid(id) {
            c |= SOLID;
        }
        if n == "water" || n == "flowing_water" || n == "bubble_column" {
            c |= WATER;
            c &= !SOLID;
        }
        if n == "lava" || n == "flowing_lava" {
            c |= LAVA | DAMAGE | LIGHT;
            c &= !SOLID;
        }
        if matches!(n, "fire" | "soul_fire" | "magma" | "cactus" | "sweet_berry_bush" | "campfire" | "soul_campfire" | "wither_rose" | "powder_snow" | "pointed_dripstone") {
            c |= DAMAGE;
        }
        if (n.ends_with("fence") || n.ends_with("_wall") || n.ends_with("fence_gate") || n == "cobblestone_wall") && !n.contains("sign") && !n.contains("banner") && !n.contains("torch") {
            c |= TALL | SOLID;
        }
        if n.ends_with("_door") || n == "wooden_door" || n == "iron_door" {
            c |= DOOR;
        }
        if matches!(n, "ladder" | "vine" | "scaffolding" | "twisting_vines" | "weeping_vines" | "cave_vines") {
            c |= CLIMB;
        }
        if matches!(n, "torch" | "soul_torch" | "lantern" | "soul_lantern" | "glowstone" | "sea_lantern" | "jack_o_lantern" | "lit_pumpkin" | "shroomlight"
            | "beacon" | "end_rod" | "lit_furnace" | "lit_blast_furnace" | "lit_smoker" | "campfire" | "soul_campfire" | "fire" | "soul_fire"
            | "redstone_lamp" | "lit_redstone_lamp" | "ochre_froglight" | "verdant_froglight" | "pearlescent_froglight" | "conduit" | "copper_torch"
            | "redstone_torch" | "magma" | "crying_obsidian" | "respawn_anchor" | "glow_lichen" | "sea_pickle" | "copper_lantern")
            || n.starts_with("light_block")
        {
            c |= LIGHT;
        }
        self.mobsys.block_classes.insert(id, c);
        c
    }

    pub(crate) fn class_at(&mut self, world: usize, b: [i32; 3]) -> u8 {
        if !self.worlds.worlds[world].is_loaded(b) {
            return SOLID; // unloaded chunks are walls
        }
        let id = self.worlds.worlds[world].get_block(b, &self.gd);
        self.block_class(id)
    }

    fn fpos(p: [f32; 3]) -> [i32; 3] {
        [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32]
    }

    /// What surrounds an entity standing at `pos`.
    pub(crate) fn env_at(&mut self, world: usize, pos: [f32; 3], height: f32) -> Env {
        let feet = Self::fpos(pos);
        if !self.worlds.worlds[world].is_loaded(feet) {
            return Env { sky: true, light: 15, biome: 1, ..Default::default() };
        }
        let at = self.worlds.worlds[world].get_block(feet, &self.gd);
        let below = self.worlds.worlds[world].get_block([feet[0], feet[1] - 1, feet[2]], &self.gd);
        let c_at = self.block_class(at);
        let up = Self::fpos([pos[0], pos[1] + 0.3, pos[2]]);
        let c_up = self.class_at(world, up);
        let eye = Self::fpos([pos[0], pos[1] + height * 0.85, pos[2]]);
        let c_eye = self.class_at(world, eye);
        let top = (pos[1] + height).ceil() as i32;
        let mut sky = true;
        for y in top..(top + 48).min(ceres_world::MAX_Y + 1) {
            let c = self.class_at(world, [feet[0], y, feet[2]]);
            if c & (SOLID | WATER) != 0 {
                sky = false;
                break;
            }
        }
        let day = self.is_day(world);
        let light = if sky {
            if day {
                if self.raining(world) { 12 } else { 15 }
            } else {
                4
            }
        } else {
            0
        };
        let biome = self.worlds.worlds[world].biome_at(feet, &self.gd);
        Env {
            in_water: (c_at | c_up) & WATER != 0,
            in_lava: (c_at | c_up) & LAVA != 0,
            underwater: c_eye & WATER != 0,
            sky,
            underground: !sky && feet[1] < 62,
            light,
            biome,
            at,
            below,
        }
    }

    /// Light level estimate at a block: sky light by time of day plus light-giving blocks nearby.
    pub(crate) fn light_at(&mut self, world: usize, b: [i32; 3]) -> u8 {
        let mut best = 0u8;
        let mut sky = true;
        for y in (b[1] + 1)..(b[1] + 48).min(ceres_world::MAX_Y + 1) {
            if self.class_at(world, [b[0], y, b[2]]) & (SOLID | WATER) != 0 {
                sky = false;
                break;
            }
        }
        if sky {
            best = if self.is_day(world) { if self.raining(world) { 12 } else { 15 } } else { 4 };
        }
        // light blocks: 15 at the source, minus 1 per block of distance (no walls considered)
        for dx in -7i32..=7 {
            for dz in -7i32..=7 {
                for dy in -4i32..=4 {
                    let d = dx.abs() + dz.abs() + dy.abs();
                    if d > 14 || 15 - d as u8 <= best {
                        continue;
                    }
                    let p = [b[0] + dx, b[1] + dy, b[2] + dz];
                    if self.class_at(world, p) & LIGHT != 0 {
                        best = best.max(15 - d as u8);
                    }
                }
            }
        }
        best
    }

    /// No solid block between two points.
    pub(crate) fn can_see(&mut self, world: usize, from: [f32; 3], to: [f32; 3]) -> bool {
        let d = super::filters::dist(from, to);
        let steps = (d * 2.0).ceil().min(160.0) as i32;
        for s in 1..steps {
            let t = s as f32 / steps as f32;
            let p = [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t, from[2] + (to[2] - from[2]) * t];
            if self.class_at(world, Self::fpos(p)) & SOLID != 0 {
                return false;
            }
        }
        true
    }

    /// Whether a box of width `w`, height `h` with its feet at `p` touches a solid block.
    pub(crate) fn box_blocked(&mut self, world: usize, p: [f32; 3], w: f32, h: f32) -> bool {
        let r = (w / 2.0 - 0.001).max(0.05);
        let (x0, x1) = ((p[0] - r).floor() as i32, (p[0] + r).floor() as i32);
        let (z0, z1) = ((p[2] - r).floor() as i32, (p[2] + r).floor() as i32);
        let (y0, y1) = ((p[1] + 0.001).floor() as i32, (p[1] + h - 0.001).floor() as i32);
        for x in x0..=x1 {
            for z in z0..=z1 {
                // fences reach half a block higher
                for y in (y0 - 1)..=y1 {
                    let c = self.class_at(world, [x, y, z]);
                    if y == y0 - 1 {
                        if c & TALL != 0 && p[1] < y as f32 + 1.5 {
                            return true;
                        }
                        continue;
                    }
                    if c & SOLID != 0 {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn standable(&mut self, world: usize, b: [i32; 3], h: i32, swims: bool, lava_ok: bool) -> bool {
        for y in 0..h.max(1) {
            let c = self.class_at(world, [b[0], b[1] + y, b[2]]);
            if c & SOLID != 0 && c & DOOR == 0 {
                return false;
            }
            if c & DAMAGE != 0 && !(lava_ok && c & LAVA != 0) {
                return false;
            }
        }
        let feet = self.class_at(world, b);
        if feet & WATER != 0 {
            return true; // swimming through
        }
        let below = self.class_at(world, [b[0], b[1] - 1, b[2]]);
        if below & DAMAGE != 0 && !(lava_ok && below & LAVA != 0) {
            return false;
        }
        below & SOLID != 0 || (swims && below & WATER != 0) || (lava_ok && below & LAVA != 0) || feet & CLIMB != 0
    }

    /// A* from `start` to `goal`; the path (without `start`) or the best part of it.
    pub(crate) fn find_path(&mut self, m: &Mob, start: [i32; 3], goal: [i32; 3], max_nodes: usize) -> Option<Vec<[i32; 3]>> {
        let world = m.world;
        let h = m.t.height.ceil() as i32;
        let swims = m.t.can_swim || !m.t.avoid_water;
        let lava_ok = m.t.walks_on_lava || m.t.fire_immune;
        let avoid_water = m.t.avoid_water;
        let mut nodes: Vec<([i32; 3], f32, usize)> = vec![(start, 0.0, usize::MAX)];
        let mut seen: HashMap<[i32; 3], usize> = HashMap::new();
        seen.insert(start, 0);
        let mut open = BinaryHeap::new();
        let hd = |p: [i32; 3]| (((p[0] - goal[0]).pow(2) + (p[1] - goal[1]).pow(2) + (p[2] - goal[2]).pow(2)) as f32).sqrt();
        open.push(Node { f: hd(start), i: 0 });
        let mut best = (hd(start), 0usize);
        let mut closed: HashSet<usize> = HashSet::new();
        while let Some(Node { i, .. }) = open.pop() {
            if !closed.insert(i) {
                continue;
            }
            let (p, g, _) = nodes[i];
            let hp = hd(p);
            if hp < best.0 {
                best = (hp, i);
            }
            if p == goal || (hp < 1.5 && (p[1] - goal[1]).abs() <= 1) {
                best = (0.0, i);
                break;
            }
            if nodes.len() >= max_nodes {
                break;
            }
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                if dx != 0 && dz != 0 {
                    // no cutting corners
                    let a = self.class_at(world, [p[0] + dx, p[1], p[2]]) | self.class_at(world, [p[0] + dx, p[1] + 1, p[2]]);
                    let b = self.class_at(world, [p[0], p[1], p[2] + dz]) | self.class_at(world, [p[0], p[1] + 1, p[2] + dz]);
                    if (a | b) & SOLID != 0 {
                        continue;
                    }
                }
                let mut found = None;
                for dy in [0, 1, -1, -2, -3] {
                    let n = [p[0] + dx, p[1] + dy, p[2] + dz];
                    if dy == 1 && self.class_at(world, [p[0], p[1] + h, p[2]]) & SOLID != 0 {
                        continue; // no room to jump
                    }
                    if dy < 0 {
                        // falling: the column above the landing spot must be open
                        let mut open_col = true;
                        for y in (dy + 1)..=0 {
                            if self.class_at(world, [n[0], p[1] + y, n[2]]) & SOLID != 0 {
                                open_col = false;
                            }
                        }
                        if !open_col {
                            break;
                        }
                    }
                    if self.standable(world, n, h, swims, lava_ok) {
                        found = Some(n);
                        break;
                    }
                    if dy == 0 && self.class_at(world, n) & SOLID != 0 && self.class_at(world, [n[0], n[1] + 1, n[2]]) & SOLID != 0 {
                        break; // a wall: only jumping (handled above) would help
                    }
                }
                let Some(n) = found else { continue };
                let mut cost = if dx != 0 && dz != 0 { 1.414 } else { 1.0 };
                let c = self.class_at(world, n);
                if c & WATER != 0 {
                    cost += if avoid_water { 8.0 } else { 2.0 };
                }
                if n[1] > p[1] {
                    cost += 0.5;
                }
                let ng = g + cost;
                match seen.get(&n) {
                    Some(&j) if nodes[j].1 <= ng => continue,
                    _ => {}
                }
                let j = nodes.len();
                nodes.push((n, ng, i));
                seen.insert(n, j);
                open.push(Node { f: ng + hd(n) * 1.1, i: j });
            }
        }
        if best.1 == 0 {
            return None;
        }
        let mut out = Vec::new();
        let mut i = best.1;
        while i != 0 && i != usize::MAX {
            out.push(nodes[i].0);
            i = nodes[i].2;
        }
        out.reverse();
        Some(out)
    }

    /// Moves the mob for one tick from what its goals asked for.
    pub(crate) fn mob_physics(&mut self, m: &mut Mob) {
        let t = m.t.clone();
        let world = m.world;
        let in_liquid = m.env.in_water || m.env.in_lava;
        let swimming = m.env.in_water && t.nav == NavKind::Swim;
        let flying = t.can_fly && !t.gravity && !m.sitting;
        if m.sitting || m.ctl.hold || m.sleeping {
            m.ctl.dest = None;
        }
        // where to head for this tick
        let mut want = [0.0f32; 3];
        if let Some(dest) = m.ctl.dest {
            let way = if !flying && !swimming && t.gravity { self.next_waypoint(m, dest) } else { dest };
            let d = [way[0] - m.pos[0], way[1] - m.pos[1], way[2] - m.pos[2]];
            let hd = (d[0] * d[0] + d[2] * d[2]).sqrt();
            let full = (hd * hd + d[1] * d[1]).sqrt();
            let mult = if m.ctl.speed > 0.0 { m.ctl.speed } else { 1.0 };
            let mut speed = if flying {
                t.flying_speed.max(t.speed * 0.43) * mult
            } else if swimming {
                t.underwater_speed * mult * if m.def.id.contains("fish") || m.def.id.contains("cod") || m.def.id.contains("salmon") { 1.0 } else { 0.8 }
            } else {
                let base = t.speed * mult * 0.43;
                if in_liquid { base * 0.5 } else { base }
            };
            if flying || swimming {
                if full > 0.05 {
                    speed = speed.min(full);
                    want = [d[0] / full * speed, d[1] / full * speed, d[2] / full * speed];
                }
            } else if hd > 0.05 {
                speed = speed.min(hd.max(0.05));
                want = [d[0] / hd * speed, 0.0, d[2] / hd * speed];
            }
            if hd > 0.1 {
                let target_yaw = d[2].atan2(d[0]).to_degrees() - 90.0;
                m.yaw = turn(m.yaw, target_yaw, 30.0);
            }
        } else {
            m.path.clear();
        }
        m.walk_speed = (want[0] * want[0] + want[2] * want[2]).sqrt();
        // head
        if let Some(l) = m.ctl.look {
            let e = m.eye();
            let (dx, dy, dz) = (l[0] - e[0], l[1] - e[1], l[2] - e[2]);
            let target = dz.atan2(dx).to_degrees() - 90.0;
            m.head_yaw = turn(m.head_yaw, target, 40.0);
            m.pitch = -(dy.atan2((dx * dx + dz * dz).sqrt())).to_degrees();
            if m.walk_speed < 0.001 {
                m.yaw = turn(m.yaw, m.head_yaw, 10.0);
            }
        } else {
            m.head_yaw = turn(m.head_yaw, m.yaw, 20.0);
            m.pitch *= 0.8;
        }
        // vertical forces
        let mut mot = m.motion;
        if flying {
            mot[1] = mot[1] * 0.6 + want[1];
            want[1] = 0.0;
            if m.ctl.dest.is_none() && m.env.in_water {
                mot[1] += 0.02;
            }
        } else if swimming {
            mot[1] = mot[1] * 0.6 + want[1];
            want[1] = 0.0;
        } else if in_liquid {
            let floats = m.goals.iter().any(|g| g.def.kind == traits::G::Float) || t.is("player");
            if (m.ctl.jump || floats) && !t.walks_on_lava {
                mot[1] = (mot[1] + 0.05).min(0.12);
            } else if t.walks_on_lava && m.env.in_lava {
                mot[1] = (mot[1] + 0.06).min(0.1);
            } else if t.gravity {
                mot[1] = (mot[1] - 0.02) * 0.8;
            }
            m.fall = 0.0;
        } else if t.gravity {
            mot[1] = (mot[1] - 0.08) * 0.98;
            if m.def.id == "minecraft:chicken" {
                mot[1] = mot[1].max(-0.08);
            }
            mot[1] = mot[1].max(-3.0);
        } else {
            mot[1] *= 0.6;
        }
        if m.ctl.jump && m.on_ground && !in_liquid {
            mot[1] = 0.42;
        }
        // fish on land flop around
        if t.nav == NavKind::Swim && !m.env.in_water && t.gravity && m.on_ground && rand::random::<u32>() % 12 == 0 {
            mot = [(rand::random::<f32>() - 0.5) * 0.2, 0.35, (rand::random::<f32>() - 0.5) * 0.2];
        }
        // horizontal: steering + knockback, wall by wall
        let mut pos = m.pos;
        let mut blocked_h = false;
        let (w, h) = (t.width.max(0.2), t.height.max(0.2));
        if t.gravity && !flying && !swimming && m.on_ground && m.walk_speed > 0.0 && !in_liquid {
            // do not walk off a cliff of more than 3 blocks without a path telling us to
            let ahead = [pos[0] + want[0] * 4.0, pos[1], pos[2] + want[2] * 4.0];
            let b = Self::fpos(ahead);
            let mut ground = false;
            for dy in 0..=3 {
                if self.class_at(world, [b[0], b[1] - dy, b[2]]) & (SOLID | WATER) != 0 {
                    ground = true;
                    break;
                }
            }
            if !ground && m.path.is_empty() && !m.panicking {
                want = [0.0; 3];
                m.walk_speed = 0.0;
            }
        }
        for axis in [0usize, 2] {
            let delta = want[axis] + mot[axis];
            if delta.abs() < 1e-4 {
                continue;
            }
            let mut moved = pos;
            moved[axis] += delta;
            if !t.collision || !self.box_blocked(world, moved, w, h) {
                pos = moved;
            } else {
                blocked_h = true;
                mot[axis] = 0.0;
            }
        }
        if blocked_h && m.walk_speed > 0.0 {
            if t.nav == NavKind::Climb {
                mot[1] = 0.2;
                m.climbing = true;
            } else if (m.on_ground || in_liquid) && !flying {
                let up = [pos[0] + want[0] * 3.0, pos[1] + 1.05, pos[2] + want[2] * 3.0];
                if !self.box_blocked(world, up, w, h) {
                    mot[1] = if in_liquid { 0.3 } else { 0.42 };
                }
            } else if flying {
                mot[1] = 0.1;
            }
        } else {
            m.climbing = false;
        }
        // vertical
        let mut on_ground = false;
        let ny = pos[1] + mot[1];
        if mot[1] < 0.0 && t.collision && self.box_blocked(world, [pos[0], ny, pos[2]], w, h) {
            pos[1] = (ny.floor() + 1.0).min(pos[1]).max(ny.floor());
            if self.box_blocked(world, pos, w, h) {
                pos[1] = m.pos[1];
            }
            if !in_liquid && m.fall > 3.0 && t.gravity {
                let dmg = (m.fall - 3.0).ceil();
                m.motion = [0.0; 3];
                self.hurt(m, dmg, "fall", None, None);
            }
            m.fall = 0.0;
            mot[1] = 0.0;
            on_ground = true;
        } else if mot[1] > 0.0 && t.collision && self.box_blocked(world, [pos[0], ny, pos[2]], w, h) {
            mot[1] = 0.0;
        } else {
            if mot[1] < 0.0 && !in_liquid {
                m.fall += -mot[1];
            }
            pos[1] = ny;
        }
        let drag = if on_ground { 0.6 } else if in_liquid { 0.8 } else { 0.91 };
        mot[0] *= drag;
        mot[2] *= drag;
        if mot[0].abs() < 0.003 {
            mot[0] = 0.0;
        }
        if mot[2].abs() < 0.003 {
            mot[2] = 0.0;
        }
        // stuck detection
        let moved = ((pos[0] - m.pos[0]).powi(2) + (pos[2] - m.pos[2]).powi(2)).sqrt();
        if m.walk_speed > 0.01 && moved < m.walk_speed * 0.2 {
            m.stuck += 1;
            if m.stuck > 30 {
                m.path.clear();
                m.path_dest = None;
            }
        } else {
            m.stuck = 0;
        }
        m.pos = pos;
        m.motion = mot;
        m.on_ground = on_ground || (in_liquid && false);
    }

    fn next_waypoint(&mut self, m: &mut Mob, dest: [f32; 3]) -> [f32; 3] {
        let goal = Self::fpos(dest);
        let here = Self::fpos([m.pos[0], m.pos[1] + 0.1, m.pos[2]]);
        let far = ((dest[0] - m.pos[0]).powi(2) + (dest[2] - m.pos[2]).powi(2)).sqrt();
        let need = m.path_dest.is_none_or(|g| (g[0] - goal[0]).abs() + (g[1] - goal[1]).abs() + (g[2] - goal[2]).abs() > 1)
            || (m.path.is_empty() && far > 1.5 && self.ticks > m.path_tick + 20)
            || self.ticks > m.path_tick + 100;
        if need && far > 1.2 && self.mobsys.paths_left > 0 {
            self.mobsys.paths_left -= 1;
            m.path_tick = self.ticks;
            m.path_dest = Some(goal);
            let budget = ((far * 24.0) as usize).clamp(64, 700);
            m.path = self.find_path(m, here, goal, budget).unwrap_or_default();
        }
        while let Some(n) = m.path.first() {
            let c = [n[0] as f32 + 0.5, n[1] as f32, n[2] as f32 + 0.5];
            let d = ((c[0] - m.pos[0]).powi(2) + (c[2] - m.pos[2]).powi(2)).sqrt();
            if d < 0.4 + m.t.width * 0.25 && (c[1] - m.pos[1]).abs() < 1.2 {
                m.path.remove(0);
            } else {
                return c;
            }
        }
        dest
    }

    /// A random spot a walker can stand on within `xz` blocks horizontally and `y` vertically.
    pub(crate) fn random_ground_spot(&mut self, m: &Mob, xz: i32, y: i32, around: [f32; 3]) -> Option<[f32; 3]> {
        let h = m.t.height.ceil() as i32;
        for _ in 0..10 {
            let x = around[0].floor() as i32 + json::rand_i(-xz, xz);
            let z = around[2].floor() as i32 + json::rand_i(-xz, xz);
            let base = around[1].floor() as i32;
            for dy in (-y..=y).rev() {
                let b = [x, base + dy, z];
                if !self.worlds.worlds[m.world].is_loaded(b) {
                    break;
                }
                if self.standable(m.world, b, h, false, m.t.walks_on_lava) {
                    let water = self.class_at(m.world, b) & WATER != 0;
                    if water && (m.t.avoid_water || rand::random::<bool>()) {
                        break;
                    }
                    return Some([x as f32 + 0.5, b[1] as f32, z as f32 + 0.5]);
                }
            }
        }
        None
    }

    /// A random open spot (air or water as asked) for flyers and swimmers.
    pub(crate) fn random_open_spot(&mut self, m: &Mob, xz: i32, y: i32, want_water: bool, around: [f32; 3]) -> Option<[f32; 3]> {
        for _ in 0..12 {
            let p = [
                around[0] + json::rand_i(-xz, xz) as f32,
                around[1] + json::rand_i(-y, y) as f32,
                around[2] + json::rand_i(-xz, xz) as f32,
            ];
            let b = Self::fpos(p);
            if !self.worlds.worlds[m.world].is_loaded(b) || b[1] <= ceres_world::MIN_Y || b[1] >= ceres_world::MAX_Y {
                continue;
            }
            let c = self.class_at(m.world, b);
            let ok = if want_water { c & WATER != 0 } else { c & (SOLID | WATER | LAVA) == 0 };
            if ok && !self.box_blocked(m.world, p, m.t.width, m.t.height) {
                return Some([b[0] as f32 + 0.5, b[1] as f32 + 0.1, b[2] as f32 + 0.5]);
            }
        }
        None
    }
}

/// Turns `from` towards `to` by at most `step` degrees.
pub(crate) fn turn(from: f32, to: f32, step: f32) -> f32 {
    let mut d = (to - from).rem_euclid(360.0);
    if d > 180.0 {
        d -= 360.0;
    }
    from + d.clamp(-step, step)
}
