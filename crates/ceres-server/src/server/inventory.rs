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
}

/// UI id given to an open crafting table: it has no slots of its own (the grid is `craft_grid`).
pub(crate) const WORKBENCH_UI: u8 = 0xff;
const CRAFT_GRID_FIRST: u8 = 28;

#[derive(Clone)]
pub(crate) struct OpenContainer {
    /// container UI id the client uses for its slots in item stack requests
    pub(crate) ui: u8,
    pub(crate) window: u8,
    pub(crate) world: usize,
    pub(crate) pos: [i32; 3],
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
}

fn locate(s: &SlotRef, open: Option<&OpenContainer>) -> Option<Loc> {
    if let Some(c) = open {
        if s.container == c.ui {
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
        container::CRAFTING_INPUT => {
            // the 3x3 grid only exists while a crafting table is open
            let last = if open.is_some_and(|c| c.ui == WORKBENCH_UI) { 40 } else { 31 };
            (CRAFT_GRID_FIRST..=last).contains(&s.slot).then_some(Loc::Craft((s.slot - CRAFT_GRID_FIRST) as usize))
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
        );
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
        let mut consumed: Vec<ItemDesc> = Vec::new();

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
                StackAction::CraftRecipe { net_id, reps, .. } => {
                    let r = gd.recipes.get(&net_id)?;
                    let workbench = self.container.as_ref().is_some_and(|c| c.ui == WORKBENCH_UI);
                    if recipe.is_some() || reps == 0 || r.block != "crafting_table" || (r.big && !workbench) {
                        return None;
                    }
                    let mut out = r.output.clone();
                    out.count = out.count.checked_mul(reps as u16)?;
                    out.stack_id = 0;
                    self.crafted = Some(out);
                    recipe = Some((r, reps as u16));
                }
                StackAction::Consume { count, src } => {
                    if recipe.is_none() || count == 0 {
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
                    s.count -= count as u16;
                    if s.count > 0 {
                        s.stack_id = self.alloc_id();
                    }
                    self.set_loc(loc, s);
                    touched.push(src);
                }
                StackAction::Move { count, src, dst } => {
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

        let craft_ok = match recipe {
            Some((r, reps)) => ingredients_match(r, reps, consumed, gd),
            None => consumed.is_empty(),
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

    /// Empties the crafting grid; returns its items.
    pub(crate) fn take_craft_grid(&mut self) -> Vec<ItemDesc> {
        self.craft_grid.iter_mut().filter(|i| !i.is_empty()).map(std::mem::take).collect()
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
        if !self.container_still_valid(sid) {
            self.close_container(sid, true);
        }
        let Some(p) = self.players.get_mut(&sid) else { return };
        let creative = p.gamemode == 1;
        let result = p.inventory.apply(&req, creative, &self.gd);
        if result.is_none() && ceres_net::debug_enabled() {
            println!("[Debug] {sid}: refused item stack request {:?}", req.actions);
        }
        let (ok, slots, dropped) = match result {
            Some((s, d)) => (true, s, d),
            None => (false, Vec::new(), Vec::new()),
        };
        p.out.push(protocol::item_stack_response(req.id, ok, &slots));
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
