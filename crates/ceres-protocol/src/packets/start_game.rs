//! StartGame and the level settings it carries.

#![allow(unused_imports)]

use crate::packets::player::mp;
use crate::{COMPRESSION_ZLIB, VERSION, id, packet};
use ceres_codec::binary::{R, Res, W};

pub struct StartGame<'a> {
    pub entity_id: i64,
    pub gamemode: i32,
    pub position: [f32; 3],
    pub pitch: f32,
    pub yaw: f32,
    pub spawn: [i32; 3],
    pub difficulty: i32,
    pub time: i32,
    pub world_name: &'a str,
    pub software: &'a str,
    /// pre-encoded (name string + network NBT) entries from block_definitions.nbt
    pub block_definitions: &'a [u8],
    pub block_definition_count: u32,
    /// the "pvp" game rule: the client only sends attacks on players when it is on
    pub pvp: bool,
}

pub fn start_game(s: &StartGame) -> Vec<u8> {
    packet(id::START_GAME, |w| {
        w.varlong(s.entity_id);
        w.uvarlong(s.entity_id as u64);
        w.varint(s.gamemode);
        for v in s.position {
            w.f32_le(v);
        }
        w.f32_le(s.pitch);
        w.f32_le(s.yaw);

        // ---- LevelSettings
        w.u64_le(u64::MAX); // seed (-1)
        w.u16_le(0); // spawn settings: biome type default
        w.string(""); // biome name
        w.varint(0); // dimension overworld
        w.varint(1); // generator overworld
        w.varint(s.gamemode); // world gamemode
        w.bool(false); // hardcore
        w.varint(s.difficulty);
        w.varint(s.spawn[0]);
        w.varint(s.spawn[1]);
        w.varint(s.spawn[2]);
        w.bool(true); // achievements disabled
        w.varint(0); // editor world type
        w.bool(false); // created in editor
        w.bool(false); // exported from editor
        w.u8(0); // editor level migration version (legacy)
        w.varint(s.time);
        w.uvarint(0); // edu offer
        w.bool(true); // edu features enabled
        w.string(""); // edu product uuid
        w.f32_le(0.0); // rain
        w.f32_le(0.0); // lightning
        w.bool(false); // confirmed platform locked content
        w.bool(true); // multiplayer
        w.bool(true); // LAN broadcast
        w.varint(4); // xbox live broadcast: public
        w.varint(4); // platform broadcast: public
        w.bool(true); // commands enabled
        w.bool(false); // texture packs required
        // game rules
        w.uvarint(3);
        for (name, value) in [("naturalregeneration", false), ("locatorbar", false), ("pvp", s.pvp)] {
            w.string(name);
            w.bool(false); // player modifiable
            w.uvarint(1); // bool type
            w.bool(value);
        }
        w.u32_le(0); // experiments
        w.bool(false); // experiments previously used
        w.bool(false); // bonus chest
        w.bool(false); // start with map
        w.u8(1); // default permission: member
        w.i32_le(4); // server chunk tick radius
        w.bool(false); // locked behavior pack
        w.bool(false); // locked resource pack
        w.bool(false); // from locked world template
        w.bool(false); // msa gamertags only
        w.bool(false); // from world template
        w.bool(false); // world template option locked
        w.bool(false); // only spawn v1 villagers
        w.bool(false); // disable persona
        w.bool(false); // disable custom skins
        w.bool(false); // mute emote announcements
        w.string(VERSION); // vanilla version
        w.i32_le(0); // limited world width
        w.i32_le(0); // limited world length
        w.bool(true); // new nether
        w.string(""); // edu shared uri: button name
        w.string(""); // edu shared uri: link
        w.bool(false); // experimental gameplay override: none
        w.u8(0); // chat restriction level
        w.bool(false); // disable player interactions
        w.varint(0); // server editor connection policy
        w.bool(false); // allow anonymous block drops in editor worlds

        // ---- rest of StartGame
        w.string(""); // level id
        w.string(s.world_name);
        w.string(""); // premium world template id
        w.bool(false); // trial
        w.varint(0); // movement: rewind history size
        w.bool(true); // server authoritative block breaking
        w.u64_le(0); // current tick
        w.varint(0); // enchantment seed
        w.uvarint(s.block_definition_count);
        w.bytes(s.block_definitions);
        w.string(""); // multiplayer correlation id
        w.bool(true); // new inventory system
        w.string(s.software);
        w.bytes(&[0x0a, 0x00, 0x00]); // player actor properties: empty compound
        w.u64_le(0); // block palette checksum
        w.bytes(&[0u8; 16]); // world template id
        w.bool(false); // client side chunk generation
        w.bool(true); // block network ids are hashes
        w.bool(true); // network permissions: disable client sounds
        w.bool(false); // server join information: none
        for _ in 0..4 {
            w.string(""); // telemetry: server, scenario, world, owner
        }
    })
}
