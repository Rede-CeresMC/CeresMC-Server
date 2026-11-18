//! WebRTC peer connections: negotiation and data channel events.

use super::*;

// ------------------------------------------------------------------ WebRTC

/// Connections still being set up (offer received, data channel not open yet), per address.
static PENDING: std::sync::LazyLock<Mutex<HashMap<std::net::IpAddr, usize>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
const MAX_PENDING: usize = 64;
const MAX_PENDING_PER_ADDRESS: usize = 8;
/// a connection not up after this long is dropped, freeing its port
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct Handler {
    pub(crate) sid: SessionId,
    pub(crate) peer: SocketAddr,
    pub(crate) sessions: Sessions,
    pub(crate) ev_tx: Mutex<std_mpsc::Sender<NetEvent>>,
    pub(crate) gathered: mpsc::UnboundedSender<()>,
    pub(crate) opened: AtomicBool,
    pub(crate) closed: AtomicBool,
    /// still counted in PENDING
    pub(crate) pending: AtomicBool,
    /// client key the offer was signed with
    pub(crate) identity: Option<String>,
}

impl Handler {
    pub(crate) fn emit(&self, e: NetEvent) {
        let _ = self.ev_tx.lock().unwrap().send(e);
    }
    /// The connection left the "being set up" state (opened or closed).
    pub(crate) fn settle(&self) {
        if self.pending.swap(false, Ordering::SeqCst) {
            let mut map = PENDING.lock().unwrap();
            if let Some(n) = map.get_mut(&self.peer.ip()) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    map.remove(&self.peer.ip());
                }
            }
        }
    }

    pub(crate) fn close(&self, reason: &str) {
        self.settle();
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let s = self.sessions.lock().unwrap().remove(&self.sid);
        if self.opened.load(Ordering::SeqCst) {
            self.emit(NetEvent::Disconnected(self.sid, reason.to_string()));
        }
        if let Some(s) = s {
            tokio::spawn(async move {
                let _ = s.pc.close().await;
            });
        }
    }
}

pub(crate) struct H(Arc<Handler>);

#[async_trait::async_trait]
impl PeerConnectionEventHandler for H {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.0.gathered.send(());
        }
    }

    async fn on_ice_connection_state_change(&self, state: webrtc::peer_connection::RTCIceConnectionState) {
        if crate::debug_enabled() {
            println!("[NetherNet] session {} ICE: {state}", self.0.sid);
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        if crate::debug_enabled() {
            println!("[NetherNet] session {} connection: {state}", self.0.sid);
        }
        match state {
            RTCPeerConnectionState::Failed => {
                crate::log(&format!(
                    "[NetherNet] session {} from {}: connection failed (no network path: for players outside the local network, forward the nethernet-udp-ports range on the router)",
                    self.0.sid, self.0.peer
                ));
                self.0.close("connection failed")
            }
            RTCPeerConnectionState::Closed => self.0.close("connection closed"),
            // "disconnected" is temporary (ICE checks missed for a few seconds) and usually comes
            // back to "connected"; only "failed" is final, like altay's transport
            RTCPeerConnectionState::Disconnected => {
                println!("[NetherNet] session {}: connection unstable, waiting for it to recover", self.0.sid);
            }
            _ => {}
        }
    }

    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        let label = dc.label().await.unwrap_or_default();
        let h = self.0.clone();
        tokio::spawn(async move {
            let reliable = label == RELIABLE_CHANNEL;
            let mut assembler = Assembler::default();
            let mark_open = |h: &Arc<Handler>| {
                if !reliable {
                    return;
                }
                if let Some(s) = h.sessions.lock().unwrap().get_mut(&h.sid) {
                    s.reliable = Some(dc.clone());
                }
                h.settle();
                if !h.opened.swap(true, Ordering::SeqCst) {
                    crate::log(&format!("[NetherNet] {} opened a session ({})", h.peer, h.sid));
                    h.emit(NetEvent::Connected(h.sid, h.peer, h.identity.clone()));
                }
            };
            if matches!(dc.ready_state().await, Ok(webrtc::data_channel::RTCDataChannelState::Open)) {
                mark_open(&h);
            }
            while let Some(ev) = dc.poll().await {
                match ev {
                    DataChannelEvent::OnOpen => mark_open(&h),
                    DataChannelEvent::OnMessage(msg) => {
                        if !reliable {
                            continue; // the game does not use the unreliable channel
                        }
                        match assembler.push(&msg.data) {
                            Ok(Some(packet)) => h.emit(NetEvent::Packet(h.sid, packet)),
                            Ok(None) => {}
                            Err(e) => {
                                eprintln!("[NetherNet] session {}: {e}", h.sid);
                                h.close("malformed message");
                                break;
                            }
                        }
                    }
                    DataChannelEvent::OnClose => {
                        if crate::debug_enabled() {
                            println!("[NetherNet] session {}: channel {label} closed by peer", h.sid);
                        }
                        if reliable {
                            h.close("data channel closed");
                        }
                        break;
                    }
                    _ => {}
                }
            }
        });
    }
}

pub(crate) fn peer_max_message_size(sdp: &str) -> usize {
    sdp.lines()
        .find_map(|l| l.trim().strip_prefix("a=max-message-size:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 1)
        .map(|v| (v - 1).min(MAX_MESSAGE_SIZE - 1))
        .unwrap_or(MAX_MESSAGE_SIZE - 1)
}

/// The public IP to offer: the configured one, or the address the player typed to join (the
/// Host header) when it is a public one. Players outside the local network can only reach that.
fn public_ip(host_header: &str) -> Option<std::net::IpAddr> {
    let configured = PUBLIC_IP.lock().unwrap().clone();
    let wanted = if configured.is_empty() { host_header.to_string() } else { configured };
    let host = match wanted.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) && !h.contains(':') => h.to_string(),
        _ => wanted.trim_matches(['[', ']']).to_string(),
    };
    if host.is_empty() {
        return None;
    }
    // only a literal IP: a host name in the header is not resolved (set nethernet-public-ip instead)
    let ip = host.parse::<std::net::IpAddr>().ok()?;
    let public = match ip {
        std::net::IpAddr::V4(v4) => !(v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()),
        std::net::IpAddr::V6(_) => false,
    };
    public.then_some(ip)
}

/// Next port of the configured range to try.
fn next_udp_port() -> (u16, usize) {
    let (first, last) = *UDP_PORTS.lock().unwrap();
    let span = (last - first) as u64 + 1;
    let n = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
    (first + (n % span) as u16, span as usize)
}

pub(crate) async fn negotiate(
    sid: SessionId,
    offer: String,
    peer: SocketAddr,
    host_header: &str,
    sessions: Sessions,
    ev_tx: std_mpsc::Sender<NetEvent>,
) -> Result<String, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let identity = identity::client_identity(&offer, now).map_err(|e| format!("invalid identity: {e}"))?;
    if identity.is_none() && REQUIRE_IDENTITY.load(Ordering::Relaxed) {
        return Err("the offer carries no identity (nethernet-require-identity=off allows it)".into());
    }
    let (gather_tx, mut gather_rx) = mpsc::unbounded_channel();
    {
        let mut map = PENDING.lock().unwrap();
        let mine = map.get(&peer.ip()).copied().unwrap_or(0);
        if mine >= MAX_PENDING_PER_ADDRESS || map.values().sum::<usize>() >= MAX_PENDING {
            return Err(format!("too many connections being set up ({} pending)", map.values().sum::<usize>()));
        }
        *map.entry(peer.ip()).or_default() += 1;
    }
    let handler = Arc::new(Handler {
        sid,
        peer,
        sessions: sessions.clone(),
        ev_tx: Mutex::new(ev_tx),
        gathered: gather_tx,
        opened: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        pending: AtomicBool::new(true),
        identity,
    });
    // a connection that never comes up must not keep its port
    {
        let h = handler.clone();
        tokio::spawn(async move {
            tokio::time::sleep(HANDSHAKE_TIMEOUT).await;
            if !h.opened.load(Ordering::SeqCst) {
                h.close("handshake timed out");
            }
        });
    }
    let public = public_ip(host_header);
    // one port of the range per player; a port still held by someone else is skipped
    let (mut port, span) = next_udp_port();
    let mut attempts = 0;
    let pc = loop {
        let mut engine = SettingEngineBuilder::new().with_sctp_max_message_size(
            rtc::peer_connection::configuration::setting_engine::SctpMaxMessageSize::Bounded(MAX_MESSAGE_SIZE as u32),
        );
        if let Some(ip) = public {
            // offered next to the local addresses, on the same port (forwarded on the router)
            engine = engine.with_nat_1to1_ips(
                vec![ip.to_string()],
                rtc::peer_connection::transport::RTCIceCandidateType::Srflx,
            );
        }
        let built = PeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::new().build())
            .with_setting_engine(engine.build())
            .with_handler(Arc::new(H(handler.clone())))
            .with_udp_addrs(vec![format!("0.0.0.0:{port}"), format!("127.0.0.1:{port}")])
            .build()
            .await;
        match built {
            Ok(pc) => break pc,
            Err(e) => {
                attempts += 1;
                if port == 0 {
                    handler.close("no UDP port");
                    return Err(format!("peer connection: {e}"));
                }
                // the range is full: any free port (works on the local network, not through a router)
                port = if attempts >= span.min(200) { 0 } else { next_udp_port().0 };
            }
        }
    };
    if crate::debug_enabled() {
        println!("[NetherNet] session {sid}: media on UDP port {port}, public address {public:?}");
    }
    let pc: Arc<dyn PeerConnection> = Arc::new(pc);
    sessions
        .lock()
        .unwrap()
        .insert(sid, Session { pc: pc.clone(), reliable: None, max_payload: peer_max_message_size(&offer) });

    let offer_copy = offer.clone();
    let result = async {
        let desc = RTCSessionDescription::offer(offer).map_err(|e| format!("bad offer: {e}"))?;
        pc.set_remote_description(desc).await.map_err(|e| format!("set remote: {e}"))?;
        add_remote_candidates(&pc, &offer_copy, peer).await;
        let answer = pc.create_answer(None).await.map_err(|e| format!("create answer: {e}"))?;
        pc.set_local_description(answer).await.map_err(|e| format!("set local: {e}"))?;
        let _ = tokio::time::timeout(GATHERING_TIMEOUT, gather_rx.recv()).await;
        let local = pc.local_description().await.ok_or("no local description")?;
        let mut answer = conform_answer(&local.sdp);
        if let Some(attr) = IDENTITY.identity_attribute(&answer) {
            if let Some(m) = answer.find("m=") {
                answer.insert_str(m, &format!("a=identity:{attr}\r\n"));
            }
        }
        if crate::debug_enabled() {
            let _ = std::fs::create_dir_all("debug");
            let _ = std::fs::write(format!("debug/offer-{sid}.sdp"), &offer_copy);
            let _ = std::fs::write(format!("debug/answer-{sid}.sdp"), &answer);
        }
        Ok::<String, String>(answer)
    }
    .await;
    if result.is_err() {
        handler.close("negotiation failed");
    }
    result
}
