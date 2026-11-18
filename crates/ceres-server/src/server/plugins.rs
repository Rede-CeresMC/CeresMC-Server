//! Plugins: Rust compiled to WebAssembly, run isolated (wasmi). A plugin that traps, loops or
//! eats memory is switched off instead of taking the server down.
//!
//! `plugins/` holds:
//! - folders with `plugin.yml` + `Cargo.toml`: compiled by the server (cargo, target
//!   wasm32-unknown-unknown) when it starts and on `/reload`;
//! - folders with `plugin.yml` + `plugin.wasm`, or loose `Name.wasm` files: used as they are.
//!
//! The plugin side (crates/ceres-plugin) sends JSON requests through one host function; the
//! server collects them during the event and carries them out right after it.

use super::*;
use ceres_protocol::ItemDesc;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use wasmi::{Caller, Config, Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

pub(crate) const DIR: &str = "plugins";
/// instructions a plugin may run per event before it is stopped
const FUEL_PER_EVENT: u64 = 200_000_000;
const MEMORY_LIMIT: usize = 128 * 1024 * 1024;
/// events one event may cause in a chain (a menu replaced by another closes the first...)
const MAX_EVENT_CHAIN: usize = 64;
const EYE_HEIGHT: f32 = 1.62;

/// A plugin ready to load: its name and compiled code.
pub(crate) struct PluginSource {
    pub name: String,
    pub version: String,
    pub wasm: Vec<u8>,
}

struct HostState {
    /// requests to carry out after the event
    actions: Vec<Value>,
    answer: Vec<u8>,
    players: Vec<String>,
    commands: Vec<(String, String)>,
    limits: StoreLimits,
}

pub(crate) struct LoadedPlugin {
    pub name: String,
    pub version: String,
    pub enabled: bool,
    /// commands it registered: (name, description)
    pub commands: Vec<(String, String)>,
    store: Store<HostState>,
    memory: Memory,
    alloc: TypedFunc<u32, u32>,
    event: TypedFunc<(u32, u32), ()>,
}

#[derive(Default)]
pub(crate) struct Plugins {
    pub list: Vec<LoadedPlugin>,
    /// events waiting to be delivered: (plugin index, event)
    pub queue: std::collections::VecDeque<(usize, Value)>,
    /// events are being delivered right now
    pub delivering: bool,
    /// a /reload compiling in the background
    pub building: Option<std::sync::mpsc::Receiver<(Vec<PluginSource>, Vec<String>)>>,
}

impl Plugins {
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.list.iter().position(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// The enabled plugin that registered this command.
    pub fn command_owner(&self, command: &str) -> Option<usize> {
        self.list.iter().position(|p| p.enabled && p.commands.iter().any(|(c, _)| c.eq_ignore_ascii_case(command)))
    }
}

impl LoadedPlugin {
    fn new(src: PluginSource) -> Result<LoadedPlugin, String> {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, &src.wasm).map_err(|e| format!("invalid wasm: {e}"))?;
        let state = HostState {
            actions: Vec::new(),
            answer: Vec::new(),
            players: Vec::new(),
            commands: Vec::new(),
            limits: StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).build(),
        };
        let mut store = Store::new(&engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(FUEL_PER_EVENT).map_err(|e| e.to_string())?;
        let mut linker = <Linker<HostState>>::new(&engine);
        linker
            .func_wrap("ceres", "call", |mut caller: Caller<'_, HostState>, ptr: u32, len: u32| -> u32 {
                let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) else { return 0 };
                let mut buf = vec![0u8; (len as usize).min(4 * 1024 * 1024)];
                if mem.read(&caller, ptr as usize, &mut buf).is_err() {
                    return 0;
                }
                let request: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
                let state = caller.data_mut();
                let answer = match request["op"].as_str().unwrap_or_default() {
                    "players" => json!({"players": state.players}),
                    "register_command" => {
                        let name = request["name"].as_str().unwrap_or_default().trim().to_ascii_lowercase();
                        let desc = request["description"].as_str().unwrap_or_default().to_string();
                        if !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                        {
                            state.commands.retain(|(c, _)| *c != name);
                            state.commands.push((name, desc));
                        }
                        json!({"ok": true})
                    }
                    "" => json!({"error": "no op"}),
                    _ => {
                        if state.actions.len() < 10_000 {
                            state.actions.push(request);
                        }
                        json!({"ok": true})
                    }
                };
                state.answer = answer.to_string().into_bytes();
                state.answer.len() as u32
            })
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap("ceres", "take_answer", |mut caller: Caller<'_, HostState>, ptr: u32| {
                let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) else { return };
                let answer = std::mem::take(&mut caller.data_mut().answer);
                let _ = mem.write(&mut caller, ptr as usize, &answer);
            })
            .map_err(|e| e.to_string())?;
        let instance = linker.instantiate_and_start(&mut store, &module).map_err(|e| format!("cannot start: {e}"))?;
        let memory = instance.get_memory(&store, "memory").ok_or("the plugin exports no memory")?;
        let alloc = instance
            .get_typed_func::<u32, u32>(&store, "ceres_alloc")
            .map_err(|_| "no ceres_alloc: is ceres_plugin::plugin!(...) missing?")?;
        let event = instance.get_typed_func::<(u32, u32), ()>(&store, "ceres_event").map_err(|_| "no ceres_event")?;
        Ok(LoadedPlugin { name: src.name, version: src.version, enabled: false, commands: Vec::new(), store, memory, alloc, event })
    }

    /// Delivers one event; returns the requests the plugin made (or why it failed).
    fn deliver(&mut self, event: &Value, players: &[String]) -> Result<Vec<Value>, String> {
        let bytes = event.to_string().into_bytes();
        let state = self.store.data_mut();
        state.players = players.to_vec();
        state.actions.clear();
        self.store.set_fuel(FUEL_PER_EVENT).map_err(|e| e.to_string())?;
        let ptr = self.alloc.call(&mut self.store, bytes.len() as u32).map_err(|e| e.to_string())?;
        self.memory.write(&mut self.store, ptr as usize, &bytes).map_err(|e| e.to_string())?;
        self.event.call(&mut self.store, (ptr, bytes.len() as u32)).map_err(|e| {
            if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) {
                "took too long (endless loop?)".to_string()
            } else {
                e.to_string()
            }
        })?;
        let state = self.store.data_mut();
        self.commands = state.commands.clone();
        Ok(std::mem::take(&mut state.actions))
    }
}

/// plugin.yml: just `key: value` lines (name, version, description...).
fn read_plugin_yml(path: &Path) -> HashMap<String, String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.starts_with('#') {
                return None;
            }
            let (k, v) = l.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().trim_matches('"').trim_matches('\'').to_string()))
        })
        .collect()
}

/// The [package] name of a Cargo.toml.
fn crate_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_package = l == "[package]";
        } else if in_package {
            if let Some(v) = l.strip_prefix("name").map(str::trim).and_then(|v| v.strip_prefix('=')) {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Finds (and compiles when needed) every plugin of `plugins/`. Returns them and the errors.
pub(crate) fn build_all() -> (Vec<PluginSource>, Vec<String>) {
    let dir = Path::new(DIR);
    let mut out = Vec::new();
    let mut errors = Vec::new();
    if !dir.exists() {
        let _ = std::fs::create_dir_all(dir);
        return (out, errors);
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    entries.sort();
    for path in entries {
        let file_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if file_name.starts_with('.') {
            continue; // .build and other hidden folders
        }
        if path.is_file() {
            if path.extension().is_some_and(|e| e == "wasm") {
                let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                match std::fs::read(&path) {
                    Ok(wasm) => out.push(PluginSource { name, version: "?".into(), wasm }),
                    Err(e) => errors.push(format!("{file_name}: {e}")),
                }
            }
            continue;
        }
        let yml = read_plugin_yml(&path.join("plugin.yml"));
        let name = yml.get("name").cloned().unwrap_or_else(|| file_name.clone());
        let version = yml.get("version").cloned().unwrap_or_else(|| "?".into());
        let manifest = path.join("Cargo.toml");
        let wasm = if manifest.is_file() {
            match compile(&manifest) {
                Ok(w) => w,
                Err(e) => {
                    errors.push(format!("{name}: {e}"));
                    continue;
                }
            }
        } else if path.join("plugin.wasm").is_file() {
            match std::fs::read(path.join("plugin.wasm")) {
                Ok(w) => w,
                Err(e) => {
                    errors.push(format!("{name}: {e}"));
                    continue;
                }
            }
        } else {
            continue;
        };
        out.push(PluginSource { name, version, wasm });
    }
    (out, errors)
}

/// `cargo build` of one plugin; returns its .wasm.
fn compile(manifest: &Path) -> Result<Vec<u8>, String> {
    let krate = crate_name(manifest).ok_or("Cargo.toml without [package] name")?;
    let target_dir = Path::new(DIR).join(".build");
    println!("[Plugins] compiling {krate}...");
    let output = std::process::Command::new("cargo")
        .args(["build", "--release", "--target", "wasm32-unknown-unknown", "--manifest-path"])
        .arg(manifest)
        .env("CARGO_TARGET_DIR", &target_dir)
        .output()
        .map_err(|e| format!("cannot run cargo (is Rust installed?): {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        // the first error lines are the useful ones
        let first: Vec<&str> = err.lines().filter(|l| !l.trim_start().starts_with("Compiling")).take(25).collect();
        return Err(format!("compilation failed:\n{}", first.join("\n")));
    }
    let wasm = target_dir.join("wasm32-unknown-unknown").join("release").join(format!("{}.wasm", krate.replace('-', "_")));
    std::fs::read(&wasm).map_err(|e| format!("{}: {e} (is crate-type = [\"cdylib\"] set?)", wasm.display()))
}

impl Server {
    /// Loads compiled plugins and enables them.
    pub(crate) fn load_plugins(&mut self, sources: Vec<PluginSource>, errors: Vec<String>) {
        for e in errors {
            eprintln!("[Plugins] {e}");
        }
        for src in sources {
            if self.plugins.index_of(&src.name).is_some() {
                eprintln!("[Plugins] two plugins named {}: the second was skipped", src.name);
                continue;
            }
            let name = src.name.clone();
            match LoadedPlugin::new(src) {
                Ok(p) => {
                    self.plugins.list.push(p);
                    let idx = self.plugins.list.len() - 1;
                    self.enable_plugin(idx);
                }
                Err(e) => eprintln!("[Plugins] {name}: {e}"),
            }
        }
        self.broadcast_available_commands();
    }

    pub(crate) fn enable_plugin(&mut self, idx: usize) {
        let Some(p) = self.plugins.list.get_mut(idx) else { return };
        if p.enabled {
            return;
        }
        p.enabled = true;
        println!("[Plugins] {} v{} enabled", p.name, p.version);
        self.plugin_event(idx, json!({"event": "enable"}));
        // players already online count as joining
        let names: Vec<String> = self.players.values().filter(|p| p.state == State::Spawned).map(|p| p.name.clone()).collect();
        for n in names {
            self.plugin_event(idx, json!({"event": "join", "player": n}));
        }
    }

    /// Switches a plugin off: it gets on_disable, its menus close and its commands go away.
    pub(crate) fn disable_plugin(&mut self, idx: usize) {
        let Some(p) = self.plugins.list.get(idx) else { return };
        if !p.enabled {
            return;
        }
        let name = p.name.clone();
        self.plugin_event(idx, json!({"event": "disable"}));
        let with_menu: Vec<SessionId> = self
            .players
            .iter()
            .filter(|(_, q)| q.menu.as_ref().is_some_and(|m| m.plugin == name))
            .map(|(s, _)| *s)
            .collect();
        if let Some(p) = self.plugins.list.get_mut(idx) {
            p.enabled = false;
            p.commands.clear();
        }
        for sid in with_menu {
            self.close_menu(sid, true, false);
        }
        println!("[Plugins] {name} disabled");
    }

    /// Queues an event for a plugin and delivers everything queued.
    pub(crate) fn plugin_event(&mut self, idx: usize, event: Value) {
        self.plugins.queue.push_back((idx, event));
        if self.plugins.delivering {
            return; // caused by a plugin request: delivered by the loop below
        }
        self.plugins.delivering = true;
        let mut delivered = 0;
        while let Some((idx, event)) = self.plugins.queue.pop_front() {
            delivered += 1;
            if delivered > MAX_EVENT_CHAIN {
                eprintln!("[Plugins] too many chained events, the rest were dropped");
                self.plugins.queue.clear();
                break;
            }
            let players: Vec<String> =
                self.players.values().filter(|p| p.state == State::Spawned).map(|p| p.name.clone()).collect();
            let is_disable = event["event"] == "disable";
            let Some(p) = self.plugins.list.get_mut(idx) else { continue };
            if !p.enabled && !is_disable {
                continue;
            }
            let name = p.name.clone();
            match p.deliver(&event, &players) {
                Ok(actions) => {
                    for a in actions {
                        self.run_plugin_request(&name, &a);
                    }
                }
                Err(e) => {
                    eprintln!("[Plugins] {name} failed during {}: {e}", event["event"]);
                    if !is_disable {
                        eprintln!("[Plugins] {name} was disabled");
                        self.plugins.queue.retain(|(i, _)| *i != idx);
                        self.disable_plugin_after_error(idx);
                    }
                }
            }
        }
        self.plugins.delivering = false;
    }

    fn disable_plugin_after_error(&mut self, idx: usize) {
        let Some(p) = self.plugins.list.get_mut(idx) else { return };
        p.enabled = false;
        p.commands.clear();
        let name = p.name.clone();
        let with_menu: Vec<SessionId> = self
            .players
            .iter()
            .filter(|(_, q)| q.menu.as_ref().is_some_and(|m| m.plugin == name))
            .map(|(s, _)| *s)
            .collect();
        for sid in with_menu {
            self.close_menu(sid, true, false);
        }
        self.broadcast_available_commands();
    }

    /// Event for every enabled plugin.
    pub(crate) fn plugin_event_all(&mut self, event: Value) {
        for idx in 0..self.plugins.list.len() {
            if self.plugins.list[idx].enabled {
                self.plugin_event(idx, event.clone());
            }
        }
    }

    fn player_by_name(&self, name: &str) -> Option<SessionId> {
        self.players.iter().find(|(_, p)| p.state == State::Spawned && p.name.eq_ignore_ascii_case(name)).map(|(s, _)| *s)
    }

    /// Carries out one request of a plugin.
    fn run_plugin_request(&mut self, plugin: &str, a: &Value) {
        let target = a["player"].as_str().and_then(|n| self.player_by_name(n));
        let text = |k: &str| a[k].as_str().unwrap_or_default().to_string();
        match a["op"].as_str().unwrap_or_default() {
            "log" => println!("[{plugin}] {}", text("text")),
            "broadcast" => self.broadcast(&protocol::system_message(&text("text")), |_, _| true),
            "message" => {
                if let Some(sid) = target {
                    self.reply(commands::Sender::Player(sid), &text("text"));
                }
            }
            "title" => {
                if let Some(p) = target.and_then(|s| self.players.get_mut(&s)) {
                    if !text("subtitle").is_empty() {
                        p.out.push(protocol::set_title(protocol::title_type::SUBTITLE, &text("subtitle"), 10, 70, 20));
                    }
                    p.out.push(protocol::set_title(protocol::title_type::TITLE, &text("title"), 10, 70, 20));
                }
            }
            "teleport" => {
                if let Some(sid) = target {
                    let (x, y, z) = (a["x"].as_f64(), a["y"].as_f64(), a["z"].as_f64());
                    if let (Some(mut x), Some(mut y), Some(mut z)) = (x, y, z) {
                        if a["relative"].as_bool() == Some(true) {
                            let p = &self.players[&sid];
                            x += p.pos[0] as f64;
                            y += (p.pos[1] - EYE_HEIGHT) as f64;
                            z += p.pos[2] as f64;
                        }
                        if [x, y, z].iter().all(|v| v.is_finite() && v.abs() < 30_000_000.0) {
                            self.teleport(sid, [x as f32, y as f32, z as f32]);
                        }
                    }
                }
            }
            "give" => {
                if let (Some(sid), Some(item)) = (target, self.menu_item(&a["item"])) {
                    self.give_item(sid, item);
                }
            }
            "console_command" => self.run_command(commands::Sender::Console, &text("line")),
            "menu_open" => {
                if let Some(sid) = target {
                    let size = if a["size"].as_u64() == Some(54) { 54 } else { 27 };
                    let mut items = vec![ItemDesc::default(); size];
                    for it in a["items"].as_array().into_iter().flatten() {
                        let slot = it["slot"].as_u64().unwrap_or(u64::MAX) as usize;
                        if slot < size {
                            if let Some(item) = self.menu_item(it) {
                                items[slot] = item;
                            }
                        }
                    }
                    self.open_menu(sid, plugin, &text("id"), &text("title"), items);
                }
            }
            "menu_set" => {
                if let Some(sid) = target {
                    let slot = a["slot"].as_u64().unwrap_or(u64::MAX) as usize;
                    let item = if a["item"].is_null() { Some(ItemDesc::default()) } else { self.menu_item(&a["item"]) };
                    if let Some(item) = item {
                        self.set_menu_item(sid, plugin, slot, item);
                    }
                }
            }
            "menu_close" => {
                if let Some(sid) = target {
                    if self.players.get(&sid).and_then(|p| p.menu.as_ref()).is_some_and(|m| m.plugin == plugin) {
                        self.close_menu(sid, true, true);
                    }
                }
            }
            other => eprintln!("[Plugins] {plugin}: unknown request {other:?}"),
        }
    }

    /// Adds an item to the player's inventory; the rest drops at their feet.
    fn give_item(&mut self, sid: SessionId, item: ItemDesc) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let (changed, leftover) = p.inventory.add_item(item.clone(), &self.gd);
        for s in &changed {
            let idx = *s as usize;
            p.out.push(protocol::inventory_slot(protocol::window::INVENTORY as u32, idx as u32, &p.inventory.main[idx]));
        }
        if changed.contains(&p.inventory.held_slot) {
            self.broadcast_equipment(sid);
        }
        if leftover > 0 {
            let mut rest = item;
            rest.count = leftover;
            self.drop_from_player(sid, rest);
        }
    }

    /// `/plugins`, `/reload`, `/plugin enable|disable <name>`
    pub(crate) fn plugin_admin_command(&mut self, sender: commands::Sender, name: &str, args: &[&str]) {
        match name {
            "plugins" => {
                let list: Vec<String> = self
                    .plugins
                    .list
                    .iter()
                    .map(|p| format!("{}{} v{}", if p.enabled { "§a" } else { "§c" }, p.name, p.version))
                    .collect();
                let msg = if list.is_empty() {
                    "§7Nenhum plugin carregado (pasta plugins/).".to_string()
                } else {
                    format!("§6Plugins ({}):§r {}", list.len(), list.join("§r, "))
                };
                self.reply(sender, &msg);
            }
            "reload" => {
                if self.plugins.building.is_some() {
                    return self.reply(sender, "§eJá está recarregando...");
                }
                self.reply(sender, "§eRecarregando plugins (compilando se precisar)...");
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(build_all());
                });
                self.plugins.building = Some(rx);
            }
            "plugin" => {
                let usage = "§cUso: /plugin <enable|disable> <nome>";
                let (Some(action), Some(pname)) = (args.first(), args.get(1)) else { return self.reply(sender, usage) };
                let Some(idx) = self.plugins.index_of(pname) else {
                    return self.reply(sender, &format!("§cPlugin não encontrado: {pname}"));
                };
                match action.to_ascii_lowercase().as_str() {
                    "enable" | "ligar" => self.enable_plugin(idx),
                    "disable" | "desligar" => self.disable_plugin(idx),
                    _ => return self.reply(sender, usage),
                }
                self.broadcast_available_commands();
                let state = if self.plugins.list[idx].enabled { "§aligado" } else { "§cdesligado" };
                let pname = self.plugins.list[idx].name.clone();
                self.reply(sender, &format!("§ePlugin {pname} {state}§e."));
            }
            _ => {}
        }
    }

    /// A finished /reload: the old plugins go, the new ones come.
    pub(crate) fn poll_plugin_reload(&mut self) {
        let Some(rx) = &self.plugins.building else { return };
        let Ok((sources, errors)) = rx.try_recv() else { return };
        self.plugins.building = None;
        for idx in 0..self.plugins.list.len() {
            self.disable_plugin(idx);
        }
        self.plugins.list.clear();
        let count = sources.len();
        let failed = errors.len();
        self.load_plugins(sources, errors);
        let loaded = self.plugins.list.iter().filter(|p| p.enabled).count();
        let msg = format!("§aPlugins recarregados: {loaded} de {count} ligados{}.", if failed > 0 {
            format!(", §c{failed} com erro (veja o console)")
        } else {
            String::new()
        });
        println!("[Plugins] reloaded: {loaded}/{count} enabled, {failed} failed");
        let ops: Vec<SessionId> = self.players.iter().filter(|(_, p)| p.op).map(|(s, _)| *s).collect();
        for sid in ops {
            self.reply(commands::Sender::Player(sid), &msg);
        }
    }

    /// The plugins get their last event before the server stops.
    pub(crate) fn disable_all_plugins(&mut self) {
        for idx in 0..self.plugins.list.len() {
            self.disable_plugin(idx);
        }
    }
}
