//! Brewing stands: a reagent on top, up to three bottles, blaze powder as fuel (20 brews each).
//! A brew takes 20 seconds; potion mixes and container mixes (gunpowder, dragon's breath) come
//! from recipes.json. Contents stay in the vanilla BrewingStand block entity.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::ItemDesc;

pub(crate) const UI_BREWING_INPUT: u8 = 9;
pub(crate) const UI_BREWING_RESULT: u8 = 10;
pub(crate) const UI_BREWING_FUEL: u8 = 11;
pub(crate) const BREWING_SLOTS: usize = 5;
const BREW_TICKS: u16 = 400;
const FUEL_PER_POWDER: u16 = 20;
const SET_DATA_BREW_TIME: i32 = 0;
const SET_DATA_FUEL_AMOUNT: i32 = 1;
const SET_DATA_FUEL_TOTAL: i32 = 2;

#[derive(Clone)]
pub(crate) struct BrewingState {
    items: Vec<ItemDesc>,
    brew_time: u16,
    fuel_amount: u16,
    fuel_total: u16,
}

impl BrewingState {
    pub(crate) fn items(&self) -> Vec<ItemDesc> {
        self.items.clone()
    }
    pub(crate) fn set_items(&mut self, items: Vec<ItemDesc>) {
        self.items = items;
    }
}

fn tag_u16(t: &Tag, key: &str) -> u16 {
    t.get(key).and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, u16::MAX as i64) as u16
}

impl Server {
    fn item_name(&self, it: &ItemDesc) -> Option<String> {
        (!it.is_empty()).then(|| self.gd.items_by_id.get(&it.id).map(|i| i.name.clone())).flatten()
    }

    /// The bottle a reagent turns `bottle` into, if any.
    fn brew_result(&self, bottle: &ItemDesc, reagent: &ItemDesc) -> Option<ItemDesc> {
        let (b, r) = (self.item_name(bottle)?, self.item_name(reagent)?);
        let (out, meta) = self.gd.brewing.result(&b, bottle.meta as i32, &r)?;
        let info = self.gd.items_by_name.get(&out)?;
        Some(ItemDesc {
            id: info.id,
            count: 1,
            meta: meta.max(0) as u32,
            stack_id: 0,
            block_runtime_id: 0,
            extra: ceres_world::extra_data(None),
        })
    }

    pub(crate) fn wake_brewing(&mut self, world: usize, pos: [i32; 3]) {
        let tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd);
        let items = containers::items_from_tile(tile.as_ref(), &self.gd, BREWING_SLOTS);
        let t = tile.unwrap_or(Tag::Compound(Vec::new()));
        self.brewers.insert(
            (world, pos),
            BrewingState {
                items,
                brew_time: tag_u16(&t, "CookTime"),
                fuel_amount: tag_u16(&t, "FuelAmount"),
                fuel_total: tag_u16(&t, "FuelTotal"),
            },
        );
    }

    fn save_brewing(&mut self, world: usize, pos: [i32; 3]) {
        let Some(b) = self.brewers.get(&(world, pos)).cloned() else { return };
        let mut tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd).unwrap_or_else(|| {
            Tag::Compound(vec![("id".into(), Tag::String("BrewingStand".into())), ("isMovable".into(), Tag::Byte(1))])
        });
        tile.set("Items", containers::items_to_list(&b.items, &self.gd));
        tile.set("CookTime", Tag::Short(b.brew_time as i16));
        tile.set("FuelAmount", Tag::Short(b.fuel_amount as i16));
        tile.set("FuelTotal", Tag::Short(b.fuel_total as i16));
        self.worlds.worlds[world].set_block_entity(pos, tile, &self.gd);
    }

    /// The stand shows which of its three bottle slots hold something.
    fn update_bottle_bits(&mut self, world: usize, pos: [i32; 3], items: &[ItemDesc]) {
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        let bits = [("brewing_stand_slot_a_bit", 1), ("brewing_stand_slot_b_bit", 2), ("brewing_stand_slot_c_bit", 3)];
        let updates: Vec<(&str, Tag)> = bits.iter().map(|(k, i)| (*k, Tag::Byte(!items[*i].is_empty() as i8))).collect();
        let Some(new) = self.gd.with_states(block, &updates) else { return };
        if new != block {
            let tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd);
            self.change_block(world, pos, new);
            if let Some(t) = tile {
                self.worlds.worlds[world].set_block_entity(pos, t, &self.gd);
            }
        }
    }

    pub(crate) fn tick_brewing(&mut self) {
        let keys: Vec<(usize, [i32; 3])> = self.brewers.keys().copied().collect();
        for (world, pos) in keys {
            let block = self.worlds.worlds[world].get_block(pos, &self.gd);
            if self.gd.block_name(block) != "minecraft:brewing_stand" {
                self.brewers.remove(&(world, pos));
                continue;
            }
            let Some(mut b) = self.brewers.get(&(world, pos)).cloned() else { continue };
            let mut changed = false;
            // blaze powder refills the fuel
            if b.fuel_amount == 0 && self.item_name(&b.items[4]).as_deref() == Some("minecraft:blaze_powder") {
                b.items[4].count -= 1;
                if b.items[4].count == 0 {
                    b.items[4] = ItemDesc::default();
                }
                b.fuel_amount = FUEL_PER_POWDER;
                b.fuel_total = FUEL_PER_POWDER;
                changed = true;
            }
            let reagent = b.items[0].clone();
            let can_brew = !reagent.is_empty() && (1..=3).any(|i| self.brew_result(&b.items[i], &reagent).is_some());
            if b.brew_time > 0 {
                if !can_brew {
                    b.brew_time = 0; // reagent or bottles taken away: the brew stops
                } else {
                    b.brew_time -= 1;
                    if b.brew_time == 0 {
                        for i in 1..=3 {
                            if let Some(out) = self.brew_result(&b.items[i], &reagent) {
                                b.items[i] = out;
                            }
                        }
                        b.items[0].count -= 1;
                        if b.items[0].count == 0 {
                            b.items[0] = ItemDesc::default();
                        }
                        changed = true;
                        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
                        self.broadcast(&protocol::level_sound_event("potion.brewed", center, -1), |_, q| q.world == world);
                    }
                }
            } else if can_brew && b.fuel_amount > 0 {
                b.brew_time = BREW_TICKS;
                b.fuel_amount -= 1;
            }
            let idle = b.brew_time == 0 && !can_brew;
            let items = b.items.clone();
            self.brewers.insert((world, pos), b.clone());
            if changed || idle || self.ticks % 20 == 0 {
                self.save_brewing(world, pos);
                self.update_bottle_bits(world, pos, &items);
            }
            // viewers
            let viewers: Vec<SessionId> = self
                .players
                .iter()
                .filter(|(_, q)| q.inventory.container.as_ref().is_some_and(|c| c.world == world && c.pos == pos))
                .map(|(s, _)| *s)
                .collect();
            for sid in viewers {
                let Some(q) = self.players.get_mut(&sid) else { continue };
                let Some(window) = q.inventory.container.as_ref().map(|c| c.window) else { continue };
                if changed {
                    let mut fresh = items.clone();
                    for it in fresh.iter_mut().filter(|it| !it.is_empty()) {
                        it.stack_id = q.inventory.alloc_id();
                    }
                    if let Some(c) = q.inventory.container.as_mut() {
                        c.items = fresh.clone();
                        c.changed = false;
                    }
                    q.out.push(protocol::inventory_content(window as u32, &fresh));
                }
                q.out.push(protocol::container_set_data(window, SET_DATA_BREW_TIME, b.brew_time as i32));
                q.out.push(protocol::container_set_data(window, SET_DATA_FUEL_AMOUNT, b.fuel_amount as i32));
                q.out.push(protocol::container_set_data(window, SET_DATA_FUEL_TOTAL, b.fuel_total as i32));
            }
            if idle {
                self.brewers.remove(&(world, pos));
            }
        }
    }
}
