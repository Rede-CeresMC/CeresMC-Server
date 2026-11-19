//! Redstone (power model of PowerNukkitX's `Level.getRedstonePower`/`getStrongPower` and its
//! `BlockRedstoneWire`, LGPL-3.0):
//! - sources give weak power to every neighbour and strong power one way: a lever or button to
//!   the block it is on, a lit redstone torch to the block above (never to its own block), a
//!   pressure plate to the block below; the redstone block only weak power;
//! - a full block passes on the strong power it gets (it is "strongly powered");
//! - dust carries the strongest power around it minus one per block (a network is worked out at
//!   once), powers the block below it and the one it points into, and takes the `side`/`up`
//!   connection shapes the client draws;
//! - what reacts: lamps (off after 4 ticks), doors/trapdoors/fence gates follow the power,
//!   powered and activator rails (up to 8 rails from the powered one), TNT lights, a redstone
//!   torch goes out while its block is powered (after 2 ticks).
//!
//! Every block change near redstone runs an update; buttons spring back after 1 or 1.5 s and
//! pressure plates follow who stands on them.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::level_event;
use std::collections::{HashMap, HashSet, VecDeque};

/// down, up, north, south, west, east
const DIRS: [[i32; 3]; 6] = [[0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]];
const DOWN: usize = 0;
const UP: usize = 1;
/// horizontal faces with their state suffix
const HORIZ: [(usize, &str); 4] = [(2, "north"), (3, "south"), (4, "west"), (5, "east")];

fn add(p: [i32; 3], d: usize) -> [i32; 3] {
    [p[0] + DIRS[d][0], p[1] + DIRS[d][1], p[2] + DIRS[d][2]]
}

/// The two horizontal faces at right angles to `d`.
fn perpendicular(d: usize) -> [usize; 2] {
    if d == 2 || d == 3 {
        [4, 5]
    } else {
        [2, 3]
    }
}

#[derive(Default)]
pub(crate) struct Redstone {
    busy: bool,
    pending: VecDeque<(usize, [i32; 3])>,
    /// (tick, world, pos): look at the block again then (buttons, lamps, torches)
    scheduled: Vec<(u64, usize, [i32; 3])>,
    /// while a dust network is worked out its wires give no power
    wires_off: bool,
    /// pressed pressure plates and the last tick something stood on them
    plates: HashMap<(usize, [i32; 3]), u64>,
    /// doors, trapdoors, fence gates and note blocks that were powered at their last look
    powered: HashSet<(usize, [i32; 3])>,
    /// daylight sensors seen (they are looked at every second)
    sensors: HashSet<(usize, [i32; 3])>,
    /// comparators seen and the signal each gives out (looked at every 4 ticks: the
    /// containers they read change without block updates)
    comparators: HashMap<(usize, [i32; 3]), i32>,
}

fn short(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}

fn is_comparator(n: &str) -> bool {
    matches!(short(n), "unpowered_comparator" | "powered_comparator")
}

fn is_repeater(n: &str) -> bool {
    matches!(short(n), "unpowered_repeater" | "powered_repeater")
}

/// A repeater's facing (towards its input, PowerNukkitX's `getFacing`): north 2, south 3,
/// west 4, east 5. Power goes in from that side and out of the opposite one.
fn cardinal(dir: &str) -> usize {
    match dir {
        "north" => 2,
        "south" => 3,
        "west" => 4,
        _ => 5,
    }
}

/// Direction index of a `minecraft:facing_direction` name.
fn facing_name(s: &str) -> usize {
    match s {
        "down" => 0,
        "up" => 1,
        "north" => 2,
        "south" => 3,
        "west" => 4,
        _ => 5,
    }
}

fn is_plate(n: &str) -> bool {
    n.ends_with("_pressure_plate")
}

fn is_button(n: &str) -> bool {
    n.ends_with("_button")
}

/// Blocks that give power (dust connects to them).
fn is_source(n: &str) -> bool {
    matches!(short(n), "lever" | "redstone_torch" | "redstone_block" | "redstone_wire" | "observer" | "detector_rail" | "daylight_detector" | "daylight_detector_inverted")
        || is_plate(n)
        || is_button(n)
        || is_repeater(n)
        || is_comparator(n)
}

/// Blocks that react to power.
fn is_consumer(n: &str) -> bool {
    let s = short(n);
    matches!(s, "redstone_lamp" | "lit_redstone_lamp" | "tnt" | "golden_rail" | "activator_rail" | "redstone_torch" | "unlit_redstone_torch")
        || is_repeater(n)
        || s.ends_with("_door")
        || s.ends_with("trapdoor")
        || s.ends_with("fence_gate")
        || s == "fence_gate"
        || s == "noteblock"
        || s == "hopper"
        || s == "dispenser"
        || s == "dropper"
        || is_comparator(n)
}

impl Server {
    fn rs_block(&mut self, world: usize, pos: [i32; 3]) -> (i32, String) {
        if pos[1] < ceres_world::MIN_Y || pos[1] > ceres_world::MAX_Y || !self.worlds.worlds[world].is_loaded(pos) {
            return (self.gd.air_id, "minecraft:air".into());
        }
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        (id, self.gd.block_name(id).to_string())
    }

    fn rs_state(&self, id: i32, key: &str) -> Option<Tag> {
        self.gd.block_states.get(&id).and_then(|s| s.states.get(key)).cloned()
    }

    fn rs_int(&self, id: i32, key: &str) -> i64 {
        self.rs_state(id, key).and_then(|t| t.as_i64()).unwrap_or(0)
    }

    fn rs_str(&self, id: i32, key: &str) -> String {
        self.rs_state(id, key).and_then(|t| t.as_str().map(String::from)).unwrap_or_default()
    }

    /// A full block that passes power on (not a source itself).
    fn conductor(&mut self, world: usize, pos: [i32; 3]) -> bool {
        let (id, name) = self.rs_block(world, pos);
        !is_source(&name) && self.gd.is_opaque_cube(id)
    }

    fn is_wire(&mut self, world: usize, pos: [i32; 3]) -> bool {
        self.rs_block(world, pos).1 == "minecraft:redstone_wire"
    }

    /// Power the block at `pos` gives to the neighbour on its `side`, where `side` is the
    /// direction from that neighbour to `pos`: (weak, strong).
    fn emitted(&mut self, world: usize, pos: [i32; 3], side: usize) -> (i32, i32) {
        let (id, name) = self.rs_block(world, pos);
        match short(&name) {
            "lever" => {
                if self.rs_int(id, "open_bit") == 0 {
                    return (0, 0);
                }
                let facing = match self.rs_str(id, "lever_direction").as_str() {
                    "east" => 5,
                    "west" => 4,
                    "south" => 3,
                    "north" => 2,
                    s if s.starts_with("up") => UP,
                    _ => DOWN,
                };
                (15, if side == facing { 15 } else { 0 })
            }
            n if is_button(n) => {
                if self.rs_int(id, "button_pressed_bit") == 0 {
                    return (0, 0);
                }
                let facing = self.rs_int(id, "facing_direction") as usize;
                (15, if side == facing { 15 } else { 0 })
            }
            "redstone_torch" => {
                // the flame points away from the block the torch is on
                let flame = match self.rs_str(id, "torch_facing_direction").as_str() {
                    "west" => 5,
                    "east" => 4,
                    "north" => 3,
                    "south" => 2,
                    _ => UP,
                };
                (if side != flame { 15 } else { 0 }, if side == DOWN { 15 } else { 0 })
            }
            "redstone_block" => (15, 0),
            // the observer pulses out of its back (it faces the block it watches)
            "observer" => {
                if self.rs_int(id, "powered_bit") == 0 {
                    return (0, 0);
                }
                let f = facing_name(&self.rs_str(id, "minecraft:facing_direction"));
                if side == f {
                    (15, 15)
                } else {
                    (0, 0)
                }
            }
            n if is_comparator(n) => {
                let f = cardinal(&self.rs_str(id, "minecraft:cardinal_direction"));
                let out = self.redstone.comparators.get(&(world, pos)).copied().unwrap_or(0);
                if side == f {
                    (out, out)
                } else {
                    (0, 0)
                }
            }
            "detector_rail" => {
                if self.rs_int(id, "rail_data_bit") == 0 {
                    return (0, 0);
                }
                (15, if side == UP { 15 } else { 0 })
            }
            "daylight_detector" | "daylight_detector_inverted" => (self.rs_int(id, "redstone_signal") as i32, 0),
            "powered_repeater" => {
                let f = cardinal(&self.rs_str(id, "minecraft:cardinal_direction"));
                if side == f {
                    (15, 15)
                } else {
                    (0, 0)
                }
            }
            n if is_plate(n) => {
                let s = self.rs_int(id, "redstone_signal") as i32;
                (s, if side == UP { s } else { 0 })
            }
            "redstone_wire" => {
                if self.redstone.wires_off {
                    return (0, 0);
                }
                let s = self.rs_int(id, "redstone_signal") as i32;
                let w = if s == 0 || side == DOWN {
                    0
                } else if side == UP {
                    s
                } else {
                    let points: Vec<usize> = HORIZ.iter().map(|(d, _)| *d).filter(|d| self.wire_points(world, pos, *d)).collect();
                    let [a, b] = perpendicular(side);
                    if points.is_empty() || (points.contains(&side) && !points.contains(&a) && !points.contains(&b)) {
                        s
                    } else {
                        0
                    }
                };
                (w, w)
            }
            _ => (0, 0),
        }
    }

    /// Strong power a block gets from around it.
    fn strong_received(&mut self, world: usize, pos: [i32; 3]) -> i32 {
        let mut best = 0;
        for d in 0..6 {
            best = best.max(self.emitted(world, add(pos, d), d).1);
            if best >= 15 {
                break;
            }
        }
        best
    }

    /// Power coming to a block from its neighbour on side `d`.
    fn power_from(&mut self, world: usize, pos: [i32; 3], d: usize) -> i32 {
        let n = add(pos, d);
        if self.conductor(world, n) {
            self.strong_received(world, n)
        } else {
            self.emitted(world, n, d).0
        }
    }

    /// The strongest power reaching a block (0 = not powered).
    pub(crate) fn power_at(&mut self, world: usize, pos: [i32; 3]) -> i32 {
        let mut best = 0;
        for d in 0..6 {
            best = best.max(self.power_from(world, pos, d));
            if best >= 15 {
                break;
            }
        }
        best
    }

    /// Whether dust at `pos` runs towards its horizontal side `d` (PowerNukkitX's
    /// `isPowerSourceAt`): up the side of a block to dust on top, to a source or dust beside
    /// it, or down to dust below.
    fn wire_points(&mut self, world: usize, pos: [i32; 3], d: usize) -> bool {
        let n = add(pos, d);
        let n_full = self.conductor(world, n);
        let covered = self.conductor(world, add(pos, UP));
        (!covered && n_full && self.is_wire(world, add(n, UP))) || self.wire_joins(world, n, d) || (!n_full && self.is_wire(world, add(n, DOWN)))
    }

    /// Dust beside `n` (on its side `d`) joins it: dust, a source, a repeater along its axis.
    fn wire_joins(&mut self, world: usize, n: [i32; 3], d: usize) -> bool {
        let (id, name) = self.rs_block(world, n);
        if is_repeater(&name) || is_comparator(&name) {
            let f = cardinal(&self.rs_str(id, "minecraft:cardinal_direction"));
            return f == d || f == (d ^ 1);
        }
        is_source(&name)
    }

    /// The dust neighbours power flows between (same level, one up past an open top, one down
    /// past an open side).
    fn wire_links(&mut self, world: usize, pos: [i32; 3]) -> Vec<[i32; 3]> {
        let mut out = Vec::new();
        let covered = self.conductor(world, add(pos, UP));
        for (d, _) in HORIZ {
            let n = add(pos, d);
            if self.is_wire(world, n) {
                out.push(n);
            }
            if !covered && self.is_wire(world, add(n, UP)) {
                out.push(add(n, UP));
            }
            if !self.conductor(world, n) && self.is_wire(world, add(n, DOWN)) {
                out.push(add(n, DOWN));
            }
        }
        out
    }

    /// The dust shape: each side `none`, `side` or `up`; a lone line stretches across, a dot
    /// with no neighbours is a cross.
    fn wire_shape(&mut self, world: usize, pos: [i32; 3]) -> [&'static str; 4] {
        let covered = self.conductor(world, add(pos, UP));
        let mut out = ["none"; 4];
        for (i, (d, _)) in HORIZ.iter().enumerate() {
            let n = add(pos, *d);
            let n_full = self.conductor(world, n);
            out[i] = if !covered && n_full && self.is_wire(world, add(n, UP)) {
                "up"
            } else if self.wire_joins(world, n, *d) || (!n_full && self.is_wire(world, add(n, DOWN))) {
                "side"
            } else {
                "none"
            };
        }
        // north, south, west, east
        let ns_none = out[0] == "none" && out[1] == "none";
        let ew_none = out[2] == "none" && out[3] == "none";
        for i in 0..2 {
            if out[i] == "none" && ew_none {
                out[i] = "side";
            }
        }
        for i in 2..4 {
            if out[i] == "none" && ns_none {
                out[i] = "side";
            }
        }
        out
    }

    /// Sets a block without the redstone update of `change_block` (the update in progress
    /// looks at what changes).
    fn rs_set(&mut self, world: usize, pos: [i32; 3], id: i32) -> bool {
        // a state change (same block) keeps the block entity: a dispenser's or hopper's items
        let old = self.worlds.worlds[world].get_block(pos, &self.gd);
        let keep = if self.gd.block_name(old) == self.gd.block_name(id) { self.worlds.worlds[world].get_block_entity(pos, &self.gd) } else { None };
        if self.worlds.worlds[world].set_block(pos, id, &self.gd) {
            if let Some(tile) = keep {
                self.worlds.worlds[world].set_block_entity(pos, tile, &self.gd);
            }
            self.broadcast(&protocol::update_block(pos, id), |_, q| q.world == world);
            self.observe(world, pos);
            return true;
        }
        false
    }

    /// Observers facing `pos` see it change: a 2-tick pulse (PowerNukkitX's BlockObserver).
    fn observe(&mut self, world: usize, pos: [i32; 3]) {
        for d in 0..6 {
            let o = add(pos, d);
            let (id, name) = self.rs_block(world, o);
            if name != "minecraft:observer" || self.rs_int(id, "powered_bit") != 0 {
                continue;
            }
            // it watches the block in front of it
            if add(o, facing_name(&self.rs_str(id, "minecraft:facing_direction"))) == pos {
                self.schedule_redstone(world, o, 2);
            }
        }
    }

    /// Works out the dust networks touching `around` again; returns the dust that changed.
    fn update_wires(&mut self, world: usize, around: [i32; 3]) -> Vec<[i32; 3]> {
        let mut seeds = vec![around];
        for d in 0..6 {
            let n = add(around, d);
            seeds.push(n);
            if d >= 2 {
                seeds.push(add(n, UP));
                seeds.push(add(n, DOWN));
            }
        }
        let mut net: Vec<[i32; 3]> = Vec::new();
        let mut seen: HashSet<[i32; 3]> = HashSet::new();
        let mut queue: VecDeque<[i32; 3]> = seeds.into_iter().filter(|p| self.is_wire(world, *p)).collect();
        while let Some(p) = queue.pop_front() {
            if !seen.insert(p) || net.len() >= 4096 {
                continue;
            }
            net.push(p);
            for n in self.wire_links(world, p) {
                if !seen.contains(&n) {
                    queue.push_back(n);
                }
            }
        }
        if net.is_empty() {
            return Vec::new();
        }
        // power from outside the network, with the network's own dust switched off
        self.redstone.wires_off = true;
        let mut level: HashMap<[i32; 3], i32> = HashMap::new();
        for p in &net {
            let v = self.power_at(world, *p);
            level.insert(*p, v);
        }
        self.redstone.wires_off = false;
        // spread: each step loses one
        let mut order: Vec<[i32; 3]> = net.clone();
        order.sort_by_key(|p| -level[p]);
        let mut q: VecDeque<[i32; 3]> = order.into_iter().filter(|p| level[p] > 1).collect();
        while let Some(p) = q.pop_front() {
            let v = level[&p] - 1;
            for n in self.wire_links(world, p) {
                if let Some(cur) = level.get_mut(&n) {
                    if *cur < v {
                        *cur = v;
                        if v > 1 {
                            q.push_back(n);
                        }
                    }
                }
            }
        }
        let mut changed = Vec::new();
        for p in net {
            let (id, _) = self.rs_block(world, p);
            let shape = self.wire_shape(world, p);
            let mut states: Vec<(&str, Tag)> = vec![("redstone_signal", Tag::Int(level[&p]))];
            for (i, (_, side)) in HORIZ.iter().enumerate() {
                let key: &'static str = match *side {
                    "north" => "redstone_north",
                    "south" => "redstone_south",
                    "west" => "redstone_west",
                    _ => "redstone_east",
                };
                states.push((key, Tag::String(shape[i].into())));
            }
            if let Some(new) = self.gd.with_states(id, &states) {
                if new != id && self.rs_set(world, p, new) {
                    changed.push(p);
                }
            }
        }
        changed
    }

    /// Something changed at `pos`: dust and the blocks that react to power around it update.
    pub(crate) fn redstone_update(&mut self, world: usize, pos: [i32; 3]) {
        self.observe(world, pos);
        self.redstone.pending.push_back((world, pos));
        if self.redstone.busy {
            return;
        }
        self.redstone.busy = true;
        let mut rounds = 0;
        while let Some((w, p)) = self.redstone.pending.pop_front() {
            rounds += 1;
            if rounds > 512 {
                self.redstone.pending.clear();
                break;
            }
            if !self.near_redstone(w, p) {
                continue;
            }
            let mut centres = self.update_wires(w, p);
            centres.push(p);
            let mut looked: HashSet<[i32; 3]> = HashSet::new();
            for c in centres {
                for dx in -2i32..=2 {
                    for dy in -2i32..=2 {
                        for dz in -2i32..=2 {
                            if dx.abs() + dy.abs() + dz.abs() > 2 {
                                continue;
                            }
                            let at = [c[0] + dx, c[1] + dy, c[2] + dz];
                            if looked.insert(at) {
                                self.update_consumer(w, at);
                            }
                        }
                    }
                }
            }
        }
        self.redstone.busy = false;
    }

    /// Whether any dust, source or reacting block is within 2 blocks (cheap test before an update).
    fn near_redstone(&mut self, world: usize, pos: [i32; 3]) -> bool {
        for dx in -2i32..=2 {
            for dy in -2i32..=2 {
                for dz in -2i32..=2 {
                    if dx.abs() + dy.abs() + dz.abs() > 2 {
                        continue;
                    }
                    let n = self.rs_block(world, [pos[0] + dx, pos[1] + dy, pos[2] + dz]).1;
                    if is_source(&n) || is_consumer(&n) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn schedule_redstone(&mut self, world: usize, pos: [i32; 3], ticks: u64) {
        let at = self.ticks + ticks;
        if !self.redstone.scheduled.iter().any(|(t, w, p)| *t == at && *w == world && *p == pos) {
            self.redstone.scheduled.push((at, world, pos));
        }
    }

    /// A block that reacts to power looks at the power it gets.
    fn update_consumer(&mut self, world: usize, pos: [i32; 3]) {
        let (id, name) = self.rs_block(world, pos);
        if name.starts_with("minecraft:daylight_detector") {
            self.redstone.sensors.insert((world, pos));
        }
        if !is_consumer(&name) {
            return;
        }
        let s = short(&name).to_string();
        match s.as_str() {
            "redstone_lamp" => {
                if self.power_at(world, pos) > 0 {
                    if let Some(lit) = self.gd.default_block("minecraft:lit_redstone_lamp") {
                        self.rs_set(world, pos, lit);
                        self.redstone.pending.push_back((world, pos));
                    }
                }
            }
            "lit_redstone_lamp" => {
                if self.power_at(world, pos) == 0 {
                    self.schedule_redstone(world, pos, 4);
                }
            }
            "dispenser" | "dropper" => {
                let on = self.power_at(world, pos) > 0;
                let was = self.rs_int(id, "triggered_bit") != 0;
                if on != was {
                    if let Some(new) = super::dispenser::with_triggered(&self.gd, id, on) {
                        self.rs_set(world, pos, new);
                    }
                    if on {
                        self.schedule_redstone(world, pos, 4);
                    }
                }
            }
            "hopper" => {
                let on = self.power_at(world, pos) > 0;
                if on != (self.rs_int(id, "toggle_bit") != 0) {
                    if let Some(new) = self.gd.with_states(id, &[("toggle_bit", Tag::Byte(on as i8))]) {
                        self.rs_set(world, pos, new);
                    }
                }
            }
            n if is_comparator(n) => {
                let have = *self.redstone.comparators.entry((world, pos)).or_insert(0);
                if self.comparator_output(world, pos, id) != have {
                    self.schedule_redstone(world, pos, 2);
                }
            }
            "noteblock" => {
                let on = self.power_at(world, pos) > 0;
                let was = self.redstone.powered.contains(&(world, pos));
                if on && !was {
                    self.redstone.powered.insert((world, pos));
                    self.play_note(world, pos);
                } else if !on && was {
                    self.redstone.powered.remove(&(world, pos));
                }
            }
            "tnt" => {
                if self.power_at(world, pos) > 0 {
                    self.prime_tnt(world, pos);
                }
            }
            "golden_rail" | "activator_rail" => {
                let on = self.rail_powered(world, pos, &name);
                if on != (self.rs_int(id, "rail_data_bit") != 0) {
                    if let Some(new) = self.gd.with_states(id, &[("rail_data_bit", Tag::Byte(on as i8))]) {
                        self.rs_set(world, pos, new);
                    }
                }
            }
            "unpowered_repeater" | "powered_repeater" => {
                let on = s == "powered_repeater";
                if !self.repeater_locked(world, pos, id) && self.repeater_input(world, pos, id) != on {
                    let delay = (self.rs_int(id, "repeater_delay").clamp(0, 3) as u64 + 1) * 2;
                    self.schedule_redstone(world, pos, delay);
                }
            }
            "redstone_torch" | "unlit_redstone_torch" => {
                let lit = s == "redstone_torch";
                if self.torch_should_light(world, pos, id) != lit {
                    self.schedule_redstone(world, pos, 2);
                }
            }
            _ => {
                // doors (both halves), trapdoors, fence gates: open while powered
                let mut on = self.power_at(world, pos) > 0;
                let other = if s.ends_with("_door") { twopart::partner(&self.gd, pos, id) } else { None };
                if let Some(o) = other {
                    on |= self.power_at(world, o) > 0;
                }
                // only a change of power moves them (as vanilla): a door opened by hand stays
                // open while no power reaches it
                let key = (world, other.map_or(pos, |o| if o[1] < pos[1] { o } else { pos }));
                let was = self.redstone.powered.contains(&key);
                if on == was {
                    return;
                }
                if on {
                    self.redstone.powered.insert(key);
                } else {
                    self.redstone.powered.remove(&key);
                }
                let open = self.rs_int(id, "open_bit") != 0;
                if on != open {
                    if let Some(new) = self.gd.with_states(id, &[("open_bit", Tag::Byte(on as i8))]) {
                        self.rs_set(world, pos, new);
                        if other.is_some() {
                            self.toggle_partner(world, pos, new);
                        }
                        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
                        let sound = if s.ends_with("trapdoor") {
                            if on { "trapdoor.open" } else { "trapdoor.close" }
                        } else if s.ends_with("_door") {
                            if on { "door.open" } else { "door.close" }
                        } else if on {
                            "fence_gate.open"
                        } else {
                            "fence_gate.close"
                        };
                        self.broadcast(&protocol::level_sound_event(sound, center, new), |_, q| q.world == world);
                    }
                }
            }
        }
    }

    /// A redstone torch is lit unless the block it is on is powered.
    fn torch_should_light(&mut self, world: usize, pos: [i32; 3], id: i32) -> bool {
        let to_support = match self.rs_str(id, "torch_facing_direction").as_str() {
            "west" => 4,
            "east" => 5,
            "north" => 2,
            "south" => 3,
            _ => DOWN,
        };
        let support = add(pos, to_support);
        // the power the support gets, not counting this torch
        let mut best = 0;
        for d in 0..6 {
            let n = add(support, d);
            if n == pos {
                continue;
            }
            best = best.max(if self.conductor(world, support) { self.emitted(world, n, d).1 } else { 0 });
            if self.is_wire(world, n) && d == DOWN {
                // dust on top of the support powers it
                best = best.max(self.emitted(world, n, d).0);
            }
        }
        best == 0
    }

    /// A powered or activator rail is on when it, or one of the next 8 rails of its kind along
    /// the track, is powered.
    fn rail_powered(&mut self, world: usize, pos: [i32; 3], name: &str) -> bool {
        let mut seen: HashSet<[i32; 3]> = HashSet::new();
        let mut q: VecDeque<([i32; 3], u32)> = VecDeque::from([(pos, 0)]);
        while let Some((p, depth)) = q.pop_front() {
            if !seen.insert(p) {
                continue;
            }
            if self.power_at(world, p) > 0 {
                return true;
            }
            if depth >= 8 {
                continue;
            }
            for (d, _) in HORIZ {
                for dy in [-1, 0, 1] {
                    let n = [p[0] + DIRS[d][0], p[1] + dy, p[2] + DIRS[d][2]];
                    if self.rs_block(world, n).1 == name {
                        q.push_back((n, depth + 1));
                    }
                }
            }
        }
        false
    }

    /// TNT lit by redstone or a lighter: the block goes and a primed TNT entity (the pack's
    /// `minecraft:tnt`, 4 s fuse) takes its place.
    pub(crate) fn prime_tnt(&mut self, world: usize, pos: [i32; 3]) {
        let air = self.gd.air_id;
        self.rs_set(world, pos, air);
        let at = [pos[0] as f32 + 0.5, pos[1] as f32, pos[2] as f32 + 0.5];
        self.spawn_entity(world, "minecraft:tnt", at, None);
        self.broadcast(&protocol::level_sound_event("fuse", at, -1), |_, q| q.world == world);
        self.redstone.pending.push_back((world, pos));
    }

    /// Use on a button: it is pressed (and springs back later).
    pub(crate) fn use_button(&mut self, sid: SessionId, pos: [i32; 3]) -> bool {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return false };
        let (id, name) = self.rs_block(world, pos);
        if !is_button(&name) {
            return false;
        }
        self.swing_arm(sid);
        if self.rs_int(id, "button_pressed_bit") != 0 {
            return true;
        }
        if let Some(new) = self.gd.with_states(id, &[("button_pressed_bit", Tag::Byte(1))]) {
            self.rs_set(world, pos, new);
            let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
            self.broadcast(&protocol::level_event(level_event::SOUND_CLICK, center, 0), |_, q| q.world == world);
            let wooden = !matches!(short(&name), "stone_button" | "polished_blackstone_button");
            self.schedule_redstone(world, pos, if wooden { 30 } else { 20 });
            self.redstone_update(world, pos);
        }
        true
    }

    /// Scheduled looks and pressure plates.
    pub(crate) fn tick_redstone(&mut self) {
        let now = self.ticks;
        if !self.redstone.scheduled.is_empty() {
            let due: Vec<(usize, [i32; 3])> = self.redstone.scheduled.iter().filter(|(t, _, _)| *t <= now).map(|(_, w, p)| (*w, *p)).collect();
            self.redstone.scheduled.retain(|(t, _, _)| *t > now);
            for (w, p) in due {
                self.scheduled_redstone(w, p);
            }
        }
        if now % 4 == 0 {
            self.tick_plates();
        }
        if now % 20 == 0 && !self.redstone.sensors.is_empty() {
            self.tick_sensors();
        }
        if now % 4 == 2 && !self.redstone.comparators.is_empty() {
            let list: Vec<(usize, [i32; 3])> = self.redstone.comparators.keys().copied().collect();
            for (w, p) in list {
                let (id, name) = self.rs_block(w, p);
                if !is_comparator(&name) {
                    self.redstone.comparators.remove(&(w, p));
                    continue;
                }
                let have = self.redstone.comparators.get(&(w, p)).copied().unwrap_or(0);
                if self.comparator_output(w, p, id) != have {
                    self.schedule_redstone(w, p, 2);
                }
            }
        }
    }

    fn scheduled_redstone(&mut self, world: usize, pos: [i32; 3]) {
        let (id, name) = self.rs_block(world, pos);
        let s = short(&name).to_string();
        if is_comparator(&s) {
            let out = self.comparator_output(world, pos, id);
            let had = self.redstone.comparators.insert((world, pos), out).unwrap_or(-1);
            let name = if out > 0 { "minecraft:powered_comparator" } else { "minecraft:unpowered_comparator" };
            if let Some(n) = self.gd.with_name(id, name).and_then(|b| self.gd.with_states(b, &[("output_lit_bit", Tag::Byte((out > 0) as i8))])) {
                if n != id {
                    self.rs_set(world, pos, n);
                }
            }
            if had != out {
                self.redstone_update(world, pos);
            }
            return;
        }
        if super::dispenser::is_dispenser(&name) {
            self.dispense(world, pos);
            return;
        }
        let new = if s == "observer" {
            let on = self.rs_int(id, "powered_bit") == 0;
            if on {
                self.schedule_redstone(world, pos, 2);
            }
            self.gd.with_states(id, &[("powered_bit", Tag::Byte(on as i8))])
        } else if is_button(&s) && self.rs_int(id, "button_pressed_bit") != 0 {
            self.gd.with_states(id, &[("button_pressed_bit", Tag::Byte(0))])
        } else if s == "lit_redstone_lamp" && self.power_at(world, pos) == 0 {
            self.gd.with_name(id, "minecraft:redstone_lamp")
        } else if is_repeater(&s) {
            let on = s == "powered_repeater";
            let want = self.repeater_input(world, pos, id);
            if want != on && !self.repeater_locked(world, pos, id) {
                self.gd.with_name(id, if want { "minecraft:powered_repeater" } else { "minecraft:unpowered_repeater" })
            } else {
                None
            }
        } else if s == "redstone_torch" || s == "unlit_redstone_torch" {
            let lit = self.torch_should_light(world, pos, id);
            if lit != (s == "redstone_torch") {
                self.gd.with_name(id, if lit { "minecraft:redstone_torch" } else { "minecraft:unlit_redstone_torch" })
            } else {
                None
            }
        } else {
            None
        };
        if let Some(n) = new.filter(|n| *n != id) {
            self.rs_set(world, pos, n);
            if is_button(&s) {
                let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
                self.broadcast(&protocol::level_event(level_event::SOUND_CLICK, center, 0), |_, q| q.world == world);
            }
            self.redstone_update(world, pos);
        }
    }

    /// Pressure plates under players and mobs: wooden and stone ones give 15, the weighted
    /// ones more the more stand on them (light: one per entity, heavy: one per 10); a plate
    /// nobody stood on for a second lets go.
    fn tick_plates(&mut self) {
        let mut on: HashMap<(usize, [i32; 3]), i32> = HashMap::new();
        let mut feet: Vec<(usize, [f32; 3])> = self
            .players
            .values()
            .filter(|p| p.state == State::Spawned && !p.dead && p.gamemode != 6)
            .map(|p| (p.world, [p.pos[0], p.pos[1] - 1.62, p.pos[2]]))
            .collect();
        feet.extend(self.mobs.iter().filter(|m| m.alive()).map(|m| (m.world, m.pos)));
        let carts: Vec<(usize, [f32; 3])> = self.mobs.iter().filter(|m| m.alive() && m.id().contains("minecart")).map(|m| (m.world, m.pos)).collect();
        for (w, f) in feet {
            let b = [f[0].floor() as i32, (f[1] + 0.05).floor() as i32, f[2].floor() as i32];
            if is_plate(&self.rs_block(w, b).1) {
                *on.entry((w, b)).or_insert(0) += 1;
            }
        }
        // detector rails under a minecart
        for (w, f) in carts {
            let b = [f[0].floor() as i32, (f[1] + 0.05).floor() as i32, f[2].floor() as i32];
            let (id, name) = self.rs_block(w, b);
            if name != "minecraft:detector_rail" {
                continue;
            }
            self.redstone.plates.insert((w, b), self.ticks);
            if self.rs_int(id, "rail_data_bit") == 0 {
                if let Some(new) = self.gd.with_states(id, &[("rail_data_bit", Tag::Byte(1))]) {
                    self.rs_set(w, b, new);
                    self.redstone_update(w, b);
                }
            }
        }
        let now = self.ticks;
        for ((w, p), count) in on {
            let (id, name) = self.rs_block(w, p);
            let signal = match short(&name) {
                "light_weighted_pressure_plate" => count.min(15),
                "heavy_weighted_pressure_plate" => ((count + 9) / 10).min(15),
                _ => 15,
            };
            self.redstone.plates.insert((w, p), now);
            if self.rs_int(id, "redstone_signal") as i32 != signal {
                if let Some(new) = self.gd.with_states(id, &[("redstone_signal", Tag::Int(signal))]) {
                    self.rs_set(w, p, new);
                    let at = [p[0] as f32 + 0.5, p[1] as f32 + 0.1, p[2] as f32 + 0.5];
                    self.broadcast(&protocol::level_event(level_event::SOUND_CLICK, at, 0), |_, q| q.world == w);
                    self.redstone_update(w, p);
                }
            }
        }
        let gone: Vec<(usize, [i32; 3])> = self.redstone.plates.iter().filter(|(_, t)| now.saturating_sub(**t) >= 20).map(|(k, _)| *k).collect();
        for (w, p) in gone {
            self.redstone.plates.remove(&(w, p));
            let (id, name) = self.rs_block(w, p);
            if is_plate(&name) && self.rs_int(id, "redstone_signal") != 0 {
                if let Some(new) = self.gd.with_states(id, &[("redstone_signal", Tag::Int(0))]) {
                    self.rs_set(w, p, new);
                    self.redstone_update(w, p);
                }
            }
            if name == "minecraft:detector_rail" && self.rs_int(id, "rail_data_bit") != 0 {
                if let Some(new) = self.gd.with_states(id, &[("rail_data_bit", Tag::Byte(0))]) {
                    self.rs_set(w, p, new);
                    self.redstone_update(w, p);
                }
            }
        }
    }
}

impl Server {
    /// Whether power comes into a repeater from behind (PowerNukkitX's `calculateInputStrength`:
    /// the power from that side, or the signal of dust there).
    fn repeater_input(&mut self, world: usize, pos: [i32; 3], id: i32) -> bool {
        let f = cardinal(&self.rs_str(id, "minecraft:cardinal_direction"));
        if self.power_from(world, pos, f) > 0 {
            return true;
        }
        let (bid, name) = self.rs_block(world, add(pos, f));
        name == "minecraft:redstone_wire" && self.rs_int(bid, "redstone_signal") > 0
    }

    /// A repeater is locked while a powered repeater points into its side.
    fn repeater_locked(&mut self, world: usize, pos: [i32; 3], id: i32) -> bool {
        let f = cardinal(&self.rs_str(id, "minecraft:cardinal_direction"));
        for d in perpendicular(f) {
            let n = add(pos, d);
            let (nid, name) = self.rs_block(world, n);
            // the side repeater's output side faces us: its facing is `d` seen from it
            if name == "minecraft:powered_repeater" && cardinal(&self.rs_str(nid, "minecraft:cardinal_direction")) == d {
                return true;
            }
        }
        false
    }

    /// Use on a repeater: the next delay (1 to 4 redstone ticks).
    pub(crate) fn use_repeater(&mut self, sid: SessionId, pos: [i32; 3]) -> bool {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return false };
        let (id, name) = self.rs_block(world, pos);
        if !is_repeater(&name) {
            return false;
        }
        let next = (self.rs_int(id, "repeater_delay") + 1) % 4;
        if let Some(new) = self.gd.with_states(id, &[("repeater_delay", Tag::Int(next as i32))]) {
            self.rs_set(world, pos, new);
        }
        self.swing_arm(sid);
        true
    }
}

impl Server {
    /// Daylight sensors follow the sun (0 at night, 15 around noon; inverted ones the other
    /// way round), under open sky only; using one swaps it between the two kinds.
    fn tick_sensors(&mut self) {
        let list: Vec<(usize, [i32; 3])> = self.redstone.sensors.iter().copied().collect();
        for (w, p) in list {
            let (id, name) = self.rs_block(w, p);
            if !name.starts_with("minecraft:daylight_detector") {
                self.redstone.sensors.remove(&(w, p));
                continue;
            }
            let t = self.worlds.worlds[w].time.rem_euclid(24000) as f32;
            let mut sky = true;
            for y in (p[1] + 1)..(p[1] + 64).min(ceres_world::MAX_Y + 1) {
                if self.conductor(w, [p[0], y, p[2]]) {
                    sky = false;
                    break;
                }
            }
            let mut day = if sky && t < 12000.0 { ((t / 12000.0 * std::f32::consts::PI).sin() * 1.25 * 15.0).round().clamp(0.0, 15.0) as i32 } else { 0 };
            if sky && self.raining(w) {
                day = (day - 3).max(0);
            }
            let signal = if name.ends_with("_inverted") { 15 - day } else { day };
            if self.rs_int(id, "redstone_signal") as i32 != signal {
                if let Some(new) = self.gd.with_states(id, &[("redstone_signal", Tag::Int(signal))]) {
                    self.rs_set(w, p, new);
                    self.redstone_update(w, p);
                }
            }
        }
    }

    /// Use on a daylight sensor: it turns into the inverted one (or back).
    pub(crate) fn use_daylight_sensor(&mut self, sid: SessionId, pos: [i32; 3]) -> bool {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return false };
        let (id, name) = self.rs_block(world, pos);
        let other = match name.as_str() {
            "minecraft:daylight_detector" => "minecraft:daylight_detector_inverted",
            "minecraft:daylight_detector_inverted" => "minecraft:daylight_detector",
            _ => return false,
        };
        if let Some(new) = self.gd.with_name(id, other) {
            self.rs_set(world, pos, new);
            self.redstone.sensors.insert((world, pos));
            self.redstone_update(world, pos);
        }
        self.swing_arm(sid);
        true
    }
}

impl Server {
    /// How full a container is as a signal: 0 empty, 1 to 15 by how full its slots are
    /// (each slot counts its items over its stack size).
    fn container_signal(&mut self, world: usize, pos: [i32; 3]) -> Option<i32> {
        // never reads (or loads) a chunk that is not loaded
        if pos[1] < ceres_world::MIN_Y || pos[1] > ceres_world::MAX_Y || !self.worlds.worlds[world].is_loaded(pos) {
            return None;
        }
        let (_, _, items) = self.stored_items(world, pos)?;
        if items.is_empty() {
            return Some(0);
        }
        let mut full = 0.0f32;
        let mut any = false;
        for it in &items {
            if it.is_empty() {
                continue;
            }
            any = true;
            full += it.count as f32 / self.gd.max_stack(it.id).max(1) as f32;
        }
        Some(if any { (1.0 + full / items.len() as f32 * 14.0).floor() as i32 } else { 0 })
    }

    /// A comparator's signal (PowerNukkitX's BlockRedstoneComparator): the power from behind, or
    /// how full the container behind it (or behind the block behind it) is; compared with the
    /// strongest side input, or that taken away in subtract mode.
    fn comparator_output(&mut self, world: usize, pos: [i32; 3], id: i32) -> i32 {
        let f = cardinal(&self.rs_str(id, "minecraft:cardinal_direction"));
        let back = add(pos, f);
        let mut input = match self.container_signal(world, back) {
            Some(v) => v,
            None => {
                let mut v = self.power_from(world, pos, f);
                let (bid, bname) = self.rs_block(world, back);
                if bname == "minecraft:redstone_wire" {
                    v = v.max(self.rs_int(bid, "redstone_signal") as i32);
                }
                if v < 15 && self.conductor(world, back) {
                    if let Some(c) = self.container_signal(world, add(back, f)) {
                        v = v.max(c);
                    }
                }
                v
            }
        };
        input = input.clamp(0, 15);
        let mut side = 0;
        for d in perpendicular(f) {
            let n = add(pos, d);
            let (nid, name) = self.rs_block(world, n);
            let v = match short(&name) {
                "redstone_wire" => self.rs_int(nid, "redstone_signal") as i32,
                "redstone_block" => 15,
                _ if is_repeater(&name) || is_comparator(&name) => self.emitted(world, n, d).0,
                _ => 0,
            };
            side = side.max(v);
        }
        if self.rs_int(id, "output_subtract_bit") != 0 {
            (input - side).max(0)
        } else if input >= side {
            input
        } else {
            0
        }
    }

    /// Use on a comparator: compare and subtract modes swap.
    pub(crate) fn use_comparator(&mut self, sid: SessionId, pos: [i32; 3]) -> bool {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return false };
        let (id, name) = self.rs_block(world, pos);
        if !is_comparator(&name) {
            return false;
        }
        let sub = self.rs_int(id, "output_subtract_bit") == 0;
        if let Some(new) = self.gd.with_states(id, &[("output_subtract_bit", Tag::Byte(sub as i8))]) {
            self.rs_set(world, pos, new);
            self.redstone.comparators.entry((world, pos)).or_insert(0);
            self.schedule_redstone(world, pos, 2);
        }
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        self.broadcast(&protocol::level_event(level_event::SOUND_CLICK, center, 0), |_, q| q.world == world);
        self.swing_arm(sid);
        true
    }
}
