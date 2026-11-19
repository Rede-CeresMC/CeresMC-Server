//! Game layer: player sessions, login flow, entities, combat, moderation and chunk streaming.

mod anvil;
mod menus;
mod mobs;
mod plugins;
mod resource_packs;
mod armor;
mod brewing;
mod blocks;
mod blockuse;
mod books;
mod cauldron;
mod chunks;
mod combat;
mod connections;
mod commands;
mod containers;
mod defense;
mod doublechest;
mod durability;
mod effects;
mod enchanting;
mod environment;
mod falling;
mod fishing;
mod farming;
mod fireworks;
mod food;
mod frames;
mod furnace;
mod trees;
mod twopart;
mod useitems;
mod weapons;
mod grindstone;
mod handler;
mod hopper;
mod inventory;
mod items;
mod lightning;
mod maps;
mod liquids;
mod loot;
mod moderation;
mod music;
mod plants;
mod players;
mod potions;
mod projectiles;
mod skin;
mod spawners;
mod support;
mod sleep;
mod xp;

use crate::config::Config;

use ceres_codec::binary::{R, Res};
use ceres_net::transport::{NetEvent, SessionId, Transport};
use ceres_protocol::{self as protocol, ActorState, PlayerInfo, id, play_status};
use ceres_world::{GameData, WorldManager};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub(crate) enum State {
    AwaitNetworkSettings,
    AwaitLogin,
    ResourcePacks,
    PreSpawn,
    Spawned,
}

pub(crate) struct Player {
    addr: SocketAddr,
    /// client key the transport tied the connection to (NetherNet identity); the login must match
    transport_key: Option<String>,
    state: State,
    compression: bool,
    name: String,
    entity_id: i64,
    world: usize,
    /// eye position
    pos: [f32; 3],
    /// pitch, yaw, head yaw
    rot: [f32; 3],
    /// set at login; what other clients need to draw this player
    info: Option<PlayerInfo>,
    inventory: inventory::Inventory,
    actor_state: ActorState,
    flying: bool,
    last_swing_tick: u64,
    health: f32,
    hunger: f32,
    saturation: f32,
    exhaustion: f32,
    /// ticks since the last regeneration / starvation step
    food_timer: u32,
    /// tick the player started eating
    eating: Option<u64>,
    /// ate; waiting for the client's own end-of-use click
    awaiting_release: bool,
    /// active status effects by id
    effects: HashMap<u8, effects::Effect>,
    /// extra hearts from Absorption, used up before health
    absorption: f32,
    /// ticks left burning
    fire_ticks: u32,
    /// tick the player started drawing a bow
    bow_since: Option<u64>,
    /// tick the player lay down in a bed, and the world of that bed
    sleep_since: u64,
    sleep_world: usize,
    /// tick of the last projectile thrown or shot
    last_throw: u64,
    xp_level: u32,
    /// points inside the current level
    xp_points: u32,
    dead: bool,
    hurt_cooldown: u8,
    fall_start_y: Option<f32>,
    spawn_point: Option<(usize, [i32; 3])>,
    /// protocol game mode: 0 survival, 1 creative, 2 adventure, 6 spectator
    gamemode: i32,
    op: bool,
    /// set by a teleport: movement far from here is stale and ignored until the client catches up
    teleport_to: Option<[f32; 3]>,
    /// sign whose editor was opened for this player, and when
    editing_sign: Option<([i32; 3], Instant)>,
    last_skin_change: Option<Instant>,
    /// survival: block being mined and when it started
    breaking: Option<([i32; 3], Instant)>,
    /// block and face being hit (for the hit particles)
    punching: Option<([i32; 3], u8)>,
    /// walks through blocks and may fly, in any game mode (/noclip, plugins)
    noclip: bool,
    /// entity id of the mob this player rides
    riding: Option<i64>,
    /// forward input of the last PlayerAuthInput (-1 to 1; drives carts)
    move_forward: f32,
    /// ticks the shield stays disabled (hit by an axe)
    shield_cooldown: u32,
    /// ticks until a raised shield makes a sound and wears again
    shield_block_cooldown: u8,
    /// last window id handed out (1..=99) for block containers
    window_counter: u8,
    radius: i32,
    center: Option<(i32, i32)>,
    sent: HashSet<(i32, i32)>,
    queue: VecDeque<(i32, i32)>,
    spawn_sent: bool,
    pkt_sec: u64,
    pkt_count: u32,
    /// packet kinds this player already sent (the first of each goes to server.log, to debug)
    seen_packets: std::collections::HashSet<(u32, u32)>,
    /// resource pack chunks already sent to this player (pack index, chunk): each is sent once
    pack_chunks_sent: HashSet<(usize, u32)>,
    /// chest menu of a plugin open on this player's screen
    menu: Option<menus::FakeMenu>,
    /// forms sent by plugins waiting for an answer, by form id
    forms: HashMap<u32, plugins::PendingForm>,
    out: Vec<Vec<u8>>,
    close: Option<String>,
}

pub struct Server {
    /// logins waiting for Mojang's keys: (session, Login packet, since)
    pending_logins: Vec<(SessionId, Vec<u8>, Instant)>,
    /// Mojang's login keys, filled in by a background thread
    auth_keys: crate::auth::SharedKeys,
    cfg: Config,
    gd: GameData,
    worlds: WorldManager,
    transports: Vec<Box<dyn Transport>>,
    /// which transport owns which session
    owner: HashMap<SessionId, usize>,
    players: HashMap<SessionId, Player>,
    items: Vec<items::ItemEntity>,
    /// falling sand, gravel...
    falling: Vec<falling::FallingBlock>,
    xp_orbs: Vec<xp::XpOrb>,
    mobs: Vec<mobs::Mob>,
    /// mob AI state: entity definitions, indexes, sensors
    mobsys: mobs::MobSys,
    /// arrows, fireballs, potions... shot by mobs
    mob_projectiles: Vec<mobs::fight::MobProjectile>,
    /// items on cooldown per player (goat horns...): tick they may be used again
    item_cooldowns: HashMap<(SessionId, String), u64>,
    /// fishing bobbers out
    bobbers: Vec<fishing::Bobber>,
    /// lightning bolts shown for a second
    bolts: Vec<lightning::Bolt>,
    /// filled maps (images)
    maps: maps::Maps,
    /// players charging with a spear: where they were last tick
    spear_prev: HashMap<SessionId, [f32; 3]>,
    projectiles: Vec<projectiles::Projectile>,
    fireworks: Vec<fireworks::Firework>,
    /// sound handle of the record each jukebox plays, to stop it
    jukebox_sounds: HashMap<(usize, [i32; 3]), u64>,
    next_sound_handle: u64,
    /// positions whose block may have lost its support
    gravity_queue: Vec<(usize, [i32; 3])>,
    /// liquid blocks due to flow, with the tick they are due
    liquid_queue: HashMap<(usize, [i32; 3]), u64>,
    moderation: moderation::Moderation,
    next_entity_id: i64,
    ticks: u64,
    /// furnaces with something to do, by (world, position)
    furnaces: HashMap<(usize, [i32; 3]), furnace::FurnaceState>,
    /// brewing stands with something to do
    brewers: HashMap<(usize, [i32; 3]), brewing::BrewingState>,
    tick_ms: VecDeque<f32>,
    started_at: Instant,
    /// lowercase names from ops.txt
    ops: HashSet<String>,
    console: std::sync::mpsc::Receiver<String>,
    software: String,
    /// resource packs sent to the players (resource_packs/)
    packs: Vec<crate::packs::Pack>,
    plugins: plugins::Plugins,
    next_form_id: u32,
}

const TICK: Duration = Duration::from_millis(50);

impl Server {
    pub fn new(
        cfg: Config,
        gd: GameData,
        worlds: WorldManager,
        transports: Vec<Box<dyn Transport>>,
        auth_keys: crate::auth::SharedKeys,
    ) -> Self {
        Server {
            auth_keys,
            pending_logins: Vec::new(),
            cfg,
            gd,
            worlds,
            transports,
            owner: HashMap::new(),
            players: HashMap::new(),
            items: Vec::new(),
            falling: Vec::new(),
            xp_orbs: Vec::new(),
            mobs: Vec::new(),
            mobsys: Default::default(),
            mob_projectiles: Vec::new(),
            item_cooldowns: HashMap::new(),
            bobbers: Vec::new(),
            bolts: Vec::new(),
            maps: maps::Maps::default(),
            spear_prev: HashMap::new(),
            projectiles: Vec::new(),
            fireworks: Vec::new(),
            jukebox_sounds: HashMap::new(),
            next_sound_handle: 1,
            gravity_queue: Vec::new(),
            liquid_queue: HashMap::new(),
            moderation: moderation::Moderation::load(),
            next_entity_id: 1,
            ticks: 0,
            furnaces: HashMap::new(),
            brewers: HashMap::new(),
            tick_ms: VecDeque::with_capacity(100),
            started_at: Instant::now(),
            ops: load_ops(),
            console: spawn_console(),
            software: format!("Ceres-RS {}", env!("CARGO_PKG_VERSION")),
            packs: crate::packs::load_all(),
            plugins: plugins::Plugins::default(),
            next_form_id: 1,
        }
    }

    pub fn run(&mut self) -> ! {
        let mut events = Vec::new();
        self.load_mobs();
        let (sources, errors) = plugins::build_all();
        self.load_plugins(sources, errors);
        let mut next_tick = Instant::now();
        self.update_status();
        loop {
            for ti in 0..self.transports.len() {
                self.transports[ti].poll(&mut events);
                for e in &events {
                    if let NetEvent::Connected(sid, _, _) = e {
                        self.owner.insert(*sid, ti);
                    }
                }
                self.pending_events(&mut events);
            }
            if SHUTDOWN.load(std::sync::atomic::Ordering::SeqCst) {
                self.shutdown("O servidor foi desligado");
            }
            while let Ok(line) = self.console.try_recv() {
                self.run_command(commands::Sender::Console, &line);
            }
            let now = Instant::now();
            if now >= next_tick {
                next_tick += TICK;
                if now > next_tick + TICK * 20 {
                    next_tick = now + TICK; // we fell way behind, don't try to catch up
                }
                self.tick();
            }
            self.flush_players();
            for t in self.transports.iter_mut() {
                t.flush(&mut events);
            }
            self.pending_events(&mut events);
        }
    }

    fn pending_events(&mut self, events: &mut Vec<NetEvent>) {
        for e in events.drain(..) {
            match e {
                NetEvent::Connected(sid, addr, transport_key) => {
                    println!("[Net] {addr} connected (session {sid})");
                    self.players.insert(
                        sid,
                        Player {
                            addr,
                            transport_key,
                            state: State::AwaitNetworkSettings,
                            compression: false,
                            name: String::new(),
                            entity_id: 0,
                            world: self.worlds.default,
                            pos: [0.0; 3],
                            rot: [0.0; 3],
                            info: None,
                            inventory: Default::default(),
                            actor_state: Default::default(),
                            flying: false,
                            last_swing_tick: 0,
                            health: 20.0,
                            hunger: 20.0,
                            saturation: 5.0,
                            exhaustion: 0.0,
                            food_timer: 0,
                            eating: None,
                            awaiting_release: false,
                            effects: HashMap::new(),
                            absorption: 0.0,
                            fire_ticks: 0,
                            bow_since: None,
                            sleep_since: 0,
                            sleep_world: 0,
                            last_throw: 0,
                            xp_level: 0,
                            xp_points: 0,
                            dead: false,
                            hurt_cooldown: 0,
                            fall_start_y: None,
                            spawn_point: None,
                            gamemode: self.cfg.gamemode,
                            op: false,
                            teleport_to: None,
                            editing_sign: None,
                            last_skin_change: None,
                            breaking: None,
                            punching: None,
                            noclip: false,
                            riding: None,
                            move_forward: 0.0,
                            shield_cooldown: 0,
                            shield_block_cooldown: 0,
                            window_counter: 0,
                            radius: 0,
                            center: None,
                            sent: HashSet::new(),
                            queue: VecDeque::new(),
                            spawn_sent: false,
                            pkt_sec: 0,
                            pkt_count: 0,
                            seen_packets: Default::default(),
                            pack_chunks_sent: HashSet::new(),
                            menu: None,
                            forms: HashMap::new(),
                            out: Vec::new(),
                            close: None,
                        },
                    );
                }
                NetEvent::Packet(sid, payload) => {
                    if payload.len() > self.cfg.max_batch_bytes {
                        eprintln!("[Net] session {sid}: oversized batch ({} bytes)", payload.len());
                        self.kick(sid, "Pacote grande demais");
                        continue;
                    }
                    if let Err(e) = self.handle_batch(sid, &payload) {
                        let who = self.players.get(&sid).map(|p| p.name.clone()).unwrap_or_default();
                        crate::logfile::log(&format!("[Net] session {sid} ({who}): {e}"));
                        self.kick(sid, "Pacote inválido");
                    }
                }
                NetEvent::Disconnected(sid, reason) => {
                    self.dismount(sid);
                    self.close_container(sid, false);
                    self.stop_sleeping(sid);
                    if let Some(p) = self.players.remove(&sid) {
                        if !p.name.is_empty() {
                            crate::logfile::log(&format!("[Game] {} left ({reason})", p.name));
                        }
                        self.on_left(&p);
                    }
                    self.owner.remove(&sid);
                    self.update_status();
                }
            }
        }
    }

    fn transport(&mut self, sid: SessionId) -> Option<&mut Box<dyn Transport>> {
        let ti = *self.owner.get(&sid)?;
        self.transports.get_mut(ti)
    }

    pub(crate) fn kick(&mut self, sid: SessionId, msg: &str) {
        if let Some(p) = self.players.get_mut(&sid) {
            crate::logfile::log(&format!("[Game] kicking {} ({}): {msg}", p.name, p.addr));
            p.out.push(protocol::disconnect(msg));
            p.close = Some(msg.to_string());
        }
    }

    pub(crate) fn update_status(&mut self) {
        let online = self.players.values().filter(|p| p.state == State::Spawned).count();
        let (motd, max) = (self.cfg.motd.clone(), self.cfg.max_players);
        for t in self.transports.iter_mut() {
            t.set_status(&motd, online, max);
        }
    }

    fn handle_batch(&mut self, sid: SessionId, payload: &[u8]) -> Res<()> {
        let sec = self.started_at.elapsed().as_secs();
        let max_rate = self.cfg.max_packets_per_sec;
        let compression = {
            let Some(p) = self.players.get_mut(&sid) else { return Ok(()) };
            if p.pkt_sec != sec {
                p.pkt_sec = sec;
                p.pkt_count = 0;
            }
            p.compression
        };
        let packets = protocol::decode_batch(payload, compression)?;
        if let Some(p) = self.players.get_mut(&sid) {
            p.pkt_count = p.pkt_count.saturating_add(packets.len() as u32);
            if p.pkt_count > max_rate {
                return Err(format!("packet flood ({}/s > {max_rate}/s)", p.pkt_count));
            }
        }
        for pk in packets {
            if let Err(e) = self.handle_packet(sid, &pk) {
                let id = R::new(&pk).uvarint().map(|h| h & 0x3ff).unwrap_or(0);
                let file = crate::logfile::dump_packet(sid, &pk).unwrap_or_default();
                return Err(format!("packet 0x{id:02x} ({} bytes, saved to {file}): {e}", pk.len()));
            }
        }
        Ok(())
    }
}

/// Set by Ctrl+C; the main loop saves and exits.
pub static SHUTDOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const OPS_FILE: &str = "ops.txt";

fn load_ops() -> HashSet<String> {
    std::fs::read_to_string(OPS_FILE)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_ascii_lowercase())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

/// Lines typed on the server console, read on their own thread.
fn spawn_console() -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("console".into())
        .spawn(move || {
            let stdin = std::io::stdin();
            let mut line = String::new();
            while stdin.read_line(&mut line).map(|n| n > 0).unwrap_or(false) {
                if tx.send(line.trim().to_string()).is_err() {
                    break;
                }
                line.clear();
            }
        })
        .ok();
    rx
}

impl Server {
    pub(crate) fn save_ops(&self) {
        let mut names: Vec<&String> = self.ops.iter().collect();
        names.sort();
        let text: String = names.iter().map(|n| format!("{n}\n")).collect();
        if let Err(e) = std::fs::write(OPS_FILE, text) {
            eprintln!("[Server] could not write {OPS_FILE}: {e}");
        }
    }

    /// Saves every world and player, disconnects everyone and exits.
    pub(crate) fn shutdown(&mut self, reason: &str) -> ! {
        self.disable_all_plugins();
        println!("[Server] stopping: saving worlds and players...");
        self.save_worlds();
        for w in self.worlds.worlds.iter_mut() {
            let _ = w.save_level_dat();
        }
        let sids: Vec<SessionId> = self.players.keys().copied().collect();
        for sid in sids {
            self.kick(sid, reason);
        }
        self.flush_players();
        for t in self.transports.iter_mut() {
            t.flush(&mut Vec::new());
        }
        std::thread::sleep(Duration::from_millis(300));
        println!("[Server] stopped");
        std::process::exit(0);
    }
}
