//! Commands: AvailableCommands, CommandRequest, and server admin packets (time, gamemode, spawn,
//! difficulty, titles, actionbars, toasts, transfers).

use crate::packet;
use crate::packets::inventory::write_block_pos;
use ceres_codec::binary::{R, Res};

pub const SET_TIME: u32 = 0x0a;
pub const SET_SPAWN_POSITION: u32 = 0x2b;
pub const SET_DIFFICULTY: u32 = 0x3c;
pub const SET_PLAYER_GAME_TYPE: u32 = 0x3e;
pub const UPDATE_PLAYER_GAME_TYPE: u32 = 0x97;
pub const AVAILABLE_COMMANDS: u32 = 0x4c;
pub const COMMAND_REQUEST: u32 = 0x4d;
pub const TRANSFER: u32 = 0x55;
pub const SET_TITLE: u32 = 0x58;
pub const TOAST_REQUEST: u32 = 0xba;

pub mod title_type {
    pub const CLEAR: i32 = 0;
    pub const RESET: i32 = 1;
    pub const TITLE: i32 = 2;
    pub const SUBTITLE: i32 = 3;
    pub const ACTIONBAR: i32 = 4;
    pub const TIMES: i32 = 5;
}

const ARG_FLAG_VALID: u32 = 0x100000;
const ARG_FLAG_ENUM: u32 = 0x200000;

#[derive(Clone, Debug)]
pub enum ParamType {
    Int,
    Float,
    Target,
    /// x y z, accepts ~ relative coordinates
    Position,
    String,
    /// the rest of the line
    Message,
    /// one of the values of the named enum
    Enum(String),
}

impl ParamType {
    fn arg_type(&self) -> u32 {
        match self {
            ParamType::Int => 1,
            ParamType::Float => 3,
            ParamType::Target => 8,
            ParamType::Position => 65,
            ParamType::String => 56,
            ParamType::Message => 68,
            ParamType::Enum(_) => 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: ParamType,
    pub optional: bool,
}

impl Param {
    pub fn new(name: &str, ty: ParamType) -> Param {
        Param { name: name.into(), ty, optional: false }
    }
    pub fn opt(name: &str, ty: ParamType) -> Param {
        Param { name: name.into(), ty, optional: true }
    }
}

#[derive(Clone, Debug)]
pub struct CommandSpec {
    pub name: String,
    pub description: String,
    pub overloads: Vec<Vec<Param>>,
}

/// AvailableCommandsPacket. `enums` are (name, values) pairs referenced by ParamType::Enum.
pub fn available_commands(commands: &[CommandSpec], enums: &[(String, Vec<String>)]) -> Vec<u8> {
    // one shared string table for every enum value
    let mut values: Vec<String> = Vec::new();
    let mut enum_indexes: Vec<(String, Vec<u32>)> = Vec::new();
    for (name, vals) in enums {
        let idx = vals
            .iter()
            .map(|v| match values.iter().position(|x| x == v) {
                Some(i) => i as u32,
                None => {
                    values.push(v.clone());
                    (values.len() - 1) as u32
                }
            })
            .collect();
        enum_indexes.push((name.clone(), idx));
    }
    let enum_index = |name: &str| enum_indexes.iter().position(|(n, _)| n == name).unwrap_or(0) as u32;

    packet(AVAILABLE_COMMANDS, |w| {
        w.uvarint(values.len() as u32);
        for v in &values {
            w.string(v);
        }
        w.uvarint(0); // chained subcommand values
        w.uvarint(0); // postfixes
        w.uvarint(enum_indexes.len() as u32);
        for (name, idx) in &enum_indexes {
            w.string(name);
            w.uvarint(idx.len() as u32);
            for i in idx {
                w.u32_le(*i);
            }
        }
        w.uvarint(0); // chained subcommand data
        w.uvarint(commands.len() as u32);
        for c in commands {
            w.string(&c.name);
            w.string(&c.description);
            w.u16_le(0); // flags
            w.string("any"); // permission
            w.i32_le(-1); // alias enum
            w.uvarint(0); // chained subcommands
            w.uvarint(c.overloads.len() as u32);
            for o in &c.overloads {
                w.bool(false); // chaining
                w.uvarint(o.len() as u32);
                for p in o {
                    w.string(&p.name);
                    let ty = match &p.ty {
                        ParamType::Enum(e) => ARG_FLAG_VALID | ARG_FLAG_ENUM | enum_index(e),
                        other => ARG_FLAG_VALID | other.arg_type(),
                    };
                    w.u32_le(ty);
                    w.bool(p.optional);
                    w.u8(0); // flags
                }
            }
        }
        w.uvarint(0); // soft enums
        w.uvarint(0); // enum constraints
    })
}

/// CommandRequestPacket: the command line as typed (with the leading slash).
pub fn read_command_request(r: &mut R) -> Res<String> {
    r.string()
}

pub fn set_player_game_type(mode: i32) -> Vec<u8> {
    packet(SET_PLAYER_GAME_TYPE, |w| w.varint(mode))
}

/// Tells the other players someone's game mode (their client uses it, e.g. not to attack a
/// player it believes is in creative).
pub fn update_player_game_type(mode: i32, entity_id: i64) -> Vec<u8> {
    packet(UPDATE_PLAYER_GAME_TYPE, |w| {
        w.varint(mode);
        w.varlong(entity_id);
        w.uvarlong(0); // tick
    })
}

pub fn set_time(time: i32) -> Vec<u8> {
    packet(SET_TIME, |w| w.varint(time))
}

pub fn set_difficulty(difficulty: u32) -> Vec<u8> {
    packet(SET_DIFFICULTY, |w| w.uvarint(difficulty))
}

/// SetSpawnPositionPacket (0x2b) for the world spawn.
pub fn set_world_spawn(pos: [i32; 3]) -> Vec<u8> {
    packet(SET_SPAWN_POSITION, |w| {
        w.varint(1); // TYPE_WORLD_SPAWN
        write_block_pos(w, pos);
        w.varint(0); // dimension: overworld
        write_block_pos(w, [i32::MIN, i32::MIN, i32::MIN]);
    })
}

/// SetSpawnPositionPacket (0x2b) for a player's personal spawnpoint.
pub fn set_player_spawn(pos: [i32; 3]) -> Vec<u8> {
    packet(SET_SPAWN_POSITION, |w| {
        w.varint(0); // TYPE_PLAYER_SPAWN
        write_block_pos(w, pos);
        w.varint(0); // dimension: overworld
        write_block_pos(w, pos);
    })
}

/// SetTitlePacket (0x58).
pub fn set_title(ty: i32, text: &str, fade_in: i32, stay: i32, fade_out: i32) -> Vec<u8> {
    packet(SET_TITLE, |w| {
        w.varint(ty);
        w.string(text);
        w.varint(fade_in);
        w.varint(stay);
        w.varint(fade_out);
        w.string(""); // xuid
        w.string(""); // platform online id
        w.string(""); // filtered title text
    })
}

/// ToastRequestPacket (0xba).
pub fn toast_request(title: &str, body: &str) -> Vec<u8> {
    packet(TOAST_REQUEST, |w| {
        w.string(title);
        w.string(body);
    })
}

/// TransferPacket (0x55, protocol 2225).
pub fn transfer(address: &str, port: u16) -> Vec<u8> {
    packet(TRANSFER, |w| {
        w.string(address);
        w.u16_le(port);
        w.bool(false); // reload world
        w.bool(false); // optional GatheringJoinInfo
    })
}
