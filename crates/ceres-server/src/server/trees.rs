//! Saplings growing into trees (random ticks, about one in seven, the first one only sets the
//! sapling's age bit; bone meal: 45 % per use). Shapes ported from df-mc/dragonfly
//! (`server/block/sapling_tree.go`, MIT), which follows Bedrock's tree features: oak and birch
//! blobs, jungle with vines left out, spruce cones, a leaning acacia and a simplified cherry.
//! The 2x2 trees (dark oak, pale oak, giant spruce and jungle) are not grown yet.

use super::*;
use ceres_codec::nbt::Tag;

fn range(min: i32, max: i32) -> i32 {
    // dragonfly's intRange.at(): max is exclusive unless the range is a single value
    if min >= max - 1 { min } else { min + (rand::random::<u32>() % (max - min) as u32) as i32 }
}

fn roll(chance: f32) -> bool {
    chance >= 100.0 || (chance > 0.0 && rand::random::<f32>() * 100.0 < chance)
}

#[derive(Clone, Copy)]
enum Shape {
    Blob { trunk: (i32, i32) },
    Spruce,
    Acacia,
    /// dark and pale oak: a 2x2 trunk under a flat crown, from four saplings in a square
    Roofed,
}

struct Kind {
    wood: &'static str,
    leaves: &'static str,
    shape: Shape,
}

fn kind_of(sapling: &str) -> Option<Kind> {
    let s = sapling.strip_prefix("minecraft:")?.strip_suffix("_sapling")?;
    Some(match s {
        "oak" => Kind { wood: "oak_log", leaves: "oak_leaves", shape: Shape::Blob { trunk: (4, 7) } },
        "birch" => Kind { wood: "birch_log", leaves: "birch_leaves", shape: Shape::Blob { trunk: (5, 8) } },
        "jungle" => Kind { wood: "jungle_log", leaves: "jungle_leaves", shape: Shape::Blob { trunk: (4, 11) } },
        "cherry" => Kind { wood: "cherry_log", leaves: "cherry_leaves", shape: Shape::Blob { trunk: (5, 8) } },
        "spruce" => Kind { wood: "spruce_log", leaves: "spruce_leaves", shape: Shape::Spruce },
        "acacia" => Kind { wood: "acacia_log", leaves: "acacia_leaves", shape: Shape::Acacia },
        "dark_oak" => Kind { wood: "dark_oak_log", leaves: "dark_oak_leaves", shape: Shape::Roofed },
        "pale_oak" => Kind { wood: "pale_oak_log", leaves: "pale_oak_leaves", shape: Shape::Roofed },
        _ => return None,
    })
}

impl Server {
    fn tree_name(&mut self, world: usize, p: [i32; 3]) -> String {
        if !self.worlds.worlds[world].is_loaded(p) {
            return String::from("?");
        }
        let id = self.worlds.worlds[world].get_block(p, &self.gd);
        self.gd.block_name(id).to_string()
    }

    fn soil(name: &str) -> bool {
        matches!(name, "minecraft:grass_block" | "minecraft:dirt" | "minecraft:podzol" | "minecraft:farmland" | "minecraft:mud" | "minecraft:coarse_dirt" | "minecraft:rooted_dirt" | "minecraft:moss_block")
    }

    fn grows_through(&mut self, world: usize, p: [i32; 3]) -> bool {
        let n = self.tree_name(world, p);
        n == "minecraft:air" || n.ends_with("_leaves") || n.ends_with("_sapling") || n == "minecraft:vine" || n == "minecraft:short_grass" || n == "minecraft:fern" || n == "minecraft:tall_grass" || n == "minecraft:azalea" || n == "minecraft:flowering_azalea" || Self::soil(&n)
    }

    /// dragonfly's treeRoom: soil below and nothing solid where the tree will be.
    fn tree_room(&mut self, world: usize, pos: [i32; 3], height: i32) -> bool {
        if !Self::soil(&self.tree_name(world, [pos[0], pos[1] - 1, pos[2]])) {
            return false;
        }
        for y in pos[1]..=pos[1] + height + 1 {
            let r = if y == pos[1] { 0 } else if y >= pos[1] + height - 1 { 2 } else { 1 };
            for x in pos[0] - r..=pos[0] + r {
                for z in pos[2] - r..=pos[2] + r {
                    if !self.grows_through(world, [x, y, z]) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn put_log(&mut self, world: usize, p: [i32; 3], log: i32) -> bool {
        let n = self.tree_name(world, p);
        if n == "minecraft:air" || n.ends_with("_leaves") || n.ends_with("_sapling") || n == "minecraft:vine" || n == "minecraft:short_grass" || n == "minecraft:fern" || n.ends_with("azalea") {
            self.change_block(world, p, log);
            return true;
        }
        false
    }

    fn put_leaf(&mut self, world: usize, p: [i32; 3], leaf: i32) {
        let n = self.tree_name(world, p);
        if n == "minecraft:air" || n.ends_with("_leaves") {
            self.change_block(world, p, leaf);
        }
    }

    /// dragonfly's simpleCanopy for oak, birch and jungle: offsets -3..=0 around `origin`.
    fn blob_canopy(&mut self, world: usize, origin: [i32; 3], leaf: i32) {
        let variation = [50.0, 50.0, 50.0, 100.0];
        for offset in -3..=0 {
            let r = 1 - (offset as f32 * 0.5) as i32;
            let var = variation[(offset + 3) as usize];
            for dx in -r..=r {
                for dz in -r..=r {
                    if dx.abs() == r && dz.abs() == r && roll(var) {
                        continue;
                    }
                    self.put_leaf(world, [origin[0] + dx, origin[1] + offset, origin[2] + dz], leaf);
                }
            }
        }
    }

    /// dragonfly's spruceCanopy: rings widening and narrowing down the trunk.
    fn spruce_canopy(&mut self, world: usize, origin: [i32; 3], ground: i32, leaf: i32) {
        let (lower, upper, widest) = (range(1, 3), range(0, 3), range(2, 4));
        let mut radius = (rand::random::<u32>() % 2) as i32;
        let (mut reach, mut narrowest) = (4 - 2 - 1, 0);
        let top = origin[1] + upper;
        for row in 0..(top - (ground + lower) + 1).max(0) {
            let y = top - row;
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    if radius != 0 && dx.abs() == radius && dz.abs() == radius {
                        continue;
                    }
                    self.put_leaf(world, [origin[0] + dx, y, origin[2] + dz], leaf);
                }
            }
            let next = (reach + 1).min(widest);
            if reach <= radius {
                radius = narrowest;
                narrowest = 1;
                reach = next;
                continue;
            }
            radius += 1;
        }
    }

    /// dragonfly's acaciaCanopy (size 3, not simplified).
    fn acacia_canopy(&mut self, world: usize, o: [i32; 3], leaf: i32) {
        let size = 3;
        for dx in -1..1 {
            for dz in -1..1 {
                self.put_leaf(world, [o[0] + dx, o[1] + 1, o[2] + dz], leaf);
            }
        }
        for arm in [[size - 1, 1, 0], [1 - size, 1, 0], [0, 1, size - 1], [0, 1, 1 - size]] {
            self.put_leaf(world, [o[0] + arm[0], o[1] + arm[1], o[2] + arm[2]], leaf);
        }
        for dx in -size..=size {
            for dz in -size..=size {
                if dx.abs() == size && dz.abs() == size {
                    continue;
                }
                self.put_leaf(world, [o[0] + dx, o[1], o[2] + dz], leaf);
            }
        }
    }

    /// The north-west corner of a 2x2 square of this sapling that `pos` is part of.
    fn sapling_square(&mut self, world: usize, pos: [i32; 3], sapling: &str) -> Option<[i32; 3]> {
        for c in [[0, 0], [-1, 0], [0, -1], [-1, -1]] {
            let corner = [pos[0] + c[0], pos[1], pos[2] + c[1]];
            let all = [[0, 0], [1, 0], [0, 1], [1, 1]].iter().all(|d| self.tree_name(world, [corner[0] + d[0], pos[1], corner[2] + d[1]]) == sapling);
            if all {
                return Some(corner);
            }
        }
        None
    }

    /// dragonfly's roofedCanopy (height 4, core 2, outer radius 2, inner radius 3) around the
    /// top of a 2x2 trunk; `origin` is the trunk's last log, as there.
    fn roofed_canopy(&mut self, world: usize, origin: [i32; 3], leaf: i32) {
        let (size, layers) = (2, 1);
        let r = 2;
        let air_leaf = |s: &mut Server, p: [i32; 3]| {
            if s.tree_name(world, p) == "minecraft:air" {
                s.change_block(world, p, leaf);
            }
        };
        for dx in -r..=size - 1 + r {
            for dz in -r..=size - 1 + r {
                air_leaf(self, [origin[0] + dx, origin[1] - 1, origin[2] + dz]);
                let (ox, oz) = ((-dx).max(dx - size + 1).max(0), (-dz).max(dz - size + 1).max(0));
                if ox + oz >= r * 2 - 1 {
                    continue;
                }
                air_leaf(self, [origin[0] + dx, origin[1] + layers, origin[2] + dz]);
            }
        }
        let r = 3;
        for y in 0..layers {
            for dx in -r..size + r {
                for dz in -r..size + r {
                    if dx.abs() >= r && dz.abs() >= r {
                        continue;
                    }
                    air_leaf(self, [origin[0] + dx, origin[1] + y, origin[2] + dz]);
                }
            }
        }
        if rand::random::<bool>() {
            for dx in 0..size {
                for dz in 0..size {
                    air_leaf(self, [origin[0] + dx, origin[1] + layers + 1, origin[2] + dz]);
                }
            }
        }
    }

    /// dragonfly's treeVines: vines hang off the free sides of `pos` (each side by `chance`
    /// percent), down to `steps` blocks.
    fn tree_vines(&mut self, world: usize, pos: [i32; 3], chance: f32, steps: i32) {
        // vine_direction_bits: south 1, west 2, north 4, east 8 (the side it clings to)
        for (side, bits) in [([-1, 0], 8), ([1, 0], 2), ([0, -1], 1), ([0, 1], 4)] {
            if !roll(chance) {
                continue;
            }
            let Some(vine) = self.gd.default_block("minecraft:vine").and_then(|v| self.gd.with_states(v, &[("vine_direction_bits", Tag::Int(bits))])) else { return };
            let mut at = [pos[0] + side[0], pos[1], pos[2] + side[1]];
            for _ in 0..steps {
                if self.tree_name(world, at) != "minecraft:air" {
                    break;
                }
                self.change_block(world, at, vine);
                at[1] -= 1;
            }
        }
    }

    /// dragonfly's treeRow: a rounded layer of leaves around a `size` x `size` core.
    fn tree_row(&mut self, world: usize, pos: [i32; 3], r: i32, size: i32, leaf: i32) {
        for dx in -r..size + r {
            for dz in -r..size + r {
                if dx * dx + dz * dz > r * r && (dx - 1).pow(2) + (dz - 1).pow(2) > r * r && dx * dx + (dz - 1).pow(2) > r * r && (dx - 1).pow(2) + dz * dz > r * r {
                    continue;
                }
                self.put_leaf(world, [pos[0] + dx, pos[1], pos[2] + dz], leaf);
            }
        }
    }

    /// Giant spruce (pine crown, podzol around) or giant jungle tree (layered crown, branches)
    /// from four saplings in a square: dragonfly's megaTrunk with megaPineCanopy / megaCanopy.
    fn grow_mega(&mut self, world: usize, corner: [i32; 3], spruce: bool, log: i32, leaf: i32) -> bool {
        let height = if spruce {
            13 + (rand::random::<u32>() % 3) as i32 + (rand::random::<u32>() % 15) as i32
        } else {
            10 + (rand::random::<u32>() % 3) as i32 + (rand::random::<u32>() % 20) as i32
        };
        if !Self::soil(&self.tree_name(world, [corner[0], corner[1] - 1, corner[2]])) {
            return false;
        }
        for i in 0..=height + 1 {
            let r = if i == 0 { 1 } else { 2 };
            for dx in -r..=r {
                for dz in -r..=r {
                    if !self.grows_through(world, [corner[0] + dx, corner[1] + i, corner[2] + dz]) {
                        return false;
                    }
                }
            }
        }
        let top = [corner[0], corner[1] + height, corner[2]];
        if spruce {
            let h = range(13, 18);
            let mut last = 0;
            for down in (0..=h).rev() {
                let y = top[1] - down;
                let r = (down as f32 / h as f32 * 3.5).floor() as i32;
                let frill = if down >= 1 && r == last && y % 2 == 0 { r + 1 } else { r };
                self.tree_row(world, [top[0], y, top[2]], frill, 2, leaf);
                last = r;
            }
        } else {
            for row in (0..3).rev() {
                self.tree_row(world, [top[0], top[1] - row, top[2]], 1 + 2 + row, 2, leaf);
            }
            // branches with small crowns
            let mut y = corner[1] - range(2, 6) + height;
            while y > corner[1] + height / 2 {
                let angle = rand::random::<f32>() * std::f32::consts::TAU;
                let (mut bx, mut bz) = (corner[0] + 1, corner[2] + 1);
                let base = (y - 1) as f32 - 4.0 * 0.5;
                for b in 0..5 {
                    bx = corner[0] + (1.5 + angle.cos() * b as f32) as i32;
                    bz = corner[2] + (1.5 + angle.sin() * b as f32) as i32;
                    self.put_log(world, [bx, (base + b as f32 * 0.5) as i32, bz], log);
                }
                for row in (0..range(2, 4)).rev() {
                    self.tree_row(world, [bx, y - row, bz], 1 + row, 1, leaf);
                }
                y -= range(2, 6);
            }
        }
        for i in 0..height {
            for d in [[0, 0], [1, 0], [0, 1], [1, 1]] {
                self.put_log(world, [corner[0] + d[0], corner[1] + i, corner[2] + d[1]], log);
            }
        }
        let base = self.gd.default_block(if spruce { "minecraft:podzol" } else { "minecraft:dirt" });
        if let Some(base) = base {
            for d in [[0, 0], [1, 0], [0, 1], [1, 1]] {
                self.change_block(world, [corner[0] + d[0], corner[1] - 1, corner[2] + d[1]], base);
            }
            if spruce {
                // podzol patches at the corners
                for c in [[-1, -1], [2, -1], [-1, 2], [2, 2]] {
                    for dx in -2..=2i32 {
                        for dz in -2..=2i32 {
                            if dx.abs() == 2 && dz.abs() == 2 {
                                continue;
                            }
                            let p = [corner[0] + c[0] + dx, corner[1] - 1, corner[2] + c[1] + dz];
                            let n = self.tree_name(world, p);
                            if n == "minecraft:grass_block" || n == "minecraft:dirt" {
                                self.change_block(world, p, base);
                            }
                        }
                    }
                }
            }
        }
        true
    }

    /// Grows the sapling at `pos` into a tree. False when there is no room.
    pub(crate) fn grow_tree(&mut self, world: usize, pos: [i32; 3], sapling: &str) -> bool {
        let Some(k) = kind_of(sapling) else { return false };
        let log = self.gd.default_block(&format!("minecraft:{}", k.wood));
        let leaf = self.gd.default_block(&format!("minecraft:{}", k.leaves));
        let (Some(log), Some(leaf)) = (log, leaf) else { return false };
        // spruce and jungle saplings in a 2x2 square make the giant kind
        if matches!(k.shape, Shape::Spruce) || k.wood == "jungle_log" {
            if let Some(corner) = self.sapling_square(world, pos, sapling) {
                let air = self.gd.air_id;
                let saplings: Vec<[i32; 3]> = [[0, 0], [1, 0], [0, 1], [1, 1]].iter().map(|d| [corner[0] + d[0], corner[1], corner[2] + d[1]]).collect();
                for s in &saplings {
                    self.change_block(world, *s, air);
                }
                if self.grow_mega(world, corner, matches!(k.shape, Shape::Spruce), log, leaf) {
                    return true;
                }
                // no room: the saplings come back
                if let Some(sap) = self.gd.default_block(sapling) {
                    for s in saplings {
                        self.change_block(world, s, sap);
                    }
                }
                return false;
            }
        }
        let height = match k.shape {
            Shape::Blob { trunk } => range(trunk.0, trunk.1),
            Shape::Spruce => range(6, 10),
            Shape::Acacia => 5 + (rand::random::<u32>() % 3) as i32 + (rand::random::<u32>() % 3) as i32,
            Shape::Roofed => 6 + (rand::random::<u32>() % 3) as i32 + (rand::random::<u32>() % 2) as i32,
        };
        if let Shape::Roofed = k.shape {
            let Some(corner) = self.sapling_square(world, pos, sapling) else { return false };
            if !self.tree_room(world, corner, height) {
                return false;
            }
            let air = self.gd.air_id;
            let dirt = self.gd.default_block("minecraft:dirt");
            for d in [[0, 0], [1, 0], [0, 1], [1, 1]] {
                self.change_block(world, [corner[0] + d[0], corner[1], corner[2] + d[1]], air);
                if let Some(dirt) = dirt {
                    self.change_block(world, [corner[0] + d[0], corner[1] - 1, corner[2] + d[1]], dirt);
                }
            }
            let mut anchor = None;
            for i in 0..height {
                for d in [[0, 0], [1, 0], [0, 1], [1, 1]] {
                    let at = [corner[0] + d[0], corner[1] + i, corner[2] + d[1]];
                    if self.put_log(world, at, log) && i >= 3 {
                        anchor = Some(at);
                    }
                }
            }
            if let Some(a) = anchor {
                self.roofed_canopy(world, a, leaf);
            }
            return true;
        }
        if !self.tree_room(world, pos, height) {
            return false;
        }
        let air = self.gd.air_id;
        self.change_block(world, pos, air);
        let mut top = pos;
        match k.shape {
            Shape::Blob { .. } | Shape::Spruce => {
                let logs = if matches!(k.shape, Shape::Spruce) { height + range(-2, 1) } else { height };
                for i in 0..logs.max(1) {
                    let at = [pos[0], pos[1] + i, pos[2]];
                    if self.put_log(world, at, log) {
                        top = at;
                    }
                }
                let origin = [top[0], top[1] + 1, top[2]];
                if matches!(k.shape, Shape::Spruce) {
                    self.spruce_canopy(world, origin, pos[1], leaf);
                } else {
                    self.blob_canopy(world, origin, leaf);
                }
                if k.wood == "jungle_log" {
                    // jungle trees wear vines: on the trunk, and hanging from the crown
                    for i in 0..logs.max(1) {
                        self.tree_vines(world, [pos[0], pos[1] + i, pos[2]], 33.33, 1);
                    }
                    for dy in -3..=0 {
                        for dx in -2..=2 {
                            for dz in -2..=2 {
                                let at = [origin[0] + dx, origin[1] + dy, origin[2] + dz];
                                if self.tree_name(world, at) == "minecraft:jungle_leaves" {
                                    self.tree_vines(world, at, 25.0, 4);
                                }
                            }
                        }
                    }
                }
            }
            Shape::Roofed => unreachable!(),
            Shape::Acacia => {
                // the trunk leans to one side after a few blocks
                let dirs = [[1, 0], [-1, 0], [0, 1], [0, -1]];
                let d = dirs[rand::random::<u32>() as usize % 4];
                let lean_at = range(1, 5).min(height - 1);
                let steps = range(1, 4);
                let mut at = pos;
                for i in 0..height {
                    if i >= lean_at && i < lean_at + steps {
                        at = [at[0] + d[0], at[1], at[2] + d[1]];
                    }
                    let p = [at[0], pos[1] + i, at[2]];
                    if self.put_log(world, p, log) {
                        top = p;
                    }
                }
                self.acacia_canopy(world, [top[0], top[1] + 1, top[2]], leaf);
            }
        }
        // grass under the trunk turns to dirt
        let below = [pos[0], pos[1] - 1, pos[2]];
        if self.tree_name(world, below) != "minecraft:dirt" {
            if let Some(dirt) = self.gd.default_block("minecraft:dirt") {
                self.change_block(world, below, dirt);
            }
        }
        true
    }

    /// Random tick of a sapling: the first sets its age bit, the next grows it.
    pub(crate) fn sapling_tick(&mut self, world: usize, pos: [i32; 3], block: i32, name: &str) -> bool {
        if kind_of(name).is_none() {
            return name.ends_with("_sapling");
        }
        if rand::random::<u32>() % 7 != 0 {
            return true;
        }
        let aged = self.gd.block_states.get(&block).and_then(|s| s.states.get("age_bit")).and_then(|t| t.as_i64()).unwrap_or(0) != 0;
        if !aged {
            if let Some(id) = self.gd.with_states(block, &[("age_bit", Tag::Byte(1))]) {
                self.change_block(world, pos, id);
            }
        } else {
            let name = name.to_string();
            self.grow_tree(world, pos, &name);
        }
        true
    }
}

impl Server {
    /// An azalea bush grows into an azalea tree: an oak trunk of 4 or 5 that bends to one side
    /// near the top, rooted dirt under it, and a loose crown of azalea leaves (one in four in
    /// flower) around its upper logs.
    pub(crate) fn grow_azalea(&mut self, world: usize, pos: [i32; 3]) -> bool {
        let log = self.gd.default_block("minecraft:oak_log");
        let leaves = self.gd.default_block("minecraft:azalea_leaves");
        let flowered = self.gd.default_block("minecraft:azalea_leaves_flowered");
        let (Some(log), Some(leaves), Some(flowered)) = (log, leaves, flowered) else { return false };
        let height = 4 + (rand::random::<u32>() % 2) as i32;
        if !self.tree_room(world, pos, height) {
            return false;
        }
        let air = self.gd.air_id;
        self.change_block(world, pos, air);
        if let Some(roots) = self.gd.default_block("minecraft:dirt_with_roots") {
            self.change_block(world, [pos[0], pos[1] - 1, pos[2]], roots);
        }
        let dirs = [[1, 0], [-1, 0], [0, 1], [0, -1]];
        let d = dirs[rand::random::<u32>() as usize % 4];
        let bend_from = height - 2;
        let mut at = pos;
        let mut upper = Vec::new();
        for i in 0..height {
            if i >= bend_from {
                at = [at[0] + d[0], at[1], at[2] + d[1]];
            }
            let p = [at[0], pos[1] + i, at[2]];
            self.put_log(world, p, log);
            if i >= 2 {
                upper.push(p);
            }
        }
        for c in upper {
            for _ in 0..40 {
                let off = [json_rand(-2, 2), json_rand(-1, 1), json_rand(-2, 2)];
                if off[0].abs() + off[2].abs() > 3 {
                    continue;
                }
                let leaf = if rand::random::<u32>() % 4 == 0 { flowered } else { leaves };
                self.put_leaf(world, [c[0] + off[0], c[1] + off[1], c[2] + off[2]], leaf);
            }
        }
        true
    }
}

fn json_rand(lo: i32, hi: i32) -> i32 {
    lo + (rand::random::<u32>() % (hi - lo + 1) as u32) as i32
}

impl Server {
    /// A planted mangrove propagule grows in stages (0-4); the last one becomes a mangrove:
    /// a trunk standing on arching roots, a round crown, and new propagules hanging under it.
    pub(crate) fn grow_propagule(&mut self, world: usize, pos: [i32; 3], block: i32) -> bool {
        let states = self.gd.block_states.get(&block).map(|s| s.states.clone());
        let hanging = states.as_ref().and_then(|s| s.get("hanging")).and_then(|t| t.as_i64()).unwrap_or(0) != 0;
        if hanging {
            return false;
        }
        let stage = states.as_ref().and_then(|s| s.get("propagule_stage")).and_then(|t| t.as_i64()).unwrap_or(0);
        if stage < 4 {
            if let Some(id) = self.gd.with_states(block, &[("propagule_stage", Tag::Int(stage as i32 + 1))]) {
                self.change_block(world, pos, id);
            }
            return true;
        }
        let ids = ["minecraft:mangrove_log", "minecraft:mangrove_leaves", "minecraft:mangrove_roots"].map(|n| self.gd.default_block(n));
        let [Some(log), Some(leaf), Some(roots)] = ids else { return false };
        let height = 4 + (rand::random::<u32>() % 3) as i32;
        // roots lift the trunk 2 blocks; they need room around
        let base = [pos[0], pos[1] + 2, pos[2]];
        for y in 0..height + 4 {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    if !self.grows_through(world, [pos[0] + dx, pos[1] + y, pos[2] + dz]) && !(dx == 0 && dz == 0 && y == 0) {
                        return false;
                    }
                }
            }
        }
        let air = self.gd.air_id;
        self.change_block(world, pos, air);
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            // each root arches out and down to the ground
            for (step, dy) in [(1, 1), (1, 0), (2, -1)] {
                let p = [pos[0] + dx * step, pos[1] + dy + 1, pos[2] + dz * step];
                let n = self.tree_name(world, p);
                if n == "minecraft:air" || n == "minecraft:water" || n.ends_with("_leaves") {
                    self.change_block(world, p, roots);
                }
            }
        }
        self.change_block(world, pos, roots);
        self.change_block(world, [pos[0], pos[1] + 1, pos[2]], roots);
        for i in 0..height {
            self.put_log(world, [base[0], base[1] + i, base[2]], log);
        }
        let top = [base[0], base[1] + height, base[2]];
        for dy in -2..=1i32 {
            let r = if dy >= 1 { 1 } else { 3 - dy.abs().min(1) };
            for dx in -r..=r {
                for dz in -r..=r {
                    if dx.abs() == r && dz.abs() == r && rand::random::<bool>() {
                        continue;
                    }
                    self.put_leaf(world, [top[0] + dx, top[1] + dy, top[2] + dz], leaf);
                }
            }
        }
        // a few propagules hang under the crown
        if let Some(prop) = self.gd.default_block("minecraft:mangrove_propagule").and_then(|p| self.gd.with_states(p, &[("hanging", Tag::Byte(1))])) {
            for _ in 0..3 {
                let p = [top[0] + json_rand(-2, 2), top[1] - 3, top[2] + json_rand(-2, 2)];
                if self.tree_name(world, p) == "minecraft:air" && self.tree_name(world, [p[0], p[1] + 1, p[2]]) == "minecraft:mangrove_leaves" {
                    self.change_block(world, p, prop);
                }
            }
        }
        true
    }
}
