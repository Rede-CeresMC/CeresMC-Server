//! Chest menus for plugins, with no chest anywhere: the window is the inventory of an invisible
//! entity (no model, no collision, no nametag) that only that player knows about and that follows
//! the player every tick, so the menu never closes for distance (flying, elytra, teleports) and
//! nothing appears in the world. It has 27 slots (the client keeps the chest minecart's size: no
//! entity tried showed 54).
//! Nobody can take the items: each click is refused and reported to the plugin.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::ItemDesc;
use serde_json::{Value, json};

const EYE_HEIGHT: f32 = 1.62;

pub(crate) struct FakeMenu {
    pub plugin: String,
    pub id: String,
    pub items: Vec<ItemDesc>,
    pub window: u8,
    /// the invisible entity holding the menu, and where the player last saw it
    pub entity: i64,
    pub entity_pos: [f32; 3],
    /// tick the window opens (the client must have the entity first)
    pub open_at: Option<u64>,
    pub window_type: u8,
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

    fn feet_of(&self, sid: SessionId) -> Option<[f32; 3]> {
        let p = self.players.get(&sid)?;
        Some([p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]])
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
        let entity = self.next_entity_id;
        self.next_entity_id += 1;
        let feet = self.feet_of(sid).unwrap_or_default();
        let tick = self.ticks;
        let Some(p) = self.players.get_mut(&sid) else { return };
        p.window_counter = p.window_counter % 99 + 1;
        let title: String = title.chars().take(64).collect();
        let window_type = protocol::WINDOW_TYPE_CONTAINER;
        p.out.push(protocol::add_menu_entity(entity, "minecraft:chest_minecart", feet, &title, items.len() as i32));
        p.menu = Some(FakeMenu {
            plugin: plugin.to_string(),
            id: id.to_string(),
            items,
            window: p.window_counter,
            entity,
            entity_pos: feet,
            open_at: Some(tick + 1),
            window_type,
        });
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
            p.out.push(protocol::container_close(m.window, m.window_type, true));
        }
        let name = p.name.clone();
        p.out.push(protocol::remove_actor(m.entity));
        // the player's own inventory may have been touched by refused clicks
        self.sync_full_inventory(sid);
        if notify {
            if let Some(idx) = self.plugins.index_of(&m.plugin) {
                self.plugin_event(idx, json!({"event": "menu_close", "player": name, "menu": m.id}));
            }
        }
    }

    /// Opens the windows the client is ready for, and keeps each menu entity on its player.
    pub(crate) fn tick_menus(&mut self) {
        let tick = self.ticks;
        for p in self.players.values_mut() {
            let feet = [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]];
            let block = [feet[0].floor() as i32, feet[1].floor() as i32, feet[2].floor() as i32];
            let Some(m) = p.menu.as_mut() else { continue };
            if m.entity_pos != feet {
                m.entity_pos = feet;
                p.out.push(protocol::move_actor_absolute(m.entity, feet, false, true));
            }
            if m.open_at.is_some_and(|t| t <= tick) {
                m.open_at = None;
                p.out.push(protocol::open_entity_container(m.window, m.window_type, block, m.entity));
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
        use protocol::container as c;
        // the menu's slots are whatever is not the player's own inventory
        let own = [c::ARMOR, c::COMBINED_HOTBAR_AND_INVENTORY, c::HOTBAR, c::INVENTORY, c::CRAFTING_INPUT, c::OFFHAND, c::CURSOR, c::CREATED_OUTPUT];
        let in_menu = |s: &protocol::SlotRef| !own.contains(&s.container);
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
        crate::logfile::log(&format!("[Menus] {} clicked slot {slot} of {} ({:?})", p.name, m.id, req.actions));
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

    /// After a teleport: the menu entity jumps along with the player (the window stays open).
    pub(crate) fn follow_menu(&mut self, sid: SessionId) {
        let Some(feet) = self.feet_of(sid) else { return };
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(m) = p.menu.as_mut() else { return };
        m.entity_pos = feet;
        p.out.push(protocol::move_actor_absolute(m.entity, feet, false, true));
    }
}
