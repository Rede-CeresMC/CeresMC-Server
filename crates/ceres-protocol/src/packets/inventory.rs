//! Items and "use item" transactions (breaking, placing, clicking), as sent inside
//! InventoryTransaction and PlayerAuthInput.

use ceres_codec::binary::{R, Res};

pub const INVENTORY_TRANSACTION: u32 = 0x1e;

pub mod use_item_action {
    pub const CLICK_BLOCK: i32 = 0;
    pub const CLICK_AIR: i32 = 1;
    pub const BREAK_BLOCK: i32 = 2;
}

pub mod block_action {
    pub const START_BREAK: i32 = 0;
    pub const ABORT_BREAK: i32 = 1;
    pub const STOP_BREAK: i32 = 2;
    pub const CREATIVE_PLAYER_DESTROY_BLOCK: i32 = 13;
    pub const PREDICT_DESTROY_BLOCK: i32 = 26;
    pub const CONTINUE_DESTROY_BLOCK: i32 = 27;
}

/// CommonTypes::getNetworkItemStackDescriptor
#[derive(Debug, Clone, Default)]
pub struct ItemDesc {
    pub id: i16,
    pub count: u16,
    pub meta: u32,
    pub stack_id: i32,
    /// hashed block network id when the item places a block, 0 otherwise
    pub block_runtime_id: i32,
    pub extra: Vec<u8>,
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

fn read_block_pos(r: &mut R) -> Res<[i32; 3]> {
    Ok([r.varint()?, r.varint()?, r.varint()?])
}

fn read_vec3(r: &mut R) -> Res<[f32; 3]> {
    Ok([r.f32_le()?, r.f32_le()?, r.f32_le()?])
}

/// TransactionData::decode() for UseItemTransactionData: inventory actions, then the use data.
fn read_use_item_body(r: &mut R) -> Res<UseItem> {
    let actions = r.uvarint()?;
    if actions > 100 {
        return Err("too many inventory actions".into());
    }
    for _ in 0..actions {
        r.uvarint()?; // source type
        if r.bool()? {
            r.u8()?; // window id
        }
        if r.bool()? {
            r.uvarint()?; // source flags
        }
        r.uvarint()?; // slot
        read_item(r)?; // old item
        read_item(r)?; // new item
    }
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

/// InventoryTransactionPacket: returns the use-item data, or None for other transaction types.
pub fn read_inventory_transaction(r: &mut R) -> Res<Option<UseItem>> {
    r.varint()?; // legacy request id
    if r.bool()? {
        skip_changed_slots(r)?;
    }
    match r.uvarint()? {
        2 => Ok(Some(read_use_item_body(r)?)),
        _ => Ok(None),
    }
}

/// What PlayerAuthInput carries after the head yaw that the server acts on.
#[derive(Debug, Default)]
pub struct AuthInputTail {
    pub use_item: Option<UseItem>,
    /// (action, block position, face)
    pub block_actions: Vec<(i32, [i32; 3], i32)>,
}

/// Reads PlayerAuthInput from the field right after the head yaw.
/// An item stack request in the same packet is not decoded yet; block actions after it are lost.
pub fn read_auth_input_tail(r: &mut R) -> Res<AuthInputTail> {
    let mut tail = AuthInputTail::default();
    for _ in 0..r.uvarint()? {
        r.varint()?; // input flags
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
        return Ok(tail); // item stack request: not supported yet
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

pub const ITEM_STACK_REQUEST: u32 = 0x93;
pub const ITEM_STACK_RESPONSE: u32 = 0x94;

/// ContainerUIIds used by item stack requests.
pub mod container {
    pub const ARMOR: u8 = 6;
    pub const COMBINED_HOTBAR_AND_INVENTORY: u8 = 12;
    pub const HOTBAR: u8 = 28;
    pub const INVENTORY: u8 = 29;
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
    /// drop into the world or destroy (creative trash)
    Remove {
        count: u8,
        src: SlotRef,
    },
    CreativeCreate {
        item: u32,
        repetitions: u8,
    },
    /// informational actions the server does not need to act on (crafting results, mining...)
    Ignored(u32),
    /// any action this server does not understand yet (crafting...)
    Unsupported(u32),
}

#[derive(Debug, Clone)]
pub struct StackRequest {
    pub id: i32,
    pub actions: Vec<StackAction>,
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
            r.i16_le()?;
        }
        3 => {
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

/// ItemStackRequestPacket. Stops at the first action type it cannot decode (its request is
/// returned with an Unsupported action and nothing after it is read).
pub fn read_item_stack_requests(r: &mut R) -> Res<Vec<StackRequest>> {
    let mut out = Vec::new();
    let n = r.uvarint()?.min(64);
    'requests: for _ in 0..n {
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
                    StackAction::Remove { count, src }
                }
                4 => StackAction::Remove { count: r.u8()?, src: read_slot(r)? },
                5 => StackAction::Remove { count: r.u8()?, src: read_slot(r)? }, // consume crafting input
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
                    out.push(StackRequest { id, actions });
                    break 'requests;
                }
            });
        }
        for _ in 0..r.uvarint()?.min(64) {
            r.string()?; // filter strings
        }
        r.i32_le()?; // filter string cause
        out.push(StackRequest { id, actions });
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
    crate::packet(ITEM_STACK_RESPONSE, |w| {
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

pub const INTERACT: u32 = 0x21;
pub const CONTAINER_OPEN: u32 = 0x2e;
pub const CONTAINER_CLOSE: u32 = 0x2f;
pub const INTERACT_OPEN_INVENTORY: u8 = 6;

/// Opens the player's own inventory (window 0, type INVENTORY = -1).
pub fn open_own_inventory(entity_id: i64) -> Vec<u8> {
    crate::packet(CONTAINER_OPEN, |w| {
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
    crate::packet(CONTAINER_CLOSE, |w| {
        w.u8(window_id);
        w.u8(window_type);
        w.bool(by_server);
    })
}
