//! Bedrock protocol v2225 (1.26.60): batch codec and the packets needed to log in and stream a world.
//! Field order mirrors pocketmine/bedrock-protocol exactly.
//!
//! Packet layouts follow altayofficial's bedrock-protocol (Altay), LGPL-3.0.

pub const PROTOCOL: u32 = 2225;
pub const VERSION: &str = "1.26.60";

pub mod id {
    pub const LOGIN: u32 = 0x01;
    pub const PLAY_STATUS: u32 = 0x02;
    pub const DISCONNECT: u32 = 0x05;
    pub const RESOURCE_PACKS_INFO: u32 = 0x06;
    pub const RESOURCE_PACK_STACK: u32 = 0x07;
    pub const RESOURCE_PACK_CLIENT_RESPONSE: u32 = 0x08;
    pub const START_GAME: u32 = 0x0b;
    pub const LEVEL_CHUNK: u32 = 0x3a;
    pub const REQUEST_CHUNK_RADIUS: u32 = 0x45;
    pub const CHUNK_RADIUS_UPDATED: u32 = 0x46;
    pub const SET_LOCAL_PLAYER_AS_INITIALIZED: u32 = 0x71;
    pub const AVAILABLE_ACTOR_IDENTIFIERS: u32 = 0x77;
    pub const NETWORK_CHUNK_PUBLISHER_UPDATE: u32 = 0x79;
    pub const BIOME_DEFINITION_LIST: u32 = 0x7a;
    pub const CLIENT_CACHE_STATUS: u32 = 0x81;
    pub const NETWORK_SETTINGS: u32 = 0x8f;
    pub const PLAYER_AUTH_INPUT: u32 = 0x90;
    pub const ITEM_REGISTRY: u32 = 0xa2;
    pub const REQUEST_NETWORK_SETTINGS: u32 = 0xc1;
    pub const JIGSAW_STRUCTURE_DATA: u32 = 0x139;
    pub const VOXEL_SHAPES: u32 = 0x151;
}

pub mod play_status {
    pub const LOGIN_SUCCESS: u32 = 0;
    pub const LOGIN_FAILED_CLIENT: u32 = 1;
    pub const LOGIN_FAILED_SERVER: u32 = 2;
    pub const PLAYER_SPAWN: u32 = 3;
}

mod batch;
pub mod packets;

pub use batch::{COMPRESSION_NONE, COMPRESSION_ZLIB, decode_batch, encode_batch, packet};
pub use packets::*;
