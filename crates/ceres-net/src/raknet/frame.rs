//! Wire encoding: encapsulated frames, addresses and ACK records.

use super::*;

pub(crate) struct Frame {
    pub(crate) reliability: u8,
    pub(crate) message_index: u32,
    pub(crate) order_index: u32,
    pub(crate) order_channel: u8,
    pub(crate) split: Option<(u32, u16, u32)>, // count, id, index
    pub(crate) body: Vec<u8>,
}

pub(crate) fn read_frame(r: &mut R) -> Result<Frame, String> {
    let flags = r.u8()?;
    let reliability = (flags & 0xe0) >> 5;
    let has_split = flags & 0x10 != 0;
    let len = (r.u16_be()? as usize).div_ceil(8);
    if len == 0 {
        return Err("empty frame".into());
    }
    let mut f =
        Frame { reliability, message_index: 0, order_index: 0, order_channel: 0, split: None, body: Vec::new() };
    if is_reliable(reliability) {
        f.message_index = r.triad_le()?;
    }
    if is_sequenced(reliability) {
        r.triad_le()?;
    }
    if is_sequenced_or_ordered(reliability) {
        f.order_index = r.triad_le()?;
        f.order_channel = r.u8()?;
    }
    if has_split {
        let count = r.u32_be()?;
        let id = r.u16_be()?;
        let index = r.u32_be()?;
        f.split = Some((count, id, index));
    }
    f.body = r.take(len)?.to_vec();
    Ok(f)
}

pub(crate) fn write_frame(
    w: &mut W,
    reliability: u8,
    message_index: u32,
    order: Option<(u32, u8)>,
    split: Option<(u32, u16, u32)>,
    body: &[u8],
) {
    w.u8((reliability << 5) | if split.is_some() { 0x10 } else { 0 });
    w.u16_be((body.len() << 3) as u16);
    if is_reliable(reliability) {
        w.triad_le(message_index);
    }
    if let Some((idx, ch)) = order {
        w.triad_le(idx);
        w.u8(ch);
    }
    if let Some((count, id, index)) = split {
        w.u32_be(count);
        w.u16_be(id);
        w.u32_be(index);
    }
    w.bytes(body);
}

pub(crate) fn write_address(w: &mut W, addr: &SocketAddr) {
    match addr.ip() {
        IpAddr::V4(ip) => {
            w.u8(4);
            for b in ip.octets() {
                w.u8(!b);
            }
            w.u16_be(addr.port());
        }
        IpAddr::V6(ip) => {
            w.u8(6);
            w.u16_le(23); // AF_INET6
            w.u16_be(addr.port());
            w.u32_be(0);
            w.bytes(&ip.octets());
            w.u32_be(0);
        }
    }
}

pub(crate) fn skip_address(r: &mut R) -> Result<(), String> {
    match r.u8()? {
        4 => {
            r.take(6)?;
        }
        6 => {
            r.take(28)?;
        }
        v => return Err(format!("bad address version {v}")),
    }
    Ok(())
}

pub(crate) fn encode_ack(header: u8, seqs: &mut Vec<u32>) -> Vec<u8> {
    seqs.sort_unstable();
    seqs.dedup();
    let mut records = W::new();
    let mut count: u16 = 0;
    let mut i = 0;
    while i < seqs.len() {
        let start = seqs[i];
        let mut end = start;
        while i + 1 < seqs.len() && seqs[i + 1] == end + 1 {
            i += 1;
            end = seqs[i];
        }
        if start == end {
            records.u8(1);
            records.triad_le(start);
        } else {
            records.u8(0);
            records.triad_le(start);
            records.triad_le(end);
        }
        count += 1;
        i += 1;
    }
    seqs.clear();
    let mut w = W::with_capacity(records.len() + 3);
    w.u8(header);
    w.u16_be(count);
    w.bytes(&records.0);
    w.0
}

pub(crate) fn decode_ack(r: &mut R) -> Result<Vec<u32>, String> {
    let count = r.u16_be()?;
    let mut out = Vec::new();
    for _ in 0..count {
        if r.eof() {
            break;
        }
        if r.u8()? == 0 {
            let start = r.triad_le()?;
            let end = r.triad_le()?.min(start + 512);
            out.extend(start..=end);
        } else {
            out.push(r.triad_le()?);
        }
        if out.len() > 4096 {
            break;
        }
    }
    Ok(out)
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
