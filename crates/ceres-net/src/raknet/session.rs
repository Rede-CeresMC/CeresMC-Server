//! One connected peer: reliability, ordering, split reassembly and resend.

use super::*;

pub(crate) struct SplitAssembly {
    pub(crate) parts: Vec<Option<Vec<u8>>>,
    pub(crate) received: u32,
}

pub(crate) struct InFlight {
    pub(crate) sent_at: Instant,
    pub(crate) frames: Vec<Vec<u8>>,
}

pub(crate) struct Session {
    pub(crate) id: SessionId,
    pub(crate) addr: SocketAddr,
    pub(crate) mtu: usize,
    pub(crate) connected: bool,
    pub(crate) last_recv: Instant,
    // receive side
    pub(crate) ack_queue: Vec<u32>,
    pub(crate) nack_queue: Vec<u32>,
    pub(crate) highest_seq: Option<u32>,
    pub(crate) reliable_seen: HashSet<u32>,
    pub(crate) reliable_highest: u32,
    pub(crate) order_expected: [u32; 32],
    pub(crate) order_buffer: Vec<HashMap<u32, Vec<u8>>>,
    pub(crate) splits: HashMap<u16, SplitAssembly>,
    // send side
    pub(crate) send_seq: u32,
    pub(crate) message_index: u32,
    pub(crate) order_index: u32,
    pub(crate) split_id: u16,
    pub(crate) out_frames: VecDeque<Vec<u8>>,
    pub(crate) in_flight: HashMap<u32, InFlight>,
}

impl Session {
    pub(crate) fn new(id: SessionId, addr: SocketAddr, mtu: usize) -> Self {
        Session {
            id,
            addr,
            mtu,
            connected: false,
            last_recv: Instant::now(),
            ack_queue: Vec::new(),
            nack_queue: Vec::new(),
            highest_seq: None,
            reliable_seen: HashSet::new(),
            reliable_highest: 0,
            order_expected: [0; 32],
            order_buffer: (0..32).map(|_| HashMap::new()).collect(),
            splits: HashMap::new(),
            send_seq: 0,
            message_index: 0,
            order_index: 0,
            split_id: 0,
            out_frames: VecDeque::new(),
            in_flight: HashMap::new(),
        }
    }

    pub(crate) fn max_frame_body(&self) -> usize {
        // datagram header (4) + frame header for reliable ordered (10)
        self.mtu - UDP_OVERHEAD - 4 - 10
    }

    pub(crate) fn queue(&mut self, body: &[u8], reliability: u8) {
        if body.len() <= self.max_frame_body() {
            let mut w = W::with_capacity(body.len() + 12);
            let mi = self.message_index;
            if is_reliable(reliability) {
                self.message_index += 1;
            }
            let order = if reliability == RELIABLE_ORDERED {
                let o = self.order_index;
                self.order_index += 1;
                Some((o, 0))
            } else {
                None
            };
            write_frame(&mut w, reliability, mi, order, None, body);
            self.out_frames.push_back(w.0);
            return;
        }
        // fragment: every part is reliable ordered and shares one order index
        let part_size = self.mtu - UDP_OVERHEAD - 4 - 20;
        let count = body.len().div_ceil(part_size) as u32;
        let split_id = self.split_id;
        self.split_id = self.split_id.wrapping_add(1);
        let order = self.order_index;
        self.order_index += 1;
        for (i, chunk) in body.chunks(part_size).enumerate() {
            let mut w = W::with_capacity(chunk.len() + 24);
            let mi = self.message_index;
            self.message_index += 1;
            write_frame(&mut w, RELIABLE_ORDERED, mi, Some((order, 0)), Some((count, split_id, i as u32)), chunk);
            self.out_frames.push_back(w.0);
        }
    }

    pub(crate) fn flush(&mut self, sock: &UdpSocket) {
        let now = Instant::now();
        // ACKs / NACKs
        if !self.ack_queue.is_empty() {
            let pkt = encode_ack(0xc0, &mut self.ack_queue);
            let _ = sock.send_to(&pkt, self.addr);
        }
        if !self.nack_queue.is_empty() {
            let pkt = encode_ack(0xa0, &mut self.nack_queue);
            let _ = sock.send_to(&pkt, self.addr);
        }
        // resend stale datagrams
        let stale: Vec<u32> = self
            .in_flight
            .iter()
            .filter(|(_, f)| now.duration_since(f.sent_at) > RESEND_AFTER)
            .map(|(s, _)| *s)
            .collect();
        for seq in stale {
            if let Some(f) = self.in_flight.remove(&seq) {
                for fr in f.frames.into_iter().rev() {
                    self.out_frames.push_front(fr);
                }
            }
        }
        // pack frames into datagrams
        let max = self.mtu - UDP_OVERHEAD;
        while !self.out_frames.is_empty() {
            let mut w = W::with_capacity(max);
            w.u8(0x84);
            w.triad_le(self.send_seq);
            let mut frames = Vec::new();
            while let Some(f) = self.out_frames.front() {
                if !frames.is_empty() && w.len() + f.len() > max {
                    break;
                }
                let f = self.out_frames.pop_front().unwrap();
                w.bytes(&f);
                frames.push(f);
            }
            let _ = sock.send_to(&w.0, self.addr);
            self.in_flight.insert(self.send_seq, InFlight { sent_at: now, frames });
            self.send_seq = (self.send_seq + 1) & 0xff_ffff;
        }
    }

    /// Handles one incoming datagram; fully reassembled, in-order bodies are pushed to `deliver`.
    pub(crate) fn handle_datagram(&mut self, buf: &[u8], deliver: &mut Vec<Vec<u8>>) -> Result<(), String> {
        let flags = buf[0];
        let mut r = R::new(&buf[1..]);
        if flags & 0x40 != 0 {
            for seq in decode_ack(&mut r)? {
                self.in_flight.remove(&seq);
            }
            return Ok(());
        }
        if flags & 0x20 != 0 {
            for seq in decode_ack(&mut r)? {
                if let Some(f) = self.in_flight.remove(&seq) {
                    for fr in f.frames.into_iter().rev() {
                        self.out_frames.push_front(fr);
                    }
                }
            }
            return Ok(());
        }
        let seq = r.triad_le()?;
        self.ack_queue.push(seq);
        match self.highest_seq {
            Some(h) if seq > h + 1 && seq - h < 512 => {
                for missing in h + 1..seq {
                    self.nack_queue.push(missing);
                }
                self.highest_seq = Some(seq);
            }
            Some(h) if seq <= h => {
                self.nack_queue.retain(|s| *s != seq);
            }
            _ => self.highest_seq = Some(seq),
        }
        while !r.eof() {
            let f = read_frame(&mut r)?;
            self.handle_frame(f, deliver)?;
        }
        Ok(())
    }

    pub(crate) fn handle_frame(&mut self, mut f: Frame, deliver: &mut Vec<Vec<u8>>) -> Result<(), String> {
        if is_reliable(f.reliability) {
            if !self.reliable_seen.insert(f.message_index) {
                return Ok(()); // duplicate
            }
            self.reliable_highest = self.reliable_highest.max(f.message_index);
            if self.reliable_seen.len() > 8192 {
                let floor = self.reliable_highest.saturating_sub(4096);
                self.reliable_seen.retain(|m| *m >= floor);
            }
        }
        if let Some((count, id, index)) = f.split {
            if count == 0 || count > MAX_SPLIT_PARTS || index >= count {
                return Err(format!("bad split {index}/{count}"));
            }
            if !self.splits.contains_key(&id) && self.splits.len() >= MAX_CONCURRENT_SPLITS {
                return Err("too many concurrent splits".into());
            }
            let a = self
                .splits
                .entry(id)
                .or_insert_with(|| SplitAssembly { parts: vec![None; count as usize], received: 0 });
            if a.parts.len() != count as usize {
                return Err("split count changed".into());
            }
            if a.parts[index as usize].is_none() {
                a.parts[index as usize] = Some(std::mem::take(&mut f.body));
                a.received += 1;
            }
            if a.received < count {
                return Ok(());
            }
            let a = self.splits.remove(&id).unwrap();
            f.body = a.parts.into_iter().flatten().flatten().collect();
        }
        if matches!(f.reliability, 3 | 7) {
            let ch = (f.order_channel as usize).min(31);
            let expected = self.order_expected[ch];
            if f.order_index == expected {
                deliver.push(f.body);
                let mut next = expected + 1;
                while let Some(b) = self.order_buffer[ch].remove(&next) {
                    deliver.push(b);
                    next += 1;
                }
                self.order_expected[ch] = next;
            } else if f.order_index > expected {
                if self.order_buffer[ch].len() < 4096 {
                    self.order_buffer[ch].insert(f.order_index, f.body);
                }
            }
        } else {
            deliver.push(f.body);
        }
        Ok(())
    }
}
