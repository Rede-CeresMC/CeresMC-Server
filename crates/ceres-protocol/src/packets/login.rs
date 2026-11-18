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

pub fn resource_packs_info() -> Vec<u8> {
    packet(id::RESOURCE_PACKS_INFO, |w| {
        w.bool(false); // must accept
        w.bool(false); // has addons
        w.bool(false); // has scripts
        w.bool(false); // force disable vibrant visuals
        w.bytes(&[0u8; 16]); // world template uuid
        w.string(""); // world template version
        w.uvarint(0); // packs
    })
}

pub fn resource_pack_stack() -> Vec<u8> {
    packet(id::RESOURCE_PACK_STACK, |w| {
        w.bool(false); // must accept
        w.uvarint(0); // stack
        w.string(VERSION); // base game version
        w.u32_le(0); // experiments
        w.bool(false); // experiments previously used
        w.bool(false); // use vanilla editor packs
    })
}
