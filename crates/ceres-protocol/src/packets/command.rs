//! Commands: the list shown to the client (AvailableCommands), requests, and the packets commands
//! need (game mode, time).

use crate::packet;
use ceres_codec::binary::{R, Res};

pub const SET_TIME: u32 = 0x0a;
pub const SET_PLAYER_GAME_TYPE: u32 = 0x3e;
pub const AVAILABLE_COMMANDS: u32 = 0x4c;
pub const COMMAND_REQUEST: u32 = 0x4d;

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

pub fn set_time(time: i32) -> Vec<u8> {
    packet(SET_TIME, |w| w.varint(time))
}
