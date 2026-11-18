//! Chunk radius, chunk publisher, LevelChunk, UpdateBlock, LevelEvent, sounds and signs.

#![allow(unused_imports)]

use crate::packets::inventory::{read_block_pos, write_block_pos};
use crate::packets::player::mp;
use crate::{COMPRESSION_ZLIB, VERSION, id, packet};
use ceres_codec::binary::{R, Res, W};
use ceres_codec::nbt::{self, Flavor, Tag};

pub const LEVEL_EVENT: u32 = 0x19;
pub const UPDATE_BLOCK: u32 = 0x15;
pub const BLOCK_ACTOR_DATA: u32 = 0x38;
pub const LEVEL_SOUND_EVENT: u32 = 0x7b;
pub const OPEN_SIGN: u32 = 0x12f;

pub mod level_event {
    pub const SOUND_CLICK: i32 = 1000;
    pub const SOUND_DOOR: i32 = 1003;
    pub const SOUND_POP: i32 = 1030;
    pub const PARTICLE_DESTROY: i32 = 2001;
    pub const START_RAIN: i32 = 3001;
    pub const START_THUNDER: i32 = 3002;
    pub const STOP_RAIN: i32 = 3003;
    pub const STOP_THUNDER: i32 = 3004;
    pub const BLOCK_START_BREAK: i32 = 3600;
    pub const BLOCK_STOP_BREAK: i32 = 3601;
}

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

/// Changes one block on the client (FLAG_NETWORK, normal data layer).
pub fn update_block(pos: [i32; 3], block_id: i32) -> Vec<u8> {
    packet(UPDATE_BLOCK, |w| {
        write_block_pos(w, pos);
        w.uvarint(block_id as u32);
        w.uvarint(0b0010); // flags: network
        w.uvarint(0); // data layer: normal
    })
}

/// LevelEventPacket (0x19): particles, break cracks, weather and block sounds.
pub fn level_event(event_id: i32, pos: [f32; 3], data: i32) -> Vec<u8> {
    packet(LEVEL_EVENT, |w| {
        w.varint(event_id);
        for v in pos {
            w.f32_le(v);
        }
        w.varint(data);
    })
}

/// LevelSoundEventPacket (0x7b, protocol 2225).
pub fn level_sound_event(sound: &str, pos: [f32; 3], extra_data: i32) -> Vec<u8> {
    packet(LEVEL_SOUND_EVENT, |w| {
        w.string(sound);
        for v in pos {
            w.f32_le(v);
        }
        w.varint(extra_data);
        w.string(":"); // non-actor entity type
        w.bool(false); // baby mob
        w.bool(false); // disable relative volume
        w.i64_le(-1); // actor unique id
        w.bool(false); // optional fire position
    })
}

/// BlockActorDataPacket (0x38): sends or reads block entity NBT (signs, chests, etc.).
pub fn block_actor_data(pos: [i32; 3], tag: &Tag) -> Vec<u8> {
    let encoded = nbt::to_network(tag);
    packet(BLOCK_ACTOR_DATA, |w| {
        write_block_pos(w, pos);
        w.bytes(&encoded);
    })
}

pub fn read_block_actor_data(r: &mut R) -> Res<([i32; 3], Tag)> {
    let pos = read_block_pos(r)?;
    let (_, tag) = nbt::read_root(r, Flavor::Network)?;
    Ok((pos, tag))
}

/// OpenSignPacket (0x12f): opens the sign editor UI on the client after placing a sign.
pub fn open_sign(pos: [i32; 3], front: bool) -> Vec<u8> {
    packet(OPEN_SIGN, |w| {
        write_block_pos(w, pos);
        w.bool(front);
    })
}
