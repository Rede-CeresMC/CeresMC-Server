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
