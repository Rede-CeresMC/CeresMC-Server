//! NetherNet transport: WebRTC data channels negotiated over HTTP on the server port (TCP).
//!
//! Flow (same as altay/network's EndpointHandler):
//!   GET  /v1/join            -> JSON server status (server list)
//!   POST /v1/join/{networkId} with an SDP offer -> SDP answer (candidates included, no trickle)
//! Then ICE + DTLS + SCTP. The client opens "ReliableDataChannel" (ordered) and
//! "UnreliableDataChannel". Every message carries one leading byte = number of segments still to
//! come; the reassembled payload is a game batch exactly like RakNet's, minus the 0xFE byte.
//!
//! WebRTC runs on its own tokio runtime thread and talks to the game loop through channels.
//!
//! Ported from altayofficial/Network (nethernet), GPL-3.0.

use crate::transport::{NetEvent, SessionId, Transport};
use bytes::BytesMut;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use webrtc::data_channel::{DataChannel, DataChannelEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder, RTCIceGatheringState,
    RTCPeerConnectionState, RTCSessionDescription, SettingEngineBuilder,
};

mod assembler;
mod http;
mod identity;
mod peer;
mod sdp;

use assembler::*;
use http::*;
use peer::*;
use sdp::*;

static IDENTITY: std::sync::LazyLock<identity::ServerIdentity> =
    std::sync::LazyLock::new(|| identity::ServerIdentity::generate("self"));

const RELIABLE_CHANNEL: &str = "ReliableDataChannel";
const MAX_MESSAGE_SIZE: usize = 262144;
const MAX_PACKET_SIZE: usize = 4 * 1024 * 1024;
const MAX_HTTP_BODY: usize = 1024 * 1024;
const GATHERING_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct Status {
    motd: String,
    online: usize,
    max: usize,
    level: String,
    game_type: i32,
    nonce: String,
}

enum Cmd {
    Send(SessionId, Vec<u8>),
    Close(SessionId),
}

struct Session {
    pc: Arc<dyn PeerConnection>,
    reliable: Option<Arc<dyn DataChannel>>,
    max_payload: usize,
}

type Sessions = Arc<Mutex<HashMap<SessionId, Session>>>;

pub struct NetherNetServer {
    events: std_mpsc::Receiver<NetEvent>,
    cmds: mpsc::UnboundedSender<Cmd>,
    status: Arc<Mutex<Status>>,
}

impl NetherNetServer {
    pub fn start(bind: SocketAddr, first_session_id: SessionId, level: &str, game_type: i32) -> std::io::Result<Self> {
        let (ev_tx, ev_rx) = std_mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let nonce: String = (0..16).map(|_| format!("{:02x}", rand::random::<u8>())).collect();
        let status = Arc::new(Mutex::new(Status {
            motd: String::new(),
            online: 0,
            max: 0,
            level: level.to_string(),
            game_type,
            nonce,
        }));
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("nethernet")
            .build()?;
        let listener = rt.block_on(TcpListener::bind(bind))?;
        let st = status.clone();
        std::thread::Builder::new().name("nethernet".into()).spawn(move || {
            rt.block_on(async move {
                let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
                let next_id = Arc::new(AtomicU64::new(first_session_id));
                tokio::spawn(command_loop(cmd_rx, sessions.clone()));
                loop {
                    let Ok((stream, peer)) = listener.accept().await else { continue };
                    let (sessions, ev_tx, st, next_id) = (sessions.clone(), ev_tx.clone(), st.clone(), next_id.clone());
                    tokio::spawn(async move {
                        if let Err(e) = handle_http(stream, peer, sessions, ev_tx, st, next_id).await {
                            eprintln!("[NetherNet] {peer}: {e}");
                        }
                    });
                }
            });
        })?;
        Ok(NetherNetServer { events: ev_rx, cmds: cmd_tx, status })
    }
}

impl Transport for NetherNetServer {
    fn name(&self) -> &'static str {
        "NetherNet"
    }
    fn poll(&mut self, out: &mut Vec<NetEvent>) {
        while let Ok(e) = self.events.try_recv() {
            out.push(e);
        }
    }
    fn send(&mut self, id: SessionId, payload: &[u8]) {
        let _ = self.cmds.send(Cmd::Send(id, payload.to_vec()));
    }
    fn flush(&mut self, _out: &mut Vec<NetEvent>) {}
    fn close(&mut self, id: SessionId) {
        let _ = self.cmds.send(Cmd::Close(id));
    }
    fn set_status(&mut self, motd: &str, online: usize, max: usize) {
        let mut s = self.status.lock().unwrap();
        s.motd = motd.to_string();
        s.online = online;
        s.max = max;
    }
}

/// Sends run in order, one session's segments never interleave with another packet of its own.
async fn command_loop(mut rx: mpsc::UnboundedReceiver<Cmd>, sessions: Sessions) {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            Cmd::Send(id, payload) => {
                let target = {
                    let s = sessions.lock().unwrap();
                    s.get(&id).and_then(|s| s.reliable.clone().map(|c| (c, s.max_payload)))
                };
                let Some((dc, max)) = target else {
                    eprintln!("[NetherNet] session {id}: dropping {} bytes, no open reliable channel", payload.len());
                    continue;
                };
                let segments = payload.len().div_ceil(max).max(1);
                if crate::debug_enabled() && payload.len() > 16 * 1024 {
                    println!(
                        "[NetherNet] session {id}: sending {} bytes in {segments} message(s) of <= {max}",
                        payload.len()
                    );
                }
                if segments > 256 {
                    eprintln!("[NetherNet] session {id}: packet of {} bytes is too large", payload.len());
                    continue;
                }
                let mut remaining = segments - 1;
                let mut chunks: Vec<&[u8]> = payload.chunks(max).collect();
                if chunks.is_empty() {
                    chunks.push(&[]);
                }
                for c in chunks {
                    let mut m = BytesMut::with_capacity(c.len() + 1);
                    m.extend_from_slice(&[remaining as u8]);
                    m.extend_from_slice(c);
                    if let Err(e) = dc.send(m).await {
                        eprintln!("[NetherNet] session {id}: send failed: {e}");
                        break;
                    }
                    remaining = remaining.saturating_sub(1);
                }
            }
            Cmd::Close(id) => {
                let s = sessions.lock().unwrap().remove(&id);
                if let Some(s) = s {
                    let _ = s.pc.close().await;
                }
            }
        }
    }
}
