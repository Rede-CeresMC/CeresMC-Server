//! Session start, login result, disconnect and resource packs.

#![allow(unused_imports)]

use crate::packets::player::mp;
use crate::{COMPRESSION_ZLIB, VERSION, id, packet};
use ceres_codec::binary::{R, Res, W};

pub fn network_settings() -> Vec<u8> {
    packet(id::NETWORK_SETTINGS, |w| {
        w.u16_le(1); // compress everything
        w.u16_le(COMPRESSION_ZLIB as u16);
        w.bool(false); // client throttling
        w.u8(0);
        w.f32_le(0.0);
    })
}

pub fn play_status(status: u32) -> Vec<u8> {
    packet(id::PLAY_STATUS, |w| w.u32_be(status))
}

pub fn disconnect(message: &str) -> Vec<u8> {
    packet(id::DISCONNECT, |w| {
        w.varint(0); // reason: unknown
        w.uvarint(0); // skip message = false
        w.string(message);
        w.string(message);
    })
}

/// A resource pack offered to the client.
pub struct PackEntry<'a> {
    pub uuid: &'a str,
    pub version: &'a str,
    pub size: u64,
}

/// ResourcePacksInfoPacket: the packs the client must have (it downloads the missing ones).
pub fn resource_packs_info(packs: &[PackEntry], must_accept: bool) -> Vec<u8> {
    packet(id::RESOURCE_PACKS_INFO, |w| {
        w.bool(must_accept);
        w.bool(false); // has addons
        w.bool(false); // has scripts
        w.bool(false); // force disable vibrant visuals
        w.bytes(&[0u8; 16]); // world template uuid
        w.string(""); // world template version
        w.uvarint(packs.len() as u32);
        for p in packs {
            w.bytes(&uuid_bytes(p.uuid));
            w.string(p.version);
            w.u64_le(p.size);
            w.string(""); // encryption key
            w.string(""); // sub pack name
            w.string(p.uuid); // content id
            w.bool(false); // has scripts
            w.bool(false); // addon pack
            w.bool(false); // RTX capable
            w.string(""); // CDN url
        }
    })
}

/// A UUID string ("xxxxxxxx-xxxx-...") in the protocol's layout: two little-endian u64 halves.
fn uuid_bytes(uuid: &str) -> [u8; 16] {
    let hex: String = uuid.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let mut b = [0u8; 16];
    for (i, byte) in b.iter_mut().enumerate() {
        *byte = hex.get(i * 2..i * 2 + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).unwrap_or(0);
    }
    b[0..8].reverse();
    b[8..16].reverse();
    b
}

/// ResourcePackDataInfoPacket: size and chunking of a pack the client asked for.
pub fn resource_pack_data_info(uuid: &str, chunk_size: u32, chunks: u32, size: u64, sha256: &[u8]) -> Vec<u8> {
    packet(0x52, |w| {
        w.string(uuid);
        w.u32_le(chunk_size);
        w.u32_le(chunks);
        w.u64_le(size);
        w.uvarint(sha256.len() as u32);
        w.bytes(sha256);
        w.bool(false); // premium
        w.u8(6); // type: resources
    })
}

/// ResourcePackChunkDataPacket: one piece of a pack.
pub fn resource_pack_chunk_data(uuid: &str, index: u32, offset: u64, data: &[u8]) -> Vec<u8> {
    packet(0x53, |w| {
        w.string(uuid);
        w.u32_le(index);
        w.u64_le(offset);
        w.uvarint(data.len() as u32);
        w.bytes(data);
    })
}

/// ResourcePackClientResponsePacket: status and (when asking for downloads) the pack ids.
pub fn read_resource_pack_response(r: &mut R) -> Res<(u32, Vec<String>)> {
    let status = r.uvarint()?;
    let mut ids = Vec::new();
    if r.eof() {
        return Ok((status, ids));
    }
    r.string()?;
    if status == 1 {
        let n = r.uvarint()?;
        if n > 64 {
            return Err("too many pack ids".into());
        }
        for _ in 0..n {
            ids.push(r.string()?);
        }
    }
    Ok((status, ids))
}

/// ResourcePackChunkRequestPacket: (pack id, chunk index).
pub fn read_resource_pack_chunk_request(r: &mut R) -> Res<(String, u32)> {
    Ok((r.string()?, r.u32_le()?))
}

pub fn resource_pack_stack(packs: &[PackEntry], must_accept: bool) -> Vec<u8> {
    packet(id::RESOURCE_PACK_STACK, |w| {
        w.bool(must_accept);
        w.uvarint(packs.len() as u32);
        for p in packs {
            w.string(p.uuid);
            w.string(p.version);
            w.string(""); // sub pack
        }
        w.string(VERSION); // base game version
        w.u32_le(0); // experiments
        w.bool(false); // experiments previously used
        w.bool(false); // use vanilla editor packs
    })
}
