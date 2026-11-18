//! Game layer: player sessions, login flow, entities, combat, moderation and chunk streaming.

mod blocks;
mod chunks;
mod combat;
mod commands;
mod containers;
mod doublechest;
mod durability;
mod effects;
mod food;
mod furnace;
mod twopart;
mod handler;
mod inventory;
mod items;
mod moderation;
mod players;
mod skin;

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
    /// last window id handed out (1..=99) for block containers
    window_counter: u8,
    radius: i32,
    center: Option<(i32, i32)>,
    sent: HashSet<(i32, i32)>,
    queue: VecDeque<(i32, i32)>,
    spawn_sent: bool,
    pkt_sec: u64,
    pkt_count: u32,
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
    moderation: moderation::Moderation,
    next_entity_id: i64,
    ticks: u64,
    /// furnaces with something to do, by (world, position)
    furnaces: HashMap<(usize, [i32; 3]), furnace::FurnaceState>,
    tick_ms: VecDeque<f32>,
    started_at: Instant,
    /// lowercase names from ops.txt
    ops: HashSet<String>,
    console: std::sync::mpsc::Receiver<String>,
    software: String,
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
            moderation: moderation::Moderation::load(),
            next_entity_id: 1,
            ticks: 0,
            furnaces: HashMap::new(),
            tick_ms: VecDeque::with_capacity(100),
            started_at: Instant::now(),
            ops: load_ops(),
            console: spawn_console(),
            software: format!("Ceres-RS {}", env!("CARGO_PKG_VERSION")),
        }
    }

    pub fn run(&mut self) -> ! {
        let mut events = Vec::new();
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
                            window_counter: 0,
                            radius: 0,
                            center: None,
                            sent: HashSet::new(),
                            queue: VecDeque::new(),
                            spawn_sent: false,
                            pkt_sec: 0,
                            pkt_count: 0,
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
                    self.close_container(sid, false);
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
