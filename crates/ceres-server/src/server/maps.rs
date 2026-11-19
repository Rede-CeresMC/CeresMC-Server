//! Maps: an empty map used in the air becomes a filled map (`map_uuid` in its NBT) of the
//! 128x128 area around the player (scale 0: one block per pixel, aligned to the vanilla grid).
//! The client asks for the image with MapInfoRequest (0x44) and gets ClientboundMapItemData
//! (0x43); while a player holds the map inside its area, the part around them is drawn again.
//! Images are kept in `<world 0>/maps/<id>.map`.
//! A map extended (map + 8 paper) is a new map one scale up, centred on the old one's grid cell;
//! it is drawn the first time it is held or asked for (its item carries the old centre).
//!
//! Colours: the vanilla base map colours, chosen for each block by its name, shaded by the
//! height of the block to the north (as vanilla: higher brighter, lower darker); water is darker
//! the deeper it is.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::ItemDesc;
use std::collections::HashMap;

const SIZE: usize = 128;

pub(crate) struct MapData {
    pub world: usize,
    /// north-west corner of the area
    pub corner: [i32; 2],
    /// 0-4: each pixel covers 2^scale blocks
    pub scale: u8,
    pub pixels: Vec<[u8; 4]>,
    /// locked on a cartography table: never drawn again
    pub locked: bool,
    /// locator map: shows where its carriers are
    pub players: bool,
    dirty: bool,
}

/// Flags kept in the scale byte of a map file (old files: none set, players shown).
const FILE_LOCKED: u8 = 0x10;
const FILE_NO_PLAYERS: u8 = 0x20;

#[derive(Default)]
pub(crate) struct Maps {
    pub maps: HashMap<i64, MapData>,
}

/// Vanilla base map colours (index = map colour id).
const BASE: [[u8; 3]; 62] = [
    [0, 0, 0],
    [127, 178, 56],
    [247, 233, 163],
    [199, 199, 199],
    [255, 0, 0],
    [160, 160, 255],
    [167, 167, 167],
    [0, 124, 0],
    [255, 255, 255],
    [164, 168, 184],
    [151, 109, 77],
    [112, 112, 112],
    [64, 64, 255],
    [143, 119, 72],
    [255, 252, 245],
    [216, 127, 51],
    [178, 76, 216],
    [102, 153, 216],
    [229, 229, 51],
    [127, 204, 25],
    [242, 127, 165],
    [76, 76, 76],
    [153, 153, 153],
    [76, 127, 153],
    [127, 63, 178],
    [51, 76, 178],
    [102, 76, 51],
    [102, 127, 51],
    [153, 51, 51],
    [25, 25, 25],
    [250, 238, 77],
    [92, 219, 213],
    [74, 128, 255],
    [0, 217, 58],
    [129, 86, 49],
    [112, 2, 0],
    [209, 177, 161],
    [159, 82, 36],
    [149, 87, 108],
    [112, 108, 138],
    [186, 133, 36],
    [103, 117, 53],
    [160, 77, 78],
    [57, 41, 35],
    [135, 107, 98],
    [87, 92, 92],
    [122, 73, 88],
    [76, 62, 92],
    [76, 50, 35],
    [76, 82, 42],
    [142, 60, 46],
    [37, 22, 16],
    [189, 48, 49],
    [148, 63, 97],
    [92, 25, 29],
    [22, 126, 134],
    [58, 142, 140],
    [86, 44, 62],
    [20, 180, 133],
    [100, 100, 100],
    [216, 175, 147],
    [127, 167, 150],
];

const WATER: u8 = 12;
const DYES: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green",
    "red", "black",
];

/// Map colour of a block by its name; None: the map sees through it.
fn colour_of(name: &str) -> Option<u8> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let has = |s: &str| n.contains(s);
    // see-through
    if matches!(n, "air" | "light_block" | "barrier" | "structure_void" | "glass" | "glass_pane" | "tripwire" | "cave_air" | "void_air")
        || n.starts_with("light_block")
        || ["torch", "rail", "button", "pressure_plate", "lever", "redstone_wire", "string", "_sign", "banner", "flower_pot", "ladder", "_head", "skull", "frame", "candle"]
            .iter()
            .any(|s| has(s))
    {
        return None;
    }
    if has("water") || has("kelp") || has("seagrass") || n == "bubble_column" {
        return Some(WATER);
    }
    if has("lava") || has("fire") || n == "magma" {
        return Some(4);
    }
    // dyed blocks
    for (i, d) in DYES.iter().enumerate().rev() {
        if n.starts_with(&format!("{d}_")) {
            if has("terracotta") {
                return Some(36 + i as u8);
            }
            if has("wool") || has("carpet") || has("concrete") || has("stained_glass") || has("shulker_box") || has("bed") || has("candle") {
                return Some(if i == 0 { 8 } else { 14 + i as u8 });
            }
        }
    }
    Some(match n {
        "grass_block" | "slime" => 1,
        "terracotta" | "hardened_clay" | "pumpkin" | "carved_pumpkin" | "lit_pumpkin" | "honey_block" => 15,
        "clay" => 9,
        "podzol" => 34,
        "netherrack" | "nether_brick" | "nether_wart_block" => 35,
        "iron_block" | "anvil" | "brewing_stand" | "heavy_weighted_pressure_plate" => 6,
        "gold_block" => 30,
        "diamond_block" => 31,
        "lapis_block" => 32,
        "emerald_block" => 33,
        "glow_lichen" => 61,
        "raw_iron_block" => 60,
        "crimson_nylium" => 52,
        "warped_nylium" => 55,
        "warped_wart_block" => 58,
        "melon_block" => 19,
        "obsidian" | "crying_obsidian" | "coal_block" => 29,
        "soul_sand" | "soul_soil" => 26,
        _ if has("snow") => 8,
        _ if has("ice") => 5,
        _ if has("leaves") || has("vine") || has("fern") || has("grass") || has("flower") || has("sapling") || has("bush") || has("tulip")
            || has("wheat") || has("carrots") || has("potatoes") || has("beetroot") || has("cactus") || has("reeds") || has("sugar_cane")
            || has("bamboo") || has("waterlily") || has("lily_pad") || has("dandelion") || has("poppy") || has("orchid") || has("allium")
            || has("daisy") || has("cornflower") || has("azalea") || has("dripleaf") || has("moss") =>
        {
            7
        }
        _ if has("deepslate") => 59,
        _ if has("quartz") => 14,
        _ if has("sand") || has("end_stone") || has("glowstone") || has("birch") || has("bone_block") => 2,
        _ if has("dirt") || has("farmland") || has("granite") || has("jungle") || has("mud") => 10,
        _ if has("spruce") || has("mangrove_roots") => 34,
        _ if has("dark_oak") => 26,
        _ if has("acacia") || has("copper") => 15,
        _ if has("mangrove") => 28,
        _ if has("cherry") => 36,
        _ if has("crimson") => 53,
        _ if has("warped") => 56,
        _ if has("oak") || has("planks") || has("log") || has("wood") || has("bookshelf") || has("chest") || has("crafting_table") || has("barrel") || has("fence") => 13,
        _ if has("wool") || has("carpet") || has("web") => 3,
        _ if has("nether") || has("blackstone") || has("basalt") => 35,
        _ => 11, // stone and the rest
    })
}

fn shade(c: u8, k: u32) -> [u8; 4] {
    if c == 0 {
        return [0, 0, 0, 0];
    }
    let b = BASE[c as usize];
    [(b[0] as u32 * k / 255) as u8, (b[1] as u32 * k / 255) as u8, (b[2] as u32 * k / 255) as u8, 255]
}

impl Server {
    /// The top block of a column the map shows: (y, colour, water depth).
    fn map_column(&mut self, world: usize, x: i32, z: i32) -> Option<(i32, u8, i32)> {
        let w = self.worlds.worlds.get_mut(world)?;
        if !w.is_loaded([x, 0, z]) {
            return None;
        }
        let mut y = ceres_world::MAX_Y;
        while y > ceres_world::MIN_Y {
            let id = w.get_block([x, y, z], &self.gd);
            if id != self.gd.air_id {
                if let Some(c) = colour_of(self.gd.block_name(id)) {
                    if c == WATER {
                        let mut depth = 1;
                        while depth < 16 {
                            let below = w.get_block([x, y - depth, z], &self.gd);
                            if colour_of(self.gd.block_name(below)) != Some(WATER) {
                                break;
                            }
                            depth += 1;
                        }
                        return Some((y, c, depth));
                    }
                    return Some((y, c, 0));
                }
            }
            y -= 1;
        }
        Some((y, 0, 0))
    }

    /// Draws the part of a map within `radius` blocks of `around` (all of it when None).
    /// True when a pixel changed.
    fn draw_map(&mut self, id: i64, around: Option<([i32; 2], i32)>) -> bool {
        let Some(m) = self.maps.maps.get(&id) else { return false };
        let (world, corner, step) = (m.world, m.corner, 1i32 << m.scale);
        let wanted = |x: i32, z: i32| match around {
            Some((c, r)) => {
                let (dx, dz) = (x - c[0], z - c[1]);
                dx * dx + dz * dz <= r * r
            }
            None => true,
        };
        // each column scanned once: the map rows plus the row north of the first one
        let mut cols: HashMap<(i32, i32), Option<(i32, u8, i32)>> = HashMap::new();
        let mut changed = Vec::new();
        for pz in 0..SIZE as i32 {
            let z = corner[1] + pz * step;
            for px in 0..SIZE as i32 {
                let x = corner[0] + px * step;
                if !wanted(x, z) {
                    continue;
                }
                let here = match cols.get(&(x, z)) {
                    Some(v) => *v,
                    None => {
                        let v = self.map_column(world, x, z);
                        cols.insert((x, z), v);
                        v
                    }
                };
                let Some((y, col, depth)) = here else { continue };
                let k = if col == WATER {
                    match depth {
                        0..=2 => 255,
                        3..=6 => 220,
                        _ => 180,
                    }
                } else {
                    let north = match cols.get(&(x, z - step)) {
                        Some(v) => *v,
                        None => {
                            let v = self.map_column(world, x, z - step);
                            cols.insert((x, z - step), v);
                            v
                        }
                    };
                    match north {
                        Some((ny, _, _)) if y > ny => 255,
                        Some((ny, _, _)) if y < ny => 180,
                        _ => 220,
                    }
                };
                changed.push(((pz as usize) * SIZE + px as usize, shade(col, k)));
            }
        }
        let Some(m) = self.maps.maps.get_mut(&id) else { return false };
        let mut any = false;
        for (i, p) in changed {
            if m.pixels[i] != p {
                m.pixels[i] = p;
                any = true;
            }
        }
        m.dirty |= any;
        any
    }

    fn maps_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.worlds.worlds.first()?.dir.join("maps"))
    }

    /// A map from memory or from disk.
    fn load_map(&mut self, id: i64) -> bool {
        if self.maps.maps.contains_key(&id) {
            return true;
        }
        let Some(path) = self.maps_dir().map(|d| d.join(format!("{id}.map"))) else { return false };
        let Ok(b) = std::fs::read(&path) else { return false };
        // old files have no scale byte
        let (scale, px_at) = match b.len() {
            n if n == 12 + SIZE * SIZE * 4 => (0, 12),
            n if n == 13 + SIZE * SIZE * 4 => (b[12], 13),
            _ => return false,
        };
        let int = |i: usize| i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let world = int(0).max(0) as usize;
        if world >= self.worlds.worlds.len() {
            return false;
        }
        let pixels = b[px_at..].chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        let (locked, players) = (scale & FILE_LOCKED != 0, scale & FILE_NO_PLAYERS == 0);
        let scale = (scale & 0x0f).min(4);
        self.maps.maps.insert(id, MapData { world, corner: [int(4), int(8)], scale, pixels, locked, players, dirty: false });
        true
    }

    /// Writes changed maps to disk.
    pub(crate) fn save_maps(&mut self) {
        let Some(dir) = self.maps_dir() else { return };
        for (id, m) in self.maps.maps.iter_mut().filter(|(_, m)| m.dirty) {
            if std::fs::create_dir_all(&dir).is_err() {
                return;
            }
            let mut b = Vec::with_capacity(12 + SIZE * SIZE * 4);
            b.extend((m.world as i32).to_le_bytes());
            b.extend(m.corner[0].to_le_bytes());
            b.extend(m.corner[1].to_le_bytes());
            b.push(m.scale | if m.locked { FILE_LOCKED } else { 0 } | if m.players { 0 } else { FILE_NO_PLAYERS });
            for p in &m.pixels {
                b.extend(p);
            }
            let tmp = dir.join(format!("{id}.map.tmp"));
            if std::fs::write(&tmp, &b).is_ok() && std::fs::rename(&tmp, dir.join(format!("{id}.map"))).is_ok() {
                m.dirty = false;
            }
        }
    }

    fn map_packet(&self, id: i64) -> Option<Vec<u8>> {
        let m = self.maps.maps.get(&id)?;
        let half = 64 << m.scale;
        let center = [m.corner[0] + half, 0, m.corner[1] + half];
        Some(protocol::map_item_data(id, 0, center, m.scale, m.locked, &m.pixels))
    }

    /// The players who carry this map, inside its area and not invisible, as white markers
    /// pointing where they look.
    fn map_marks_packet(&self, id: i64) -> Option<Vec<u8>> {
        let m = self.maps.maps.get(&id)?;
        if !m.players {
            return None;
        }
        let half = 64 << m.scale;
        let step = (1 << m.scale) as f32;
        let center = [m.corner[0] + half, m.corner[1] + half];
        let marks: Vec<(u8, u8, i8, i8)> = self
            .players
            .values()
            .filter(|p| p.state == State::Spawned && !p.dead && p.world == m.world && p.gamemode != 6)
            .filter(|p| !p.effects.contains_key(&INVISIBILITY) && carries_map(p, id))
            .filter_map(|p| {
                let dx = ((p.pos[0] - center[0] as f32) * 2.0 / step).round();
                let dz = ((p.pos[2] - center[1] as f32) * 2.0 / step).round();
                if !(-128.0..=127.0).contains(&dx) || !(-128.0..=127.0).contains(&dz) {
                    return None;
                }
                let rot = ((p.rot[1].rem_euclid(360.0) * 16.0 / 360.0).round() as u8) % 16;
                Some((0, rot, dx as i8, dz as i8))
            })
            .collect();
        Some(protocol::map_decorations(id, 0, [center[0], 0, center[1]], m.scale, &marks))
    }

    /// An empty map used: a new filled map of the area around the player.
    pub(crate) fn create_map(&mut self, sid: SessionId) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm, pos) = (p.world, p.gamemode, p.pos);
        // an empty locator map (data 2) makes a map that shows where its carriers are
        let players = p.inventory.held_item().meta == super::cartography::LOCATOR;
        // one new map per second per player (each one is drawn in full and saved)
        let key = (sid, "minecraft:empty_map".to_string());
        if self.item_cooldowns.get(&key).is_some_and(|t| self.ticks < *t) {
            return true;
        }
        self.item_cooldowns.insert(key, self.ticks + 20);
        let corner = map_corner([pos[0].floor() as i32, pos[2].floor() as i32], 0);
        let mut id = (rand::random::<u32>() as i64) | 1 << 32;
        if self.load_map(id) {
            id += 1;
        }
        self.maps.maps.insert(id, MapData { world, corner, scale: 0, pixels: vec![[0, 0, 0, 0]; SIZE * SIZE], locked: false, players, dirty: true });
        self.draw_map(id, None);
        let Some(mut item) = self.simple_item("minecraft:filled_map", 1) else { return false };
        if players {
            item.meta = super::cartography::LOCATOR;
        }
        durability::set_item_field(&mut item, "map_display_players", Some(Tag::Byte(players as i8)));
        durability::set_item_field(&mut item, "map_uuid", Some(Tag::Long(id)));
        durability::set_item_field(&mut item, "map_scale", Some(Tag::Int(0)));
        durability::set_item_field(&mut item, "map_is_init", Some(Tag::Byte(1)));
        // the centre, so the map can be extended without the server's map store
        durability::set_item_field(&mut item, "ceres_map_x", Some(Tag::Int(corner[0] + 64)));
        durability::set_item_field(&mut item, "ceres_map_z", Some(Tag::Int(corner[1] + 64)));
        if gm != 1 {
            self.consume_held(sid, gm);
        }
        self.give_stack(sid, item);
        if let Some(pk) = self.map_packet(id) {
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(pk);
            }
        }
        self.swing_arm(sid);
        true
    }

    /// MapInfoRequest: the client wants the image of a map.
    /// Only for a map the player carries or sees in an item frame within 64 blocks (a map shows
    /// where its carriers are).
    pub(crate) fn on_map_request(&mut self, sid: SessionId, id: i64) {
        let Some(p) = self.players.get(&sid) else { return };
        let (world, pos) = (p.world, p.pos);
        if !carries_map(p, id) && !self.map_in_frame_near(world, pos, id) {
            return;
        }
        if !self.load_map(id) {
            let Some(p) = self.players.get(&sid) else { return };
            let (world, held) = (p.world, p.inventory.held_item().clone());
            if !self.init_extended_map(id, world, &held) {
                return;
            }
        }
        for pk in [self.map_packet(id), self.map_marks_packet(id)].into_iter().flatten() {
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(pk);
            }
        }
    }

    /// Players holding a map inside its area see it filled in around them.
    pub(crate) fn tick_maps(&mut self) {
        if self.ticks % 20 != 0 {
            return;
        }
        // every minute: saved maps that nobody holds leave memory (they load again when asked)
        if self.ticks % 1200 == 0 && !self.maps.maps.is_empty() {
            self.save_maps();
            let held: std::collections::HashSet<i64> = self
                .players
                .values()
                .filter_map(|p| match durability::item_field(p.inventory.held_item(), "map_uuid") {
                    Some(Tag::Long(id)) => Some(id),
                    _ => None,
                })
                .collect();
            self.maps.maps.retain(|id, m| m.dirty || held.contains(id));
        }
        let filled = self.gd.find_item("minecraft:filled_map").map(|i| i.id);
        let holders: Vec<(SessionId, usize, [i32; 2], i64, ItemDesc)> = self
            .players
            .iter()
            .filter(|(_, p)| p.state == State::Spawned && !p.dead)
            .filter_map(|(s, p)| {
                let held = p.inventory.held_item();
                if Some(held.id) != filled {
                    return None;
                }
                let Some(Tag::Long(id)) = durability::item_field(held, "map_uuid") else { return None };
                Some((*s, p.world, [p.pos[0].floor() as i32, p.pos[2].floor() as i32], id, held.clone()))
            })
            .collect();
        for (sid, world, at, id, held) in holders {
            if !self.load_map(id) && !self.init_extended_map(id, world, &held) {
                continue;
            }
            let Some(m) = self.maps.maps.get(&id) else { continue };
            if m.locked {
                continue;
            }
            let span = (SIZE as i32) << m.scale;
            let radius = (32 << m.scale).min(64);
            let inside = m.world == world && (0..span).contains(&(at[0] - m.corner[0])) && (0..span).contains(&(at[1] - m.corner[1]));
            if inside && self.draw_map(id, Some((at, radius))) {
                if let Some(pk) = self.map_packet(id) {
                    if let Some(p) = self.players.get_mut(&sid) {
                        p.out.push(pk);
                    }
                }
            }
            if let Some(pk) = self.map_marks_packet(id) {
                if let Some(p) = self.players.get_mut(&sid) {
                    p.out.push(pk);
                }
            }
        }
    }
}

/// North-west corner of the vanilla map cell holding `at` at a scale: cells of 128 x 2^scale
/// blocks, the scale 0 ones centred on multiples of 128.
pub(crate) fn map_corner(at: [i32; 2], scale: u8) -> [i32; 2] {
    let size = 128i32 << scale;
    let c = |v: i32| (v + 64).div_euclid(size) * size - 64;
    [c(at[0]), c(at[1])]
}

impl Server {
    /// A map made by extending another (its item has `map_scale` and the old centre in
    /// `ceres_map_x`/`ceres_map_z`): created and drawn the first time it is seen.
    fn init_extended_map(&mut self, id: i64, world: usize, item: &ItemDesc) -> bool {
        if durability::item_field(item, "map_uuid") != Some(Tag::Long(id)) {
            return false;
        }
        // a copy locked on the cartography table: the image of its source, frozen
        if let Some(Tag::Long(src)) = durability::item_field(item, "ceres_lock_of") {
            if !self.load_map(src) {
                return false;
            }
            let Some(s) = self.maps.maps.get(&src) else { return false };
            let m = MapData { world: s.world, corner: s.corner, scale: s.scale, pixels: s.pixels.clone(), locked: true, players: false, dirty: true };
            self.maps.maps.insert(id, m);
            return true;
        }
        let players = !matches!(durability::item_field(item, "map_display_players"), Some(Tag::Byte(0)));
        let int = |k: &str| match durability::item_field(item, k) {
            Some(Tag::Int(v)) => Some(v),
            _ => None,
        };
        let (Some(scale), Some(x), Some(z)) = (int("map_scale"), int("ceres_map_x"), int("ceres_map_z")) else { return false };
        let scale = scale.clamp(0, 4) as u8;
        let corner = map_corner([x, z], scale);
        self.maps.maps.insert(id, MapData { world, corner, scale, pixels: vec![[0, 0, 0, 0]; SIZE * SIZE], locked: false, players, dirty: true });
        self.draw_map(id, Some(([x, z], 64)));
        true
    }
}

const INVISIBILITY: u8 = 14;

fn map_id_of(item: &ItemDesc) -> Option<i64> {
    match durability::item_field(item, "map_uuid") {
        Some(Tag::Long(id)) => Some(id),
        _ => None,
    }
}

/// The player has this map in their inventory (hand, offhand, cursor included).
fn carries_map(p: &Player, id: i64) -> bool {
    let inv = &p.inventory;
    inv.main.iter().chain([&inv.offhand, &inv.cursor]).any(|it| !it.is_empty() && map_id_of(it) == Some(id))
}

impl Server {
    fn map_in_frame_near(&self, world: usize, pos: [f32; 3], id: i64) -> bool {
        let Some(w) = self.worlds.worlds.get(world) else { return false };
        let in_frame = |t: &Tag| match t.get("Item").and_then(|i| i.get("tag")).and_then(|t| t.get("map_uuid")) {
            Some(Tag::Long(v)) => *v == id,
            _ => false,
        };
        w.find_block_entities(in_frame).iter().any(|b| {
            let d = [b[0] as f32 + 0.5 - pos[0], b[1] as f32 + 0.5 - pos[1], b[2] as f32 + 0.5 - pos[2]];
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= 64.0 * 64.0
        })
    }
}
