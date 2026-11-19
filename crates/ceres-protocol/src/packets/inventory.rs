//! Items, inventory windows, equipment, item stack requests and "use item" transactions.

use crate::packet;
use ceres_codec::binary::{R, Res, W};

pub const INVENTORY_TRANSACTION: u32 = 0x1e;
pub const MOB_EQUIPMENT: u32 = 0x1f;
pub const MOB_ARMOR_EQUIPMENT: u32 = 0x20;
pub const INTERACT: u32 = 0x21;
pub const BLOCK_PICK_REQUEST: u32 = 0x22;
pub const CONTAINER_OPEN: u32 = 0x2e;
pub const CONTAINER_CLOSE: u32 = 0x2f;
pub const PLAYER_HOTBAR: u32 = 0x30;
pub const INVENTORY_CONTENT: u32 = 0x31;
pub const INVENTORY_SLOT: u32 = 0x32;
pub const ITEM_STACK_REQUEST: u32 = 0x93;
pub const ITEM_STACK_RESPONSE: u32 = 0x94;

pub const INTERACT_OPEN_INVENTORY: u8 = 6;

pub mod window {
    pub const INVENTORY: u8 = 0;
    pub const OFFHAND: u8 = 119;
    pub const ARMOR: u8 = 120;
}

pub mod use_item_action {
    pub const CLICK_BLOCK: i32 = 0;
    pub const CLICK_AIR: i32 = 1;
    pub const BREAK_BLOCK: i32 = 2;
}

pub mod entity_use_action {
    pub const INTERACT: u32 = 0;
    pub const ATTACK: u32 = 1;
    pub const ITEM_INTERACT: u32 = 2;
}

pub mod block_action {
    pub const START_BREAK: i32 = 0;
    pub const ABORT_BREAK: i32 = 1;
    pub const STOP_BREAK: i32 = 2;
    pub const CREATIVE_PLAYER_DESTROY_BLOCK: i32 = 13;
    pub const PREDICT_DESTROY_BLOCK: i32 = 26;
    pub const CONTINUE_DESTROY_BLOCK: i32 = 27;
}

/// PlayerAuthInputFlags bit indices (protocol 2225).
pub mod input_flag {
    pub const SNEAKING: i32 = 8;
    pub const START_SPRINTING: i32 = 25;
    pub const STOP_SPRINTING: i32 = 26;
    pub const START_SNEAKING: i32 = 27;
    pub const STOP_SNEAKING: i32 = 28;
    pub const START_SWIMMING: i32 = 29;
    pub const STOP_SWIMMING: i32 = 30;
    pub const START_JUMPING: i32 = 31;
    pub const START_GLIDING: i32 = 32;
    pub const STOP_GLIDING: i32 = 33;
    pub const MISSED_SWING: i32 = 39;
    pub const START_CRAWLING: i32 = 40;
    pub const STOP_CRAWLING: i32 = 41;
    pub const START_FLYING: i32 = 42;
    pub const STOP_FLYING: i32 = 43;
}

/// CommonTypes::getNetworkItemStackDescriptor / putNetworkItemStackDescriptor
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemDesc {
    pub id: i16,
    pub count: u16,
    pub meta: u32,
    pub stack_id: i32,
    /// hashed block network id when the item places a block, 0 otherwise
    pub block_runtime_id: i32,
    pub extra: Vec<u8>,
}

impl ItemDesc {
    pub fn is_empty(&self) -> bool {
        self.id == 0 || self.count == 0
    }

    pub fn same_type(&self, other: &ItemDesc) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.id == other.id
            && self.meta == other.meta
            && self.block_runtime_id == other.block_runtime_id
            && self.extra == other.extra
    }
}

pub fn read_item(r: &mut R) -> Res<ItemDesc> {
    let id = r.i16_le()?;
    let count = r.u16_le()?;
    let meta = r.uvarint()?;
    let stack_id = if r.bool()? { r.varint()? } else { 0 };
    let block_runtime_id = r.uvarint()? as i32;
    let n = r.uvarint()? as usize;
    let extra = r.take(n)?.to_vec();
    Ok(ItemDesc { id, count, meta, stack_id, block_runtime_id, extra })
}

pub fn write_item(w: &mut W, it: &ItemDesc) {
    if it.is_empty() {
        w.bytes(&[0; 8]);
        return;
    }
    w.i16_le(it.id);
    w.u16_le(it.count);
    w.uvarint(it.meta);
    let has_net_id = it.stack_id != 0;
    w.bool(has_net_id);
    if has_net_id {
        w.varint(it.stack_id);
    }
    w.uvarint(it.block_runtime_id as u32);
    if it.extra.is_empty() {
        // Empty ItemStackExtraData: no NBT (0u16), 0 canPlaceOn, 0 canDestroy
        w.uvarint(10);
        w.u16_le(0);
        w.u32_le(0);
        w.u32_le(0);
    } else {
        w.uvarint(it.extra.len() as u32);
        w.bytes(&it.extra);
    }
}

/// Sends a full inventory window (0 = main 36, 119 = offhand 1, 120 = armor 4).
pub fn inventory_content(window_id: u32, items: &[ItemDesc]) -> Vec<u8> {
    packet(INVENTORY_CONTENT, |w| {
        w.uvarint(window_id);
        w.uvarint(items.len() as u32);
        for it in items {
            write_item(w, it);
        }
        w.u8(0); // container name: id 0
        w.bool(false); // no dynamic id
        w.bytes(&[0; 8]); // storage item: air
    })
}

/// Updates a single slot in an inventory window.
pub fn inventory_slot(window_id: u32, slot: u32, item: &ItemDesc) -> Vec<u8> {
    packet(INVENTORY_SLOT, |w| {
        w.uvarint(window_id);
        w.uvarint(slot);
        w.bool(false); // optional FullContainerName: none
        w.bool(false); // optional storage item: none
        write_item(w, item);
    })
}

/// Selects a hotbar slot on the client.
pub fn player_hotbar(slot: u32, window_id: u8, select: bool) -> Vec<u8> {
    packet(PLAYER_HOTBAR, |w| {
        w.uvarint(slot);
        w.u8(window_id);
        w.bool(select);
    })
}

pub fn mob_equipment(entity_id: i64, item: &ItemDesc, inv_slot: u8, hotbar_slot: u8, window_id: u8) -> Vec<u8> {
    packet(MOB_EQUIPMENT, |w| {
        w.uvarlong(entity_id as u64);
        write_item(w, item);
        w.u8(inv_slot);
        w.u8(hotbar_slot);
        w.u8(window_id);
    })
}

pub fn read_mob_equipment(r: &mut R) -> Res<(ItemDesc, u8, u8, u8)> {
    let _eid = r.uvarlong()?;
    let item = read_item(r)?;
    let inv_slot = r.u8()?;
    let hotbar_slot = r.u8()?;
    let window_id = r.u8()?;
    Ok((item, inv_slot, hotbar_slot, window_id))
}

/// Armor equipment visible to other players (head, chest, legs, feet, body).
pub fn mob_armor_equipment(entity_id: i64, armor: &[ItemDesc; 4], body: &ItemDesc) -> Vec<u8> {
    packet(MOB_ARMOR_EQUIPMENT, |w| {
        w.uvarlong(entity_id as u64);
        for piece in armor {
            write_item(w, piece);
        }
        write_item(w, body);
    })
}

pub fn read_block_pick_request(r: &mut R) -> Res<([i32; 3], bool, u8)> {
    let pos = read_block_pos(r)?;
    let add_data = r.bool()?;
    let hotbar_slot = r.u8()?;
    Ok((pos, add_data, hotbar_slot))
}

#[derive(Debug, Clone)]
pub struct UseItem {
    pub action: i32,
    pub block_pos: [i32; 3],
    pub face: u8,
    pub hotbar_slot: i32,
    pub item: ItemDesc,
    pub player_pos: [f32; 3],
    pub click_pos: [f32; 3],
    pub block_runtime_id: i32,
}

#[derive(Debug, Clone)]
pub enum InvTransaction {
    UseItem(UseItem),
    Drop { slot: u8, count: u16, expected: ItemDesc },
    UseOnEntity { target_eid: i64, action: u32, hotbar_slot: i32 },
    /// let go of the use button (bow, eating)
    Release,
    Mismatch,
}

pub(crate) fn read_block_pos(r: &mut R) -> Res<[i32; 3]> {
    Ok([r.varint()?, r.varint()?, r.varint()?])
}

pub(crate) fn write_block_pos(w: &mut W, pos: [i32; 3]) {
    w.varint(pos[0]);
    w.varint(pos[1]);
    w.varint(pos[2]);
}

fn read_vec3(r: &mut R) -> Res<[f32; 3]> {
    Ok([r.f32_le()?, r.f32_le()?, r.f32_le()?])
}

struct NetInvAction {
    source_type: u32,
    window_id: Option<u8>,
    slot: u32,
    old_item: ItemDesc,
    new_item: ItemDesc,
}

fn read_net_actions(r: &mut R) -> Res<Vec<NetInvAction>> {
    let n = r.uvarint()?;
    if n > 100 {
        return Err("too many inventory actions".into());
    }
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let source_type = r.uvarint()?;
        let window_id = if r.bool()? { Some(r.u8()?) } else { None };
        if r.bool()? {
            r.uvarint()?; // source flags
        }
        let slot = r.uvarint()?;
        let old_item = read_item(r)?;
        let new_item = read_item(r)?;
        out.push(NetInvAction { source_type, window_id, slot, old_item, new_item });
    }
    Ok(out)
}

/// TransactionData::decode() for UseItemTransactionData: inventory actions, then the use data.
fn read_use_item_body(r: &mut R) -> Res<UseItem> {
    let _ = read_net_actions(r)?;
    let action = r.varint()?;
    r.u8()?; // trigger type
    let block_pos = read_block_pos(r)?;
    let face = r.u8()?;
    let hotbar_slot = r.varint()?;
    r.u8()?; // hand slot
    let item = read_item(r)?;
    let player_pos = read_vec3(r)?;
    let click_pos = read_vec3(r)?;
    let block_runtime_id = r.uvarint()? as i32;
    r.u8()?; // client interact prediction
    r.u8()?; // client cooldown state
    Ok(UseItem { action, block_pos, face, hotbar_slot, item, player_pos, click_pos, block_runtime_id })
}

fn skip_changed_slots(r: &mut R) -> Res<()> {
    for _ in 0..r.uvarint()? {
        r.u8()?; // container id
        let n = r.uvarint()? as usize;
        r.take(n)?;
    }
    Ok(())
}

/// InventoryTransactionPacket: handles UseItem, Q-key drops, entity attacks/interactions and resyncs.
pub fn read_inventory_transaction(r: &mut R) -> Res<Option<InvTransaction>> {
    r.varint()?; // legacy request id
    if r.bool()? {
        skip_changed_slots(r)?;
    }
    match r.uvarint()? {
        0 => {
            // NormalTransactionData: used when pressing Q to drop from hotbar
            let actions = read_net_actions(r)?;
            if actions.len() != 2 {
                return Ok(None);
            }
            let mut src_slot: Option<(u8, ItemDesc)> = None;
            let mut dropped: Option<u16> = None;
            for a in actions {
                if a.source_type == 2 && a.slot == 0 && a.new_item.count > 0 {
                    dropped = Some(a.new_item.count);
                } else if a.source_type == 0 && a.window_id == Some(window::INVENTORY) && a.slot < 36 {
                    src_slot = Some((a.slot as u8, a.old_item));
                }
            }
            match (src_slot, dropped) {
                (Some((slot, expected)), Some(count)) => Ok(Some(InvTransaction::Drop { slot, count, expected })),
                _ => Ok(None),
            }
        }
        1 => Ok(Some(InvTransaction::Mismatch)),
        2 => Ok(Some(InvTransaction::UseItem(read_use_item_body(r)?))),
        3 => {
            let _ = read_net_actions(r)?;
            let target_eid = r.uvarlong()? as i64;
            let action = r.uvarint()?;
            let hotbar_slot = r.varint()?;
            r.u8()?; // hand slot
            let _item = read_item(r)?;
            let _player_pos = read_vec3(r)?;
            let _click_pos = read_vec3(r)?;
            Ok(Some(InvTransaction::UseOnEntity { target_eid, action, hotbar_slot }))
        }
        4 => Ok(Some(InvTransaction::Release)),
        _ => Ok(None),
    }
}

/// What PlayerAuthInput carries after the head yaw that the server acts on.
#[derive(Debug, Default)]
pub struct AuthInputTail {
    pub flags: Vec<i32>,
    pub use_item: Option<UseItem>,
    pub stack_request: Option<StackRequest>,
    /// (action, block position, face)
    pub block_actions: Vec<(i32, [i32; 3], i32)>,
}

impl AuthInputTail {
    pub fn has_flag(&self, f: i32) -> bool {
        self.flags.contains(&f)
    }
}

/// Reads PlayerAuthInput from the field right after the head yaw.
pub fn read_auth_input_tail(r: &mut R) -> Res<AuthInputTail> {
    let mut tail = AuthInputTail::default();
    let flag_count = r.uvarint()?.min(128);
    for _ in 0..flag_count {
        tail.flags.push(r.varint()?);
    }
    r.uvarint()?; // input mode
    r.uvarint()?; // play mode
    r.varint()?; // interaction mode
    r.f32_le()?; // interact rotation
    r.f32_le()?;
    r.uvarlong()?; // tick
    read_vec3(r)?; // delta
    if r.bool()? {
        let request_id = r.varint()?;
        if r.bool()? && request_id < -1 && request_id & 1 == 0 {
            skip_changed_slots(r)?;
        }
        tail.use_item = Some(read_use_item_body(r)?);
    }
    if r.bool()? {
        let req = read_single_stack_request(r)?;
        let unsupported = req.actions.iter().any(|a| matches!(a, StackAction::Unsupported(_)));
        tail.stack_request = Some(req);
        if unsupported {
            return Ok(tail);
        }
    }
    if r.bool()? {
        let n = r.uvarint()?;
        if n > 100 {
            return Err("too many block actions".into());
        }
        for _ in 0..n {
            let action = r.varint()?;
            let pos = read_block_pos(r)?;
            let face = r.varint()?;
            tail.block_actions.push((action, pos, face));
        }
    }
    Ok(tail)
}

/// ContainerUIIds used by item stack requests.
pub mod container {
    pub const ARMOR: u8 = 6;
    pub const COMBINED_HOTBAR_AND_INVENTORY: u8 = 12;
    pub const HOTBAR: u8 = 28;
    pub const INVENTORY: u8 = 29;
    pub const CRAFTING_INPUT: u8 = 13;
    pub const OFFHAND: u8 = 34;
    pub const CURSOR: u8 = 59;
    pub const CREATED_OUTPUT: u8 = 60;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotRef {
    pub container: u8,
    pub slot: u8,
    pub stack_id: i32,
}

#[derive(Debug, Clone)]
pub enum StackAction {
    /// take and place are the same move: `count` items from `src` to `dst`
    Move {
        count: u8,
        src: SlotRef,
        dst: SlotRef,
    },
    Swap {
        a: SlotRef,
        b: SlotRef,
    },
    /// drop into the world as an ItemEntity
    Drop {
        count: u8,
        src: SlotRef,
    },
    /// destroy (creative trash / crafting consume)
    Remove {
        count: u8,
        src: SlotRef,
    },
    CreativeCreate {
        item: u32,
        repetitions: u8,
    },
    /// crafting: an ingredient used up by the recipe of this request
    Consume {
        count: u8,
        src: SlotRef,
    },
    /// crafting: `reps` times the recipe with this network id; `auto` = recipe book (ingredients taken
    /// straight from the inventory)
    CraftRecipe {
        net_id: i32,
        reps: u8,
        auto: bool,
    },
    /// grindstone: the result is decided by the server
    Grindstone,
    /// anvil: the result is decided by the server; `filter` indexes the request's texts (the new name)
    Anvil { filter: i32 },
    /// informational actions the server does not need to act on (crafting results, mining...)
    Ignored(u32),
    /// any action this server does not understand yet (crafting...)
    Unsupported(u32),
}

#[derive(Debug, Clone)]
pub struct StackRequest {
    pub id: i32,
    pub actions: Vec<StackAction>,
    /// texts sent with the request (anvil rename, book pages)
    pub filters: Vec<String>,
}

/// RecipeIngredient (descriptor + count), skipped.
fn skip_ingredient(r: &mut R) -> Res<()> {
    let ty = r.uvarint()?;
    r.u8()?; // type again
    match ty {
        1 => {
            r.string()?;
            r.varint()?;
        }
        2 => {
            r.string()?;
            r.u8()?;
        }
        3 | 5 => {
            r.string()?;
        }
        _ => {}
    }
    r.i16_le()?; // count
    Ok(())
}

fn read_slot(r: &mut R) -> Res<SlotRef> {
    let container = r.u8()?;
    if r.bool()? {
        r.u32_le()?; // dynamic container id
    }
    Ok(SlotRef { container, slot: r.u8()?, stack_id: r.i32_le()? })
}

fn read_single_stack_request(r: &mut R) -> Res<StackRequest> {
    let id = r.varint()?;
    let mut actions = Vec::new();
    let count = r.uvarint()?.min(64);
    for _ in 0..count {
        let ty = r.uvarint()?;
        r.u8()?; // legacy type id
        actions.push(match ty {
            0 | 1 => StackAction::Move { count: r.u8()?, src: read_slot(r)?, dst: read_slot(r)? },
            2 => StackAction::Swap { a: read_slot(r)?, b: read_slot(r)? },
            3 => {
                let (count, src) = (r.u8()?, read_slot(r)?);
                r.bool()?; // randomly
                StackAction::Drop { count, src }
            }
            4 => StackAction::Remove { count: r.u8()?, src: read_slot(r)? },
            5 => StackAction::Consume { count: r.u8()?, src: read_slot(r)? },
            10 => StackAction::CraftRecipe { net_id: r.varint()?, reps: r.u8()?, auto: false },
            11 => {
                let (net_id, reps) = (r.varint()?, r.u8()?);
                for _ in 0..r.uvarint()?.min(64) {
                    skip_ingredient(r)?;
                }
                StackAction::CraftRecipe { net_id, reps, auto: true }
            }
            6 => {
                r.u8()?; // result slot
                StackAction::Ignored(6)
            }
            9 => {
                r.varint()?; // hotbar slot
                r.varint()?; // predicted durability
                r.i32_le()?; // stack id
                StackAction::Ignored(9)
            }
            12 => StackAction::CreativeCreate { item: r.uvarint()?, repetitions: r.u8()? },
            13 => {
                r.varint()?; // recipe id
                StackAction::Anvil { filter: r.i32_le()? }
            }
            14 => {
                r.varint()?; // recipe id
                r.varint()?; // repair cost
                r.u8()?; // repetitions
                StackAction::Grindstone
            }
            17 => StackAction::Ignored(17),
            18 => {
                // deprecated "crafting results": the items the previous action produced
                for _ in 0..r.uvarint()?.min(64) {
                    skip_ingredient(r)?;
                    r.uvarint()?; // block runtime id
                    r.string()?; // extra data
                }
                r.u8()?; // iterations
                StackAction::Ignored(18)
            }
            other => {
                actions.push(StackAction::Unsupported(other));
                return Ok(StackRequest { id, actions, filters: Vec::new() });
            }
        });
    }
    let mut filters = Vec::new();
    for _ in 0..r.uvarint()?.min(64) {
        filters.push(r.string()?);
    }
    r.i32_le()?; // filter string cause
    Ok(StackRequest { id, actions, filters })
}

/// ItemStackRequestPacket. Stops at the first action type it cannot decode.
pub fn read_item_stack_requests(r: &mut R) -> Res<Vec<StackRequest>> {
    let mut out = Vec::new();
    let n = r.uvarint()?.min(64);
    for _ in 0..n {
        let req = read_single_stack_request(r)?;
        let stop = req.actions.iter().any(|a| matches!(a, StackAction::Unsupported(_)));
        out.push(req);
        if stop {
            break;
        }
    }
    Ok(out)
}

/// Final state of one slot after a request.
pub struct SlotResult {
    pub container: u8,
    pub slot: u8,
    pub count: u8,
    pub stack_id: i32,
}

/// ItemStackResponse for one request: ok with the changed slots, or an error.
pub fn item_stack_response(request_id: i32, ok: bool, slots: &[SlotResult]) -> Vec<u8> {
    packet(ITEM_STACK_RESPONSE, |w| {
        w.uvarint(1);
        w.u8(if ok { 0 } else { 1 });
        w.varint(request_id);
        if !ok {
            w.bool(false); // no container info (always written, even for errors)
            return;
        }
        let mut containers: Vec<u8> = slots.iter().map(|s| s.container).collect();
        containers.sort_unstable();
        containers.dedup();
        w.bool(!containers.is_empty());
        if containers.is_empty() {
            return;
        }
        w.uvarint(containers.len() as u32);
        for c in containers {
            w.u8(c);
            w.bool(false); // no dynamic id
            let in_c: Vec<&SlotResult> = slots.iter().filter(|s| s.container == c).collect();
            w.uvarint(in_c.len() as u32);
            for s in in_c {
                w.u8(s.slot);
                w.u8(s.slot); // hotbar slot
                w.u8(s.count);
                w.bool(true);
                w.varint(s.stack_id);
                w.string(""); // custom name
                w.bool(true);
                w.string(""); // filtered custom name
                w.varint(0); // durability correction
            }
        }
    })
}

/// Opens the player's own inventory (window 0, type INVENTORY = -1).
pub fn open_own_inventory(entity_id: i64) -> Vec<u8> {
    packet(CONTAINER_OPEN, |w| {
        w.u8(0);
        w.u8(0xff); // WindowTypes::INVENTORY (-1)
        w.varint(0);
        w.varint(0);
        w.varint(0);
        w.varlong(entity_id);
    })
}

/// Confirms that the client closed a window; without it the window cannot be opened again.
pub fn container_close(window_id: u8, window_type: u8, by_server: bool) -> Vec<u8> {
    packet(CONTAINER_CLOSE, |w| {
        w.u8(window_id);
        w.u8(window_type);
        w.bool(by_server);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_descriptor_roundtrip() {
        let empty = ItemDesc::default();
        let mut w = W::new();
        write_item(&mut w, &empty);
        assert_eq!(w.0, [0u8; 8]);
        let decoded_empty = read_item(&mut R::new(&w.0)).unwrap();
        assert!(decoded_empty.is_empty());

        let diamond = ItemDesc {
            id: 335,
            count: 16,
            meta: 0,
            stack_id: 7,
            block_runtime_id: 0,
            extra: vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        };
        let mut w2 = W::new();
        write_item(&mut w2, &diamond);
        let decoded = read_item(&mut R::new(&w2.0)).unwrap();
        assert_eq!(decoded, diamond);
    }
}

/// Container UI ids used by item stack requests for block containers.
pub mod block_container {
    pub const CHEST: u8 = 7; // LEVEL_ENTITY: chests and trapped chests
    pub const SHULKER_BOX: u8 = 30;
    pub const BARREL: u8 = 58;
}

pub const BLOCK_EVENT: u32 = 0x1a;
/// WindowTypes::CONTAINER
pub const WINDOW_TYPE_CONTAINER: u8 = 0;
pub const WINDOW_TYPE_WORKBENCH: u8 = 1;
pub const WINDOW_TYPE_FURNACE: u8 = 2;
pub const WINDOW_TYPE_ENCHANTMENT: u8 = 3;
pub const WINDOW_TYPE_BREWING_STAND: u8 = 4;
pub const WINDOW_TYPE_ANVIL: u8 = 5;
pub const WINDOW_TYPE_HOPPER: u8 = 8;
pub const WINDOW_TYPE_STONECUTTER: u8 = 29;
pub const WINDOW_TYPE_CARTOGRAPHY: u8 = 30;
pub const WINDOW_TYPE_GRINDSTONE: u8 = 26;
pub const WINDOW_TYPE_BLAST_FURNACE: u8 = 27;
pub const WINDOW_TYPE_SMOKER: u8 = 28;
pub const CONTAINER_SET_DATA: u32 = 0x33;

/// ContainerSetData: a window property (furnace progress and fuel, brewing time...).
pub fn container_set_data(window: u8, property: i32, value: i32) -> Vec<u8> {
    crate::packet(CONTAINER_SET_DATA, |w| {
        w.u8(window);
        w.varint(property);
        w.varint(value);
    })
}

/// Opens a block container window (chest, barrel, shulker box) at `pos`.
pub fn open_block_container(window_id: u8, window_type: u8, pos: [i32; 3]) -> Vec<u8> {
    crate::packet(CONTAINER_OPEN, |w| {
        w.u8(window_id);
        w.u8(window_type);
        w.varint(pos[0]);
        w.varint(pos[1]);
        w.varint(pos[2]);
        w.varlong(-1); // no entity
    })
}

/// BlockEvent: type 1 on chests opens (data 1) or closes (data 0) the lid.
pub fn block_event(pos: [i32; 3], event_type: i32, data: i32) -> Vec<u8> {
    crate::packet(BLOCK_EVENT, |w| {
        w.varint(pos[0]);
        w.varint(pos[1]);
        w.varint(pos[2]);
        w.varint(event_type);
        w.varint(data);
    })
}
