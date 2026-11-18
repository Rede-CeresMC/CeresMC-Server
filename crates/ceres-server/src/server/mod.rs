//! Game layer: player sessions, login flow and chunk streaming. Single-threaded tick loop.

mod blocks;
mod chunks;
mod commands;
mod handler;
mod inventory;
mod players;
mod skin;

use crate::config::Config;

use ceres_codec::binary::{R, Res};
use ceres_net::transport::{NetEvent, SessionId, Transport};
use ceres_protocol::{self as protocol, PlayerInfo, id, play_status};
use ceres_world::GameData;
use ceres_world::WorldManager;
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
    /// protocol game mode: 0 survival, 1 creative, 2 adventure, 6 spectator
    gamemode: i32,
    op: bool,
    radius: i32,
    center: Option<(i32, i32)>,
    sent: HashSet<(i32, i32)>,
    queue: VecDeque<(i32, i32)>,
    spawn_sent: bool,
    out: Vec<Vec<u8>>,
    close: Option<String>,
}

pub struct Server {
    cfg: Config,
    gd: GameData,
    worlds: WorldManager,
    transports: Vec<Box<dyn Transport>>,
    /// which transport owns which session
    owner: HashMap<SessionId, usize>,
    players: HashMap<SessionId, Player>,
    next_entity_id: i64,
    ticks: u64,
    /// lowercase names from ops.txt
    ops: HashSet<String>,
    console: std::sync::mpsc::Receiver<String>,
    software: String,
}

const TICK: Duration = Duration::from_millis(50);

impl Server {
    pub fn new(cfg: Config, gd: GameData, worlds: WorldManager, transports: Vec<Box<dyn Transport>>) -> Self {
        Server {
            cfg,
            gd,
            worlds,
            transports,
            owner: HashMap::new(),
            players: HashMap::new(),
            next_entity_id: 1,
            ticks: 0,
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
                    if let NetEvent::Connected(sid, _) = e {
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
                NetEvent::Connected(sid, addr) => {
                    println!("[Net] {addr} connected (session {sid})");
                    self.players.insert(
                        sid,
                        Player {
                            addr,
                            state: State::AwaitNetworkSettings,
                            compression: false,
                            name: String::new(),
                            entity_id: 0,
                            world: self.worlds.default,
                            pos: [0.0; 3],
                            rot: [0.0; 3],
                            info: None,
                            inventory: Default::default(),
                            gamemode: self.cfg.gamemode,
                            op: false,
                            radius: 0,
                            center: None,
                            sent: HashSet::new(),
                            queue: VecDeque::new(),
                            spawn_sent: false,
                            out: Vec::new(),
                            close: None,
                        },
                    );
                }
                NetEvent::Packet(sid, payload) => {
                    if let Err(e) = self.handle_batch(sid, &payload) {
                        eprintln!("[Net] session {sid}: {e}");
                        self.kick(sid, "Bad packet");
                    }
                }
                NetEvent::Disconnected(sid, reason) => {
                    if let Some(p) = self.players.remove(&sid) {
                        if !p.name.is_empty() {
                            println!("[Game] {} left ({reason})", p.name);
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

    fn kick(&mut self, sid: SessionId, msg: &str) {
        if let Some(p) = self.players.get_mut(&sid) {
            p.out.push(protocol::disconnect(msg));
            p.close = Some(msg.to_string());
        }
    }

    fn update_status(&mut self) {
        let online = self.players.values().filter(|p| p.state == State::Spawned).count();
        let (motd, max) = (self.cfg.motd.clone(), self.cfg.max_players);
        for t in self.transports.iter_mut() {
            t.set_status(&motd, online, max);
        }
    }

    fn handle_batch(&mut self, sid: SessionId, payload: &[u8]) -> Res<()> {
        let Some(p) = self.players.get(&sid) else { return Ok(()) };
        let packets = protocol::decode_batch(payload, p.compression)?;
        for pk in packets {
            self.handle_packet(sid, &pk)?;
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
        let text: String = names
            .iter()
            .map(|n| {
                format!(
                    "{n}
"
                )
            })
            .collect();
        if let Err(e) = std::fs::write(OPS_FILE, text) {
            eprintln!("[Server] could not write {OPS_FILE}: {e}");
        }
    }

    /// Saves every world, disconnects everyone and exits.
    pub(crate) fn shutdown(&mut self, reason: &str) -> ! {
        println!("[Server] stopping: saving worlds...");
        self.save_worlds();
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
