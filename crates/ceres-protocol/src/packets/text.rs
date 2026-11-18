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

/// DeathInfoPacket (0xbd): the cause of death shown on the death screen.
pub fn death_info(message: &str) -> Vec<u8> {
    packet(0xbd, |w| {
        w.string(message);
        w.uvarint(0); // translation parameters
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

/// ModalFormRequestPacket (0x64): a form (JSON) for the client to show.
pub fn modal_form_request(form_id: u32, json: &str) -> Vec<u8> {
    packet(0x64, |w| {
        w.uvarint(form_id);
        w.string(json);
    })
}

/// ModalFormResponsePacket (0x65): (form id, answer JSON or None when closed).
pub fn read_modal_form_response(r: &mut R) -> Res<(u32, Option<String>)> {
    let id = r.uvarint()?;
    let data = if r.bool()? { Some(r.string()?) } else { None };
    Ok((id, data))
}

/// SetDisplayObjectivePacket (0x6b): shows an objective ("sidebar", "list", "belowname").
pub fn set_display_objective(slot: &str, objective: &str, title: &str, ascending: bool) -> Vec<u8> {
    packet(0x6b, |w| {
        w.string(slot);
        w.string(objective);
        w.string(title);
        w.string("dummy");
        w.varint(if ascending { 0 } else { 1 });
    })
}

/// RemoveObjectivePacket (0x6a).
pub fn remove_objective(objective: &str) -> Vec<u8> {
    packet(0x6a, |w| w.string(objective))
}

/// SetScorePacket (0x6c) with text lines: (scoreboard id, score, text).
pub fn set_score_lines(objective: &str, lines: &[(i64, i32, String)]) -> Vec<u8> {
    packet(0x6c, |w| {
        w.uvarint(lines.len() as u32);
        for (id, score, text) in lines {
            w.uvarint(3); // fake player
            w.string("changefakeplayer");
            w.varlong(*id);
            w.string(objective);
            w.i32_le(*score);
            w.string(text);
        }
    })
}

/// BossEventPacket (0x4a). `event`: 0 show, 2 hide, 4 health, 5 title, 6 properties.
pub fn boss_event(entity_id: i64, event: u8, title: &str, percent: f32, color: u8) -> Vec<u8> {
    packet(0x4a, |w| {
        w.varlong(entity_id);
        w.u8(event);
        w.string(title);
        w.string(title); // filtered title
        w.f32_le(percent.clamp(0.0, 1.0));
        w.u8(color);
        w.u8(0); // overlay: progress
    })
}
