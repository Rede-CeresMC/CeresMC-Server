//! Server-side inventory: enough bookkeeping to answer item stack requests the way the client
//! expects (final count and a server stack id for every changed slot).
//! TODO: real items (ids, NBT), max stack sizes, survival rules, saving.

use super::*;
use ceres_protocol::{SlotRef, SlotResult, StackAction, StackRequest, container};
use std::collections::HashMap as Map;

#[derive(Clone, Copy, Default)]
struct Stack {
    /// creative item net id the stack came from (0 = unknown)
    item: u32,
    count: u8,
    stack_id: i32,
}

#[derive(Default)]
pub(crate) struct Inventory {
    slots: Map<(u8, u8), Stack>,
    /// item made by the last CreativeCreate, taken from CREATED_OUTPUT
    created: u32,
    next_stack_id: i32,
}

/// Hotbar, main inventory and their combined view address the same 36 slots.
fn key(s: &SlotRef) -> (u8, u8) {
    match s.container {
        container::COMBINED_HOTBAR_AND_INVENTORY | container::HOTBAR | container::INVENTORY => {
            (container::COMBINED_HOTBAR_AND_INVENTORY, s.slot)
        }
        c => (c, s.slot),
    }
}

impl Inventory {
    fn new_id(&mut self) -> i32 {
        self.next_stack_id += 1;
        self.next_stack_id
    }

    fn get(&self, s: &SlotRef) -> Stack {
        self.slots.get(&key(s)).copied().unwrap_or_default()
    }

    fn put(&mut self, s: &SlotRef, mut st: Stack) {
        if st.count == 0 {
            self.slots.remove(&key(s));
        } else {
            st.stack_id = self.new_id();
            self.slots.insert(key(s), st);
        }
    }

    /// Applies a request. Returns the final state of every slot it touched, or None if the
    /// request does not make sense (the client then rolls its prediction back).
    pub(crate) fn apply(&mut self, req: &StackRequest, creative: bool) -> Option<Vec<SlotResult>> {
        let mut touched: Vec<SlotRef> = Vec::new();
        for action in &req.actions {
            match *action {
                StackAction::CreativeCreate { item, .. } => {
                    if !creative {
                        return None;
                    }
                    self.created = item;
                }
                StackAction::Move { count, src, dst } => {
                    let from_creative = src.container == container::CREATED_OUTPUT;
                    let source = if from_creative {
                        Stack { item: self.created, count: 64, stack_id: 0 }
                    } else {
                        self.get(&src)
                    };
                    if count == 0 || source.count < count {
                        return None;
                    }
                    let mut target = self.get(&dst);
                    if target.count > 0 && target.item != source.item {
                        return None;
                    }
                    target.item = source.item;
                    target.count = target.count.saturating_add(count);
                    if !from_creative {
                        self.put(&src, Stack { count: source.count - count, ..source });
                        touched.push(src);
                    }
                    self.put(&dst, target);
                    touched.push(dst);
                }
                StackAction::Swap { a, b } => {
                    let (sa, sb) = (self.get(&a), self.get(&b));
                    self.put(&a, sb);
                    self.put(&b, sa);
                    touched.push(a);
                    touched.push(b);
                }
                StackAction::Remove { count, src } => {
                    if src.container == container::CREATED_OUTPUT {
                        continue;
                    }
                    let s = self.get(&src);
                    self.put(&src, Stack { count: s.count.saturating_sub(count), ..s });
                    touched.push(src);
                }
                StackAction::Ignored(_) => {}
                StackAction::Unsupported(_) => return None,
            }
        }
        let mut out: Vec<SlotResult> = Vec::new();
        for s in touched {
            if out.iter().any(|o| o.container == s.container && o.slot == s.slot) {
                continue;
            }
            let st = self.get(&s);
            out.push(SlotResult { container: s.container, slot: s.slot, count: st.count, stack_id: st.stack_id });
        }
        Some(out)
    }
}

impl Server {
    pub(crate) fn on_item_stack_request(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let requests = protocol::read_item_stack_requests(r)?;
        let creative = self.cfg.gamemode == 1;
        let Some(p) = self.players.get_mut(&sid) else { return Ok(()) };
        for req in requests {
            let result = p.inventory.apply(&req, creative);
            if result.is_none() && ceres_net::debug_enabled() {
                println!("[Debug] {sid}: refused item stack request {:?}", req.actions);
            }
            let slots = result.as_deref().unwrap_or(&[]);
            p.out.push(protocol::item_stack_response(req.id, result.is_some(), slots));
        }
        Ok(())
    }
}
