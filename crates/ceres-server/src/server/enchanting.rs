//! Enchanting table: bookshelves around it, three options with vanilla costs and enchantments
//! chosen by enchanting power and the item's enchantability (Java/PocketMine algorithm), lapis
//! and levels paid on the server. Books become enchanted books.

use super::*;
use ceres_protocol::ItemDesc;

pub(crate) const ENCHANT_UI: u8 = 0xfd;
pub(crate) const UI_ENCHANT_INPUT: u8 = 22;
pub(crate) const UI_ENCHANT_LAPIS: u8 = 23;
pub(crate) const SLOT_ENCHANT_INPUT: u8 = 14;
pub(crate) const SLOT_ENCHANT_LAPIS: u8 = 15;
const PLAYER_ENCHANT_OPTIONS: u32 = 0x92;
const MAX_BOOKSHELVES: u32 = 15;
/// option ids live far above recipe network ids
static NEXT_OFFER_ID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1_000_000);

#[derive(Clone, Debug)]
pub(crate) struct EnchantOffer {
    pub net_id: i32,
    /// minimum level shown and needed
    pub cost: u32,
    pub enchants: Vec<(u8, u8)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Sword,
    Tool,
    Bow,
    Helmet,
    Chest,
    Legs,
    Boots,
    Book,
}

/// (id, max level, weight, min power of level 1, extra min power per level, power window)
const TABLE: [(u8, u8, u32, u32, u32, u32); 21] = [
    (0, 4, 10, 1, 11, 11),   // protection
    (1, 4, 5, 10, 8, 8),     // fire protection
    (2, 4, 5, 5, 6, 6),      // feather falling
    (3, 4, 2, 5, 8, 8),      // blast protection
    (4, 4, 5, 3, 6, 6),      // projectile protection
    (5, 3, 1, 10, 20, 50),   // thorns
    (6, 3, 2, 10, 10, 30),   // respiration
    (8, 1, 2, 1, 0, 40),     // aqua affinity
    (9, 5, 10, 1, 11, 20),   // sharpness
    (10, 5, 5, 5, 8, 20),    // smite
    (11, 5, 5, 5, 8, 20),    // bane of arthropods
    (12, 2, 5, 5, 20, 50),   // knockback
    (13, 2, 2, 10, 20, 50),  // fire aspect
    (14, 3, 2, 15, 9, 50),   // looting
    (15, 5, 10, 1, 10, 50),  // efficiency
    (16, 1, 1, 15, 0, 50),   // silk touch
    (17, 3, 5, 5, 8, 50),    // unbreaking
    (18, 3, 2, 15, 9, 50),   // fortune
    (19, 5, 10, 1, 10, 15),  // power
    (20, 2, 2, 12, 20, 25),  // punch
    (21, 1, 2, 20, 0, 30),   // flame
];
const INFINITY: (u8, u8, u32, u32, u32, u32) = (22, 1, 1, 20, 0, 30);

fn applies(id: u8, kind: Kind) -> bool {
    match kind {
        Kind::Book => true,
        Kind::Sword => matches!(id, 9..=14 | 17),
        Kind::Tool => matches!(id, 15..=18),
        Kind::Bow => matches!(id, 17 | 19..=22),
        Kind::Helmet => matches!(id, 0..=6 | 8 | 17),
        Kind::Chest | Kind::Legs => matches!(id, 0 | 1 | 3..=5 | 17),
        Kind::Boots => matches!(id, 0..=5 | 17),
    }
}

/// Enchantments that cannot be on the same item.
fn compatible(a: u8, b: u8) -> bool {
    let group = |id: u8| match id {
        0 | 1 | 3 | 4 => 1,  // protections
        9..=11 => 2,         // damage
        16 | 18 => 3,        // silk touch / fortune
        _ => 0,
    };
    a != b && (group(a) == 0 || group(a) != group(b))
}

fn rand_range(lo: u32, hi: u32) -> u32 {
    if hi <= lo { lo } else { lo + rand::random::<u32>() % (hi - lo + 1) }
}

impl Server {
    /// Kind and enchantability of an item, None if it cannot be enchanted.
    fn enchant_kind(&self, item: &ItemDesc) -> Option<(Kind, u32)> {
        let name = self.gd.items_by_id.get(&item.id)?.name.clone();
        let n = name.strip_prefix("minecraft:").unwrap_or(&name);
        if n == "book" {
            return Some((Kind::Book, 1));
        }
        if n == "bow" {
            return Some((Kind::Bow, 1));
        }
        let material = |m: &str, armor: bool| match m {
            "wooden" => 15,
            "stone" => 5,
            "iron" => if armor { 9 } else { 14 },
            "golden" => if armor { 25 } else { 22 },
            "diamond" => 10,
            "netherite" => 15,
            "leather" => 15,
            "chainmail" => 12,
            "copper" => if armor { 8 } else { 13 },
            "turtle" => 9,
            _ => 1,
        };
        let (m, piece) = n.rsplit_once('_')?;
        Some(match piece {
            "sword" => (Kind::Sword, material(m, false)),
            "pickaxe" | "axe" | "shovel" | "hoe" => (Kind::Tool, material(m, false)),
            "helmet" => (Kind::Helmet, material(m, true)),
            "chestplate" => (Kind::Chest, material(m, true)),
            "leggings" => (Kind::Legs, material(m, true)),
            "boots" => (Kind::Boots, material(m, true)),
            _ => return None,
        })
    }

    fn count_bookshelves(&mut self, world: usize, table: [i32; 3]) -> u32 {
        let mut count = 0;
        for x in -2..=2i32 {
            for z in -2..=2i32 {
                if x.abs() != 2 && z.abs() != 2 {
                    continue;
                }
                // the block between the shelf and the table must be air
                let (sx, sz) = (x.clamp(-1, 1), z.clamp(-1, 1));
                let blocked = (0..=1).any(|y| {
                    let b = self.worlds.worlds[world].get_block([table[0] + sx, table[1] + y, table[2] + sz], &self.gd);
                    b != self.gd.air_id
                });
                if blocked {
                    continue;
                }
                for y in 0..=1 {
                    let b = self.worlds.worlds[world].get_block([table[0] + x, table[1] + y, table[2] + z], &self.gd);
                    if self.gd.block_name(b) == "minecraft:bookshelf" {
                        count += 1;
                        if count >= MAX_BOOKSHELVES {
                            return count;
                        }
                    }
                }
            }
        }
        count
    }

    fn roll_enchants(&self, kind: Kind, enchantability: u32, cost: u32) -> Vec<(u8, u8)> {
        let mut power = cost + rand_range(0, enchantability / 4) + rand_range(0, enchantability / 4) + 1;
        let bonus = 1.0 + (rand::random::<f32>() + rand::random::<f32>() - 1.0) * 0.15;
        power = ((power as f32) * bonus).round().max(1.0) as u32;
        let available = |power: u32| -> Vec<(u8, u8, u32)> {
            TABLE
                .iter()
                .chain(std::iter::once(&INFINITY))
                .filter(|e| applies(e.0, kind))
                .filter_map(|&(id, max, weight, base, step, window)| {
                    (1..=max).rev().find(|l| {
                        let min = base + step * (*l as u32 - 1);
                        power >= min && power <= min + window
                    })
                    .map(|l| (id, l, weight))
                })
                .collect()
        };
        let pick = |list: &[(u8, u8, u32)]| -> Option<(u8, u8)> {
            let total: u32 = list.iter().map(|e| e.2).sum();
            if total == 0 {
                return None;
            }
            let mut r = rand_range(1, total) as i64;
            list.iter().find(|e| {
                r -= e.2 as i64;
                r <= 0
            })
            .map(|e| (e.0, e.1))
        };
        let mut list = available(power);
        let mut out = Vec::new();
        let Some(first) = pick(&list) else { return out };
        out.push(first);
        while rand::random::<f32>() <= (power + 1) as f32 / 50.0 {
            list.retain(|e| out.iter().all(|o| compatible(o.0, e.0)));
            let Some(next) = pick(&list) else { break };
            out.push(next);
            power /= 2;
        }
        out
    }

    /// New options for the item in the enchanting table (sent to the player).
    pub(crate) fn refresh_enchant_offers(&mut self, sid: SessionId) {
        let Some(p) = self.players.get(&sid) else { return };
        let Some(c) = p.inventory.container.as_ref().filter(|c| c.ui == ENCHANT_UI) else { return };
        let (world, table) = (c.world, c.pos);
        let input = p.inventory.enchant_input.clone();
        let offers = match self.enchant_kind(&input).filter(|_| !input.is_empty() && !has_any_enchant(&input)) {
            None => Vec::new(),
            Some((kind, enchantability)) => {
                let shelves = self.count_bookshelves(world, table);
                let base = rand_range(1, 8) + (shelves >> 1) + rand_range(0, shelves);
                let costs = [(base / 3).max(1), base * 2 / 3 + 1, base.max(shelves * 2)];
                costs
                    .iter()
                    .map(|&cost| EnchantOffer {
                        net_id: NEXT_OFFER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                        cost,
                        enchants: self.roll_enchants(kind, enchantability, cost),
                    })
                    .filter(|o| !o.enchants.is_empty())
                    .collect()
            }
        };
        let pk = enchant_options_packet(&offers);
        let p = self.players.get_mut(&sid).unwrap();
        p.inventory.enchant_offers = offers;
        p.out.push(pk);
    }

    /// An enchantment was bought: pay the levels and offer new options.
    pub(crate) fn finish_enchanting(&mut self, sid: SessionId) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let Some(levels) = p.inventory.enchant_paid.take() else { return };
        if p.gamemode != 1 {
            self.add_xp_levels(sid, -(levels as i64));
        }
        if let Some(p) = self.players.get(&sid) {
            let (world, pos) = (p.world, p.pos);
            self.broadcast(&protocol::level_sound_event("random.enchant", pos, -1), |_, q| q.world == world);
        }
        self.refresh_enchant_offers(sid);
    }
}

fn has_any_enchant(item: &ItemDesc) -> bool {
    (0..=40).any(|id| blocks::enchant_level(item, id) > 0)
}

fn enchant_options_packet(offers: &[EnchantOffer]) -> Vec<u8> {
    protocol::packet(PLAYER_ENCHANT_OPTIONS, |w| {
        w.uvarint(offers.len() as u32);
        for o in offers {
            w.u8(o.cost.min(255) as u8);
            w.u32_le(0); // slot flags
            w.uvarint(0); // equip-activated enchantments
            w.uvarint(o.enchants.len() as u32); // held-activated (shown in the list)
            for (id, lvl) in &o.enchants {
                w.uvarint(*id as u32);
                w.u8(*lvl);
            }
            w.uvarint(0); // self-activated
            w.string("");
            w.varint(o.net_id);
        }
    })
}

/// The item an offer makes: the input with the enchantments (a book becomes an enchanted book).
pub(crate) fn enchanted(input: &ItemDesc, offer: &EnchantOffer, gd: &GameData) -> ItemDesc {
    let mut out = input.clone();
    out.count = 1;
    out.stack_id = 0;
    if gd.items_by_id.get(&input.id).is_some_and(|i| i.name == "minecraft:book") {
        if let Some(book) = gd.items_by_name.get("minecraft:enchanted_book") {
            out.id = book.id;
            out.meta = 0;
            out.block_runtime_id = 0;
            out.extra = ceres_world::extra_data(None);
        }
    }
    for (id, lvl) in &offer.enchants {
        durability::set_enchant(&mut out, *id as i16, *lvl as i16);
    }
    out
}
