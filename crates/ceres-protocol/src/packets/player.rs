//! Player list, AddPlayer, movement and abilities.

#![allow(unused_imports)]

use crate::{COMPRESSION_ZLIB, VERSION, id, packet};
use ceres_codec::binary::{R, Res, W};

pub mod mp {
    pub const TEXT: u32 = 0x09;
    pub const ADD_PLAYER: u32 = 0x0c;
    pub const REMOVE_ACTOR: u32 = 0x0e;
    pub const MOVE_PLAYER: u32 = 0x13;
    pub const PLAYER_LIST: u32 = 0x3f;
    pub const UPDATE_ABILITIES: u32 = 0xbb;
}

/// Everything another client needs to draw a player.
#[derive(Clone)]
pub struct PlayerInfo {
    pub uuid: [u8; 16],
    pub entity_id: i64,
    pub name: String,
    pub xuid: String,
    pub device_id: String,
    pub build_platform: i32,
    /// CommonTypes::putSkin() output
    pub skin: std::rc::Rc<Vec<u8>>,
}

pub fn player_list_add(players: &[&PlayerInfo]) -> Vec<u8> {
    packet(mp::PLAYER_LIST, |w| {
        w.uvarint(players.len() as u32);
        for p in players {
            w.uvarint(1); // TYPE_ADD
            w.u8(0); // legacy add
            w.bytes(&p.uuid);
            w.varlong(p.entity_id);
            w.string(&p.name);
            w.string(&p.xuid);
            w.string(""); // playfab id
            w.string(""); // platform chat id
            w.i32_le(p.build_platform);
            w.bytes(&p.skin);
            w.bool(false); // teacher
            w.bool(false); // host
            w.bool(false); // sub client
            w.u32_le(0xffff_ffff); // color
        }
    })
}

pub fn player_list_remove(uuid: &[u8; 16]) -> Vec<u8> {
    packet(mp::PLAYER_LIST, |w| {
        w.uvarint(1);
        w.uvarint(0); // TYPE_REMOVE
        w.u8(1); // legacy remove
        w.bytes(uuid);
    })
}

pub struct Abilities {
    pub entity_id: i64,
    pub operator: bool,
    pub creative: bool,
    pub spectator: bool,
    pub flying: bool,
}

pub fn write_abilities(w: &mut W, a: &Abilities) {
    w.bytes(&a.entity_id.to_le_bytes());
    w.u8(if a.operator { 2 } else { 1 }); // player permission
    w.u8(if a.operator { 1 } else { 0 }); // command permission
    w.u8(1); // layers
    w.u16_le(1); // base layer
    let act = !a.spectator;
    let bools: [(u32, bool); 17] = [
        (10, a.creative || a.spectator), // allow flight
        (9, a.flying || a.spectator),    // flying
        (17, a.spectator),               // no clip
        (6, a.operator),                 // operator
        (7, a.operator),                 // teleport
        (8, a.creative),                 // invulnerable
        (15, false),                     // muted
        (16, false),                     // world builder
        (11, a.creative),                // infinite resources
        (12, false),                     // lightning
        (0, act),                        // build
        (1, act),                        // mine
        (2, act),                        // doors and switches
        (3, act),                        // open containers
        (4, act),                        // attack players
        (5, act),                        // attack mobs
        (18, false),                     // privileged builder
    ];
    let mut set = (1u32 << 13) | (1 << 14) | (1 << 19); // fly speed, walk speed, vertical fly speed
    let mut values = 0u32;
    for (bit, v) in bools {
        set |= 1 << bit;
        if v {
            values |= 1 << bit;
        }
    }
    w.u32_le(set);
    w.u32_le(values);
    w.f32_le(0.05); // fly speed
    w.f32_le(1.0); // vertical fly speed
    w.f32_le(0.1); // walk speed
}

pub fn update_abilities(a: &Abilities) -> Vec<u8> {
    packet(mp::UPDATE_ABILITIES, |w| write_abilities(w, a))
}

fn player_metadata(w: &mut W, name: &str) {
    let flags: i64 = (1 << 14) | (1 << 15) | (1 << 35) | (1 << 48) | (1 << 49);
    w.uvarint(5);
    // key, type, type again (that's how putEntityMetadata writes it)
    w.uvarint(0);
    w.uvarint(7);
    w.u8(7);
    w.varlong(flags);
    w.uvarint(4);
    w.uvarint(4);
    w.u8(4);
    w.string(name);
    w.uvarint(38);
    w.uvarint(3);
    w.u8(3);
    w.f32_le(1.0);
    w.uvarint(53);
    w.uvarint(3);
    w.u8(3);
    w.f32_le(0.6);
    w.uvarint(54);
    w.uvarint(3);
    w.u8(3);
    w.f32_le(1.8);
}

/// `feet` is the position of the feet (not the eyes).
pub fn add_player(p: &PlayerInfo, feet: [f32; 3], pitch: f32, yaw: f32, head_yaw: f32, gamemode: i32) -> Vec<u8> {
    packet(mp::ADD_PLAYER, |w| {
        w.bytes(&p.uuid);
        w.string(&p.name);
        w.uvarlong(p.entity_id as u64);
        w.string(""); // platform chat id
        for v in feet {
            w.f32_le(v);
        }
        for _ in 0..3 {
            w.f32_le(0.0); // motion
        }
        w.f32_le(pitch);
        w.f32_le(yaw);
        w.f32_le(head_yaw);
        // held item: air
        w.bytes(&[0, 0, 0, 0]);
        w.uvarint(0);
        w.bool(false);
        w.uvarint(0);
        w.string("");
        w.varint(gamemode);
        player_metadata(w, &p.name);
        w.uvarint(0); // synced int properties
        w.uvarint(0); // synced float properties
        write_abilities(
            w,
            &Abilities { entity_id: p.entity_id, operator: false, creative: false, spectator: false, flying: false },
        );
        w.uvarint(0); // links
        w.bool(false); // passenger of block
        w.string(&p.device_id);
        w.i32_le(p.build_platform);
    })
}

pub fn remove_actor(entity_id: i64) -> Vec<u8> {
    packet(mp::REMOVE_ACTOR, |w| w.varlong(entity_id))
}

/// `eyes` = feet + 1.62, which is what MovePlayer carries for players.
pub fn move_player(
    entity_id: i64,
    eyes: [f32; 3],
    pitch: f32,
    yaw: f32,
    head_yaw: f32,
    mode: u8,
    on_ground: bool,
) -> Vec<u8> {
    packet(mp::MOVE_PLAYER, |w| {
        w.uvarlong(entity_id as u64);
        for v in eyes {
            w.f32_le(v);
        }
        w.f32_le(pitch);
        w.f32_le(yaw);
        w.f32_le(head_yaw);
        w.u8(mode);
        w.bool(on_ground);
        w.uvarlong(0); // riding
        w.bool(false); // teleport data
        w.uvarlong(0); // tick
    })
}

pub const UPDATE_ATTRIBUTES: u32 = 0x1d;
pub const SET_ACTOR_DATA: u32 = 0x27;
pub const INVENTORY_CONTENT: u32 = 0x31;

const F32_MAX: f32 = f32::MAX;

/// Default player attributes (values from PocketMine's AttributeFactory). Without
/// minecraft:movement the client walks at speed 0 and can only fly.
pub fn update_attributes(entity_id: i64) -> Vec<u8> {
    let attrs: [(&str, f32, f32, f32); 12] = [
        ("minecraft:absorption", 0.0, F32_MAX, 0.0),
        ("minecraft:player.saturation", 0.0, 20.0, 20.0),
        ("minecraft:player.exhaustion", 0.0, 5.0, 0.0),
        ("minecraft:knockback_resistance", 0.0, 1.0, 0.0),
        ("minecraft:health", 0.0, 20.0, 20.0),
        ("minecraft:movement", 0.0, F32_MAX, 0.1),
        ("minecraft:player.hunger", 0.0, 20.0, 20.0),
        ("minecraft:player.level", 0.0, 24791.0, 0.0),
        ("minecraft:player.experience", 0.0, 1.0, 0.0),
        ("minecraft:underwater_movement", 0.0, F32_MAX, 0.02),
        ("minecraft:luck", -1024.0, 1024.0, 0.0),
        ("minecraft:lava_movement", 0.0, F32_MAX, 0.02),
    ];
    packet(UPDATE_ATTRIBUTES, |w| {
        w.uvarlong(entity_id as u64);
        w.uvarint(attrs.len() as u32);
        for (name, min, max, value) in attrs {
            w.f32_le(min);
            w.f32_le(max);
            w.f32_le(value);
            w.f32_le(min); // default min
            w.f32_le(max); // default max
            w.f32_le(value); // default
            w.string(name);
            w.uvarint(0); // modifiers
        }
        w.uvarlong(0); // tick
    })
}

/// The player's own entity metadata (flags such as gravity and collision).
pub fn set_actor_data(entity_id: i64, name: &str) -> Vec<u8> {
    packet(SET_ACTOR_DATA, |w| {
        w.uvarlong(entity_id as u64);
        player_metadata(w, name);
        w.uvarint(0); // synced int properties
        w.uvarint(0); // synced float properties
        w.uvarlong(0); // tick
    })
}

/// A whole inventory window filled with air.
pub fn empty_inventory(window_id: u32, slots: usize) -> Vec<u8> {
    const AIR: [u8; 8] = [0; 8]; // id, count, meta, no stack id, block id 0, no extra data
    packet(INVENTORY_CONTENT, |w| {
        w.uvarint(window_id);
        w.uvarint(slots as u32);
        for _ in 0..slots {
            w.bytes(&AIR);
        }
        w.u8(0); // container name: id 0
        w.bool(false); // no dynamic id
        w.bytes(&AIR); // storage item
    })
}
