//! Reassembles segmented NetherNet messages (1 leading byte = segments still to come).

use super::*;

#[derive(Default)]
pub(crate) struct Assembler {
    pub(crate) pending: usize,
    pub(crate) buffer: Vec<u8>,
}

impl Assembler {
    pub(crate) fn push(&mut self, data: &[u8]) -> Result<Option<Vec<u8>>, String> {
        if data.len() < 2 {
            return Err("message without a payload".into());
        }
        let remaining = data[0] as usize;
        if self.pending > 0 && self.pending - 1 != remaining {
            return Err(format!("expected segment counter {}, got {remaining}", self.pending - 1));
        }
        if self.buffer.len() + data.len() - 1 > MAX_PACKET_SIZE {
            return Err("assembled packet is too large".into());
        }
        self.pending = remaining;
        self.buffer.extend_from_slice(&data[1..]);
        if remaining > 0 {
            return Ok(None);
        }
        Ok(Some(std::mem::take(&mut self.buffer)))
    }
}
