//! Game packet batches: varint-length-prefixed packets, optionally deflated.

use ceres_codec::binary::{R, Res, W};
use flate2::Compression;
use std::io::{Read, Write};

pub const COMPRESSION_ZLIB: u8 = 0;
pub const COMPRESSION_NONE: u8 = 0xff;
const COMPRESSION_THRESHOLD: usize = 256;
const MAX_DECOMPRESSED: u64 = 8 * 1024 * 1024;

/// Packet = unsigned varint header (id, sub-client ids 0) + payload.
pub fn packet(id: u32, f: impl FnOnce(&mut W)) -> Vec<u8> {
    let mut w = W::new();
    w.uvarint(id);
    f(&mut w);
    w.0
}

pub fn encode_batch(packets: &[Vec<u8>], compression: bool) -> Vec<u8> {
    let total: usize = packets.iter().map(|p| p.len() + 5).sum();
    let mut raw = W::with_capacity(total);
    for p in packets {
        raw.uvarint(p.len() as u32);
        raw.bytes(p);
    }
    if !compression {
        return raw.0;
    }
    if raw.len() < COMPRESSION_THRESHOLD {
        let mut out = Vec::with_capacity(raw.len() + 1);
        out.push(COMPRESSION_NONE);
        out.extend_from_slice(&raw.0);
        return out;
    }
    let mut out = Vec::with_capacity(raw.len() / 3 + 16);
    out.push(COMPRESSION_ZLIB);
    let mut enc = flate2::write::DeflateEncoder::new(out, Compression::new(6));
    enc.write_all(&raw.0).expect("deflate to memory");
    enc.finish().expect("deflate to memory")
}

pub fn decode_batch(payload: &[u8], compression: bool) -> Res<Vec<Vec<u8>>> {
    let raw: Vec<u8>;
    let data: &[u8] = if compression {
        if payload.is_empty() {
            return Err("empty batch".into());
        }
        match payload[0] {
            COMPRESSION_NONE => &payload[1..],
            COMPRESSION_ZLIB => {
                let mut out = Vec::new();
                flate2::read::DeflateDecoder::new(&payload[1..])
                    .take(MAX_DECOMPRESSED)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("inflate: {e}"))?;
                raw = out;
                &raw
            }
            a => return Err(format!("unsupported compression algorithm {a}")),
        }
    } else {
        payload
    };
    let mut r = R::new(data);
    let mut out = Vec::new();
    while !r.eof() {
        let n = r.uvarint()? as usize;
        out.push(r.take(n)?.to_vec());
        if out.len() > 2048 {
            return Err("too many packets in batch".into());
        }
    }
    Ok(out)
}
