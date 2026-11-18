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
