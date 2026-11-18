//! A chunk column held in memory: 24 subchunks (y -4..19) of editable block layers, plus the raw
//! 3D biome data. Loaded from LevelDB, encoded for the network, written back when changed.

use super::*;

#[derive(Clone)]
pub(crate) struct SubChunk {
    pub(crate) layers: Vec<Storage>,
}

impl SubChunk {
    fn read_disk(d: &[u8], gd: &GameData) -> Res<Option<SubChunk>> {
        let mut r = R::new(d);
        let layers = match r.u8()? {
            8 => r.u8()?,
            9 => {
                let n = r.u8()?;
                r.u8()?; // y index
                n
            }
            1 => 1,
            _ => return Ok(None), // pre-1.2.13 formats: not supported yet
        };
        let mut out = Vec::with_capacity(layers as usize);
        for _ in 0..layers {
            out.push(Storage::read_disk(&mut r, gd)?);
        }
        Ok(Some(SubChunk { layers: out }))
    }

    fn is_empty(&self, air: i32) -> bool {
        self.layers.iter().all(|l| l.only(air))
    }
}

#[derive(Clone)]
pub(crate) struct Column {
    pub(crate) subs: Vec<Option<SubChunk>>,
    /// raw Data3D value (512 bytes heightmap + biome palettes)
    pub(crate) biomes: Option<Vec<u8>>,
    /// bit n = subchunk n changed since the last save
    pub(crate) dirty: u32,
    /// false for chunks that only exist in memory (flat template copies, empty chunks)
    pub(crate) on_disk: bool,
}

pub(crate) fn block_index(x: i32, y: i32, z: i32) -> usize {
    (((x & 15) << 8) | ((z & 15) << 4) | (y & 15)) as usize
}

impl Column {
    pub(crate) fn empty() -> Column {
        Column { subs: vec![None; SUBCHUNK_COUNT], biomes: None, dirty: 0, on_disk: false }
    }

    pub(crate) fn load(db: &mut DB, x: i32, z: i32, gd: &GameData) -> Res<Option<Column>> {
        if db.get(&chunk_key(x, z, TAG_VERSION, None)).is_none()
            && db.get(&chunk_key(x, z, TAG_VERSION_OLD, None)).is_none()
        {
            return Ok(None);
        }
        let mut c = Column::empty();
        c.on_disk = true;
        for (i, y) in (MIN_SUBCHUNK..=MAX_SUBCHUNK).enumerate() {
            if let Some(d) = db.get(&chunk_key(x, z, TAG_SUBCHUNK, Some(y as i8))) {
                c.subs[i] = SubChunk::read_disk(&d, gd)?;
            }
        }
        c.biomes = db.get(&chunk_key(x, z, TAG_DATA_3D, None)).map(|b| b.to_vec());
        Ok(Some(c))
    }

    fn sub_index(y: i32) -> Option<usize> {
        let s = (y >> 4) - MIN_SUBCHUNK;
        (0..SUBCHUNK_COUNT as i32).contains(&s).then_some(s as usize)
    }

    pub(crate) fn get(&self, x: i32, y: i32, z: i32, gd: &GameData) -> i32 {
        match Column::sub_index(y).and_then(|s| self.subs[s].as_ref()) {
            Some(sub) if !sub.layers.is_empty() => sub.layers[0].get(block_index(x, y, z)),
            _ => gd.air_id,
        }
    }

    /// Sets the block on layer 0. Returns false when y is outside the world.
    pub(crate) fn set(&mut self, x: i32, y: i32, z: i32, id: i32, gd: &GameData) -> bool {
        let Some(s) = Column::sub_index(y) else { return false };
        let sub = self.subs[s].get_or_insert_with(|| SubChunk { layers: vec![Storage::filled(gd.entry(gd.air_id))] });
        if sub.layers.is_empty() {
            sub.layers.push(Storage::filled(gd.entry(gd.air_id)));
        }
        sub.layers[0].set(block_index(x, y, z), gd.entry(id));
        self.dirty |= 1 << s;
        true
    }

    /// (subchunk count, LevelChunk payload)
    pub(crate) fn network_payload(&self, gd: &GameData) -> (u32, Vec<u8>) {
        let count = self
            .subs
            .iter()
            .rposition(|s| s.as_ref().is_some_and(|s| !s.is_empty(gd.air_id)))
            .map(|i| i + 1)
            .unwrap_or(0);
        let mut w = W::with_capacity(16 * 1024);
        for s in self.subs.iter().take(count) {
            w.u8(8);
            match s {
                Some(sub) => {
                    w.u8(sub.layers.len() as u8);
                    for l in &sub.layers {
                        l.write_network(&mut w);
                    }
                }
                None => w.u8(0),
            }
        }
        encode_biomes(self.biomes.as_deref(), &mut w);
        w.u8(0); // border blocks
        // TODO: block entities (signs, chests...) from tag 0x31
        (count as u32, w.0)
    }

    /// Writes the changed subchunks (and, for chunks new on disk, the chunk header keys).
    pub(crate) fn save(&mut self, batch: &mut rusty_leveldb::WriteBatch, x: i32, z: i32, gd: &GameData) {
        let new = !self.on_disk;
        if new {
            batch.put(&chunk_key(x, z, TAG_VERSION, None), &[CHUNK_VERSION]);
            batch.put(&chunk_key(x, z, TAG_FINALIZATION, None), &2i32.to_le_bytes());
            if let Some(b) = &self.biomes {
                batch.put(&chunk_key(x, z, TAG_DATA_3D, None), b);
            }
        }
        for (i, y) in (MIN_SUBCHUNK..=MAX_SUBCHUNK).enumerate() {
            if !new && self.dirty & (1 << i) == 0 {
                continue;
            }
            let key = chunk_key(x, z, TAG_SUBCHUNK, Some(y as i8));
            match &self.subs[i] {
                Some(sub) if !sub.is_empty(gd.air_id) => {
                    let mut w = W::with_capacity(8 * 1024);
                    w.u8(8); // subchunk version: paletted multi-layer
                    w.u8(sub.layers.len() as u8);
                    for l in &sub.layers {
                        l.write_disk(&mut w);
                    }
                    batch.put(&key, &w.0);
                }
                _ => {
                    if !new {
                        batch.delete(&key);
                    }
                }
            }
        }
        self.dirty = 0;
        self.on_disk = true;
    }
}
