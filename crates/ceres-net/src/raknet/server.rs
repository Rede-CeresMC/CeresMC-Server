//! The UDP socket, the offline handshake and the Transport implementation.

use super::*;

pub struct RakNetServer {
    pub(crate) sock: UdpSocket,
    pub(crate) guid: u64,
    pub(crate) port: u16,
    pub(crate) sessions: HashMap<SocketAddr, Session>,
    pub(crate) by_id: HashMap<SessionId, SocketAddr>,
    pub(crate) next_id: SessionId,
    pub(crate) pong: Vec<u8>,
    pub(crate) buf: Vec<u8>,
    pub(crate) start: Instant,
}

impl RakNetServer {
    pub fn bind(addr: SocketAddr, first_session_id: SessionId) -> std::io::Result<Self> {
        let sock = UdpSocket::bind(addr)?;
        sock.set_read_timeout(Some(Duration::from_millis(2)))?;
        Ok(RakNetServer {
            sock,
            guid: rand::random::<u64>(),
            port: addr.port(),
            sessions: HashMap::new(),
            by_id: HashMap::new(),
            next_id: first_session_id,
            pong: Vec::new(),
            buf: vec![0u8; 2048],
            start: Instant::now(),
        })
    }

    pub(crate) fn handle_offline(&mut self, data: &[u8], from: SocketAddr) -> Result<(), String> {
        let mut r = R::new(data);
        let id = r.u8()?;
        match id {
            ID_UNCONNECTED_PING | ID_UNCONNECTED_PING_OPEN => {
                let ping_time = r.u64_be()?;
                let mut w = W::new();
                w.u8(ID_UNCONNECTED_PONG);
                w.u64_be(ping_time);
                w.u64_be(self.guid);
                w.bytes(&MAGIC);
                w.u16_be(self.pong.len() as u16);
                w.bytes(&self.pong);
                let _ = self.sock.send_to(&w.0, from);
            }
            ID_OPEN_CONNECTION_REQUEST_1 => {
                if r.take(16)? != MAGIC {
                    return Err("bad magic".into());
                }
                let protocol = r.u8()?;
                if crate::debug_enabled() {
                    println!(
                        "[RakNet] OpenConnectionRequest1 from {from} (raknet protocol {protocol}, {} bytes)",
                        data.len()
                    );
                }
                if protocol != RAKNET_PROTOCOL {
                    let mut w = W::new();
                    w.u8(ID_INCOMPATIBLE_PROTOCOL);
                    w.u8(RAKNET_PROTOCOL);
                    w.bytes(&MAGIC);
                    w.u64_be(self.guid);
                    let _ = self.sock.send_to(&w.0, from);
                    return Ok(());
                }
                let mtu = (data.len() + UDP_OVERHEAD).min(MAX_MTU);
                let mut w = W::new();
                w.u8(ID_OPEN_CONNECTION_REPLY_1);
                w.bytes(&MAGIC);
                w.u64_be(self.guid);
                w.u8(0);
                w.u16_be(mtu as u16);
                let _ = self.sock.send_to(&w.0, from);
            }
            ID_OPEN_CONNECTION_REQUEST_2 => {
                if r.take(16)? != MAGIC {
                    return Err("bad magic".into());
                }
                skip_address(&mut r)?;
                let mtu = r.u16_be()? as usize;
                if crate::debug_enabled() {
                    println!("[RakNet] OpenConnectionRequest2 from {from} (mtu {mtu})");
                }
                if mtu < MIN_MTU {
                    return Ok(());
                }
                let mtu = mtu.min(MAX_MTU);
                let mut w = W::new();
                w.u8(ID_OPEN_CONNECTION_REPLY_2);
                w.bytes(&MAGIC);
                w.u64_be(self.guid);
                write_address(&mut w, &from);
                w.u16_be(mtu as u16);
                w.u8(0);
                let _ = self.sock.send_to(&w.0, from);
                if let Some(old) = self.sessions.remove(&from) {
                    self.by_id.remove(&old.id);
                }
                let id = self.next_id;
                self.next_id += 1;
                self.sessions.insert(from, Session::new(id, from, mtu));
                self.by_id.insert(id, from);
            }
            other => println!("[RakNet] unknown offline message 0x{other:02x} from {from} ({} bytes)", data.len()),
        }
        Ok(())
    }

    pub(crate) fn handle_connected(
        &mut self,
        body: Vec<u8>,
        from: SocketAddr,
        out: &mut Vec<NetEvent>,
    ) -> Result<(), String> {
        let Some(s) = self.sessions.get_mut(&from) else { return Ok(()) };
        if body.is_empty() {
            return Ok(());
        }
        match body[0] {
            ID_CONNECTED_PING => {
                let mut r = R::new(&body[1..]);
                let t = r.u64_be()?;
                let mut w = W::new();
                w.u8(ID_CONNECTED_PONG);
                w.u64_be(t);
                w.u64_be(self.start.elapsed().as_millis() as u64);
                s.queue(&w.0, UNRELIABLE);
            }
            ID_CONNECTION_REQUEST => {
                if crate::debug_enabled() {
                    println!("[RakNet] ConnectionRequest from {from}");
                }
                let mut r = R::new(&body[1..]);
                let _client_guid = r.u64_be()?;
                let t = r.u64_be()?;
                let mut w = W::new();
                w.u8(ID_CONNECTION_REQUEST_ACCEPTED);
                write_address(&mut w, &from);
                w.u16_be(0);
                let local: SocketAddr = "127.0.0.1:0".parse().unwrap();
                let dummy: SocketAddr = "0.0.0.0:0".parse().unwrap();
                for i in 0..20 {
                    write_address(&mut w, if i == 0 { &local } else { &dummy });
                }
                w.u64_be(t);
                w.u64_be(now_ms());
                s.queue(&w.0, RELIABLE);
            }
            ID_NEW_INCOMING_CONNECTION => {
                if !s.connected {
                    s.connected = true;
                    out.push(NetEvent::Connected(s.id, from, None));
                }
            }
            ID_DISCONNECTION_NOTIFICATION => {
                let id = s.id;
                self.sessions.remove(&from);
                self.by_id.remove(&id);
                out.push(NetEvent::Disconnected(id, "client disconnect".into()));
            }
            ID_GAME => {
                if s.connected {
                    out.push(NetEvent::Packet(s.id, body[1..].to_vec()));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl Transport for RakNetServer {
    fn name(&self) -> &'static str {
        "RakNet"
    }

    fn poll(&mut self, out: &mut Vec<NetEvent>) {
        // drain everything that is ready; the first recv may block for the read timeout
        for _ in 0..512 {
            let (n, from) = match self.sock.recv_from(&mut self.buf) {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                    break;
                }
                Err(_) => continue, // e.g. WSAECONNRESET on Windows after an ICMP unreachable
            };
            if n == 0 {
                continue;
            }
            let data = self.buf[..n].to_vec();
            if let Some(s) = self.sessions.get_mut(&from) {
                if data[0] & 0x80 != 0 {
                    s.last_recv = Instant::now();
                    let mut deliver = Vec::new();
                    if let Err(e) = s.handle_datagram(&data, &mut deliver) {
                        eprintln!("[RakNet] bad datagram from {from}: {e}");
                        continue;
                    }
                    for body in deliver {
                        if let Err(e) = self.handle_connected(body, from, out) {
                            eprintln!("[RakNet] bad packet from {from}: {e}");
                        }
                    }
                    continue;
                }
            }
            if let Err(e) = self.handle_offline(&data, from) {
                eprintln!("[RakNet] bad offline message from {from}: {e}");
            }
        }
    }

    fn send(&mut self, id: SessionId, payload: &[u8]) {
        let Some(addr) = self.by_id.get(&id) else { return };
        let Some(s) = self.sessions.get_mut(addr) else { return };
        let mut body = Vec::with_capacity(payload.len() + 1);
        body.push(ID_GAME);
        body.extend_from_slice(payload);
        s.queue(&body, RELIABLE_ORDERED);
    }

    fn flush(&mut self, out: &mut Vec<NetEvent>) {
        let now = Instant::now();
        let mut dead = Vec::new();
        for (addr, s) in self.sessions.iter_mut() {
            if now.duration_since(s.last_recv) > SESSION_TIMEOUT {
                dead.push(*addr);
                continue;
            }
            s.flush(&self.sock);
        }
        for addr in dead {
            if let Some(s) = self.sessions.remove(&addr) {
                self.by_id.remove(&s.id);
                out.push(NetEvent::Disconnected(s.id, "timeout".into()));
            }
        }
    }

    fn close(&mut self, id: SessionId) {
        if let Some(addr) = self.by_id.remove(&id) {
            if let Some(mut s) = self.sessions.remove(&addr) {
                s.queue(&[ID_DISCONNECTION_NOTIFICATION], RELIABLE_ORDERED);
                s.flush(&self.sock);
            }
        }
    }

    fn set_status(&mut self, motd: &str, online: usize, max: usize) {
        self.pong = format!(
            "MCPE;{motd};{};{};{online};{max};{};Ceres-RS;Creative;1;{};{};0;",
            ceres_protocol::PROTOCOL,
            ceres_protocol::VERSION,
            self.guid,
            self.port,
            self.port + 1
        )
        .into_bytes();
    }
}
