//! Vanilla (LevelDB) worlds, several loaded at once.
//!
//! Chunks are decoded into editable columns (palette + indices per layer) when first needed. The
//! compressed LevelChunk batch of each chunk is cached and shared by every player who needs it,
//! and dropped as soon as a block in the chunk changes. Changed chunks are written back to LevelDB
//! by `save()`.

use crate::data::GameData;
use ceres_codec::binary::{R, Res, W};
use ceres_codec::nbt::{self, Flavor};
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
const TAG_FINALIZATION: u8 = 0x36;
const TAG_VERSION_OLD: u8 = 0x76;
/// ChunkVersion::v1_21_120, what PocketMine-MP and vanilla currently write
const CHUNK_VERSION: u8 = 42;

const BIOME_PLAINS: i32 = 1;

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
    pub folder: String,
    pub display_name: String,
    pub spawn: [i32; 3],
    pub time: i64,
    pub flat: bool,
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
        let level_dat = std::fs::read(dir.join("level.dat")).map_err(|e| format!("level.dat: {e}"))?;
        if level_dat.len() < 8 {
            return Err("level.dat too short".into());
        }
        let (_, ld) = nbt::read_root(&mut R::new(&level_dat[8..]), Flavor::LittleEndian)?;
        let geti = |k: &str| ld.get(k).and_then(|t| t.as_i64());
        let spawn = [
            geti("SpawnX").unwrap_or(0) as i32,
            geti("SpawnY").unwrap_or(64) as i32,
            geti("SpawnZ").unwrap_or(0) as i32,
        ];
        let display_name = ld.get("LevelName").and_then(|t| t.as_str()).unwrap_or(&folder).to_string();
        let flat = geti("Generator") == Some(2)
            || ld
                .get("generatorName")
                .and_then(|t| t.as_str())
                .map(|s| s.eq_ignore_ascii_case("flat"))
                .unwrap_or(false);
        let db = DB::open(dir.join("db"), bedrock_options()).map_err(|e| format!("leveldb {}: {e}", dir.display()))?;
        Ok(World {
            folder,
            display_name,
            spawn,
            time: geti("Time").unwrap_or(0),
            flat,
            db,
            columns: HashMap::new(),
            cache: HashMap::new(),
            flat_template: None,
        })
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
        if self.flat {
            if self.flat_template.is_none() {
                let (sx, sz) = (self.spawn[0] >> 4, self.spawn[2] >> 4);
                if let Ok(Some(c)) = Column::load(&mut self.db, sx, sz, gd) {
                    self.flat_template = Some(c);
                }
            }
            if let Some(t) = &self.flat_template {
                let mut c = t.clone();
                c.on_disk = false;
                c.dirty = 0;
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

    /// Writes every changed chunk to LevelDB. Returns how many chunks were written.
    pub fn save(&mut self, gd: &GameData) -> Res<usize> {
        let mut batch = WriteBatch::default();
        let mut n = 0;
        for ((x, z), col) in self.columns.iter_mut() {
            if col.dirty != 0 {
                col.save(&mut batch, *x, *z, gd);
                n += 1;
            }
        }
        if n > 0 {
            self.db.write(batch, false).map_err(|e| format!("leveldb write: {e}"))?;
            self.db.flush().map_err(|e| format!("leveldb flush: {e}"))?;
        }
        Ok(n)
    }

    /// Forgets chunks nobody is near. Changed chunks stay until they have been saved.
    pub fn evict_except(&mut self, keep: impl Fn(i32, i32) -> bool) {
        self.cache.retain(|(x, z), _| keep(*x, *z));
        self.columns.retain(|(x, z), c| c.dirty != 0 || keep(*x, *z));
    }
}
