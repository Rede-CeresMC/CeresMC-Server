//! Transport abstraction. RakNet implements it today; NetherNet (WebRTC) will implement the same
//! trait so the game layer never knows which wire a player came in on.

use std::net::SocketAddr;

pub type SessionId = u64;

pub enum NetEvent {
    /// A client finished the transport handshake and may now send game packets. The last field is
    /// the client key the transport proved the peer holds (NetherNet identity), if any.
    Connected(SessionId, SocketAddr, Option<String>),
    /// One game-layer payload (the bytes after RakNet's 0xFE / NetherNet's message framing).
    Packet(SessionId, Vec<u8>),
    Disconnected(SessionId, String),
}

pub trait Transport {
    fn name(&self) -> &'static str;
    /// Pump the socket for up to a few milliseconds and push whatever happened into `out`.
    fn poll(&mut self, out: &mut Vec<NetEvent>);
    /// Queue a game-layer payload for a session; it goes out on the next `flush`.
    fn send(&mut self, id: SessionId, payload: &[u8]);
    /// Flush queued data, ACKs, resends and time out dead sessions.
    fn flush(&mut self, out: &mut Vec<NetEvent>);
    fn close(&mut self, id: SessionId);
    fn set_status(&mut self, motd: &str, online: usize, max: usize);
}
