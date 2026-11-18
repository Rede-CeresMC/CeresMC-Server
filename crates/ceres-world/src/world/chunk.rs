//! 3D biome data: disk format -> network format.

use super::*;

pub(crate) fn write_single_biome(w: &mut W, id: i32) {
    w.u8(1); // 0 bits, runtime
    w.varint(id);
}

pub(crate) fn encode_biomes(data: Option<&[u8]>, w: &mut W) {
    let mut written = 0;
    if let Some(d) = data {
        if d.len() > 512 {
            let mut r = R::new(&d[512..]);
            let mut prev: Option<Vec<u8>> = None;
            while written < SUBCHUNK_COUNT && !r.eof() {
                let Ok(header) = r.u8() else { break };
                let bits = header >> 1;
                let encoded = if bits == 127 {
                    match &prev {
                        Some(p) => p.clone(),
                        None => break,
                    }
                } else {
                    let mut p = W::new();
                    let ok = (|| -> Res<()> {
                        let words = r.take(words_len(bits)?)?;
                        let size = if bits == 0 { 1 } else { r.i32_le()?.max(0) as usize };
                        p.u8((bits << 1) | 1);
                        p.bytes(words);
                        if bits != 0 {
                            p.varint(size as i32);
                        }
                        for _ in 0..size {
                            p.varint(r.i32_le()?);
                        }
                        Ok(())
                    })();
                    if ok.is_err() {
                        break;
                    }
                    p.0
                };
                w.bytes(&encoded);
                prev = Some(encoded);
                written += 1;
            }
        }
    }
    for _ in written..SUBCHUNK_COUNT {
        write_single_biome(w, BIOME_PLAINS);
    }
}

/// The biome id at a block inside a column's raw Data3D value (plains when there is none).
pub(crate) fn biome_in(data: Option<&[u8]>, x: i32, y: i32, z: i32) -> i32 {
    let Some(d) = data.filter(|d| d.len() > 512) else { return BIOME_PLAINS };
    let want = ((y - MIN_Y) >> 4).max(0) as usize;
    let mut r = R::new(&d[512..]);
    let mut prev = BIOME_PLAINS;
    let mut index = 0usize;
    while !r.eof() && index <= want {
        let Ok(header) = r.u8() else { break };
        let bits = header >> 1;
        if bits == 127 {
            if index == want {
                return prev;
            }
            index += 1;
            continue;
        }
        let Ok(len) = words_len(bits) else { break };
        let Ok(words) = r.take(len) else { break };
        let size = if bits == 0 { 1 } else { r.i32_le().unwrap_or(0).max(0) as usize };
        let mut palette = Vec::with_capacity(size.min(4096));
        for _ in 0..size.min(4096) {
            palette.push(r.i32_le().unwrap_or(BIOME_PLAINS));
        }
        let value = if bits == 0 {
            palette.first().copied().unwrap_or(BIOME_PLAINS)
        } else {
            let i = (((x & 15) << 8) | ((z & 15) << 4) | (y & 15)) as usize;
            let per_word = 32 / bits as usize;
            let w = i / per_word;
            let shift = (i % per_word) * bits as usize;
            let word = words.get(w * 4..w * 4 + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0);
            let p = ((word >> shift) & ((1u32 << bits) - 1)) as usize;
            palette.get(p).copied().unwrap_or(BIOME_PLAINS)
        };
        if index == want {
            return value;
        }
        prev = value;
        index += 1;
    }
    prev
}
