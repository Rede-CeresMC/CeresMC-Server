//! Disk chunk format -> network chunk format (subchunks and 3D biomes).

use super::*;

pub(crate) fn words_len(bits: u8) -> Res<usize> {
    match bits {
        0 => Ok(0),
        1 | 2 | 3 | 4 | 5 | 6 | 8 | 16 => {
            let per_word = 32 / bits as usize;
            Ok(4096usize.div_ceil(per_word) * 4)
        }
        _ => Err(format!("invalid bits per block {bits}")),
    }
}

/// Disk subchunk -> network subchunk. Returns None when it holds nothing but air.
pub(crate) fn encode_subchunk(d: &[u8], gd: &GameData) -> Res<Option<Vec<u8>>> {
    let mut r = R::new(d);
    let version = r.u8()?;
    let layers = match version {
        8 => r.u8()?,
        9 => {
            let n = r.u8()?;
            r.u8()?; // y index
            n
        }
        1 => 1,
        _ => return Ok(None), // pre-1.2.13 formats: not supported yet
    };
    let mut out = W::with_capacity(d.len());
    out.u8(8);
    out.u8(layers);
    let mut solid = false;
    for _ in 0..layers {
        let header = r.u8()?;
        let bits = header >> 1;
        let words = r.take(words_len(bits)?)?;
        let size = if bits == 0 {
            if r.b.get(r.p) != Some(&10) {
                r.i32_le()?; // PM4 plugin bug: length-prefixed 0 bpb palette
            }
            1
        } else {
            r.i32_le()?.max(0) as usize
        };
        out.u8((bits << 1) | 1);
        out.bytes(words);
        if bits != 0 {
            out.varint(size as i32);
        }
        for _ in 0..size {
            let (_, state) = nbt::read_root(&mut r, Flavor::LittleEndian)?;
            let nid = gd.lookup_block(&state);
            if nid != gd.air_id {
                solid = true;
            }
            out.varint(nid);
        }
    }
    Ok(if solid { Some(out.0) } else { None })
}

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
