//! Chest menus for plugins, with no chest in the world: a chest that only the player sees is put
//! under their feet (inside the ground), the window is opened on it and the real blocks come back
//! when it closes. Nobody can take the items: each click is refused and reported to the plugin.
//! Teleporting moves the menu along (it reopens at the destination with the same contents).

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::ItemDesc;
use serde_json::{Value, json};

const EYE_HEIGHT: f32 = 1.62;

pub(crate) struct FakeMenu {
    pub plugin: String,
    pub id: String,
    pub title: String,
    pub items: Vec<ItemDesc>,
    pub window: u8,
    pub world: usize,
    /// the fake chest (and its second half for 54 slots)
    pub pos: [i32; 3],
    pub pair: Option<[i32; 3]>,
    /// tick the window opens (the client must have the chest first; a double chest pairs later)
    pub open_at: Option<u64>,
}

impl Server {
    /// Item of a plugin request: {"id", "count", "meta", "name", "lore"}.
    pub(crate) fn menu_item(&self, v: &Value) -> Option<ItemDesc> {
        let info = self.gd.find_item(v["id"].as_str()?)?;
        let mut display = Vec::new();
        if let Some(name) = v["name"].as_str() {
            display.push(("Name".to_string(), Tag::String(name.chars().take(256).collect())));
        }
        let lore: Vec<Tag> = v["lore"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l.as_str())
            .take(32)
            .map(|l| Tag::String(l.chars().take(256).collect()))
            .collect();
        if !lore.is_empty() {
            display.push(("Lore".to_string(), Tag::List(8, lore)));
        }
        let extra = if display.is_empty() {
            ceres_world::data::extra_data(None)
        } else {
            let tag = Tag::Compound(vec![("display".to_string(), Tag::Compound(display))]);
            let mut w = ceres_codec::binary::W::new();
            ceres_codec::nbt::write_root(&mut w, "", &tag, ceres_codec::nbt::Flavor::LittleEndian);
            ceres_world::data::extra_data(Some(&w.0))
        };
        Some(ItemDesc {
            id: info.id,
            count: v["count"].as_u64().unwrap_or(1).clamp(1, 64) as u16,
            meta: v["meta"].as_u64().unwrap_or(0) as u32,
            stack_id: 0,
            block_runtime_id: info.block_runtime_id,
            extra,
        })
    }

    /// Where the fake chest goes: two blocks under the player's feet (inside the ground).
    fn menu_spot(&self, sid: SessionId) -> Option<(usize, [i32; 3])> {
        let p = self.players.get(&sid)?;
        let feet = [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]];
        let y = (feet[1].floor() as i32 - 2).clamp(ceres_world::MIN_Y, ceres_world::MAX_Y - 1);
        Some((p.world, [feet[0].floor() as i32, y, feet[2].floor() as i32]))
    }

    /// Shows the fake chest to the player (and asks for the window a moment later).
    fn place_menu_chest(&mut self, sid: SessionId) {
        let Some((world, pos)) = self.menu_spot(sid) else { return };
        let Some(chest) = self.gd.default_block("minecraft:chest") else { return };
        let tick = self.ticks;
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(m) = p.menu.as_mut() else { return };
        let double = m.items.len() > 27;
        m.world = world;
        m.pos = pos;
        m.pair = double.then_some([pos[0] + 1, pos[1], pos[2]]);
        // a double chest needs a few ticks to pair on the client
        m.open_at = Some(tick + if double { 4 } else { 1 });
        p.window_counter = p.window_counter % 99 + 1;
        m.window = p.window_counter;
        let title = m.title.clone();
        let halves: Vec<([i32; 3], Option<[i32; 3]>)> = match m.pair {
            Some(pair) => vec![(pos, Some(pair)), (pair, Some(pos))],
            None => vec![(pos, None)],
        };
        for (half, other) in halves {
            p.out.push(protocol::update_block(half, chest));
            let mut fields = vec![
                ("id".to_string(), Tag::String("Chest".into())),
                ("x".to_string(), Tag::Int(half[0])),
                ("y".to_string(), Tag::Int(half[1])),
                ("z".to_string(), Tag::Int(half[2])),
                ("CustomName".to_string(), Tag::String(title.clone())),
            ];
            if let Some(o) = other {
                fields.push(("pairx".to_string(), Tag::Int(o[0])));
                fields.push(("pairz".to_string(), Tag::Int(o[2])));
                fields.push(("pairlead".to_string(), Tag::Byte((half == pos) as i8)));
            }
            p.out.push(protocol::block_actor_data(half, &Tag::Compound(fields)));
        }
    }

    /// Puts the real blocks (and their block entities) back where the fake chest was.
    fn remove_menu_chest(&mut self, sid: SessionId, world: usize, pos: [i32; 3], pair: Option<[i32; 3]>) {
        for half in std::iter::once(pos).chain(pair) {
            let real = self.worlds.worlds[world].get_block(half, &self.gd);
            let tile = self.worlds.worlds[world].get_block_entity(half, &self.gd);
            let Some(p) = self.players.get_mut(&sid) else { return };
            if p.world != world {
                continue;
            }
            p.out.push(protocol::update_block(half, real));
            if let Some(tile) = tile {
                p.out.push(protocol::block_actor_data(half, &tile));
            }
        }
    }

    /// Opens a plugin menu for the player (closing what they had open).
    pub(crate) fn open_menu(&mut self, sid: SessionId, plugin: &str, id: &str, title: &str, items: Vec<ItemDesc>) {
        self.close_container(sid, true);
        if self.players.get(&sid).is_some_and(|p| p.menu.is_some()) {
            self.close_menu(sid, true, true);
        }
        let Some(p) = self.players.get_mut(&sid) else { return };
        if p.dead || p.state != State::Spawned {
            return;
        }
        let mut items = items;
        for it in items.iter_mut().filter(|it| !it.is_empty()) {
            it.stack_id = p.inventory.alloc_id();
        }
        p.menu = Some(FakeMenu {
            plugin: plugin.to_string(),
            id: id.to_string(),
            title: title.chars().take(64).collect(),
            items,
            window: 0,
            world: p.world,
            pos: [0; 3],
            pair: None,
            open_at: None,
        });
        self.place_menu_chest(sid);
    }

    /// Changes one slot of the open menu of this plugin.
    pub(crate) fn set_menu_item(&mut self, sid: SessionId, plugin: &str, slot: usize, mut item: ItemDesc) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(m) = p.menu.as_mut().filter(|m| m.plugin == plugin) else { return };
        if slot >= m.items.len() {
            return;
        }
        if !item.is_empty() {
            item.stack_id = p.inventory.alloc_id();
        }
        m.items[slot] = item;
        if m.open_at.is_none() {
            p.out.push(protocol::inventory_slot(m.window as u32, slot as u32, &m.items[slot]));
        }
    }

    /// Closes the menu. `by_server`: the client's window must be closed too. `notify`: tell the
    /// plugin (on_menu_close).
    pub(crate) fn close_menu(&mut self, sid: SessionId, by_server: bool, notify: bool) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(m) = p.menu.take() else { return };
        if by_server && m.open_at.is_none() {
            p.out.push(protocol::container_close(m.window, protocol::WINDOW_TYPE_CONTAINER, true));
        }
        let name = p.name.clone();
        self.remove_menu_chest(sid, m.world, m.pos, m.pair);
        // the player's own inventory may have been touched by refused clicks
        self.sync_full_inventory(sid);
        if notify {
            if let Some(idx) = self.plugins.index_of(&m.plugin) {
                self.plugin_event(idx, json!({"event": "menu_close", "player": name, "menu": m.id}));
            }
        }
    }

    /// Opens the windows whose chest the client has had long enough.
    pub(crate) fn tick_menus(&mut self) {
        let tick = self.ticks;
        for p in self.players.values_mut() {
            let Some(m) = p.menu.as_mut() else { continue };
            if m.open_at.is_some_and(|t| t <= tick) {
                m.open_at = None;
                p.out.push(protocol::open_block_container(m.window, protocol::WINDOW_TYPE_CONTAINER, m.pos));
                p.out.push(protocol::inventory_content(m.window as u32, &m.items));
            }
        }
    }

    /// The client closed a window: true if it was the menu.
    pub(crate) fn on_menu_window_closed(&mut self, sid: SessionId, window: u8) -> bool {
        let is_menu = self.players.get(&sid).and_then(|p| p.menu.as_ref()).is_some_and(|m| m.window == window && m.open_at.is_none());
        if is_menu {
            self.close_menu(sid, false, true);
        }
        is_menu
    }

    /// An item stack request while a menu is open: anything touching the menu is refused and
    /// becomes a click for the plugin. Returns true when the request was handled here.
    pub(crate) fn menu_stack_request(&mut self, sid: SessionId, req: &protocol::StackRequest) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let Some(m) = p.menu.as_ref() else { return false };
        let in_menu = |s: &protocol::SlotRef| s.container == protocol::block_container::CHEST;
        let mut clicked: Option<u8> = None;
        for a in &req.actions {
            let refs: Vec<&protocol::SlotRef> = match a {
                protocol::StackAction::Move { src, dst, .. } => vec![src, dst],
                protocol::StackAction::Swap { a, b } => vec![a, b],
                protocol::StackAction::Drop { src, .. } | protocol::StackAction::Remove { src, .. } => vec![src],
                _ => vec![],
            };
            if let Some(s) = refs.into_iter().find(|s| in_menu(s)) {
                clicked.get_or_insert(s.slot);
            }
        }
        let Some(slot) = clicked else { return false };
        let (window, items, plugin, id, name) = (m.window, m.items.clone(), m.plugin.clone(), m.id.clone(), p.name.clone());
        let p = self.players.get_mut(&sid).unwrap();
        p.out.push(protocol::item_stack_response(req.id, false, &[]));
        p.out.push(protocol::inventory_content(window as u32, &items));
        self.sync_full_inventory(sid);
        if (slot as usize) < items.len() {
            if let Some(idx) = self.plugins.index_of(&plugin) {
                self.plugin_event(idx, json!({"event": "menu_click", "player": name, "menu": id, "slot": slot}));
            }
        }
        true
    }

    /// After a teleport: the menu moves with the player (it reopens at the destination).
    pub(crate) fn follow_menu(&mut self, sid: SessionId) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(m) = p.menu.as_ref() else { return };
        let (world, pos, pair, window, opened) = (m.world, m.pos, m.pair, m.window, m.open_at.is_none());
        if opened {
            p.out.push(protocol::container_close(window, protocol::WINDOW_TYPE_CONTAINER, true));
        }
        self.remove_menu_chest(sid, world, pos, pair);
        self.place_menu_chest(sid);
    }
}
