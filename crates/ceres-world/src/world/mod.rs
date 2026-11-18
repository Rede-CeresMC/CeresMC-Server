//! Vanilla (LevelDB) worlds, several loaded at once.
//!
//! Chunks are converted straight from disk format to network format: the packed block words are
//! copied untouched and only the palette is remapped (disk NBT state -> hashed network id). The
//! final compressed LevelChunk batch is cached per chunk and shared by every player who needs it.

use crate::data::GameData;
use ceres_codec::binary::{R, Res, W};
use ceres_codec::nbt::{self, Flavor};
use ceres_protocol as protocol;
use rusty_leveldb::{CompressorList, DB, Options};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

mod chunk;
mod leveldb;
mod manager;

pub use manager::WorldManager;

use chunk::*;
use leveldb::*;

const MIN_SUBCHUNK: i32 = -4;
const MAX_SUBCHUNK: i32 = 19;
const SUBCHUNK_COUNT: usize = (MAX_SUBCHUNK - MIN_SUBCHUNK + 1) as usize;

const TAG_DATA_3D: u8 = 0x2b;
const TAG_VERSION: u8 = 0x2c;
const TAG_SUBCHUNK: u8 = 0x2f;
const TAG_VERSION_OLD: u8 = 0x76;

const BIOME_PLAINS: i32 = 1;

pub struct World {
    pub folder: String,
    pub display_name: String,
    pub spawn: [i32; 3],
    pub time: i64,
    pub flat: bool,
    db: DB,
    /// compressed LevelChunk batch per chunk
    cache: HashMap<(i32, i32), Rc<Vec<u8>>>,
    /// for flat worlds: what an ungenerated chunk looks like (payload + subchunk count)
    flat_template: Option<(u32, Rc<Vec<u8>>)>,
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
            cache: HashMap::new(),
            flat_template: None,
        })
    }

    pub fn has_chunk(&mut self, x: i32, z: i32) -> bool {
        self.db.get(&chunk_key(x, z, TAG_VERSION, None)).is_some()
            || self.db.get(&chunk_key(x, z, TAG_VERSION_OLD, None)).is_some()
    }

    /// Ready-to-send batch payload (compression prefix included) for one chunk.
    pub fn chunk_batch(&mut self, x: i32, z: i32, gd: &GameData) -> Rc<Vec<u8>> {
        if let Some(b) = self.cache.get(&(x, z)) {
            return b.clone();
        }
        let (count, payload) = match self.encode_chunk(x, z, gd) {
            Ok(Some(v)) => v,
            Ok(None) => self.missing_chunk(gd),
            Err(e) => {
                eprintln!("[World {}] chunk {x},{z} is corrupted: {e}", self.folder);
                self.missing_chunk(gd)
            }
        };
        let pk = protocol::level_chunk(x, z, count, &payload);
        let batch = Rc::new(protocol::encode_batch(&[pk], true));
        self.cache.insert((x, z), batch.clone());
        batch
    }

    pub fn evict_except(&mut self, keep: impl Fn(i32, i32) -> bool) {
        self.cache.retain(|(x, z), _| keep(*x, *z));
    }

    fn missing_chunk(&mut self, gd: &GameData) -> (u32, Vec<u8>) {
        if self.flat {
            if self.flat_template.is_none() {
                let (sx, sz) = (self.spawn[0] >> 4, self.spawn[2] >> 4);
                if let Ok(Some((c, p))) = self.encode_chunk(sx, sz, gd) {
                    self.flat_template = Some((c, Rc::new(p)));
                }
            }
            if let Some((c, p)) = &self.flat_template {
                return (*c, p.as_ref().clone());
            }
        }
        let mut w = W::new();
        for _ in 0..SUBCHUNK_COUNT {
            write_single_biome(&mut w, BIOME_PLAINS);
        }
        w.u8(0); // border blocks
        (0, w.0)
    }

    fn encode_chunk(&mut self, x: i32, z: i32, gd: &GameData) -> Res<Option<(u32, Vec<u8>)>> {
        if !self.has_chunk(x, z) {
            return Ok(None);
        }
        let mut subs: Vec<Option<Vec<u8>>> = Vec::with_capacity(SUBCHUNK_COUNT);
        for y in MIN_SUBCHUNK..=MAX_SUBCHUNK {
            let data = self.db.get(&chunk_key(x, z, TAG_SUBCHUNK, Some(y as i8)));
            subs.push(match data {
                Some(d) => encode_subchunk(&d, gd)?,
                None => None,
            });
        }
        let count = subs.iter().rposition(|s| s.is_some()).map(|i| i + 1).unwrap_or(0);
        let mut w = W::with_capacity(16 * 1024);
        for s in subs.iter().take(count) {
            match s {
                Some(b) => w.bytes(b),
                None => w.bytes(&[8, 0]), // empty subchunk: version 8, no layers
            }
        }
        let biomes = self.db.get(&chunk_key(x, z, TAG_DATA_3D, None));
        encode_biomes(biomes.as_deref(), &mut w);
        w.u8(0); // border blocks
        // TODO: block entities (signs, chests...) from tag 0x31
        Ok(Some((count as u32, w.0)))
    }
}
