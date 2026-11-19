//! API for CeresMC Server plugins.
//!
//! A plugin is a Rust library compiled to WebAssembly (`wasm32-unknown-unknown`). The server runs
//! it isolated: a bug in a plugin cannot crash the server, and `/reload` loads a new version
//! without restarting.
//!
//! ```ignore
//! use ceres_plugin::*;
//!
//! #[derive(Default)]
//! struct Hello;
//!
//! impl Plugin for Hello {
//!     fn on_enable(&mut self) {
//!         register_command("oi", "Diz oi");
//!     }
//!     fn on_command(&mut self, player: &Player, command: &str, _args: &[String]) {
//!         if command == "oi" {
//!             player.send_message("§aOi!");
//!         }
//!     }
//! }
//!
//! ceres_plugin::plugin!(Hello);
//! ```
//!
//! Everything a plugin asks for (messages, menus, teleports...) is done by the server right after
//! the event the plugin is handling.

pub use serde_json::{Value, json};

/// A player online (identified by name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    pub name: String,
}

impl Player {
    pub fn new(name: impl Into<String>) -> Self {
        Player { name: name.into() }
    }

    /// Chat message to this player (§ colour codes work).
    pub fn send_message(&self, text: &str) {
        call(json!({"op": "message", "player": self.name, "text": text}));
    }

    /// Shows a title (and subtitle) on the screen.
    pub fn send_title(&self, title: &str, subtitle: &str) {
        call(json!({"op": "title", "player": self.name, "title": title, "subtitle": subtitle}));
    }

    /// Moves the player in its world (feet position). An open menu goes along.
    pub fn teleport(&self, x: f64, y: f64, z: f64) {
        call(json!({"op": "teleport", "player": self.name, "x": x, "y": y, "z": z}));
    }

    /// Text above the hotbar.
    pub fn send_actionbar(&self, text: &str) {
        call(json!({"op": "actionbar", "player": self.name, "text": text}));
    }

    /// Sidebar scoreboard: a title and lines from top to bottom (up to 15). Call again to change it.
    pub fn set_scoreboard(&self, title: &str, lines: &[&str]) {
        call(json!({"op": "scoreboard", "player": self.name, "title": title, "lines": lines}));
    }

    pub fn clear_scoreboard(&self) {
        call(json!({"op": "scoreboard", "player": self.name, "clear": true}));
    }

    /// Bar at the top of the screen: `progress` from 0.0 to 1.0. Call again to change it.
    pub fn set_bossbar(&self, title: &str, progress: f32, color: BossBarColor) {
        call(json!({"op": "bossbar", "player": self.name, "title": title, "progress": progress, "color": color as u8}));
    }

    pub fn clear_bossbar(&self) {
        call(json!({"op": "bossbar", "player": self.name, "clear": true}));
    }

    /// Moves the player by an offset from where it is now.
    pub fn teleport_by(&self, dx: f64, dy: f64, dz: f64) {
        call(json!({"op": "teleport", "player": self.name, "x": dx, "y": dy, "z": dz, "relative": true}));
    }

    /// Puts an item in the player's inventory (what does not fit drops at their feet).
    pub fn give(&self, item: &Item) {
        call(json!({"op": "give", "player": self.name, "item": item.to_json(0)}));
    }

    /// Opens a menu for this player (replacing any open one).
    pub fn open_menu(&self, menu: &Menu) {
        call(json!({
            "op": "menu_open",
            "player": self.name,
            "id": menu.id,
            "title": menu.title,
            "size": menu.size.slots(),
            "items": menu.items.iter().map(|(slot, item)| item.to_json(*slot)).collect::<Vec<_>>(),
        }));
    }

    /// Changes one slot of the menu this player has open.
    pub fn set_menu_item(&self, slot: u32, item: Option<&Item>) {
        let item = item.map(|i| i.to_json(slot)).unwrap_or(Value::Null);
        call(json!({"op": "menu_set", "player": self.name, "slot": slot, "item": item}));
    }

    /// Shows a form (window with buttons or fields). The answer comes in [`Plugin::on_form`].
    pub fn send_form(&self, id: &str, form: &Form) {
        call(json!({"op": "form", "player": self.name, "id": id, "form": form.0}));
    }

    /// Closes this player's menu, if it has one.
    pub fn close_menu(&self) {
        call(json!({"op": "menu_close", "player": self.name}));
    }
}

/// Bossbar colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BossBarColor {
    Pink = 0,
    Blue = 1,
    Red = 2,
    Green = 3,
    Yellow = 4,
    Purple = 5,
    White = 6,
}

/// Chest menu size. Only 27 slots for now: the menu is an invisible entity's inventory and the
/// client shows 27 slots for every entity tried. For more items, use pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuSize {
    /// 27 slots (3 rows)
    Chest,
}

impl MenuSize {
    pub fn slots(self) -> u32 {
        27
    }
}

/// A chest-looking menu, without any chest in the world. Nobody can take its items: each click
/// becomes [`Plugin::on_menu_click`].
#[derive(Debug, Clone)]
pub struct Menu {
    pub id: String,
    pub title: String,
    pub size: MenuSize,
    pub items: Vec<(u32, Item)>,
}

impl Menu {
    /// `id` is how the plugin recognises the menu in clicks.
    pub fn new(id: &str, title: &str, size: MenuSize) -> Self {
        Menu { id: id.into(), title: title.into(), size, items: Vec::new() }
    }

    pub fn set(&mut self, slot: u32, item: Item) -> &mut Self {
        self.items.retain(|(s, _)| *s != slot);
        if slot < self.size.slots() {
            self.items.push((slot, item));
        }
        self
    }

    /// Fills every empty slot (glass panes as a border, for example).
    pub fn fill(&mut self, item: Item) -> &mut Self {
        for slot in 0..self.size.slots() {
            if !self.items.iter().any(|(s, _)| *s == slot) {
                self.items.push((slot, item.clone()));
            }
        }
        self
    }
}

/// An item to show (in menus).
#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub count: u32,
    pub meta: u32,
    pub name: Option<String>,
    pub lore: Vec<String>,
}

impl Item {
    /// `id` like "minecraft:diamond" (or just "diamond").
    pub fn new(id: &str) -> Self {
        Item { id: id.into(), count: 1, meta: 0, name: None, lore: Vec::new() }
    }

    pub fn count(mut self, count: u32) -> Self {
        self.count = count.clamp(1, 64);
        self
    }

    pub fn meta(mut self, meta: u32) -> Self {
        self.meta = meta;
        self
    }

    /// Custom name (§ colours work).
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Lines under the name.
    pub fn lore(mut self, lines: &[&str]) -> Self {
        self.lore = lines.iter().map(|s| s.to_string()).collect();
        self
    }

    fn to_json(&self, slot: u32) -> Value {
        json!({"slot": slot, "id": self.id, "count": self.count, "meta": self.meta, "name": self.name, "lore": self.lore})
    }
}

/// A form: a window with buttons ([`Form::simple`]), yes/no ([`Form::modal`]) or fields
/// ([`Form::custom`]).
#[derive(Debug, Clone)]
pub struct Form(Value);

impl Form {
    /// Text and a list of buttons; the answer is [`FormResponse::Button`].
    pub fn simple(title: &str, content: &str) -> Self {
        Form(json!({"type": "form", "title": title, "content": content, "buttons": []}))
    }

    /// Text and two buttons; the answer is [`FormResponse::Modal`] (true = first button).
    pub fn modal(title: &str, content: &str, yes: &str, no: &str) -> Self {
        Form(json!({"type": "modal", "title": title, "content": content, "button1": yes, "button2": no}))
    }

    /// Fields (text boxes, switches, sliders, lists); the answer is [`FormResponse::Custom`],
    /// one value per field in order.
    pub fn custom(title: &str) -> Self {
        Form(json!({"type": "custom_form", "title": title, "content": []}))
    }

    pub fn button(mut self, text: &str) -> Self {
        self.push("buttons", json!({"text": text}));
        self
    }

    /// Button with an image: a texture path ("textures/items/diamond") or an https URL.
    pub fn button_image(mut self, text: &str, image: &str) -> Self {
        let kind = if image.starts_with("http") { "url" } else { "path" };
        self.push("buttons", json!({"text": text, "image": {"type": kind, "data": image}}));
        self
    }

    pub fn label(mut self, text: &str) -> Self {
        self.push("content", json!({"type": "label", "text": text}));
        self
    }

    pub fn input(mut self, text: &str, placeholder: &str, default: &str) -> Self {
        self.push("content", json!({"type": "input", "text": text, "placeholder": placeholder, "default": default}));
        self
    }

    pub fn toggle(mut self, text: &str, default: bool) -> Self {
        self.push("content", json!({"type": "toggle", "text": text, "default": default}));
        self
    }

    pub fn slider(mut self, text: &str, min: f64, max: f64, step: f64, default: f64) -> Self {
        self.push("content", json!({"type": "slider", "text": text, "min": min, "max": max, "step": step, "default": default}));
        self
    }

    pub fn dropdown(mut self, text: &str, options: &[&str], default: usize) -> Self {
        self.push("content", json!({"type": "dropdown", "text": text, "options": options, "default": default}));
        self
    }

    fn push(&mut self, key: &str, v: Value) {
        if let Some(a) = self.0[key].as_array_mut() {
            a.push(v);
        }
    }
}

/// The player's answer to a form.
#[derive(Debug, Clone, PartialEq)]
pub enum FormResponse {
    /// closed without answering
    Closed,
    /// simple form: index of the button
    Button(u32),
    /// modal form: true = first button
    Modal(bool),
    /// custom form: one value per field (input: string, toggle: bool, slider: number,
    /// dropdown: index, label: null)
    Custom(Vec<Value>),
}

/// A block in a world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// e.g. "minecraft:stone"
    pub name: String,
    pub world: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// What a plugin reacts to. Every method has an empty default.
#[allow(unused_variables)]
pub trait Plugin {
    /// The plugin was loaded (register commands here).
    fn on_enable(&mut self) {}
    /// The plugin is being unloaded (`/reload`, `/plugin disable`, server stop).
    fn on_disable(&mut self) {}
    /// A player joined the game.
    fn on_join(&mut self, player: &Player) {}
    /// A player left.
    fn on_quit(&mut self, player: &Player) {}
    /// A player ran one of the commands this plugin registered.
    fn on_command(&mut self, player: &Player, command: &str, args: &[String]) {}
    /// A player clicked a slot of a menu this plugin opened.
    fn on_menu_click(&mut self, player: &Player, menu: &str, slot: u32) {}
    /// A menu of this plugin was closed (by the player or by the server).
    fn on_menu_close(&mut self, player: &Player, menu: &str) {}
    /// A player sends a chat message. Return false to cancel it.
    fn on_chat(&mut self, player: &Player, message: &str) -> bool {
        true
    }
    /// A player breaks a block. Return false to cancel it (the block stays).
    fn on_block_break(&mut self, player: &Player, block: &Block) -> bool {
        true
    }
    /// A player places a block. Return false to cancel it.
    fn on_block_place(&mut self, player: &Player, block: &Block) -> bool {
        true
    }
    /// A player uses an item or their hand on a block in a way that changes it or the world
    /// around it (lighting candles, eating cake, brushing, tying a lead to a fence, placing a
    /// boat...). Return false to cancel it.
    fn on_block_interact(&mut self, player: &Player, block: &Block) -> bool {
        true
    }
    /// A player is about to take damage. Return false to cancel it.
    fn on_damage(&mut self, player: &Player, amount: f32) -> bool {
        true
    }
    /// A player hits another player. Return false to cancel the hit.
    fn on_attack(&mut self, attacker: &Player, target: &Player) -> bool {
        true
    }
    /// A player answered (or closed) a form this plugin sent.
    fn on_form(&mut self, player: &Player, form: &str, response: FormResponse) {}
    /// A task from [`schedule`] / [`schedule_repeating`] is due.
    fn on_task(&mut self, task: u32) {}
}

/// Runs [`Plugin::on_task`] once after `delay_ticks` (20 ticks = 1 second). Returns the task id.
pub fn schedule(delay_ticks: u32) -> u32 {
    call(json!({"op": "schedule", "delay": delay_ticks}))["task"].as_u64().unwrap_or(0) as u32
}

/// Runs [`Plugin::on_task`] every `period_ticks`, the first time after `delay_ticks`.
pub fn schedule_repeating(delay_ticks: u32, period_ticks: u32) -> u32 {
    call(json!({"op": "schedule", "delay": delay_ticks, "period": period_ticks.max(1)}))["task"].as_u64().unwrap_or(0) as u32
}

/// Stops a scheduled task.
pub fn cancel_task(task: u32) {
    call(json!({"op": "cancel_task", "task": task}));
}

/// A command players can use (shown in the command list with autocompletion).
pub fn register_command(name: &str, description: &str) {
    call(json!({"op": "register_command", "name": name, "description": description}));
}

/// Writes to the server console.
pub fn log(text: &str) {
    call(json!({"op": "log", "text": text}));
}

/// Message to everyone.
pub fn broadcast(text: &str) {
    call(json!({"op": "broadcast", "text": text}));
}

/// Runs a command as the console (e.g. "give Steve diamond 1").
pub fn run_console_command(line: &str) {
    call(json!({"op": "console_command", "line": line}));
}

/// Players online when the current event started.
pub fn online_players() -> Vec<Player> {
    call(json!({"op": "players"}))["players"]
        .as_array()
        .map(|a| a.iter().filter_map(|p| p.as_str()).map(Player::new).collect())
        .unwrap_or_default()
}

/// Sends a request to the server and returns its answer.
pub fn call(request: Value) -> Value {
    let bytes = request.to_string().into_bytes();
    let answer = host::call(&bytes);
    serde_json::from_slice(&answer).unwrap_or(Value::Null)
}

#[cfg(target_arch = "wasm32")]
mod host {
    #[link(wasm_import_module = "ceres")]
    unsafe extern "C" {
        /// handles a JSON request; returns the length of the JSON answer
        #[link_name = "call"]
        fn host_call(ptr: *const u8, len: usize) -> usize;
        /// copies the last answer to `ptr`
        fn take_answer(ptr: *mut u8);
    }

    pub fn call(request: &[u8]) -> Vec<u8> {
        unsafe {
            let len = host_call(request.as_ptr(), request.len());
            let mut out = vec![0u8; len];
            take_answer(out.as_mut_ptr());
            out
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod host {
    /// outside the server (unit tests of a plugin) requests go nowhere
    pub fn call(_request: &[u8]) -> Vec<u8> {
        b"null".to_vec()
    }
}

/// Used by [`plugin!`]: routes one event from the server to the plugin.
#[doc(hidden)]
pub fn dispatch(plugin: &mut dyn Plugin, event: &[u8]) {
    let Ok(e) = serde_json::from_slice::<Value>(event) else { return };
    let player = || Player::new(e["player"].as_str().unwrap_or_default());
    match e["event"].as_str().unwrap_or_default() {
        "enable" => plugin.on_enable(),
        "disable" => plugin.on_disable(),
        "join" => plugin.on_join(&player()),
        "quit" => plugin.on_quit(&player()),
        "command" => {
            let args: Vec<String> =
                e["args"].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default();
            plugin.on_command(&player(), e["command"].as_str().unwrap_or_default(), &args)
        }
        "menu_click" => plugin.on_menu_click(&player(), e["menu"].as_str().unwrap_or_default(), e["slot"].as_u64().unwrap_or(0) as u32),
        "menu_close" => plugin.on_menu_close(&player(), e["menu"].as_str().unwrap_or_default()),
        "task" => plugin.on_task(e["task"].as_u64().unwrap_or(0) as u32),
        "chat" | "block_break" | "block_place" | "block_interact" | "damage" | "attack" => {
            let block = || Block {
                name: e["block"].as_str().unwrap_or_default().to_string(),
                world: e["world"].as_str().unwrap_or_default().to_string(),
                x: e["x"].as_i64().unwrap_or(0) as i32,
                y: e["y"].as_i64().unwrap_or(0) as i32,
                z: e["z"].as_i64().unwrap_or(0) as i32,
            };
            let allowed = match e["event"].as_str().unwrap_or_default() {
                "chat" => plugin.on_chat(&player(), e["message"].as_str().unwrap_or_default()),
                "block_break" => plugin.on_block_break(&player(), &block()),
                "block_place" => plugin.on_block_place(&player(), &block()),
                "block_interact" => plugin.on_block_interact(&player(), &block()),
                "damage" => plugin.on_damage(&player(), e["amount"].as_f64().unwrap_or(0.0) as f32),
                _ => plugin.on_attack(&player(), &Player::new(e["target"].as_str().unwrap_or_default())),
            };
            if !allowed {
                call(json!({"op": "cancel"}));
            }
        }
        "form" => {
            let r = &e["response"];
            let response = match (e["kind"].as_str().unwrap_or_default(), r) {
                (_, Value::Null) => FormResponse::Closed,
                ("form", Value::Number(n)) => FormResponse::Button(n.as_u64().unwrap_or(0) as u32),
                ("modal", Value::Bool(b)) => FormResponse::Modal(*b),
                ("custom_form", Value::Array(a)) => FormResponse::Custom(a.clone()),
                _ => FormResponse::Closed,
            };
            plugin.on_form(&player(), e["form"].as_str().unwrap_or_default(), response)
        }
        _ => {}
    }
}

/// Declares the plugin type (it must implement [`Plugin`] and [`Default`]).
#[macro_export]
macro_rules! plugin {
    ($ty:ty) => {
        static mut CERES_PLUGIN: Option<$ty> = None;

        /// memory for the server to write events into
        #[unsafe(no_mangle)]
        pub extern "C" fn ceres_alloc(len: usize) -> *mut u8 {
            let mut buf = Vec::<u8>::with_capacity(len.max(1));
            let ptr = buf.as_mut_ptr();
            std::mem::forget(buf);
            ptr
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn ceres_event(ptr: *mut u8, len: usize) {
            let event = unsafe { Vec::from_raw_parts(ptr, len, len.max(1)) };
            #[allow(static_mut_refs)]
            let plugin = unsafe { CERES_PLUGIN.get_or_insert_with(<$ty as Default>::default) };
            $crate::dispatch(plugin, &event);
        }
    };
}
