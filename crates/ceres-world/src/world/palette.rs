//! Paletted block storage (one layer of a subchunk): a palette plus 4096 indices.
//! Index of a block = (x << 8) | (z << 4) | y. The same packing is used on disk and on the wire.

use super::*;

/// A palette entry keeps the network id and the exact NBT it was stored with, so blocks the
/// server does not know survive a load/save round trip untouched.
#[derive(Clone)]
pub(crate) struct PaletteEntry {
    pub(crate) id: i32,
    pub(crate) disk: Rc<[u8]>,
}

#[derive(Clone)]
pub(crate) struct Storage {
    pub(crate) palette: Vec<PaletteEntry>,
    pub(crate) idx: Box<[u16; 4096]>,
}

const BIT_SIZES: [u8; 8] = [1, 2, 3, 4, 5, 6, 8, 16];

pub(crate) fn words_len(bits: u8) -> Res<usize> {
    match bits {
        0 => Ok(0),
        1 | 2 | 3 | 4 | 5 | 6 | 8 | 16 => Ok(4096usize.div_ceil(32 / bits as usize) * 4),
        _ => Err(format!("invalid bits per block {bits}")),
    }
}

fn bits_for(len: usize) -> u8 {
    if len <= 1 {
        return 0;
    }
    BIT_SIZES.into_iter().find(|b| (1usize << b) >= len).unwrap_or(16)
}

fn unpack(words: &[u8], bits: u8, palette_len: usize) -> Box<[u16; 4096]> {
    let mut idx = Box::new([0u16; 4096]);
    if bits == 0 {
        return idx;
    }
    let per_word = 32 / bits as usize;
    let mask = (1u32 << bits) - 1;
    for (i, slot) in idx.iter_mut().enumerate() {
        let w = (i / per_word) * 4;
        let word = u32::from_le_bytes([words[w], words[w + 1], words[w + 2], words[w + 3]]);
        let v = (word >> ((i % per_word) * bits as usize)) & mask;
        *slot = if (v as usize) < palette_len { v as u16 } else { 0 };
    }
    idx
}

fn pack(w: &mut W, idx: &[u16; 4096], bits: u8) {
    if bits == 0 {
        return;
    }
    let per_word = 32 / bits as usize;
    for chunk in idx.chunks(per_word) {
        let mut word = 0u32;
        for (j, v) in chunk.iter().enumerate() {
            word |= (*v as u32) << (j * bits as usize);
        }
        w.u32_le(word);
    }
}

impl Storage {
    pub(crate) fn filled(entry: PaletteEntry) -> Storage {
        Storage { palette: vec![entry], idx: Box::new([0; 4096]) }
    }

    /// One layer in disk format (header, words, palette of little-endian NBT states).
    pub(crate) fn read_disk(r: &mut R, gd: &GameData) -> Res<Storage> {
        let bits = r.u8()? >> 1;
        let words = r.take(words_len(bits)?)?;
        let size = if bits == 0 {
            if r.b.get(r.p) != Some(&10) {
                r.i32_le()?; // PM4 plugin bug: length-prefixed 0 bpb palette
            }
            1
        } else {
            r.i32_le()?.clamp(0, 4096) as usize
        };
        let mut palette = Vec::with_capacity(size);
        for _ in 0..size {
            let start = r.p;
            let (_, state) = nbt::read_root(r, Flavor::LittleEndian)?;
            palette.push(PaletteEntry { id: gd.lookup_block(&state), disk: Rc::from(&r.b[start..r.p]) });
        }
        if palette.is_empty() {
            palette.push(gd.entry(gd.air_id));
        }
        let idx = unpack(words, bits, palette.len());
        Ok(Storage { palette, idx })
    }

    pub(crate) fn get(&self, i: usize) -> i32 {
        self.palette[self.idx[i] as usize].id
    }

    pub(crate) fn set(&mut self, i: usize, entry: PaletteEntry) {
        let p = match self.palette.iter().position(|e| e.id == entry.id) {
            Some(p) => p,
            None => {
                self.palette.push(entry);
                self.palette.len() - 1
            }
        };
        self.idx[i] = p as u16;
    }

    pub(crate) fn only(&self, id: i32) -> bool {
        let used = self.used();
        used.iter().all(|u| self.palette[*u].id == id)
    }

    /// Palette indices actually referenced, in palette order.
    fn used(&self) -> Vec<usize> {
        let mut seen = vec![false; self.palette.len()];
        for v in self.idx.iter() {
            seen[*v as usize] = true;
        }
        (0..self.palette.len()).filter(|i| seen[*i]).collect()
    }

    /// Drops unused palette entries; returns (palette, indices) ready to write.
    fn compacted(&self) -> (Vec<&PaletteEntry>, Box<[u16; 4096]>) {
        let used = self.used();
        let mut remap = vec![0u16; self.palette.len()];
        for (new, old) in used.iter().enumerate() {
            remap[*old] = new as u16;
        }
        let mut idx = Box::new([0u16; 4096]);
        for (i, v) in self.idx.iter().enumerate() {
            idx[i] = remap[*v as usize];
        }
        (used.iter().map(|i| &self.palette[*i]).collect(), idx)
    }

    pub(crate) fn write_network(&self, w: &mut W) {
        let (palette, idx) = self.compacted();
        let bits = bits_for(palette.len());
        w.u8((bits << 1) | 1);
        pack(w, &idx, bits);
        if bits != 0 {
            w.varint(palette.len() as i32);
        }
        for e in palette {
            w.varint(e.id);
        }
    }

    pub(crate) fn write_disk(&self, w: &mut W) {
        let (palette, idx) = self.compacted();
        let bits = bits_for(palette.len());
        w.u8(bits << 1);
        pack(w, &idx, bits);
        if bits != 0 {
            w.i32_le(palette.len() as i32);
        }
        for e in palette {
            w.bytes(&e.disk);
        }
    }
}
