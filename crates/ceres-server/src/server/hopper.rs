//! Hoppers: every 8 ticks each loaded hopper pulls one item from the container above it (or an
//! item lying on top of it) and pushes one item into the container it points at. Furnaces take
//! from above into the input, from the side into the fuel slot, and give their result downwards.
//! TODO: redstone (a powered hopper stops), minecarts.

use super::*;
use ceres_protocol::ItemDesc;

const TRANSFER_TICKS: u64 = 8;
pub(crate) const HOPPER_SLOTS: usize = 5;

fn facing_offset(facing: i64) -> [i32; 3] {
    match facing {
        0 => [0, -1, 0],
        2 => [0, 0, -1],
        3 => [0, 0, 1],
        4 => [-1, 0, 0],
        5 => [1, 0, 0],
        _ => [0, -1, 0],
    }
}

impl Server {
    /// Items of the container at `pos` (furnaces: their live state) and its slot layout.
    pub(crate) fn stored_items(&mut self, world: usize, pos: [i32; 3]) -> Option<(u8, &'static str, Vec<ItemDesc>)> {
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        let (ui, tile_id, _) = containers::container_kind(self.gd.block_name(block))?;
        if tile_id.is_empty() || tile_id == "BrewingStand" {
            return None; // crafting table, stonecutter; brewing stands are not fed by hoppers yet
        }
        if furnace::is_furnace_ui(ui) {
            if !self.furnaces.contains_key(&(world, pos)) {
                self.wake_furnace(world, pos, ui);
            }
            return self.furnaces.get(&(world, pos)).map(|f| (ui, tile_id, f.items()));
        }
        let tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd);
        Some((ui, tile_id, containers::items_from_tile(tile.as_ref(), &self.gd, containers::slots_of(tile_id))))
    }

    /// Stores new contents of a container and shows them to whoever is looking at it.
    pub(crate) fn store_items(&mut self, world: usize, pos: [i32; 3], ui: u8, tile_id: &str, items: Vec<ItemDesc>) {
        if furnace::is_furnace_ui(ui) {
            if let Some(f) = self.furnaces.get_mut(&(world, pos)) {
                f.set_items(items);
            }
            return; // the furnace tick saves it and updates its viewers
        }
        let mut tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd).unwrap_or_else(|| {
            ceres_codec::nbt::Tag::Compound(vec![
                ("id".into(), ceres_codec::nbt::Tag::String(tile_id.into())),
                ("isMovable".into(), ceres_codec::nbt::Tag::Byte(1)),
            ])
        });
        tile.set("Items", containers::items_to_list(&items, &self.gd));
        self.worlds.worlds[world].set_block_entity(pos, tile, &self.gd);
        self.refresh_container_viewers(world, pos);
    }

    /// Re-sends a container to everyone looking at it (after something other than them changed it).
    pub(crate) fn refresh_container_viewers(&mut self, world: usize, pos: [i32; 3]) {
        let viewers: Vec<(SessionId, [i32; 3], Option<[i32; 3]>)> = self
            .players
            .iter()
            .filter_map(|(s, q)| {
                let c = q.inventory.container.as_ref()?;
                (c.world == world && (c.pos == pos || c.pair == Some(pos))).then_some((*s, c.pos, c.pair))
            })
            .collect();
        for (sid, first, pair) in viewers {
            let items = match pair {
                Some(_) => self.double_chest_items(world, first).map(|(_, _, i)| i),
                None => self.stored_items(world, first).map(|(_, _, i)| i),
            };
            let Some(mut items) = items else { continue };
            let Some(q) = self.players.get_mut(&sid) else { continue };
            for it in items.iter_mut().filter(|it| !it.is_empty()) {
                it.stack_id = q.inventory.alloc_id();
            }
            let Some(c) = q.inventory.container.as_mut() else { continue };
            if c.items.len() != items.len() {
                continue;
            }
            c.items = items.clone();
            c.changed = false;
            let window = c.window;
            q.out.push(protocol::inventory_content(window as u32, &items));
        }
    }

    /// Puts one of `item` into a slot of `items` (allowed slots only). Returns false if full.
    pub(crate) fn insert_one(&self, items: &mut [ItemDesc], slots: std::ops::Range<usize>, item: &ItemDesc) -> bool {
        let max = self.gd.max_stack(item.id) as u16;
        for i in slots.clone() {
            if items[i].same_type(item) && items[i].count < max {
                items[i].count += 1;
                return true;
            }
        }
        for i in slots {
            if items[i].is_empty() {
                let mut one = item.clone();
                one.count = 1;
                one.stack_id = 0;
                items[i] = one;
                return true;
            }
        }
        false
    }

    fn take_one(items: &mut [ItemDesc], slot: usize) -> ItemDesc {
        let mut one = items[slot].clone();
        one.count = 1;
        items[slot].count -= 1;
        if items[slot].count == 0 {
            items[slot] = ItemDesc::default();
        }
        one
    }

    fn tick_hopper(&mut self, world: usize, pos: [i32; 3]) {
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        if self.gd.block_name(block) != "minecraft:hopper" {
            return;
        }
        // locked by redstone
        if self.gd.block_states.get(&block).and_then(|s| s.states.get("toggle_bit")).and_then(|t| t.as_i64()) == Some(1) {
            return;
        }
        let facing =
            self.gd.block_states.get(&block).and_then(|s| s.states.get("facing_direction")).and_then(|t| t.as_i64()).unwrap_or(0);
        let Some((hui, htile, mut hitems)) = self.stored_items(world, pos) else { return };
        let mut changed = false;

        // 1. push one item forward
        let off = facing_offset(facing);
        let target = [pos[0] + off[0], pos[1] + off[1], pos[2] + off[2]];
        if let Some(src) = hitems.iter().position(|i| !i.is_empty()) {
            if let Some((tui, ttile, mut titems)) = self.stored_items(world, target) {
                let slots = if furnace::is_furnace_ui(tui) {
                    let is_fuel = self.gd.items_by_id.get(&hitems[src].id).and_then(|i| furnace::fuel_ticks(&i.name)).is_some();
                    if facing == 0 { 0..1 } else if is_fuel { 1..2 } else { 0..0 }
                } else {
                    0..titems.len()
                };
                let item = hitems[src].clone();
                if self.insert_one(&mut titems, slots, &item) {
                    Self::take_one(&mut hitems, src);
                    changed = true;
                    self.store_items(world, target, tui, ttile, titems);
                }
            }
        }

        // 2. pull one item from above
        let above = [pos[0], pos[1] + 1, pos[2]];
        if let Some((sui, stile, mut sitems)) = self.stored_items(world, above) {
            let candidates: Vec<usize> = if furnace::is_furnace_ui(sui) { vec![2] } else { (0..sitems.len()).collect() };
            for slot in candidates {
                if sitems[slot].is_empty() {
                    continue;
                }
                let item = sitems[slot].clone();
                if self.insert_one(&mut hitems, 0..HOPPER_SLOTS, &item) {
                    Self::take_one(&mut sitems, slot);
                    changed = true;
                    self.store_items(world, above, sui, stile, sitems);
                    break;
                }
            }
        } else {
            // items lying on top of the hopper fall in, a whole stack at a time if it fits
            let found = self.items.iter().position(|e| {
                e.world == world
                    && e.pos[0].floor() as i32 == pos[0]
                    && e.pos[2].floor() as i32 == pos[2]
                    && (e.pos[1] - (pos[1] + 1) as f32).abs() < 0.6
            });
            if let Some(i) = found {
                let item = self.items[i].item.clone();
                let mut moved = 0;
                for _ in 0..item.count {
                    if !self.insert_one(&mut hitems, 0..HOPPER_SLOTS, &item) {
                        break;
                    }
                    moved += 1;
                }
                if moved > 0 {
                    changed = true;
                    let e = &mut self.items[i];
                    e.item.count -= moved;
                    let (eid, left) = (e.entity_id, e.item.count);
                    if left == 0 {
                        self.items.swap_remove(i);
                        self.broadcast(&protocol::remove_actor(eid), |_, q| q.world == world);
                    }
                }
            }
        }
        if changed {
            self.store_items(world, pos, hui, htile, hitems);
        }
    }

    /// Every 8 ticks, every hopper in the loaded parts of the worlds.
    pub(crate) fn tick_hoppers(&mut self) {
        if self.ticks % TRANSFER_TICKS != 0 {
            return;
        }
        for world in 0..self.worlds.worlds.len() {
            let hoppers = self.worlds.worlds[world].block_entities_with_id("Hopper");
            for pos in hoppers {
                self.tick_hopper(world, pos);
            }
        }
    }
}
