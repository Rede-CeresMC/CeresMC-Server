//! Grindstone: takes the enchantments off an item (curses stay) and gives some experience
//! back; two items of the same kind are merged with their durability added up (+5%).
//! Enchanted books become plain books.

use super::*;
use ceres_protocol::ItemDesc;

pub(crate) const GRINDSTONE_UI: u8 = 0xfc;
pub(crate) const UI_GRIND_INPUT: u8 = 50;
pub(crate) const UI_GRIND_ADDITIONAL: u8 = 51;
pub(crate) const SLOT_GRIND_INPUT: u8 = 16;
pub(crate) const SLOT_GRIND_ADDITIONAL: u8 = 17;
const CURSES: [i16; 2] = [27, 28]; // binding, vanishing

/// Minimum enchanting power of an enchantment level (for the experience given back).
fn min_power(id: i16, level: i16) -> u32 {
    let l = level.max(1) as u32;
    match id {
        0 | 9 => 1 + (l - 1) * 11,
        1 => 10 + (l - 1) * 8,
        2 | 4 => if id == 2 { 5 + (l - 1) * 6 } else { 3 + (l - 1) * 6 },
        3 | 10 | 11 | 17 => 5 + (l - 1) * 8,
        5 | 12 | 13 => if id == 5 || id == 13 { 10 + (l - 1) * 20 } else { 5 + (l - 1) * 20 },
        6 => 10 * l,
        14 | 18 => 15 + (l - 1) * 9,
        15 | 19 => 1 + (l - 1) * 10,
        16 => 15,
        20 => 12 + (l - 1) * 20,
        21 | 22 => 20,
        _ => 1 + (l - 1) * 10,
    }
}

/// The grindstone's result for these two inputs (either may be empty), and the experience.
pub(crate) fn grind(a: &ItemDesc, b: &ItemDesc, gd: &GameData) -> Option<(ItemDesc, u32)> {
    let (first, second) = match (a.is_empty(), b.is_empty()) {
        (true, true) => return None,
        (false, true) => (a, None),
        (true, false) => (b, None),
        (false, false) => {
            if a.id != b.id || a.count != 1 || b.count != 1 {
                return None;
            }
            (a, Some(b))
        }
    };
    let name = gd.items_by_id.get(&first.id)?.name.clone();
    let max = durability::max_durability(&name);
    if second.is_some() && max.is_none() {
        return None; // only things with durability can be merged
    }
    let mut out = first.clone();
    out.count = 1;
    out.stack_id = 0;
    let mut power = 0;
    for it in std::iter::once(first).chain(second) {
        for (id, lvl) in durability::enchantments(it) {
            if !CURSES.contains(&id) {
                power += min_power(id, lvl);
            }
        }
    }
    // a book loses everything and becomes a plain book
    if name == "minecraft:enchanted_book" {
        let book = gd.items_by_name.get("minecraft:book")?;
        out = ItemDesc { id: book.id, count: 1, meta: 0, stack_id: 0, block_runtime_id: 0, extra: ceres_world::extra_data(None) };
    } else {
        durability::keep_only_enchantments(&mut out, &CURSES);
        if let (Some(other), Some(max)) = (second, max) {
            let left = |it: &ItemDesc| max.saturating_sub(durability::item_damage(it));
            let total = (left(first) + left(other) + max * 5 / 100).min(max);
            durability::set_damage(&mut out, max - total);
        }
    }
    if power == 0 && second.is_none() {
        return None; // nothing to take off
    }
    let xp = if power == 0 { 0 } else { power.div_ceil(2) + rand::random::<u32>() % (power / 2 + 1) };
    Some((out, xp))
}

impl Server {
    /// Experience given back by the last grindstone use.
    pub(crate) fn finish_grinding(&mut self, sid: SessionId) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(xp) = p.inventory.grind_xp.take() else { return };
        let (world, pos) = (p.world, p.pos);
        self.broadcast(&protocol::level_sound_event("block.grindstone.use", pos, -1), |_, q| q.world == world);
        if xp > 0 {
            self.spawn_xp(world, [pos[0], pos[1] - 1.0, pos[2]], xp);
        }
    }
}
