//! Vanilla (LevelDB) worlds, several loaded at once, plus flat/void world creation, safe spawn,
//! block entities (0x31) and level.dat persistence.

use crate::data::GameData;
use ceres_codec::binary::{R, Res, W};
use ceres_codec::nbt::{self, Flavor, Tag};
use ceres_protocol as protocol;
use rusty_leveldb::{CompressorList, DB, Options, WriteBatch};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

mod chunk;
mod column;
mod leveldb;
mod manager;
mod palette;

pub use manager::WorldManager;

use chunk::*;
use column::*;
use leveldb::*;
use palette::*;

const MIN_SUBCHUNK: i32 = -4;
const MAX_SUBCHUNK: i32 = 19;
const SUBCHUNK_COUNT: usize = (MAX_SUBCHUNK - MIN_SUBCHUNK + 1) as usize;
pub const MIN_Y: i32 = MIN_SUBCHUNK * 16;
pub const MAX_Y: i32 = MAX_SUBCHUNK * 16 + 15;

const TAG_DATA_3D: u8 = 0x2b;
const TAG_VERSION: u8 = 0x2c;
const TAG_SUBCHUNK: u8 = 0x2f;
const TAG_BLOCK_ENTITIES: u8 = 0x31;
const TAG_ENTITIES: u8 = 0x32;
const TAG_FINALIZATION: u8 = 0x36;
const TAG_VERSION_OLD: u8 = 0x76;
/// ChunkVersion::v1_21_120, what PocketMine-MP and vanilla currently write
const CHUNK_VERSION: u8 = 42;
const LEVEL_DAT_VERSION: i32 = 10;

const BIOME_PLAINS: i32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weather {
    Clear,
    Rain,
    Thunder,
}

impl GameData {
    /// Palette entry for a network id (unknown ids become the fallback block).
    pub(crate) fn entry(&self, id: i32) -> PaletteEntry {
        match self.state_nbt.get(&id) {
            Some(disk) => PaletteEntry { id, disk: disk.clone() },
            None => PaletteEntry { id: self.fallback_id, disk: self.state_nbt[&self.fallback_id].clone() },
        }
    }
}

pub struct World {
    pub dir: PathBuf,
    pub folder: String,
    pub display_name: String,
    pub spawn: [i32; 3],
    pub time: i64,
    pub do_daylight_cycle: bool,
    pub weather: Weather,
    pub weather_ticks: u32,
    pub flat: bool,
    pub void_world: bool,
    level_dat_version: i32,
    level_dat: Tag,
    level_dat_dirty: bool,
    db: DB,
    columns: HashMap<(i32, i32), Column>,
    /// compressed LevelChunk batch per chunk
    cache: HashMap<(i32, i32), Rc<Vec<u8>>>,
    /// for flat worlds: what an ungenerated chunk looks like
    flat_template: Option<Column>,
}

impl World {
    pub fn open(dir: &Path) -> Res<World> {
        let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let raw = std::fs::read(dir.join("level.dat")).map_err(|e| format!("level.dat: {e}"))?;
        if raw.len() < 8 {
            return Err("level.dat too short".into());
        }
        let level_dat_version = i32::from_le_bytes(raw[..4].try_into().unwrap());
        let (_, ld) = nbt::read_root(&mut R::new(&raw[8..]), Flavor::LittleEndian)?;
        let geti = |k: &str| ld.get(k).and_then(|t| t.as_i64());
        let spawn = [
            geti("SpawnX").unwrap_or(0) as i32,
            geti("SpawnY").unwrap_or(64) as i32,
            geti("SpawnZ").unwrap_or(0) as i32,
        ];
        let display_name = ld.get("LevelName").and_then(|t| t.as_str()).unwrap_or(&folder).to_string();
        let gen_name = ld.get("generatorName").and_then(|t| t.as_str()).unwrap_or("");
        let void_world = gen_name.eq_ignore_ascii_case("void");
        let flat = !void_world && (geti("Generator") == Some(2) || gen_name.eq_ignore_ascii_case("flat"));
        let do_daylight_cycle = geti("dodaylightcycle").map(|v| v != 0).unwrap_or(true);
        let db = DB::open(dir.join("db"), bedrock_options()).map_err(|e| format!("leveldb {}: {e}", dir.display()))?;
        Ok(World {
            dir: dir.to_path_buf(),
            folder,
            display_name,
            spawn,
            time: geti("Time").unwrap_or(0),
            do_daylight_cycle,
            weather: Weather::Clear,
            weather_ticks: 0,
            flat,
            void_world,
            level_dat_version,
            level_dat: ld,
            level_dat_dirty: false,
            db,
            columns: HashMap::new(),
            cache: HashMap::new(),
            flat_template: None,
        })
    }

    /// Creates a brand-new flat or void world on disk with `level.dat` and initial spawn chunk.
    pub fn create(dir: &Path, name: &str, void_world: bool, gd: &GameData) -> Res<World> {
        std::fs::create_dir_all(dir.join("db")).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        let spawn_y = if void_world { 65 } else { 4 };
        let ld = Tag::Compound(vec![
            ("LevelName".into(), Tag::String(name.to_string())),
            ("Generator".into(), Tag::Int(2)),
            ("generatorName".into(), Tag::String(if void_world { "void" } else { "flat" }.into())),
            ("SpawnX".into(), Tag::Int(0)),
            ("SpawnY".into(), Tag::Int(spawn_y)),
            ("SpawnZ".into(), Tag::Int(0)),
            ("Time".into(), Tag::Long(1000)),
            ("dodaylightcycle".into(), Tag::Byte(1)),
            ("StorageVersion".into(), Tag::Int(LEVEL_DAT_VERSION)),
        ]);
        write_level_dat_file(&dir.join("level.dat"), LEVEL_DAT_VERSION, &ld)?;
        let mut w = World::open(dir)?;
        if void_world {
            // Single bedrock platform at (0, 64, 0) so players don't fall into the void immediately
            let stone = gd.default_block("minecraft:stone").unwrap_or(gd.fallback_id);
            for dx in -2..=2 {
                for dz in -2..=2 {
                    w.set_block([dx, 64, dz], stone, gd);
                }
            }
        } else {
            let bedrock = gd.default_block("minecraft:bedrock").unwrap_or(gd.fallback_id);
            let dirt = gd.default_block("minecraft:dirt").unwrap_or(gd.fallback_id);
            let grass = gd.default_block("minecraft:grass_block").unwrap_or(dirt);
            for x in 0..16 {
                for z in 0..16 {
                    w.set_block([x, 0, z], bedrock, gd);
                    w.set_block([x, 1, z], dirt, gd);
                    w.set_block([x, 2, z], dirt, gd);
                    w.set_block([x, 3, z], grass, gd);
                }
            }
        }
        w.save(gd)?;
        Ok(w)
    }

    pub fn mark_level_dat_dirty(&mut self) {
        self.level_dat_dirty = true;
    }

    pub fn save_level_dat(&mut self) -> Res<()> {
        self.level_dat.set("SpawnX", Tag::Int(self.spawn[0]));
        self.level_dat.set("SpawnY", Tag::Int(self.spawn[1]));
        self.level_dat.set("SpawnZ", Tag::Int(self.spawn[2]));
        self.level_dat.set("Time", Tag::Long(self.time));
        self.level_dat.set("dodaylightcycle", Tag::Byte(self.do_daylight_cycle as i8));
        write_level_dat_file(&self.dir.join("level.dat"), self.level_dat_version, &self.level_dat)?;
        self.level_dat_dirty = false;
        Ok(())
    }

    /// Finds a safe feet Y near `self.spawn` (standing on solid ground with 2 non-solid blocks for body/head).
    pub fn safe_spawn(&mut self, gd: &GameData) -> [i32; 3] {
        let [sx, mut sy, sz] = self.spawn;
        if !(MIN_Y + 1..=MAX_Y - 2).contains(&sy) {
            sy = 100;
        }
        let clear_at = |w: &mut World, y: i32| {
            !gd.is_solid(w.get_block([sx, y, sz], gd)) && !gd.is_solid(w.get_block([sx, y + 1, sz], gd))
        };
        if gd.is_solid(self.get_block([sx, sy - 1, sz], gd)) && clear_at(self, sy) {
            return [sx, sy, sz];
        }
        // Scan down from top of world for the highest solid block with headroom
        for y in (MIN_Y + 1..=MAX_Y - 2).rev() {
            if gd.is_solid(self.get_block([sx, y - 1, sz], gd)) && clear_at(self, y) {
                return [sx, y, sz];
            }
        }
        [sx, sy, sz]
    }

    /// True if the column holding `pos` is in memory (it will not be loaded from disk).
    pub fn is_loaded(&self, pos: [i32; 3]) -> bool {
        self.columns.contains_key(&(pos[0] >> 4, pos[2] >> 4))
    }

    pub fn loaded_chunk_count(&self) -> usize {
        self.columns.len()
    }

    /// The column at chunk x/z, loading it (or making it from the flat template) if needed.
    fn column(&mut self, x: i32, z: i32, gd: &GameData) -> &mut Column {
        if !self.columns.contains_key(&(x, z)) {
            let col = match Column::load(&mut self.db, x, z, gd) {
                Ok(Some(c)) => c,
                Ok(None) => self.missing_column(gd),
                Err(e) => {
                    eprintln!("[World {}] chunk {x},{z} is corrupted: {e}", self.folder);
                    self.missing_column(gd)
                }
            };
            self.columns.insert((x, z), col);
        }
        self.columns.get_mut(&(x, z)).unwrap()
    }

    fn missing_column(&mut self, gd: &GameData) -> Column {
        if self.flat && !self.void_world {
            if self.flat_template.is_none() {
                let (sx, sz) = (self.spawn[0] >> 4, self.spawn[2] >> 4);
                if let Ok(Some(mut c)) = Column::load(&mut self.db, sx, sz, gd) {
                    c.block_entities.clear();
                    c.entities_raw = None;
                    self.flat_template = Some(c);
                }
            }
            if let Some(t) = &self.flat_template {
                let mut c = t.clone();
                c.on_disk = false;
                c.dirty = 0;
                c.block_entities_dirty = false;
                return c;
            }
        }
        Column::empty()
    }

    /// Ready-to-send batch payload (compression prefix included) for one chunk.
    pub fn chunk_batch(&mut self, x: i32, z: i32, gd: &GameData) -> Rc<Vec<u8>> {
        if let Some(b) = self.cache.get(&(x, z)) {
            return b.clone();
        }
        let (count, payload) = self.column(x, z, gd).network_payload(gd);
        let pk = protocol::level_chunk(x, z, count, &payload);
        let batch = Rc::new(protocol::encode_batch(&[pk], true));
        self.cache.insert((x, z), batch.clone());
        batch
    }

    pub fn get_block(&mut self, pos: [i32; 3], gd: &GameData) -> i32 {
        if !(MIN_Y..=MAX_Y).contains(&pos[1]) {
            return gd.air_id;
        }
        self.column(pos[0] >> 4, pos[2] >> 4, gd).get(pos[0], pos[1], pos[2], gd)
    }

    /// Changes a block. Returns false when the position is outside the world height.
    pub fn set_block(&mut self, pos: [i32; 3], id: i32, gd: &GameData) -> bool {
        if !(MIN_Y..=MAX_Y).contains(&pos[1]) {
            return false;
        }
        let key = (pos[0] >> 4, pos[2] >> 4);
        let changed = self.column(key.0, key.1, gd).set(pos[0], pos[1], pos[2], id, gd);
        if changed {
            self.cache.remove(&key);
        }
        changed
    }

    pub fn get_block_entity(&mut self, pos: [i32; 3], gd: &GameData) -> Option<Tag> {
        if !(MIN_Y..=MAX_Y).contains(&pos[1]) {
            return None;
        }
        self.column(pos[0] >> 4, pos[2] >> 4, gd).block_entities.get(&pos).cloned()
    }

    pub fn set_block_entity(&mut self, pos: [i32; 3], mut tag: Tag, gd: &GameData) {
        if !(MIN_Y..=MAX_Y).contains(&pos[1]) {
            return;
        }
        tag.set("x", Tag::Int(pos[0]));
        tag.set("y", Tag::Int(pos[1]));
        tag.set("z", Tag::Int(pos[2]));
        let key = (pos[0] >> 4, pos[2] >> 4);
        let col = self.column(key.0, key.1, gd);
        col.block_entities.insert(pos, tag);
        col.block_entities_dirty = true;
        self.cache.remove(&key);
    }

    /// Writes every changed chunk and dirty level.dat to disk. Returns how many chunks were written.
    pub fn save(&mut self, gd: &GameData) -> Res<usize> {
        let mut batch = WriteBatch::default();
        let mut n = 0;
        for ((x, z), col) in self.columns.iter_mut() {
            if col.is_dirty() {
                col.save(&mut batch, *x, *z, gd);
                n += 1;
            }
        }
        if n > 0 {
            self.db.write(batch, false).map_err(|e| format!("leveldb write: {e}"))?;
            self.db.flush().map_err(|e| format!("leveldb flush: {e}"))?;
        }
        if self.level_dat_dirty {
            let _ = self.save_level_dat();
        }
        Ok(n)
    }

    /// Forgets chunks nobody is near. Changed chunks stay until they have been saved.
    pub fn evict_except(&mut self, keep: impl Fn(i32, i32) -> bool) {
        self.cache.retain(|(x, z), _| keep(*x, *z));
        self.columns.retain(|(x, z), c| c.is_dirty() || keep(*x, *z));
    }
}

fn write_level_dat_file(path: &Path, version: i32, tag: &Tag) -> Res<()> {
    let payload = nbt::to_le(tag);
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&(payload.len() as i32).to_le_bytes());
    out.extend_from_slice(&payload);
    std::fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))
}
