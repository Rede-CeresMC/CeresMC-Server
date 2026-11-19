//! Player list, AddPlayer, movement, abilities, animations, emotes, skins, health/combat and item entities.

#![allow(unused_imports)]

use crate::packets::inventory::{INVENTORY_CONTENT, ItemDesc, write_item};
use crate::{COMPRESSION_ZLIB, VERSION, id, packet};
use ceres_codec::binary::{R, Res, W};

pub mod mp {
    pub const TEXT: u32 = 0x09;
    pub const ADD_PLAYER: u32 = 0x0c;
    pub const REMOVE_ACTOR: u32 = 0x0e;
    pub const ADD_ITEM_ACTOR: u32 = 0x0f;
    pub const TAKE_ITEM_ACTOR: u32 = 0x11;
    pub const MOVE_ACTOR_ABSOLUTE: u32 = 0x12;
    pub const MOVE_PLAYER: u32 = 0x13;
    pub const ACTOR_EVENT: u32 = 0x1b;
    pub const SET_ACTOR_MOTION: u32 = 0x28;
    pub const ANIMATE: u32 = 0x2c;
    pub const RESPAWN: u32 = 0x2d;
    pub const PLAYER_LIST: u32 = 0x3f;
    pub const PLAYER_SKIN: u32 = 0x5d;
    pub const EMOTE: u32 = 0x8a;
    pub const UPDATE_ABILITIES: u32 = 0xbb;
}

pub mod actor_event {
    pub const HURT_ANIMATION: u8 = 2;
    pub const DEATH_ANIMATION: u8 = 3;
    pub const RESPAWN: u8 = 18;
}

pub mod respawn_state {
    pub const SEARCHING_FOR_SPAWN: u8 = 0;
    pub const READY_TO_SPAWN: u8 = 1;
    pub const CLIENT_READY_TO_SPAWN: u8 = 2;
}

/// Synced entity state flags for a player (EntityMetadataFlags).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActorState {
    pub sneaking: bool,
    pub sprinting: bool,
    pub swimming: bool,
    pub gliding: bool,
    pub crawling: bool,
    pub emoting: bool,
    pub on_fire: bool,
    /// ticks of air used up under water (0 = full air)
    pub air_lost: i16,
    /// bed the player sleeps in
    pub sleeping: Option<[i32; 3]>,
    /// raising a shield
    pub blocking: bool,
}

/// Ticks of air a full bubble bar holds.
pub const MAX_AIR_SUPPLY: i16 = 300;

impl ActorState {
    fn flags(&self) -> (i64, i64) {
        let mut f1: i64 = (1 << 14) | (1 << 15) | (1 << 48) | (1 << 49);
        let mut f2: i64 = 0;
        if self.air_lost == 0 {
            f1 |= 1 << 35; // BREATHING: no bubbles shown
        }
        if self.on_fire {
            f1 |= 1;
        }
        if self.sneaking {
            f1 |= 1 << 1;
        }
        if self.sprinting {
            f1 |= 1 << 3;
        }
        if self.gliding {
            f1 |= 1 << 32;
        }
        if self.swimming {
            f1 |= 1 << 57;
        }
        if self.emoting {
            f2 |= 1 << (92 - 64);
        }
        if self.crawling {
            f2 |= 1 << (114 - 64);
        }
        if self.blocking {
            f2 |= 1 << (72 - 64);
        }
        (f1, f2)
    }

    pub fn hitbox_height(&self) -> f32 {
        if self.swimming || self.gliding || self.crawling {
            0.6
        } else if self.sneaking {
            1.5
        } else {
            1.8
        }
    }
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
    /// walks through blocks like a spectator, in any game mode
    pub no_clip: bool,
    /// may fly outside creative/spectator
    pub may_fly: bool,
}

pub fn write_abilities(w: &mut W, a: &Abilities) {
    w.bytes(&a.entity_id.to_le_bytes());
    w.u8(if a.operator { 2 } else { 1 }); // player permission
    w.u8(if a.operator { 1 } else { 0 }); // command permission
    w.u8(1); // layers
    w.u16_le(1); // base layer
    let act = !a.spectator;
    let bools: [(u32, bool); 17] = [
        (10, a.creative || a.spectator || a.may_fly), // allow flight
        (9, a.flying || a.spectator),                 // flying
        (17, a.spectator || a.no_clip),               // no clip
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

fn player_metadata(w: &mut W, name: &str, state: &ActorState) {
    let (flags, flags2) = state.flags();
    w.uvarint(if state.sleeping.is_some() { 11 } else { 10 });
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
    w.f32_le(state.hitbox_height());
    w.uvarint(92); // FLAGS2
    w.uvarint(7);
    w.u8(7);
    w.varlong(flags2);
    w.uvarint(7); // AIR (short)
    w.uvarint(1);
    w.u8(1);
    w.i16_le(MAX_AIR_SUPPLY - state.air_lost.clamp(0, MAX_AIR_SUPPLY));
    w.uvarint(42); // MAX_AIR (short)
    w.uvarint(1);
    w.u8(1);
    w.i16_le(MAX_AIR_SUPPLY);
    w.uvarint(81); // ALWAYS_SHOW_NAMETAG (byte): 1 = the name shows whenever the player is drawn
    w.uvarint(0);
    w.u8(0);
    w.u8(1);
    w.uvarint(26); // PLAYER_FLAGS (byte): bit 1 = sleeping
    w.uvarint(0);
    w.u8(0);
    w.u8(if state.sleeping.is_some() { 1 << 1 } else { 0 });
    // the bed position is only sent while sleeping (a 0,0,0 position moves the player there)
    if let Some(bed) = state.sleeping {
        w.uvarint(28); // PLAYER_BED_POSITION (block position)
        w.uvarint(6);
        w.u8(6);
        w.varint(bed[0]);
        w.varint(bed[1]);
        w.varint(bed[2]);
    }
}

/// `feet` is the position of the feet (not the eyes).
#[allow(clippy::too_many_arguments)]
pub fn add_player(
    p: &PlayerInfo,
    feet: [f32; 3],
    pitch: f32,
    yaw: f32,
    head_yaw: f32,
    gamemode: i32,
    held: &ItemDesc,
    nametag: &str,
    state: &ActorState,
    abilities: &Abilities,
) -> Vec<u8> {
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
        write_item(w, held);
        w.varint(gamemode);
        player_metadata(w, nametag, state);
        w.uvarint(0); // synced int properties
        w.uvarint(0); // synced float properties
        write_abilities(w, abilities);
        w.uvarint(0); // links
        w.bool(false); // passenger of block
        w.string(&p.device_id);
        w.i32_le(p.build_platform);
    })
}

pub const ADD_ACTOR: u32 = 0x0d;

/// AddActor for a falling block (entity type minecraft:falling_block, block in VARIANT).
pub fn add_falling_block(entity_id: i64, pos: [f32; 3], block: i32) -> Vec<u8> {
    packet(ADD_ACTOR, |w| {
        w.varlong(entity_id);
        w.uvarlong(entity_id as u64);
        w.string("minecraft:falling_block");
        for v in pos {
            w.f32_le(v);
        }
        for _ in 0..3 {
            w.f32_le(0.0); // motion
        }
        for _ in 0..4 {
            w.f32_le(0.0); // pitch, yaw, head yaw, body yaw
        }
        w.uvarint(0); // attributes
        w.uvarint(3); // metadata: key, type, type again, value
        w.uvarint(2); // VARIANT: the block
        w.uvarint(2);
        w.u8(2);
        w.varint(block);
        w.uvarint(53); // bounding box width
        w.uvarint(3);
        w.u8(3);
        w.f32_le(0.98);
        w.uvarint(54); // bounding box height
        w.uvarint(3);
        w.u8(3);
        w.f32_le(0.98);
        w.uvarint(0); // synced int properties
        w.uvarint(0); // synced float properties
        w.uvarint(0); // links
        w.bool(false); // passenger of block
    })
}

/// AddActor for a mob: health attribute, gravity and collision, its size, and optionally a
/// colour (sheep wool) and the baby flag.
#[allow(clippy::too_many_arguments)]
pub fn add_mob(
    entity_id: i64,
    kind: &str,
    pos: [f32; 3],
    yaw: f32,
    health: f32,
    max_health: f32,
    size: [f32; 2],
    color: Option<u8>,
    flags: i64,
) -> Vec<u8> {
    packet(ADD_ACTOR, |w| {
        w.varlong(entity_id);
        w.uvarlong(entity_id as u64);
        w.string(kind);
        for v in pos {
            w.f32_le(v);
        }
        for _ in 0..3 {
            w.f32_le(0.0); // motion
        }
        w.f32_le(0.0); // pitch
        w.f32_le(yaw);
        w.f32_le(yaw); // head yaw
        w.f32_le(yaw); // body yaw
        w.uvarint(1); // attributes
        w.string("minecraft:health");
        w.f32_le(0.0);
        w.f32_le(health);
        w.f32_le(max_health);
        w.uvarint(if color.is_some() { 4 } else { 3 });
        w.uvarint(0); // FLAGS
        w.uvarint(7);
        w.u8(7);
        w.varlong(flags);
        w.uvarint(53); // BOUNDING_BOX_WIDTH
        w.uvarint(3);
        w.u8(3);
        w.f32_le(size[0]);
        w.uvarint(54); // BOUNDING_BOX_HEIGHT
        w.uvarint(3);
        w.u8(3);
        w.f32_le(size[1]);
        if let Some(c) = color {
            w.uvarint(3); // COLOR (byte)
            w.uvarint(0);
            w.u8(0);
            w.u8(c);
        }
        w.uvarint(0); // synced int properties
        w.uvarint(0); // synced float properties
        w.uvarint(0); // links
        w.bool(false);
    })
}

/// SetActorData with only the flags (and the wool colour of a sheep).
pub fn set_mob_flags(entity_id: i64, flags: i64, color: Option<u8>) -> Vec<u8> {
    packet(SET_ACTOR_DATA, |w| {
        w.uvarlong(entity_id as u64);
        w.uvarint(if color.is_some() { 2 } else { 1 });
        w.uvarint(0);
        w.uvarint(7);
        w.u8(7);
        w.varlong(flags);
        if let Some(c) = color {
            w.uvarint(3);
            w.uvarint(0);
            w.u8(0);
            w.u8(c);
        }
        w.uvarint(0); // synced int properties
        w.uvarint(0); // synced float properties
        w.uvarlong(0); // tick
    })
}

/// MoveActorAbsolute with the body and head turned (degrees).
pub fn move_mob(entity_id: i64, pos: [f32; 3], yaw: f32, on_ground: bool) -> Vec<u8> {
    packet(mp::MOVE_ACTOR_ABSOLUTE, |w| {
        w.uvarlong(entity_id as u64);
        w.u8(on_ground as u8);
        for v in pos {
            w.f32_le(v);
        }
        let angle = ((yaw.rem_euclid(360.0) / 360.0) * 256.0) as u8;
        w.u8(0); // pitch
        w.u8(angle); // yaw
        w.u8(angle); // head yaw
    })
}

/// AddActor for an invisible container entity (plugin menus): no model, no collision, no
/// gravity, no nametag; its inventory size comes from CONTAINER_BASE_SIZE.
pub fn add_menu_entity(entity_id: i64, kind: &str, pos: [f32; 3], title: &str, slots: i32) -> Vec<u8> {
    packet(ADD_ACTOR, |w| {
        w.varlong(entity_id);
        w.uvarlong(entity_id as u64);
        w.string(kind);
        for v in pos {
            w.f32_le(v);
        }
        for _ in 0..7 {
            w.f32_le(0.0); // motion, pitch, yaw, head yaw, body yaw
        }
        w.uvarint(0); // attributes
        w.uvarint(8); // metadata: key, type, type again, value
        w.uvarint(0); // FLAGS: only INVISIBLE (no gravity, no collision, nametag hidden)
        w.uvarint(7);
        w.u8(7);
        w.varlong(1 << 5);
        w.uvarint(4); // NAMETAG: the window title
        w.uvarint(4);
        w.u8(4);
        w.string(title);
        w.uvarint(38); // SCALE
        w.uvarint(3);
        w.u8(3);
        w.f32_le(0.0);
        w.uvarint(53); // BOUNDING_BOX_WIDTH
        w.uvarint(3);
        w.u8(3);
        w.f32_le(0.0);
        w.uvarint(54); // BOUNDING_BOX_HEIGHT
        w.uvarint(3);
        w.u8(3);
        w.f32_le(0.0);
        w.uvarint(44); // CONTAINER_TYPE: plain container
        w.uvarint(0);
        w.u8(0);
        w.u8(0);
        w.uvarint(45); // CONTAINER_BASE_SIZE
        w.uvarint(2);
        w.u8(2);
        w.varint(slots);
        w.uvarint(46); // CONTAINER_EXTRA_SLOTS_PER_STRENGTH
        w.uvarint(2);
        w.u8(2);
        w.varint(0);
        w.uvarint(0); // synced int properties
        w.uvarint(0); // synced float properties
        w.uvarint(0); // links
        w.bool(false);
    })
}

/// ContainerOpen of an entity's inventory.
pub fn open_entity_container(window_id: u8, window_type: u8, pos: [i32; 3], entity_id: i64) -> Vec<u8> {
    crate::packet(crate::packets::inventory::CONTAINER_OPEN, |w| {
        w.u8(window_id);
        w.u8(window_type);
        w.varint(pos[0]);
        w.varint(pos[1]);
        w.varint(pos[2]);
        w.varlong(entity_id);
    })
}

/// AddActor for a projectile (snowball, egg, ender pearl, arrow) with its shooter.
pub fn add_projectile(entity_id: i64, kind: &str, pos: [f32; 3], motion: [f32; 3], owner: i64, potion: Option<i16>) -> Vec<u8> {
    let (pitch, yaw) = projectile_rotation(motion);
    packet(ADD_ACTOR, |w| {
        w.varlong(entity_id);
        w.uvarlong(entity_id as u64);
        w.string(kind);
        for v in pos.into_iter().chain(motion) {
            w.f32_le(v);
        }
        w.f32_le(pitch);
        w.f32_le(yaw);
        w.f32_le(yaw); // head yaw
        w.f32_le(yaw); // body yaw
        w.uvarint(0); // attributes
        w.uvarint(if potion.is_some() { 2 } else { 1 }); // metadata
        w.uvarint(5); // OWNER_EID (long)
        w.uvarint(7);
        w.u8(7);
        w.varlong(owner);
        if let Some(aux) = potion {
            w.uvarint(36); // POTION_AUX_VALUE (short): which potion, for its colour
            w.uvarint(1);
            w.u8(1);
            w.i16_le(aux);
        }
        w.uvarint(0);
        w.uvarint(0);
        w.uvarint(0);
        w.bool(false);
    })
}

/// (pitch, yaw) in degrees that make an arrow-like entity point along its motion.
pub fn projectile_rotation(motion: [f32; 3]) -> (f32, f32) {
    let h = (motion[0] * motion[0] + motion[2] * motion[2]).sqrt();
    (motion[1].atan2(h).to_degrees(), motion[0].atan2(motion[2]).to_degrees())
}

/// MoveActorAbsolute for a projectile, turned along its motion.
pub fn move_projectile(entity_id: i64, pos: [f32; 3], motion: [f32; 3], on_ground: bool) -> Vec<u8> {
    let (pitch, yaw) = projectile_rotation(motion);
    move_entity(entity_id, pos, pitch, yaw, yaw, on_ground)
}

/// AddActor for a firework rocket; `item_nbt` is the network NBT of {Fireworks: {...}}.
pub fn add_firework(entity_id: i64, pos: [f32; 3], motion: [f32; 3], item_nbt: &[u8]) -> Vec<u8> {
    packet(ADD_ACTOR, |w| {
        w.varlong(entity_id);
        w.uvarlong(entity_id as u64);
        w.string("minecraft:fireworks_rocket");
        for v in pos.into_iter().chain(motion) {
            w.f32_le(v);
        }
        for _ in 0..4 {
            w.f32_le(0.0);
        }
        w.uvarint(0); // attributes
        w.uvarint(1); // metadata
        w.uvarint(16); // FIREWORK_ITEM (compound tag)
        w.uvarint(5);
        w.u8(5);
        w.bytes(item_nbt);
        w.uvarint(0);
        w.uvarint(0);
        w.uvarint(0);
        w.bool(false);
    })
}

/// AddActor for an experience orb worth `value` points.
pub fn add_xp_orb(entity_id: i64, pos: [f32; 3], motion: [f32; 3], value: i32) -> Vec<u8> {
    packet(ADD_ACTOR, |w| {
        w.varlong(entity_id);
        w.uvarlong(entity_id as u64);
        w.string("minecraft:xp_orb");
        for v in pos.into_iter().chain(motion) {
            w.f32_le(v);
        }
        for _ in 0..4 {
            w.f32_le(0.0);
        }
        w.uvarint(0); // attributes
        w.uvarint(1); // metadata
        w.uvarint(15); // EXPERIENCE_VALUE
        w.uvarint(2);
        w.u8(2);
        w.varint(value);
        w.uvarint(0);
        w.uvarint(0);
        w.uvarint(0);
        w.bool(false);
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
        // teleports carry a cause and the item that caused them (both 0, like Altay)
        w.bool(mode == 2);
        if mode == 2 {
            w.i32_le(0);
            w.i32_le(0);
        }
        w.uvarlong(0); // tick
    })
}

pub const UPDATE_ATTRIBUTES: u32 = 0x1d;
pub const SET_ACTOR_DATA: u32 = 0x27;

const F32_MAX: f32 = f32::MAX;

/// Default player attributes (values from PocketMine's AttributeFactory). Without
/// minecraft:movement the client walks at speed 0 and can only fly.
pub fn update_attributes_health(entity_id: i64, health: f32) -> Vec<u8> {
    update_attributes_vitals(entity_id, &Vitals { health, ..Vitals::default() })
}

/// Everything the player attributes show.
#[derive(Clone, Copy, Debug)]
pub struct Vitals {
    pub health: f32,
    pub hunger: f32,
    pub saturation: f32,
    pub exhaustion: f32,
    pub absorption: f32,
    /// sent only when it changes (speed/slowness, spawn): a plain value without modifiers
    /// wipes the client's own sprint modifier, which feels like the movement resetting
    pub movement: Option<f32>,
    pub xp_level: u32,
    /// 0..1 towards the next level
    pub xp_progress: f32,
}

impl Default for Vitals {
    fn default() -> Self {
        Vitals {
            health: 20.0,
            hunger: 20.0,
            saturation: 5.0,
            exhaustion: 0.0,
            absorption: 0.0,
            movement: Some(0.1),
            xp_level: 0,
            xp_progress: 0.0,
        }
    }
}

pub const MOB_EFFECT: u32 = 0x1c;

/// MobEffectPacket: add (1), modify (2) or remove (3) a status effect. Duration -1 = infinite.
pub fn mob_effect(entity_id: i64, event: u8, effect: i32, amplifier: i32, particles: bool, duration: i32) -> Vec<u8> {
    packet(MOB_EFFECT, |w| {
        w.uvarlong(entity_id as u64);
        w.u8(event);
        w.varint(effect);
        w.varint(amplifier);
        w.bool(particles);
        w.varint(duration);
        w.uvarlong(0); // tick
        w.bool(false); // ambient
    })
}

/// Every player attribute, with the real health, hunger, saturation and exhaustion.
pub fn update_attributes_vitals(entity_id: i64, v: &Vitals) -> Vec<u8> {
    let Vitals { health, hunger, saturation, exhaustion, absorption, movement, xp_level, xp_progress } = *v;
    let mut attrs: Vec<(&str, f32, f32, f32, f32)> = vec![
        ("minecraft:absorption", 0.0, F32_MAX, absorption.max(0.0), 0.0),
        ("minecraft:player.saturation", 0.0, 20.0, saturation.clamp(0.0, 20.0), 5.0),
        ("minecraft:player.exhaustion", 0.0, 5.0, exhaustion.clamp(0.0, 5.0), 0.0),
        ("minecraft:knockback_resistance", 0.0, 1.0, 0.0, 0.0),
        ("minecraft:health", 0.0, 20.0, health.clamp(0.0, 20.0), 20.0),
        ("minecraft:player.hunger", 0.0, 20.0, hunger.clamp(0.0, 20.0), 20.0),
        ("minecraft:player.level", 0.0, 24791.0, (xp_level as f32).min(24791.0), 0.0),
        ("minecraft:player.experience", 0.0, 1.0, xp_progress.clamp(0.0, 1.0), 0.0),
        ("minecraft:underwater_movement", 0.0, F32_MAX, 0.02, 0.02),
        ("minecraft:luck", -1024.0, 1024.0, 0.0, 0.0),
        ("minecraft:lava_movement", 0.0, F32_MAX, 0.02, 0.02),
    ];
    if let Some(m) = movement {
        attrs.push(("minecraft:movement", 0.0, F32_MAX, m.max(0.0), 0.1));
    }
    packet(UPDATE_ATTRIBUTES, |w| {
        w.uvarlong(entity_id as u64);
        w.uvarint(attrs.len() as u32);
        for (name, min, max, value, def) in attrs {
            w.f32_le(min);
            w.f32_le(max);
            w.f32_le(value);
            w.f32_le(min); // default min
            w.f32_le(max); // default max
            w.f32_le(def); // default
            w.string(name);
            w.uvarint(0); // modifiers
        }
        w.uvarlong(0); // tick
    })
}

pub fn update_attributes(entity_id: i64) -> Vec<u8> {
    update_attributes_health(entity_id, 20.0)
}

/// Entity metadata with custom nametag and state flags (sneak, sprint, swim, glide, crawl, emote).
pub fn set_actor_data_state(entity_id: i64, nametag: &str, state: &ActorState) -> Vec<u8> {
    packet(SET_ACTOR_DATA, |w| {
        w.uvarlong(entity_id as u64);
        player_metadata(w, nametag, state);
        w.uvarint(0); // synced int properties
        w.uvarint(0); // synced float properties
        w.uvarlong(0); // tick
    })
}

pub fn set_actor_data(entity_id: i64, name: &str) -> Vec<u8> {
    set_actor_data_state(entity_id, name, &ActorState::default())
}

/// A whole inventory window filled with air.
pub fn empty_inventory(window_id: u32, slots: usize) -> Vec<u8> {
    const AIR: [u8; 8] = [0; 8];
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

/// AnimatePacket (protocol 2225): u8 action, uvarlong runtimeId, f32 data, optional string, u8 handSlot.
pub fn animate(entity_id: i64, action: u8) -> Vec<u8> {
    packet(mp::ANIMATE, |w| {
        w.u8(action);
        w.uvarlong(entity_id as u64);
        w.f32_le(0.0);
        w.bool(false); // no swing source
        w.u8(0); // HandSlot::MAIN_HAND
    })
}

pub fn animate_swing(entity_id: i64) -> Vec<u8> {
    animate(entity_id, 1)
}

pub fn read_animate(r: &mut R) -> Res<(u8, i64)> {
    let action = r.u8()?;
    let eid = r.uvarlong()? as i64;
    let _data = r.f32_le()?;
    if r.bool()? {
        r.string()?;
    }
    let _hand = r.u8()?;
    Ok((action, eid))
}

/// EmotePacket (0x8a).
pub fn emote(entity_id: i64, emote_id: &str, ticks: u32, xuid: &str, platform_id: &str, flags: u8) -> Vec<u8> {
    packet(mp::EMOTE, |w| {
        w.uvarlong(entity_id as u64);
        w.string(emote_id);
        w.uvarint(ticks);
        w.string(xuid);
        w.string(platform_id);
        w.u8(flags);
    })
}

pub fn read_emote(r: &mut R) -> Res<(String, u32, u8)> {
    let _eid = r.uvarlong()?;
    let emote_id = r.string()?;
    let ticks = r.uvarint()?;
    let _xuid = r.string()?;
    let _platform = r.string()?;
    let flags = r.u8()?;
    Ok((emote_id, ticks, flags))
}

/// Advances `r` past one CommonTypes::getSkin() payload and returns its exact wire bytes.
pub fn read_skin_bytes<'a>(r: &mut R<'a>) -> Res<&'a [u8]> {
    let start = r.p;
    let skip_str = |r: &mut R<'a>| -> Res<()> {
        let n = r.uvarint()? as usize;
        r.take(n)?;
        Ok(())
    };
    let skip_img = |r: &mut R<'a>| -> Res<()> {
        r.u32_le()?;
        r.u32_le()?;
        skip_str(r)
    };
    skip_str(r)?; // skinId
    skip_str(r)?; // resourcePatch
    skip_img(r)?; // skinImage
    let anims = r.uvarint()?.min(128);
    for _ in 0..anims {
        skip_img(r)?;
        r.uvarint()?;
        r.f32_le()?;
        r.uvarint()?;
    }
    skip_img(r)?; // capeImage
    skip_str(r)?; // geometryData
    skip_str(r)?; // geometryDataEngineVersion
    skip_str(r)?; // animationData
    skip_str(r)?; // capeId
    skip_str(r)?; // fullSkinId
    r.u8()?; // armSize
    r.i32_le()?; // skinColor
    let pieces = r.uvarint()?.min(128);
    for _ in 0..pieces {
        skip_str(r)?;
        r.i32_le()?;
        r.take(16)?;
        r.bool()?;
        skip_str(r)?;
    }
    let tints = r.uvarint()?.min(128);
    for _ in 0..tints {
        skip_str(r)?;
        for _ in 0..4 {
            r.i32_le()?;
        }
    }
    for _ in 0..5 {
        r.bool()?;
    }
    skip_str(r)?; // trustedSkinFlag
    skip_str(r)?; // profileHash
    Ok(&r.b[start..r.p])
}

/// PlayerSkinPacket (0x5d): returns (skin_wire_bytes, old_name, new_name).
pub fn read_player_skin(r: &mut R) -> Res<(Vec<u8>, String, String)> {
    let _uuid = r.take(16)?;
    let skin = read_skin_bytes(r)?.to_vec();
    let old_name = r.string()?;
    let new_name = r.string()?;
    Ok((skin, old_name, new_name))
}

pub fn player_skin(uuid: &[u8; 16], skin: &[u8], old_name: &str, new_name: &str) -> Vec<u8> {
    packet(mp::PLAYER_SKIN, |w| {
        w.bytes(uuid);
        w.bytes(skin);
        w.string(old_name);
        w.string(new_name);
    })
}

/// ActorEventPacket (0x1b): hurt flash, death animation, etc.
pub fn actor_event(entity_id: i64, event_id: u8, data: i32) -> Vec<u8> {
    packet(mp::ACTOR_EVENT, |w| {
        w.uvarlong(entity_id as u64);
        w.u8(event_id);
        w.varint(data);
        w.bool(false); // optional firePosition
    })
}

/// SetActorMotionPacket (0x28): knockback / velocity.
pub fn set_actor_motion(entity_id: i64, motion: [f32; 3]) -> Vec<u8> {
    packet(mp::SET_ACTOR_MOTION, |w| {
        w.uvarlong(entity_id as u64);
        for v in motion {
            w.f32_le(v);
        }
        w.uvarlong(0); // tick
    })
}

/// RespawnPacket (0x2d).
pub fn respawn(pos: [f32; 3], state: u8, entity_id: i64) -> Vec<u8> {
    packet(mp::RESPAWN, |w| {
        for v in pos {
            w.f32_le(v);
        }
        w.u8(state);
        w.uvarlong(entity_id as u64);
    })
}

pub fn read_respawn(r: &mut R) -> Res<([f32; 3], u8, i64)> {
    let pos = [r.f32_le()?, r.f32_le()?, r.f32_le()?];
    let state = r.u8()?;
    let eid = r.uvarlong()? as i64;
    Ok((pos, state, eid))
}

/// AddItemActorPacket (0x0f): spawns a dropped item entity in the world.
pub fn add_item_actor(entity_id: i64, item: &ItemDesc, pos: [f32; 3], motion: [f32; 3]) -> Vec<u8> {
    let mut legacy = item.clone();
    legacy.stack_id = 0;
    packet(mp::ADD_ITEM_ACTOR, |w| {
        w.varlong(entity_id);
        w.uvarlong(entity_id as u64);
        write_item(w, &legacy);
        for v in pos {
            w.f32_le(v);
        }
        for v in motion {
            w.f32_le(v);
        }
        // metadata: affected by gravity + collision, hitbox 0.25 x 0.25
        let flags: i64 = (1 << 48) | (1 << 49);
        w.uvarint(3);
        w.uvarint(0);
        w.uvarint(7);
        w.u8(7);
        w.varlong(flags);
        w.uvarint(53);
        w.uvarint(3);
        w.u8(3);
        w.f32_le(0.25);
        w.uvarint(54);
        w.uvarint(3);
        w.u8(3);
        w.f32_le(0.25);
        w.bool(false); // isFromFishing
    })
}

/// TakeItemActorPacket (0x11): plays the item pickup animation toward `taker_eid`.
pub fn take_item_actor(item_eid: i64, taker_eid: i64) -> Vec<u8> {
    packet(mp::TAKE_ITEM_ACTOR, |w| {
        w.uvarlong(item_eid as u64);
        w.uvarlong(taker_eid as u64);
    })
}

/// MoveActorAbsolutePacket (0x12): updates non-player entity position.
pub fn move_actor_absolute(entity_id: i64, pos: [f32; 3], on_ground: bool, teleport: bool) -> Vec<u8> {
    packet(mp::MOVE_ACTOR_ABSOLUTE, |w| {
        w.uvarlong(entity_id as u64);
        let flags = (on_ground as u8) | ((teleport as u8) << 1);
        w.u8(flags);
        for v in pos {
            w.f32_le(v);
        }
        w.u8(0); // pitch
        w.u8(0); // yaw
        w.u8(0); // head yaw
    })
}

/// One entity metadata value (key, value) for [`add_entity`] and [`set_actor_metadata`].
#[derive(Clone, Debug, PartialEq)]
pub enum Meta {
    Byte(u32, u8),
    Short(u32, i16),
    Int(u32, i32),
    Float(u32, f32),
    Str(u32, String),
    Long(u32, i64),
    Vec3(u32, [f32; 3]),
}

/// Metadata keys used for mobs.
pub mod meta_key {
    pub const FLAGS: u32 = 0;
    pub const VARIANT: u32 = 2;
    pub const COLOR: u32 = 3;
    pub const NAMETAG: u32 = 4;
    pub const OWNER_EID: u32 = 5;
    pub const TARGET_EID: u32 = 6;
    pub const AIR: u32 = 7;
    pub const SCALE: u32 = 38;
    pub const MAX_AIR: u32 = 42;
    pub const MARK_VARIANT: u32 = 43;
    pub const BOUNDING_BOX_WIDTH: u32 = 53;
    pub const BOUNDING_BOX_HEIGHT: u32 = 54;
    pub const FUSE_LENGTH: u32 = 55;
    pub const POSE_INDEX: u32 = 79;
    pub const COLOR_2: u32 = 82;
    pub const SKIN_ID: u32 = 104;
    pub const ALWAYS_SHOW_NAMETAG: u32 = 81;
    pub const FLAGS2: u32 = 92;
}

fn write_meta(w: &mut W, meta: &[Meta]) {
    w.uvarint(meta.len() as u32);
    for m in meta {
        // key, type, type again, value
        match m {
            Meta::Byte(k, v) => {
                w.uvarint(*k);
                w.uvarint(0);
                w.u8(0);
                w.u8(*v);
            }
            Meta::Short(k, v) => {
                w.uvarint(*k);
                w.uvarint(1);
                w.u8(1);
                w.i16_le(*v);
            }
            Meta::Int(k, v) => {
                w.uvarint(*k);
                w.uvarint(2);
                w.u8(2);
                w.varint(*v);
            }
            Meta::Float(k, v) => {
                w.uvarint(*k);
                w.uvarint(3);
                w.u8(3);
                w.f32_le(*v);
            }
            Meta::Str(k, v) => {
                w.uvarint(*k);
                w.uvarint(4);
                w.u8(4);
                w.string(v);
            }
            Meta::Long(k, v) => {
                w.uvarint(*k);
                w.uvarint(7);
                w.u8(7);
                w.varlong(*v);
            }
            Meta::Vec3(k, v) => {
                w.uvarint(*k);
                w.uvarint(8);
                w.u8(8);
                for f in v {
                    w.f32_le(*f);
                }
            }
        }
    }
}

/// AddActor for any entity: attributes as (name, min, value, max) and the metadata given.
#[allow(clippy::too_many_arguments)]
pub fn add_entity(
    entity_id: i64,
    kind: &str,
    pos: [f32; 3],
    motion: [f32; 3],
    pitch: f32,
    yaw: f32,
    head_yaw: f32,
    attributes: &[(&str, f32, f32, f32)],
    meta: &[Meta],
) -> Vec<u8> {
    packet(ADD_ACTOR, |w| {
        w.varlong(entity_id);
        w.uvarlong(entity_id as u64);
        w.string(kind);
        for v in pos.into_iter().chain(motion) {
            w.f32_le(v);
        }
        w.f32_le(pitch);
        w.f32_le(yaw);
        w.f32_le(head_yaw);
        w.f32_le(yaw); // body yaw
        w.uvarint(attributes.len() as u32);
        for (name, min, value, max) in attributes {
            w.string(name);
            w.f32_le(*min);
            w.f32_le(*value);
            w.f32_le(*max);
        }
        write_meta(w, meta);
        w.uvarint(0); // synced int properties
        w.uvarint(0); // synced float properties
        w.uvarint(0); // links
        w.bool(false);
    })
}

/// SetActorData with the metadata given.
pub fn set_actor_metadata(entity_id: i64, meta: &[Meta]) -> Vec<u8> {
    packet(SET_ACTOR_DATA, |w| {
        w.uvarlong(entity_id as u64);
        write_meta(w, meta);
        w.uvarint(0); // synced int properties
        w.uvarint(0); // synced float properties
        w.uvarlong(0); // tick
    })
}

/// MoveActorAbsolute with pitch, body yaw and head yaw (degrees).
pub fn move_entity(entity_id: i64, pos: [f32; 3], pitch: f32, yaw: f32, head_yaw: f32, on_ground: bool) -> Vec<u8> {
    packet(mp::MOVE_ACTOR_ABSOLUTE, |w| {
        w.uvarlong(entity_id as u64);
        w.u8(on_ground as u8);
        for v in pos {
            w.f32_le(v);
        }
        let angle = |a: f32| ((a.rem_euclid(360.0) / 360.0) * 256.0) as u8;
        w.u8(angle(pitch));
        w.u8(angle(yaw));
        w.u8(angle(head_yaw));
    })
}

/// PlayerStartItemCooldown (176): the client greys out items of that cooldown category.
pub fn start_item_cooldown(category: &str, ticks: i32) -> Vec<u8> {
    packet(0xb0, |w| {
        w.string(category);
        w.varint(ticks);
    })
}

/// SetActorLink (41): `rider` gets on (type 1 driver, 2 passenger) or off (0) `vehicle`.
pub fn set_actor_link(vehicle: i64, rider: i64, link_type: u8, immediate: bool, passenger_initiated: bool) -> Vec<u8> {
    packet(0x29, |w| {
        w.varlong(vehicle);
        w.varlong(rider);
        w.u8(link_type);
        w.bool(immediate);
        w.bool(passenger_initiated);
        w.f32_le(0.0); // vehicle angular velocity
    })
}

/// ClientboundMapItemData (0x43): the whole 128x128 image of a map. `pixels` are RGBA.
pub fn map_item_data(map_id: i64, dimension: u8, origin: [i32; 3], scale: u8, pixels: &[[u8; 4]]) -> Vec<u8> {
    packet(0x43, |w| {
        w.varlong(map_id);
        w.u8(dimension);
        w.bool(false); // locked
        for v in origin {
            w.varint(v);
        }
        w.bool(true); // maps included in
        w.uvarint(1);
        w.varlong(map_id);
        w.bool(true); // scale
        w.u8(scale);
        w.bool(false); // tracked objects
        w.bool(false); // decorations
        w.bool(true); // width
        w.varint(128);
        w.bool(true); // height
        w.varint(128);
        w.bool(true); // x offset
        w.varint(0);
        w.bool(true); // y offset
        w.varint(0);
        w.bool(true); // pixels
        w.uvarint(pixels.len() as u32);
        for p in pixels {
            // big-endian A | R<<8 | G<<16 | B<<24
            let v = p[3] as u32 | (p[0] as u32) << 8 | (p[1] as u32) << 16 | (p[2] as u32) << 24;
            w.u32_be(v);
        }
    })
}

/// ClientboundMapItemData (0x43) with only the markers of a map: (type, rotation 0-15, x, y)
/// where x and y run from -128 to 127 across the map (2 per block at scale 0).
pub fn map_decorations(map_id: i64, dimension: u8, origin: [i32; 3], scale: u8, marks: &[(u8, u8, i8, i8)]) -> Vec<u8> {
    packet(0x43, |w| {
        w.varlong(map_id);
        w.u8(dimension);
        w.bool(false); // locked
        for v in origin {
            w.varint(v);
        }
        w.bool(false); // maps included in
        w.bool(true); // scale
        w.u8(scale);
        w.bool(true); // tracked objects (none)
        w.uvarint(0);
        w.bool(true); // decorations
        w.uvarint(marks.len() as u32);
        for (ty, rot, x, y) in marks {
            w.u8(*ty);
            w.u8(*rot);
            w.u8(*x as u8);
            w.u8(*y as u8);
            w.string("");
            w.u32_be(0xffff_ffff);
        }
        for _ in 0..5 {
            w.bool(false); // width, height, offsets, pixels
        }
    })
}
