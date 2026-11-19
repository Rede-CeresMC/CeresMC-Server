//! Real server-side inventory: main (36), armor (4), offhand (1) and cursor (1), with max stack
//! sizes, survival rules, equipment broadcasting, middle-click pick block and NBT persistence.

use super::*;
use ceres_codec::nbt::{self, Flavor, Tag};
use ceres_protocol::{ItemDesc, SlotRef, SlotResult, StackAction, StackRequest, container, window};
use ceres_world::extra_data;

pub(crate) struct Inventory {
    pub(crate) main: [ItemDesc; 36],
    pub(crate) armor: [ItemDesc; 4],
    pub(crate) offhand: ItemDesc,
    pub(crate) cursor: ItemDesc,
    pub(crate) held_slot: u8,
    created: Option<ItemDesc>,
    next_stack_id: i32,
    pub(crate) dirty: bool,
    /// block container (chest, barrel...) the player has open
    pub(crate) container: Option<OpenContainer>,
    /// crafting grid, slots 28..=40 of the crafting input container (28..=31 is the 2x2 grid)
    pub(crate) craft_grid: [ItemDesc; 13],
    /// output of the recipe crafted in the current request that has not been taken yet
    crafted: Option<ItemDesc>,
    /// input slot of an open stonecutter
    pub(crate) cutter: ItemDesc,
    /// enchanting table slots and the options shown for the item in it
    pub(crate) enchant_input: ItemDesc,
    pub(crate) enchant_lapis: ItemDesc,
    pub(crate) enchant_offers: Vec<super::enchanting::EnchantOffer>,
    /// levels to take after an enchantment was bought in the last request
    pub(crate) enchant_paid: Option<u32>,
    /// the player's level, for enchanting costs
    pub(crate) player_level: u32,
    /// grindstone slots, and the experience its last use gives back
    pub(crate) grind_input: ItemDesc,
    pub(crate) grind_additional: ItemDesc,
    pub(crate) grind_xp: Option<u32>,
    /// anvil slots, and the levels its last use costs
    pub(crate) anvil_input: ItemDesc,
    pub(crate) anvil_material: ItemDesc,
    pub(crate) anvil_paid: Option<u32>,
}

/// UI id given to an open crafting table: it has no slots of its own (the grid is `craft_grid`).
pub(crate) const WORKBENCH_UI: u8 = 0xff;
/// UI id given to an open stonecutter (its input slot is `cutter`).
pub(crate) const STONECUTTER_UI: u8 = 0xfe;
const STONECUTTER_INPUT: u8 = 53;
const STONECUTTER_INPUT_SLOT: u8 = 3;
const CRAFT_GRID_FIRST: u8 = 28;

#[derive(Clone)]
pub(crate) struct OpenContainer {
    /// container UI id the client uses for its slots in item stack requests
    pub(crate) ui: u8,
    pub(crate) window: u8,
    pub(crate) world: usize,
    /// position of the container (the first half of a double chest)
    pub(crate) pos: [i32; 3],
    /// second half of a double chest
    pub(crate) pair: Option<[i32; 3]>,
    pub(crate) items: Vec<ItemDesc>,
    /// changed since it was last written back to the block entity
    pub(crate) changed: bool,
}

impl Default for Inventory {
    fn default() -> Self {
        Inventory {
            main: std::array::from_fn(|_| ItemDesc::default()),
            armor: std::array::from_fn(|_| ItemDesc::default()),
            offhand: ItemDesc::default(),
            cursor: ItemDesc::default(),
            held_slot: 0,
            created: None,
            next_stack_id: 1,
            dirty: false,
            container: None,
            craft_grid: std::array::from_fn(|_| ItemDesc::default()),
            crafted: None,
            cutter: ItemDesc::default(),
            enchant_input: ItemDesc::default(),
            enchant_lapis: ItemDesc::default(),
            enchant_offers: Vec::new(),
            enchant_paid: None,
            player_level: 0,
            grind_input: ItemDesc::default(),
            grind_additional: ItemDesc::default(),
            grind_xp: None,
            anvil_input: ItemDesc::default(),
            anvil_material: ItemDesc::default(),
            anvil_paid: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Loc {
    Main(usize),
    Armor(usize),
    Offhand,
    Cursor,
    Container(usize),
    Craft(usize),
    Cutter,
    EnchantInput,
    EnchantLapis,
    GrindInput,
    GrindAdditional,
    AnvilInput,
    AnvilMaterial,
}

/// Slots of the enchanting table, grindstone and anvil: once one of them was used in a request,
/// nothing may be moved in or out of them any more in that request.
fn is_station(loc: Loc) -> bool {
    matches!(
        loc,
        Loc::EnchantInput | Loc::EnchantLapis | Loc::GrindInput | Loc::GrindAdditional | Loc::AnvilInput | Loc::AnvilMaterial
    )
}

fn locate(s: &SlotRef, open: Option<&OpenContainer>) -> Option<Loc> {
    if let Some(c) = open {
        // furnaces split their three slots over three UI ids (input, fuel, result)
        let furnace_slot = super::furnace::is_furnace_ui(c.ui)
            && matches!(s.container, super::furnace::UI_FUEL | super::furnace::UI_RESULT);
        // brewing stands: reagent (9), bottles (10) and fuel (11) are slots 0, 1-3 and 4 of one inventory
        let brewing_slot = c.ui == super::brewing::UI_BREWING_INPUT
            && matches!(s.container, super::brewing::UI_BREWING_RESULT | super::brewing::UI_BREWING_FUEL);
        if s.container == c.ui || furnace_slot || brewing_slot {
            return ((s.slot as usize) < c.items.len()).then_some(Loc::Container(s.slot as usize));
        }
    }
    match s.container {
        container::COMBINED_HOTBAR_AND_INVENTORY | container::HOTBAR | container::INVENTORY => {
            (s.slot < 36).then_some(Loc::Main(s.slot as usize))
        }
        container::ARMOR => (s.slot < 4).then_some(Loc::Armor(s.slot as usize)),
        container::OFFHAND => (s.slot == 0).then_some(Loc::Offhand),
        container::CURSOR => (s.slot == 0).then_some(Loc::Cursor),
        super::enchanting::UI_ENCHANT_INPUT | super::enchanting::UI_ENCHANT_LAPIS => {
            if !open.is_some_and(|c| c.ui == super::enchanting::ENCHANT_UI) {
                return None;
            }
            match (s.container, s.slot) {
                (super::enchanting::UI_ENCHANT_INPUT, super::enchanting::SLOT_ENCHANT_INPUT) => Some(Loc::EnchantInput),
                (super::enchanting::UI_ENCHANT_LAPIS, super::enchanting::SLOT_ENCHANT_LAPIS) => Some(Loc::EnchantLapis),
                _ => None,
            }
        }
        super::anvil::UI_ANVIL_INPUT | super::anvil::UI_ANVIL_MATERIAL
            if open.is_some_and(|c| c.ui == super::anvil::ANVIL_UI) =>
        {
            match (s.container, s.slot) {
                (super::anvil::UI_ANVIL_INPUT, super::anvil::SLOT_ANVIL_INPUT) => Some(Loc::AnvilInput),
                (super::anvil::UI_ANVIL_MATERIAL, super::anvil::SLOT_ANVIL_MATERIAL) => Some(Loc::AnvilMaterial),
                _ => None,
            }
        }
        super::grindstone::UI_GRIND_INPUT | super::grindstone::UI_GRIND_ADDITIONAL => {
            if !open.is_some_and(|c| c.ui == super::grindstone::GRINDSTONE_UI) {
                return None;
            }
            match (s.container, s.slot) {
                (super::grindstone::UI_GRIND_INPUT, super::grindstone::SLOT_GRIND_INPUT) => Some(Loc::GrindInput),
                (super::grindstone::UI_GRIND_ADDITIONAL, super::grindstone::SLOT_GRIND_ADDITIONAL) => Some(Loc::GrindAdditional),
                _ => None,
            }
        }
        STONECUTTER_INPUT => {
            (open.is_some_and(|c| c.ui == STONECUTTER_UI) && s.slot == STONECUTTER_INPUT_SLOT).then_some(Loc::Cutter)
        }
        container::CRAFTING_INPUT => {
            // the 3x3 grid only exists while a crafting table is open
            let last = if open.is_some_and(|c| c.ui == WORKBENCH_UI) { 40 } else { 31 };
            (CRAFT_GRID_FIRST..=last).contains(&s.slot).then_some(Loc::Craft((s.slot - CRAFT_GRID_FIRST) as usize))
        }
        _ => None,
    }
}

/// The result of a "multi" recipe from what lies in the crafting grid, and the items (id, count)
/// it uses up. Supported: copying a written book (one written book plus book and quills: as many
/// copies, one generation further; copies of copies of copies cannot be made; the original stays).
///
/// Repairing by combining (as PowerNukkitX): two of the same damageable item, one each, give one
/// with both durabilities plus 5 % of the maximum, without enchantments or name.
/// Shield and banner: the shield takes the banner's base colour and patterns.
/// Firework rocket: one paper, 1-3 gunpowder (flight duration) and up to 7 firework stars (its
/// explosions) make 3 rockets.
/// Decorated pot (crafting table): bricks or pottery sherds in a plus shape; the pot keeps the
/// sherds (back, left, right, front: top, left, right and bottom cells).
///
/// Besides the result it returns what must be used up, as (grid cell, count): every item in the
/// grid once, except the written book being copied (it stays) and all the book and quills. The
/// request must take exactly those from exactly those cells.
fn multi_result(uuid: &str, grid: &[ItemDesc], gd: &GameData) -> Option<(ItemDesc, Vec<(usize, u16, ItemDesc)>)> {
    let (out, _) = multi_result_items(uuid, grid, gd)?;
    let name = |it: &ItemDesc| gd.items_by_id.get(&it.id).map(|i| i.name.as_str()).unwrap_or("");
    let book = uuid == "d1ca6b84-338e-4f2f-9c6b-76cc8b4bd98d";
    let banner = uuid == "b5c5d105-75a2-4076-af2b-923ea2bf4bf0";
    // the item being copied stays in the grid
    let kept = |it: &ItemDesc| {
        (book && name(it) == "minecraft:written_book") || (banner && super::durability::item_field(it, "Patterns").and_then(|p| p.as_list().map(|l| !l.is_empty())).unwrap_or(false))
    };
    let cells = grid
        .iter()
        .enumerate()
        .filter(|(_, it)| !it.is_empty())
        .filter(|(_, it)| !kept(it))
        .map(|(i, it)| (i, if book { it.count } else { 1 }, it.clone()))
        .collect();
    Some((out, cells))
}

fn multi_result_items(uuid: &str, grid: &[ItemDesc], gd: &GameData) -> Option<(ItemDesc, Vec<(i16, u16)>)> {
    const BOOK_CLONING: &str = "d1ca6b84-338e-4f2f-9c6b-76cc8b4bd98d";
    const REPAIR: &str = "00000000-0000-0000-0000-000000000001";
    const SHIELD_BANNER: &str = "00000000-0000-0000-0000-0000000000c8";
    const DECORATED_POT: &str = "685a742a-c42e-4a4e-88ea-5eb83fc98e5b";
    const FIREWORK_ROCKET: &str = "00000000-0000-0000-0000-000000000002";
    const BANNER_DUPLICATE: &str = "b5c5d105-75a2-4076-af2b-923ea2bf4bf0";
    const MAP_CLONING: &str = "85939755-ba10-4d9d-a4cc-efb7a8e943c4";
    use super::durability as dur;
    let name = |it: &ItemDesc| gd.items_by_id.get(&it.id).map(|i| i.name.as_str()).unwrap_or("");
    let items: Vec<&ItemDesc> = grid.iter().filter(|i| !i.is_empty()).collect();
    match uuid {
        BOOK_CLONING => {
            let written: Vec<&&ItemDesc> = items.iter().filter(|i| name(i) == "minecraft:written_book").collect();
            let blanks: Vec<&&ItemDesc> = items.iter().filter(|i| name(i) == "minecraft:writable_book").collect();
            if written.len() != 1 || blanks.is_empty() || written.len() + blanks.len() != items.len() {
                return None;
            }
            let generation = match super::durability::item_field(written[0], "generation") {
                Some(Tag::Int(g)) => g,
                _ => 0,
            };
            if generation >= 2 {
                return None;
            }
            let n: u16 = blanks.iter().map(|b| b.count).sum::<u16>();
            if n > 16 {
                return None;
            }
            let mut out = (*written[0]).clone();
            super::durability::set_item_field(&mut out, "generation", Some(Tag::Int(generation + 1)));
            out.count = n;
            out.stack_id = 0;
            Some((out, vec![(blanks[0].id, n)]))
        }
        REPAIR => {
            let [a, b] = items.as_slice() else { return None };
            let max = dur::max_durability(name(a))?;
            if a.id != b.id || a.count != 1 || b.count != 1 || max == 0 {
                return None;
            }
            let left = (max - dur::item_damage(a).min(max)) + (max - dur::item_damage(b).min(max)) + max * 5 / 100;
            // the item without its enchantments, name and repair cost (keeping the shield's extra data)
            let mut out = (*a).clone();
            out.stack_id = 0;
            dur::keep_only_enchantments(&mut out, &[]);
            dur::set_item_field(&mut out, "display", None);
            dur::set_item_field(&mut out, "RepairCost", None);
            dur::set_damage(&mut out, (max + 1).saturating_sub(left));
            Some((out, vec![(a.id, 2)]))
        }
        SHIELD_BANNER => {
            let [a, b] = items.as_slice() else { return None };
            let (shield, banner) = match (name(a), name(b)) {
                ("minecraft:shield", "minecraft:banner") => (a, b),
                ("minecraft:banner", "minecraft:shield") => (b, a),
                _ => return None,
            };
            if shield.count != 1 || dur::item_field(shield, "Base").is_some() {
                return None;
            }
            let mut out = (*shield).clone();
            out.stack_id = 0;
            for key in ["Type", "Patterns"] {
                if let Some(v) = dur::item_field(banner, key) {
                    dur::set_item_field(&mut out, key, Some(v));
                }
            }
            dur::set_item_field(&mut out, "Base", Some(Tag::Int((banner.meta & 0x0f) as i32)));
            Some((out, vec![(shield.id, 1), (banner.id, 1)]))
        }
        MAP_CLONING => {
            // a filled map and empty maps: as many copies plus the original
            let filled: Vec<&&ItemDesc> = items.iter().filter(|i| name(i) == "minecraft:filled_map").collect();
            let empty: u16 = items.iter().filter(|i| name(i) == "minecraft:empty_map").map(|i| i.count).sum();
            let empty_cells = items.iter().filter(|i| name(i) == "minecraft:empty_map").count();
            if filled.len() != 1 || filled[0].count != 1 || empty == 0 || empty as usize != empty_cells || 1 + empty_cells != items.len() {
                return None;
            }
            let mut out = (*filled[0]).clone();
            out.count = empty + 1;
            out.stack_id = 0;
            Some((out, Vec::new()))
        }
        BANNER_DUPLICATE => {
            // a banner with patterns and a blank one of the same colour: the blank one gets the
            // patterns (the ominous banner cannot be copied)
            let [a, b] = items.as_slice() else { return None };
            if name(a) != "minecraft:banner" || name(b) != "minecraft:banner" || a.meta != b.meta {
                return None;
            }
            let pats = |it: &ItemDesc| dur::item_field(it, "Patterns").and_then(|p| p.as_list().map(|l| l.len())).unwrap_or(0);
            let (src, blank) = match (pats(a), pats(b)) {
                (n, 0) if n > 0 => (a, b),
                (0, n) if n > 0 => (b, a),
                _ => return None,
            };
            if blank.count != 1 || matches!(dur::item_field(src, "Type"), Some(Tag::Int(1))) {
                return None;
            }
            let mut out = (*src).clone();
            out.count = 1;
            out.stack_id = 0;
            Some((out, Vec::new()))
        }
        FIREWORK_ROCKET => {
            let mut paper = 0;
            let mut powder = 0u8;
            let mut stars = Vec::new();
            for it in &items {
                match name(it) {
                    "minecraft:paper" => paper += 1,
                    "minecraft:gunpowder" => powder += 1,
                    "minecraft:firework_star" => {
                        stars.push(dur::item_field(it, "FireworksItem").unwrap_or(Tag::Compound(Vec::new())));
                    }
                    _ => return None,
                }
            }
            if paper != 1 || !(1..=3).contains(&powder) || stars.len() > 7 {
                return None;
            }
            let info = gd.find_item("minecraft:firework_rocket")?;
            let mut out = ItemDesc { id: info.id, count: 3, meta: 0, stack_id: 0, block_runtime_id: info.block_runtime_id, extra: extra_data(None) };
            let fw = Tag::Compound(vec![("Explosions".into(), Tag::List(10, stars)), ("Flight".into(), Tag::Byte(powder as i8))]);
            dur::set_item_field(&mut out, "Fireworks", Some(fw));
            Some((out, Vec::new()))
        }
        DECORATED_POT => {
            // the 3x3 grid is craft_grid[4..13]
            let cells = grid.get(4..13)?;
            let sides = [1usize, 3, 5, 7];
            let ok_item = |it: &ItemDesc| name(it) == "minecraft:brick" || name(it).ends_with("_pottery_sherd");
            if cells.iter().enumerate().any(|(i, c)| sides.contains(&i) != !c.is_empty()) || !sides.iter().all(|&i| ok_item(&cells[i])) {
                return None;
            }
            let info = gd.find_item("minecraft:decorated_pot")?;
            let mut out = ItemDesc { id: info.id, count: 1, meta: 0, stack_id: 0, block_runtime_id: info.block_runtime_id, extra: extra_data(None) };
            let sherds = sides.iter().map(|&i| Tag::String(name(&cells[i]).to_string())).collect();
            super::durability::set_item_field(&mut out, "sherds", Some(Tag::List(8, sherds)));
            let mut used: Vec<(i16, u16)> = Vec::new();
            for &i in &sides {
                match used.iter_mut().find(|(id, _)| *id == cells[i].id) {
                    Some(u) => u.1 += 1,
                    None => used.push((cells[i].id, 1)),
                }
            }
            Some((out, used))
        }
        _ => None,
    }
}

/// The used-up items must be exactly the recipe's ingredients, `reps` times.
/// Exact item ingredients are matched before wildcard and tag ones, so a specific ingredient is not
/// taken by a looser one.
fn ingredients_match(r: &ceres_world::Recipe, reps: u16, mut pool: Vec<ItemDesc>, gd: &GameData) -> bool {
    use ceres_world::Ingredient;
    let rank = |i: &Ingredient| match i {
        Ingredient::Name { aux: Some(_), .. } => 0,
        Ingredient::Name { aux: None, .. } => 1,
        Ingredient::Tag(_) => 2,
    };
    let mut wanted: Vec<&(Ingredient, u16)> = r.inputs.iter().collect();
    wanted.sort_by_key(|(i, _)| rank(i));
    for (ing, count) in wanted {
        let mut need = *count as u32 * reps as u32;
        for it in pool.iter_mut().filter(|it| it.count > 0) {
            if need == 0 {
                break;
            }
            if gd.ingredient_matches(ing, it) {
                let take = need.min(it.count as u32);
                it.count -= take as u16;
                need -= take;
            }
        }
        if need > 0 {
            return false;
        }
    }
    pool.iter().all(|it| it.count == 0)
}

impl Inventory {
    pub(crate) fn alloc_id(&mut self) -> i32 {
        let id = self.next_stack_id;
        self.next_stack_id = self.next_stack_id.wrapping_add(1).max(1);
        id
    }

    pub(crate) fn held_item(&self) -> &ItemDesc {
        &self.main[(self.held_slot as usize).min(8)]
    }

    fn slot_ref(&self, loc: Loc) -> &ItemDesc {
        match loc {
            Loc::Main(i) => &self.main[i],
            Loc::Armor(i) => &self.armor[i],
            Loc::Offhand => &self.offhand,
            Loc::Cursor => &self.cursor,
            Loc::Container(i) => &self.container.as_ref().expect("open container").items[i],
            Loc::Craft(i) => &self.craft_grid[i],
            Loc::Cutter => &self.cutter,
            Loc::EnchantInput => &self.enchant_input,
            Loc::EnchantLapis => &self.enchant_lapis,
            Loc::GrindInput => &self.grind_input,
            Loc::GrindAdditional => &self.grind_additional,
            Loc::AnvilInput => &self.anvil_input,
            Loc::AnvilMaterial => &self.anvil_material,
        }
    }

    fn set_loc(&mut self, loc: Loc, mut item: ItemDesc) {
        if item.is_empty() {
            item = ItemDesc::default();
        } else if item.stack_id == 0 {
            item.stack_id = self.alloc_id();
        }
        match loc {
            Loc::Main(i) => self.main[i] = item,
            Loc::Armor(i) => self.armor[i] = item,
            Loc::Offhand => self.offhand = item,
            Loc::Cursor => self.cursor = item,
            Loc::Craft(i) => self.craft_grid[i] = item,
            Loc::Cutter => self.cutter = item,
            Loc::EnchantInput => self.enchant_input = item,
            Loc::EnchantLapis => self.enchant_lapis = item,
            Loc::GrindInput => self.grind_input = item,
            Loc::GrindAdditional => self.grind_additional = item,
            Loc::AnvilInput => self.anvil_input = item,
            Loc::AnvilMaterial => self.anvil_material = item,
            Loc::Container(i) => {
                if let Some(c) = self.container.as_mut() {
                    c.items[i] = item;
                    c.changed = true;
                }
                return;
            }
        }
        self.dirty = true;
    }

    pub(crate) fn set_main(&mut self, slot: usize, mut item: ItemDesc) {
        if slot >= 36 {
            return;
        }
        if item.is_empty() {
            item = ItemDesc::default();
        } else {
            item.stack_id = self.alloc_id();
        }
        self.main[slot] = item;
        self.dirty = true;
    }

    pub(crate) fn clear(&mut self) {
        for s in &mut self.main {
            *s = ItemDesc::default();
        }
        for s in &mut self.armor {
            *s = ItemDesc::default();
        }
        self.offhand = ItemDesc::default();
        self.cursor = ItemDesc::default();
        self.dirty = true;
    }

    /// Takes every non-empty item out of main/armor/offhand/cursor (used on survival death).
    pub(crate) fn drain_all(&mut self) -> Vec<ItemDesc> {
        let mut out = Vec::new();
        for s in &mut self.main {
            if !s.is_empty() {
                out.push(std::mem::take(s));
            }
        }
        for s in &mut self.armor {
            if !s.is_empty() {
                out.push(std::mem::take(s));
            }
        }
        if !self.offhand.is_empty() {
            out.push(std::mem::take(&mut self.offhand));
        }
        if !self.cursor.is_empty() {
            out.push(std::mem::take(&mut self.cursor));
        }
        self.dirty = true;
        out
    }

    /// Adds `item` to existing matching stacks first, then empty slots. Returns `(changed_slots, leftover_count)`.
    pub(crate) fn add_item(&mut self, mut item: ItemDesc, gd: &GameData) -> (Vec<u8>, u16) {
        if item.is_empty() {
            return (Vec::new(), 0);
        }
        let max_stack = gd.max_stack(item.id) as u16;
        let mut changed = Vec::new();
        // 1. Merge into existing non-full stacks
        for i in 0..36 {
            if item.count == 0 {
                break;
            }
            if self.main[i].same_type(&item) && self.main[i].count < max_stack {
                let take = (max_stack - self.main[i].count).min(item.count);
                self.main[i].count += take;
                self.main[i].stack_id = self.alloc_id();
                item.count -= take;
                changed.push(i as u8);
                self.dirty = true;
            }
        }
        // 2. Place into empty slots
        for i in 0..36 {
            if item.count == 0 {
                break;
            }
            if self.main[i].is_empty() {
                let take = max_stack.min(item.count);
                let mut placed = item.clone();
                placed.count = take;
                placed.stack_id = self.alloc_id();
                self.main[i] = placed;
                item.count -= take;
                changed.push(i as u8);
                self.dirty = true;
            }
        }
        (changed, item.count)
    }

    /// Deducts `count` from `slot`, returning the updated slot state.
    pub(crate) fn consume_slot(&mut self, slot: u8, count: u16) -> Option<ItemDesc> {
        let idx = slot as usize;
        if idx >= 36 || self.main[idx].is_empty() || self.main[idx].count < count {
            return None;
        }
        self.main[idx].count -= count;
        if self.main[idx].count == 0 {
            self.main[idx] = ItemDesc::default();
        } else {
            self.main[idx].stack_id = self.alloc_id();
        }
        self.dirty = true;
        Some(self.main[idx].clone())
    }

    /// Applies an ItemStackRequest transactionally: rolls back completely if any action is invalid.
    /// Returns `(slot_results, dropped_items)`.
    pub(crate) fn apply(
        &mut self,
        req: &StackRequest,
        creative: bool,
        gd: &GameData,
    ) -> Option<(Vec<SlotResult>, Vec<ItemDesc>)> {
        let backup = (
            self.main.clone(),
            self.armor.clone(),
            self.offhand.clone(),
            self.cursor.clone(),
            self.created.clone(),
            self.next_stack_id,
            self.container.clone(),
            self.craft_grid.clone(),
            self.cutter.clone(),
            self.enchant_input.clone(),
            self.enchant_lapis.clone(),
            self.grind_input.clone(),
            self.grind_additional.clone(),
            self.anvil_input.clone(),
            self.anvil_material.clone(),
        );
        self.anvil_paid = None;
        self.enchant_paid = None;
        self.grind_xp = None;
        let res = self.apply_inner(req, creative, gd);
        self.crafted = None;
        if res.is_none() {
            self.main = backup.0;
            self.armor = backup.1;
            self.offhand = backup.2;
            self.cursor = backup.3;
            self.created = backup.4;
            self.next_stack_id = backup.5;
            self.container = backup.6;
            self.craft_grid = backup.7;
            self.cutter = backup.8;
            self.enchant_input = backup.9;
            self.enchant_lapis = backup.10;
            self.grind_input = backup.11;
            self.grind_additional = backup.12;
            self.anvil_input = backup.13;
            self.anvil_material = backup.14;
            self.anvil_paid = None;
            self.enchant_paid = None;
            self.grind_xp = None;
        }
        res
    }

    fn apply_inner(
        &mut self,
        req: &StackRequest,
        creative: bool,
        gd: &GameData,
    ) -> Option<(Vec<SlotResult>, Vec<ItemDesc>)> {
        let mut touched: Vec<SlotRef> = Vec::new();
        let mut dropped: Vec<ItemDesc> = Vec::new();
        // crafting: the recipe of this request and the ingredients the client says it used up
        let mut recipe: Option<(&ceres_world::Recipe, u16)> = None;
        // a multi recipe: the items (id, count) it must use up
        let mut multi_use: Option<Vec<(usize, u16, ItemDesc)>> = None;
        let mut consumed: Vec<ItemDesc> = Vec::new();
        // where each consumed item came from
        let mut consumed_from: Vec<Loc> = Vec::new();
        // enchanting table: index of the option bought
        let mut enchant_choice: Option<usize> = None;
        // grindstone: (expected input, expected additional) and the experience
        let mut grinding: Option<(ItemDesc, ItemDesc, u32)> = None;
        // anvil: (input, material, material used, cost)
        let mut anvil_use: Option<(ItemDesc, ItemDesc, u16, u32)> = None;

        for action in &req.actions {
            match *action {
                StackAction::CreativeCreate { item, .. } => {
                    if !creative {
                        return None;
                    }
                    let ce = gd.creative_items.get(item as usize)?;
                    self.created = Some(ItemDesc {
                        id: ce.id,
                        count: gd.max_stack(ce.id) as u16,
                        meta: ce.meta,
                        stack_id: 0,
                        block_runtime_id: ce.block_runtime_id,
                        extra: ce.extra.clone(),
                    });
                }
                StackAction::CraftRecipe { net_id, .. }
                    if self.enchant_offers.iter().any(|o| o.net_id == net_id) =>
                {
                    let open_ui = self.container.as_ref().map(|c| c.ui);
                    let i = self.enchant_offers.iter().position(|o| o.net_id == net_id)?;
                    let offer = self.enchant_offers[i].clone();
                    if open_ui != Some(super::enchanting::ENCHANT_UI) || recipe.is_some() || enchant_choice.is_some() {
                        return None;
                    }
                    // levels and lapis (creative players need neither)
                    let lapis_ok = self.enchant_lapis.count as usize > i
                        && gd.items_by_id.get(&self.enchant_lapis.id).is_some_and(|it| it.name == "minecraft:lapis_lazuli");
                    if !creative && (self.player_level < offer.cost || self.player_level < i as u32 + 1 || !lapis_ok) {
                        return None;
                    }
                    // the option is for exactly the item it was rolled for
                    let snap = &offer.input;
                    let same = !self.enchant_input.is_empty()
                        && self.enchant_input.id == snap.id
                        && self.enchant_input.meta == snap.meta
                        && self.enchant_input.extra == snap.extra;
                    if !same {
                        return None;
                    }
                    self.crafted = Some(super::enchanting::enchanted(&self.enchant_input, &offer, gd));
                    enchant_choice = Some(i);
                }
                StackAction::Anvil { filter } => {
                    let open_ui = self.container.as_ref().map(|c| c.ui);
                    if open_ui != Some(super::anvil::ANVIL_UI)
                        || recipe.is_some()
                        || enchant_choice.is_some()
                        || grinding.is_some()
                        || anvil_use.is_some()
                    {
                        return None;
                    }
                    let rename = usize::try_from(filter).ok().and_then(|i| req.filters.get(i)).map(String::as_str);
                    let res = super::anvil::combine(&self.anvil_input, &self.anvil_material, rename, gd)?;
                    if !creative && (super::anvil::too_expensive(res.cost) || self.player_level < res.cost) {
                        return None;
                    }
                    self.crafted = Some(res.item);
                    anvil_use = Some((self.anvil_input.clone(), self.anvil_material.clone(), res.material_used, res.cost));
                }
                StackAction::Grindstone => {
                    let open_ui = self.container.as_ref().map(|c| c.ui);
                    if open_ui != Some(super::grindstone::GRINDSTONE_UI) || recipe.is_some() || enchant_choice.is_some() || grinding.is_some() {
                        return None;
                    }
                    let (out, xp) = super::grindstone::grind(&self.grind_input, &self.grind_additional, gd)?;
                    self.crafted = Some(out);
                    grinding = Some((self.grind_input.clone(), self.grind_additional.clone(), xp));
                }
                StackAction::CraftRecipe { net_id, reps, .. } => {
                    if enchant_choice.is_some() || grinding.is_some() {
                        return None;
                    }
                    let r = gd.recipes.get(&net_id)?;
                    let open_ui = self.container.as_ref().map(|c| c.ui);
                    let allowed = match r.block.as_str() {
                        "crafting_table" => !r.big || open_ui == Some(WORKBENCH_UI),
                        "stonecutter" => open_ui == Some(STONECUTTER_UI),
                        _ => false,
                    };
                    if recipe.is_some() || reps == 0 || !allowed {
                        return None;
                    }
                    if let Some(u) = &r.multi {
                        let (out, used) = multi_result(u, &self.craft_grid, gd)?;
                        multi_use = Some(used);
                        self.crafted = Some(out);
                        recipe = Some((r, 1));
                        continue;
                    }
                    let mut out = r.output.clone();
                    out.count = out.count.checked_mul(reps as u16)?;
                    out.stack_id = 0;
                    self.crafted = Some(out);
                    recipe = Some((r, reps as u16));
                }
                StackAction::Consume { count, src } => {
                    if (recipe.is_none() && enchant_choice.is_none() && grinding.is_none() && anvil_use.is_none()) || count == 0 {
                        return None;
                    }
                    let loc = locate(&src, self.container.as_ref())?;
                    let mut s = self.slot_ref(loc).clone();
                    if s.is_empty() || s.count < count as u16 {
                        return None;
                    }
                    let mut used = s.clone();
                    used.count = count as u16;
                    consumed.push(used);
                    consumed_from.push(loc);
                    s.count -= count as u16;
                    if s.count > 0 {
                        s.stack_id = self.alloc_id();
                    }
                    self.set_loc(loc, s);
                    touched.push(src);
                }
                StackAction::Move { count, src, dst } => {
                    let station_used = enchant_choice.is_some() || grinding.is_some() || anvil_use.is_some();
                    if station_used
                        && [locate(&src, self.container.as_ref()), locate(&dst, self.container.as_ref())]
                            .into_iter()
                            .flatten()
                            .any(is_station)
                    {
                        return None;
                    }
                    if count == 0 {
                        return None;
                    }
                    let from_creative = src.container == container::CREATED_OUTPUT;
                    let source = if from_creative {
                        if let Some(c) = self.crafted.as_mut() {
                            // crafted output: only as much as the recipe made
                            if c.count < count as u16 {
                                return None;
                            }
                            c.count -= count as u16;
                            let mut out = c.clone();
                            out.count = count as u16;
                            out
                        } else {
                            if !creative {
                                return None;
                            }
                            let mut c = self.created.clone()?;
                            c.count = 64;
                            c
                        }
                    } else {
                        self.slot_ref(locate(&src, self.container.as_ref())?).clone()
                    };
                    if source.is_empty() || source.count < count as u16 {
                        return None;
                    }
                    // nothing can be put into a furnace's result slot
                    if dst.container == super::furnace::UI_RESULT {
                        return None;
                    }
                    let dst_loc = locate(&dst, self.container.as_ref())?;
                    let max_stack = if matches!(dst_loc, Loc::Armor(_)) { 1 } else { gd.max_stack(source.id) as u16 };
                    let mut target = self.slot_ref(dst_loc).clone();
                    if !target.is_empty() && !target.same_type(&source) {
                        return None;
                    }
                    let new_count = if target.is_empty() { count as u16 } else { target.count + count as u16 };
                    if new_count > max_stack {
                        return None;
                    }
                    if target.is_empty() {
                        target = source.clone();
                    }
                    target.count = new_count;
                    target.stack_id = self.alloc_id();
                    if !from_creative {
                        let src_loc = locate(&src, self.container.as_ref())?;
                        let mut rem = source;
                        rem.count -= count as u16;
                        if rem.count > 0 {
                            rem.stack_id = self.alloc_id();
                        }
                        self.set_loc(src_loc, rem);
                        touched.push(src);
                    }
                    self.set_loc(dst_loc, target);
                    touched.push(dst);
                }
                StackAction::Swap { a, b } => {
                    let station_used = enchant_choice.is_some() || grinding.is_some() || anvil_use.is_some();
                    if station_used
                        && [locate(&a, self.container.as_ref()), locate(&b, self.container.as_ref())]
                            .into_iter()
                            .flatten()
                            .any(is_station)
                    {
                        return None;
                    }
                    if a.container == super::furnace::UI_RESULT || b.container == super::furnace::UI_RESULT {
                        return None;
                    }
                    let (la, lb) = (locate(&a, self.container.as_ref())?, locate(&b, self.container.as_ref())?);
                    let (mut sa, mut sb) = (self.slot_ref(la).clone(), self.slot_ref(lb).clone());
                    if matches!(la, Loc::Armor(_)) && !sb.is_empty() && sb.count > 1 {
                        return None;
                    }
                    if matches!(lb, Loc::Armor(_)) && !sa.is_empty() && sa.count > 1 {
                        return None;
                    }
                    if !sa.is_empty() {
                        sa.stack_id = self.alloc_id();
                    }
                    if !sb.is_empty() {
                        sb.stack_id = self.alloc_id();
                    }
                    self.set_loc(la, sb);
                    self.set_loc(lb, sa);
                    touched.push(a);
                    touched.push(b);
                }
                StackAction::Drop { count, src } => {
                    let station_used = enchant_choice.is_some() || grinding.is_some() || anvil_use.is_some();
                    if station_used && locate(&src, self.container.as_ref()).is_some_and(is_station) {
                        return None;
                    }
                    if count == 0 {
                        return None;
                    }
                    if src.container == container::CREATED_OUTPUT {
                        if let Some(c) = self.crafted.as_mut() {
                            if c.count < count as u16 {
                                return None;
                            }
                            c.count -= count as u16;
                            let mut out = c.clone();
                            out.count = count as u16;
                            dropped.push(out);
                            continue;
                        }
                        if !creative {
                            return None;
                        }
                        if let Some(mut c) = self.created.clone() {
                            c.count = (count as u16).min(gd.max_stack(c.id) as u16);
                            c.stack_id = 0;
                            dropped.push(c);
                        }
                        continue;
                    }
                    let loc = locate(&src, self.container.as_ref())?;
                    let mut s = self.slot_ref(loc).clone();
                    if s.is_empty() || s.count < count as u16 {
                        return None;
                    }
                    let mut drop_it = s.clone();
                    drop_it.count = count as u16;
                    drop_it.stack_id = 0;
                    dropped.push(drop_it);

                    s.count -= count as u16;
                    if s.count > 0 {
                        s.stack_id = self.alloc_id();
                    }
                    self.set_loc(loc, s);
                    touched.push(src);
                }
                StackAction::Remove { count, src } => {
                    let station_used = enchant_choice.is_some() || grinding.is_some() || anvil_use.is_some();
                    if station_used && locate(&src, self.container.as_ref()).is_some_and(is_station) {
                        return None;
                    }
                    if src.container == container::CREATED_OUTPUT {
                        continue;
                    }
                    let loc = locate(&src, self.container.as_ref())?;
                    let mut s = self.slot_ref(loc).clone();
                    s.count = s.count.saturating_sub(count as u16);
                    if s.count > 0 {
                        s.stack_id = self.alloc_id();
                    }
                    self.set_loc(loc, s);
                    touched.push(src);
                }
                StackAction::Ignored(_) => {}
                StackAction::Unsupported(_) => return None,
            }
        }

        // anvil: the whole input and exactly the material it needed, nothing else
        if let Some((input, material, material_used, cost)) = anvil_use {
            let mut from_input = 0;
            let mut from_material = 0;
            for (loc, c) in consumed_from.iter().zip(&consumed) {
                match loc {
                    Loc::AnvilInput if c.same_type(&input) => from_input += c.count,
                    Loc::AnvilMaterial if c.same_type(&material) => from_material += c.count,
                    _ => return None,
                }
            }
            if from_input != input.count || from_material != material_used {
                return None;
            }
            self.anvil_paid = Some(cost);
            consumed.clear();
        }
        // grindstone: both slots used up exactly (one each), nothing else
        if let Some((input, additional, xp)) = grinding {
            if recipe.is_some() || enchant_choice.is_some() {
                return None;
            }
            let mut from_input = 0;
            let mut from_additional = 0;
            for (loc, c) in consumed_from.iter().zip(&consumed) {
                match loc {
                    Loc::GrindInput if c.same_type(&input) => from_input += c.count,
                    Loc::GrindAdditional if c.same_type(&additional) => from_additional += c.count,
                    _ => return None,
                }
            }
            let want = |it: &ItemDesc| if it.is_empty() { 0 } else { 1 };
            if from_input != want(&input) || from_additional != want(&additional) {
                return None;
            }
            self.grind_xp = Some(xp);
            consumed.clear();
        }
        let craft_ok = match (recipe, enchant_choice) {
            // a recipe and an enchantment in one request: refused
            (Some(_), Some(_)) => false,
            (Some(_), None) if multi_use.is_some() => {
                let mut want = multi_use.unwrap_or_default();
                let mut ok = true;
                // only from the grid cells the result was made from
                for (loc, c) in consumed_from.iter().zip(&consumed) {
                    let Loc::Craft(i) = loc else {
                        ok = false;
                        break;
                    };
                    // and the very item the result was made from (not one moved in afterwards)
                    match want.iter_mut().find(|(cell, n, it)| cell == i && *n >= c.count && c.same_type(it) && c.extra == it.extra) {
                        Some(w) => w.1 -= c.count,
                        None => ok = false,
                    }
                }
                ok && want.iter().all(|(_, n, _)| *n == 0)
            }
            (Some((r, reps)), None) => ingredients_match(r, reps, consumed, gd),
            // enchanting: exactly the item, plus 1-3 lapis (none needed in creative)
            (None, Some(i)) => {
                // exactly one item taken from the item slot, lapis only from the lapis slot
                let mut item_used = 0u16;
                let mut lapis_used = 0u16;
                let mut other = false;
                for (loc, c) in consumed_from.iter().zip(&consumed) {
                    match loc {
                        Loc::EnchantInput => item_used += c.count,
                        Loc::EnchantLapis => lapis_used += c.count,
                        _ => other = true,
                    }
                }
                let ok = !other && item_used == 1 && (if creative { lapis_used as usize <= i + 1 } else { lapis_used as usize == i + 1 });
                if ok {
                    self.enchant_paid = Some(i as u32 + 1);
                }
                ok
            }
            (None, None) => consumed.is_empty(),
        };
        if !craft_ok {
            return None;
        }

        let mut out: Vec<SlotResult> = Vec::new();
        for s in touched {
            if out.iter().any(|o| o.container == s.container && o.slot == s.slot) {
                continue;
            }
            let Some(loc) = locate(&s, self.container.as_ref()) else { continue };
            let st = self.slot_ref(loc);
            out.push(SlotResult { container: s.container, slot: s.slot, count: st.count as u8, stack_id: st.stack_id });
        }
        Some((out, dropped))
    }

    /// Empties the crafting grid and the stonecutter slot; returns their items.
    pub(crate) fn take_craft_grid(&mut self) -> Vec<ItemDesc> {
        let mut out: Vec<ItemDesc> = self.craft_grid.iter_mut().filter(|i| !i.is_empty()).map(std::mem::take).collect();
        if !self.cutter.is_empty() {
            out.push(std::mem::take(&mut self.cutter));
        }
        for slot in [
            &mut self.enchant_input,
            &mut self.enchant_lapis,
            &mut self.grind_input,
            &mut self.grind_additional,
            &mut self.anvil_input,
            &mut self.anvil_material,
        ] {
            if !slot.is_empty() {
                out.push(std::mem::take(slot));
            }
        }
        self.enchant_offers.clear();
        out
    }

    pub(crate) fn to_nbt(&self) -> Tag {
        let encode_slot = |slot: usize, it: &ItemDesc| -> Tag {
            Tag::Compound(vec![
                ("Slot".into(), Tag::Byte(slot as i8)),
                ("Id".into(), Tag::Short(it.id)),
                ("Count".into(), Tag::Short(it.count as i16)),
                ("Meta".into(), Tag::Int(it.meta as i32)),
                ("BlockId".into(), Tag::Int(it.block_runtime_id)),
                ("Extra".into(), Tag::ByteArray(it.extra.clone())),
            ])
        };
        let main_tags: Vec<Tag> =
            self.main.iter().enumerate().filter(|(_, it)| !it.is_empty()).map(|(i, it)| encode_slot(i, it)).collect();
        let armor_tags: Vec<Tag> =
            self.armor.iter().enumerate().filter(|(_, it)| !it.is_empty()).map(|(i, it)| encode_slot(i, it)).collect();
        let mut fields = vec![
            ("HeldSlot".into(), Tag::Byte(self.held_slot as i8)),
            ("Inventory".into(), Tag::List(10, main_tags)),
            ("Armor".into(), Tag::List(10, armor_tags)),
        ];
        if !self.offhand.is_empty() {
            fields.push(("Offhand".into(), encode_slot(0, &self.offhand)));
        }
        Tag::Compound(fields)
    }

    pub(crate) fn load_from_nbt(&mut self, tag: &Tag) {
        let decode_item = |inv: &mut Inventory, t: &Tag| -> Option<(usize, ItemDesc)> {
            let slot = t.get("Slot")?.as_i64()? as usize;
            let id = t.get("Id")?.as_i64()? as i16;
            let count = t.get("Count")?.as_i64()?.max(0) as u16;
            if id == 0 || count == 0 {
                return None;
            }
            let meta = t.get("Meta").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
            let block_runtime_id = t.get("BlockId").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let extra = match t.get("Extra") {
                Some(Tag::ByteArray(b)) if !b.is_empty() => b.clone(),
                _ => extra_data(None),
            };
            Some((slot, ItemDesc { id, count, meta, stack_id: inv.alloc_id(), block_runtime_id, extra }))
        };
        if let Some(hs) = tag.get("HeldSlot").and_then(|t| t.as_i64()) {
            self.held_slot = (hs as u8).min(8);
        }
        if let Some(list) = tag.get("Inventory").and_then(|t| t.as_list()) {
            for entry in list {
                if let Some((slot, it)) = decode_item(self, entry) {
                    if slot < 36 {
                        self.main[slot] = it;
                    }
                }
            }
        }
        if let Some(list) = tag.get("Armor").and_then(|t| t.as_list()) {
            for entry in list {
                if let Some((slot, it)) = decode_item(self, entry) {
                    if slot < 4 {
                        self.armor[slot] = it;
                    }
                }
            }
        }
        if let Some(off) = tag.get("Offhand") {
            if let Some((_, it)) = decode_item(self, off) {
                self.offhand = it;
            }
        }
        self.dirty = false;
    }
}

const PLAYERS_DIR: &str = "players";

fn player_file_path(name: &str) -> Option<std::path::PathBuf> {
    // hex of the whole lowercase name: distinct names can never share a file
    // TODO: key on the verified XUID once Xbox login verification exists
    if name.is_empty() || name.len() > 64 {
        return None;
    }
    let hex: String = name.to_lowercase().bytes().map(|b| format!("{b:02x}")).collect();
    Some(std::path::Path::new(PLAYERS_DIR).join(format!("{hex}.nbt")))
}

impl Server {
    pub(crate) fn load_player_state(&mut self, sid: SessionId) {
        let Some(name) = self.players.get(&sid).map(|p| p.name.clone()) else { return };
        let Some(path) = player_file_path(&name) else { return };
        let Ok(raw) = std::fs::read(&path) else { return };
        let Ok((_, tag)) = nbt::read_root(&mut R::new(&raw), Flavor::LittleEndian) else { return };
        let spawn_point = match (
            tag.get("SpawnWorld").and_then(|t| t.as_str()),
            tag.get("SpawnX").and_then(|t| t.as_i64()),
            tag.get("SpawnY").and_then(|t| t.as_i64()),
            tag.get("SpawnZ").and_then(|t| t.as_i64()),
        ) {
            (Some(wf), Some(x), Some(y), Some(z)) => {
                self.worlds.worlds.iter().position(|w| w.folder == wf).map(|wi| (wi, [x as i32, y as i32, z as i32]))
            }
            _ => None,
        };
        if let Some(p) = self.players.get_mut(&sid) {
            p.inventory.load_from_nbt(&tag);
            if let Some(h) = tag.get("Health").and_then(|t| t.as_f32()) {
                p.health = h.clamp(1.0, 20.0);
            }
            if let Some(v) = tag.get("Hunger").and_then(|t| t.as_f32()) {
                p.hunger = v.clamp(0.0, 20.0);
            }
            if let Some(v) = tag.get("Saturation").and_then(|t| t.as_f32()) {
                p.saturation = v.clamp(0.0, p.hunger);
            }
            if let Some(v) = tag.get("Exhaustion").and_then(|t| t.as_f32()) {
                p.exhaustion = v.clamp(0.0, 40.0);
            }
            if let Some(v) = tag.get("XpLevel").and_then(|t| t.as_i64()) {
                p.xp_level = v.clamp(0, 24791) as u32;
            }
            if let Some(v) = tag.get("XpP").and_then(|t| t.as_i64()) {
                p.xp_points = (v.max(0) as u32).min(xp::points_for_level(p.xp_level).saturating_sub(1));
            }
            p.spawn_point = spawn_point;
        }
    }

    pub(crate) fn save_player_state_of(&self, p: &Player) {
        if p.name.is_empty() || p.state != State::Spawned {
            return;
        }
        let Some(path) = player_file_path(&p.name) else { return };
        let _ = std::fs::create_dir_all(PLAYERS_DIR);
        let mut tag = p.inventory.to_nbt();
        tag.set("Health", Tag::Float(p.health));
        tag.set("Hunger", Tag::Float(p.hunger));
        tag.set("Saturation", Tag::Float(p.saturation));
        tag.set("Exhaustion", Tag::Float(p.exhaustion));
        tag.set("XpLevel", Tag::Int(p.xp_level as i32));
        tag.set("XpP", Tag::Int(p.xp_points as i32));
        if let Some((wi, sp)) = p.spawn_point {
            if let Some(w) = self.worlds.worlds.get(wi) {
                tag.set("SpawnWorld", Tag::String(w.folder.clone()));
                tag.set("SpawnX", Tag::Int(sp[0]));
                tag.set("SpawnY", Tag::Int(sp[1]));
                tag.set("SpawnZ", Tag::Int(sp[2]));
            }
        }
        let _ = std::fs::write(path, nbt::to_le(&tag));
    }

    pub(crate) fn save_all_players(&self) {
        for p in self.players.values() {
            if p.inventory.dirty || p.spawn_point.is_some() {
                self.save_player_state_of(p);
            }
        }
    }

    /// Puts what is left in the crafting grid back into the inventory (or drops it).
    pub(crate) fn return_craft_grid(&mut self, sid: SessionId) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        let items = p.inventory.take_craft_grid();
        if items.is_empty() {
            return;
        }
        let mut leftovers = Vec::new();
        for it in items {
            let mut rest = it.clone();
            let (_, left) = p.inventory.add_item(it, &self.gd);
            if left > 0 {
                rest.count = left;
                rest.stack_id = 0;
                leftovers.push(rest);
            }
        }
        self.sync_full_inventory(sid);
        for it in leftovers {
            self.drop_from_player(sid, it);
        }
    }

    /// Sends all inventory windows + selected hotbar slot to `sid`.
    pub(crate) fn sync_full_inventory(&mut self, sid: SessionId) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        p.out.push(protocol::inventory_content(window::INVENTORY as u32, &p.inventory.main));
        p.out.push(protocol::inventory_content(window::OFFHAND as u32, std::slice::from_ref(&p.inventory.offhand)));
        p.out.push(protocol::inventory_content(window::ARMOR as u32, &p.inventory.armor));
        p.out.push(protocol::player_hotbar(p.inventory.held_slot as u32, window::INVENTORY, true));
    }

    /// Broadcasts `sid`'s main-hand, off-hand and armor equipment to everyone else in its world.
    pub(crate) fn broadcast_equipment(&mut self, sid: SessionId) {
        let Some(p) = self.players.get(&sid) else { return };
        if p.state != State::Spawned {
            return;
        }
        let (world, eid, slot) = (p.world, p.entity_id, p.inventory.held_slot);
        let main_pk = protocol::mob_equipment(eid, p.inventory.held_item(), slot, slot, window::INVENTORY);
        let off_pk = protocol::mob_equipment(eid, &p.inventory.offhand, 0, 0, window::OFFHAND);
        let armor_pk = protocol::mob_armor_equipment(eid, &p.inventory.armor, &ItemDesc::default());
        self.broadcast(&main_pk, |o, q| o != sid && q.world == world);
        self.broadcast(&off_pk, |o, q| o != sid && q.world == world);
        self.broadcast(&armor_pk, |o, q| o != sid && q.world == world);
    }

    /// Packets needed for one viewer to see `p`'s armor and offhand (mainhand is already in AddPlayer).
    pub(crate) fn equipment_packets_of(&self, p: &Player) -> [Vec<u8>; 2] {
        [
            protocol::mob_equipment(p.entity_id, &p.inventory.offhand, 0, 0, window::OFFHAND),
            protocol::mob_armor_equipment(p.entity_id, &p.inventory.armor, &ItemDesc::default()),
        ]
    }

    pub(crate) fn handle_single_stack_request(&mut self, sid: SessionId, req: StackRequest) {
        if self.menu_stack_request(sid, &req) {
            return;
        }
        if !self.container_still_valid(sid) {
            self.close_container(sid, true);
        }
        let Some(p) = self.players.get_mut(&sid) else { return };
        let creative = p.gamemode == 1;
        p.inventory.player_level = p.xp_level;
        let enchant_before = p.inventory.enchant_input.clone();
        let result = p.inventory.apply(&req, creative, &self.gd);
        if result.is_none() && ceres_net::debug_enabled() {
            println!("[Debug] {sid}: refused item stack request {:?}", req.actions);
        }
        let (ok, slots, dropped) = match result {
            Some((s, d)) => (true, s, d),
            None => (false, Vec::new(), Vec::new()),
        };
        p.out.push(protocol::item_stack_response(req.id, ok, &slots));
        let paid = p.inventory.enchant_paid.is_some();
        let input_changed = p.inventory.enchant_input != enchant_before;
        let anvil_used = self.players.get(&sid).is_some_and(|p| p.inventory.anvil_paid.is_some());
        if ok && anvil_used {
            self.finish_anvil(sid);
        }
        let ground = self.players.get(&sid).is_some_and(|p| p.inventory.grind_xp.is_some());
        if ok && ground {
            self.finish_grinding(sid);
        }
        if ok && paid {
            self.finish_enchanting(sid);
        } else if ok && input_changed {
            self.refresh_enchant_offers(sid);
        }
        if ok {
            self.broadcast_equipment(sid);
            self.sync_container(sid);
            for item in dropped {
                self.drop_from_player(sid, item);
            }
        }
    }

    pub(crate) fn on_item_stack_request(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let requests = protocol::read_item_stack_requests(r)?;
        for req in requests {
            self.handle_single_stack_request(sid, req);
        }
        Ok(())
    }

    pub(crate) fn on_mob_equipment(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let (_item, _inv_slot, hotbar_slot, win) = protocol::read_mob_equipment(r)?;
        if win != window::INVENTORY || hotbar_slot >= 9 {
            return Ok(());
        }
        let Some(p) = self.players.get_mut(&sid) else { return Ok(()) };
        if p.inventory.held_slot != hotbar_slot {
            p.eating = None;
            p.bow_since = None;
        }
        p.inventory.held_slot = hotbar_slot;
        p.inventory.dirty = true;
        let (world, eid) = (p.world, p.entity_id);
        let pk = protocol::mob_equipment(eid, p.inventory.held_item(), hotbar_slot, hotbar_slot, window::INVENTORY);
        self.broadcast(&pk, |o, q| o != sid && q.world == world);
        Ok(())
    }

    /// Middle-click block pick in creative mode (BlockPickRequestPacket 0x22).
    pub(crate) fn on_block_pick(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let (pos, _add_data, _hotbar) = protocol::read_block_pick_request(r)?;
        let Some(p) = self.players.get(&sid) else { return Ok(()) };
        if p.gamemode != 1 {
            return Ok(());
        }
        let world = p.world;
        let bid = self.worlds.worlds[world].get_block(pos, &self.gd);
        let Some(ce) = self.gd.block_to_item.get(&bid).cloned() else { return Ok(()) };
        let picked = ItemDesc {
            id: ce.id,
            count: 1,
            meta: ce.meta,
            stack_id: 0,
            block_runtime_id: ce.block_runtime_id,
            extra: ce.extra,
        };
        let Some(p) = self.players.get_mut(&sid) else { return Ok(()) };
        // If already in hotbar, just select that slot!
        if let Some(existing) = (0..9usize).find(|&i| p.inventory.main[i].same_type(&picked)) {
            p.inventory.held_slot = existing as u8;
            p.out.push(protocol::player_hotbar(existing as u32, window::INVENTORY, true));
            self.broadcast_equipment(sid);
            return Ok(());
        }
        // Otherwise put into an empty hotbar slot, or overwrite the currently held slot
        let target = (0..9usize).find(|&i| p.inventory.main[i].is_empty()).unwrap_or(p.inventory.held_slot as usize);
        p.inventory.held_slot = target as u8;
        p.inventory.set_main(target, picked);
        let slot_pk = protocol::inventory_slot(window::INVENTORY as u32, target as u32, &p.inventory.main[target]);
        p.out.push(slot_pk);
        p.out.push(protocol::player_hotbar(target as u32, window::INVENTORY, true));
        self.broadcast_equipment(sid);
        Ok(())
    }
}
