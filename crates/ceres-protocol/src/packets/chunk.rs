//! Chunk radius, chunk publisher and LevelChunk.

#![allow(unused_imports)]

use crate::packets::player::mp;
use crate::{COMPRESSION_ZLIB, VERSION, id, packet};
use ceres_codec::binary::{R, Res, W};

pub fn chunk_radius_updated(radius: i32) -> Vec<u8> {
    packet(id::CHUNK_RADIUS_UPDATED, |w| w.varint(radius))
}

pub fn network_chunk_publisher_update(pos: [i32; 3], radius_blocks: u32) -> Vec<u8> {
    packet(id::NETWORK_CHUNK_PUBLISHER_UPDATE, |w| {
        w.varint(pos[0]);
        w.varint(pos[1]);
        w.varint(pos[2]);
        w.uvarint(radius_blocks);
        w.u32_le(0); // saved chunks
    })
}

pub fn level_chunk(x: i32, z: i32, sub_chunk_count: u32, payload: &[u8]) -> Vec<u8> {
    let mut w = W::with_capacity(payload.len() + 24);
    w.uvarint(id::LEVEL_CHUNK);
    w.varint(x);
    w.varint(z);
    w.varint(0); // dimension
    w.uvarint(sub_chunk_count);
    w.bool(false); // client request sub chunk limit: none
    w.bool(false); // cache enabled
    w.uvarint(0); // blob hashes
    w.uvarint(payload.len() as u32);
    w.bytes(payload);
    w.bool(false); // client biome update
    w.0
}
