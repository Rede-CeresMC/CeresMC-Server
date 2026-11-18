//! Skin from the login client data (JWT payload) -> network skin (CommonTypes::putSkin).
//! Port of ClientDataToSkinDataHelper from the Altay bedrock-protocol.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ceres_codec::binary::W;
use serde_json::Value;

const PIECE_TYPES: [&str; 27] = [
    "persona_skeleton",
    "persona_body",
    "persona_skin",
    "persona_bottom",
    "persona_feet",
    "persona_dress",
    "persona_top",
    "persona_high_pants",
    "persona_hand",
    "persona_outerwear",
    "persona_facial_hair",
    "persona_mouth",
    "persona_eyes",
    "persona_hair",
    "persona_hood",
    "persona_back",
    "persona_face_accessory",
    "persona_head",
    "persona_legs",
    "persona_left_leg",
    "persona_right_leg",
    "persona_arms",
    "persona_left_arm",
    "persona_right_arm",
    "persona_capes",
    "persona_classic_skin",
    "persona_emote",
];

fn b64(v: &Value) -> Vec<u8> {
    v.as_str().and_then(|s| STANDARD.decode(s).ok()).unwrap_or_default()
}

fn int(v: &Value) -> i64 {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).unwrap_or(0)
}

fn byte_string(w: &mut W, b: &[u8]) {
    w.uvarint(b.len() as u32);
    w.bytes(b);
}

fn image(w: &mut W, width: &Value, height: &Value, data: &[u8]) {
    w.u32_le(int(width) as u32);
    w.u32_le(int(height) as u32);
    byte_string(w, data);
}

fn color(v: &Value) -> i32 {
    v.as_str().and_then(|s| u32::from_str_radix(s.trim_start_matches('#'), 16).ok()).unwrap_or(0) as i32
}

/// Tint colours carry the piece type as a string, and newer clients only accept the bare enum
/// name ("eyes"), not the client data spelling ("persona_eyes").
fn tint_piece_type(s: &str) -> String {
    match s.strip_prefix("persona_").unwrap_or(s) {
        "hand" => "hands".to_string(),
        other => other.to_string(),
    }
}

/// UUID string -> wire form (two little-endian halves, as CommonTypes::putUUID writes it).
pub fn uuid_wire(s: &str) -> [u8; 16] {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let mut b = [0u8; 16];
    for i in 0..16.min(hex.len() / 2) {
        b[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    wire(b)
}

/// Raw UUID bytes -> wire form.
pub fn wire(mut b: [u8; 16]) -> [u8; 16] {
    b[..8].reverse();
    b[8..].reverse();
    b
}

pub fn random_uuid() -> [u8; 16] {
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant
    b
}

fn uuid_string(b: &[u8; 16]) -> String {
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Encodes the skin carried in the client data JSON.
pub fn encode(cd: &Value) -> Vec<u8> {
    let mut w = W::with_capacity(64 * 1024);
    w.string(cd["SkinId"].as_str().unwrap_or(""));
    byte_string(&mut w, &b64(&cd["SkinResourcePatch"]));
    image(&mut w, &cd["SkinImageWidth"], &cd["SkinImageHeight"], &b64(&cd["SkinData"]));

    let animations = cd["AnimatedImageData"].as_array().cloned().unwrap_or_default();
    w.uvarint(animations.len() as u32);
    for a in &animations {
        image(&mut w, &a["ImageWidth"], &a["ImageHeight"], &b64(&a["Image"]));
        w.uvarint(int(&a["Type"]) as u32);
        w.f32_le(a["Frames"].as_f64().unwrap_or(0.0) as f32);
        w.uvarint(int(&a["AnimationExpression"]) as u32);
    }

    image(&mut w, &cd["CapeImageWidth"], &cd["CapeImageHeight"], &b64(&cd["CapeData"]));
    byte_string(&mut w, &b64(&cd["SkinGeometryData"]));
    byte_string(&mut w, &b64(&cd["SkinGeometryDataEngineVersion"]));
    byte_string(&mut w, &b64(&cd["SkinAnimationData"]));
    w.string(cd["CapeId"].as_str().unwrap_or(""));
    w.string(&uuid_string(&random_uuid())); // full skin id
    w.u8(if cd["ArmSize"].as_str() == Some("slim") { 0 } else { 1 });
    w.i32_le(color(&cd["SkinColor"]));

    let pieces = cd["PersonaPieces"].as_array().cloned().unwrap_or_default();
    w.uvarint(pieces.len() as u32);
    for p in &pieces {
        w.string(p["PieceId"].as_str().unwrap_or(""));
        let ty = p["PieceType"].as_str().unwrap_or("");
        w.i32_le(PIECE_TYPES.iter().position(|t| *t == ty).map(|i| i as i32 + 1).unwrap_or(0));
        w.bytes(&uuid_wire(p["PackId"].as_str().unwrap_or("")));
        w.bool(p["IsDefault"].as_bool().unwrap_or(false));
        w.string(p["ProductId"].as_str().unwrap_or(""));
    }

    let tints = cd["PieceTintColors"].as_array().cloned().unwrap_or_default();
    w.uvarint(tints.len() as u32);
    for t in &tints {
        w.string(&tint_piece_type(t["PieceType"].as_str().unwrap_or("")));
        let colors = t["Colors"].as_array().cloned().unwrap_or_default();
        for i in 0..4 {
            w.i32_le(colors.get(i).map(color).unwrap_or(0));
        }
    }

    w.bool(cd["PremiumSkin"].as_bool().unwrap_or(false));
    w.bool(cd["PersonaSkin"].as_bool().unwrap_or(false));
    w.bool(cd["CapeOnClassicSkin"].as_bool().unwrap_or(false));
    w.bool(true); // primary user
    w.bool(cd["OverrideSkin"].as_bool().unwrap_or(true));
    w.string(if cd["TrustedSkin"].as_bool().unwrap_or(false) { "true" } else { "false" });
    w.string(""); // profile hash
    w.0
}
