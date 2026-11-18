//! Chat and system messages.

#![allow(unused_imports)]

use crate::packets::player::mp;
use crate::{COMPRESSION_ZLIB, VERSION, id, packet};
use ceres_codec::binary::{R, Res, W};

pub fn chat(source: &str, message: &str) -> Vec<u8> {
    packet(mp::TEXT, |w| {
        w.bool(false); // needs translation
        w.u8(1); // category: authored message
        w.u8(1); // type: chat
        w.string(source);
        w.string(message);
        w.string(""); // xuid
        w.string(""); // platform chat id
        w.bool(false); // filtered message
    })
}

pub fn system_message(message: &str) -> Vec<u8> {
    packet(mp::TEXT, |w| {
        w.bool(false);
        w.u8(0); // category: message only
        w.u8(0); // type: raw
        w.string(message);
        w.string("");
        w.string("");
        w.bool(false);
    })
}

/// Reads an incoming TextPacket; returns (type, message) for chat.
pub fn read_text(r: &mut R) -> Res<(u8, String)> {
    let _needs_translation = r.bool()?;
    let _category = r.u8()?;
    let ty = r.u8()?;
    let msg = match ty {
        1 | 7 | 8 => {
            let _source = r.string()?;
            r.string()?
        }
        _ => r.string()?,
    };
    Ok((ty, msg))
}
