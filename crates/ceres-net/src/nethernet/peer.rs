//! WebRTC peer connections: negotiation and data channel events.

use super::*;

// ------------------------------------------------------------------ WebRTC

pub(crate) struct Handler {
    pub(crate) sid: SessionId,
    pub(crate) peer: SocketAddr,
    pub(crate) sessions: Sessions,
    pub(crate) ev_tx: Mutex<std_mpsc::Sender<NetEvent>>,
    pub(crate) gathered: mpsc::UnboundedSender<()>,
    pub(crate) opened: AtomicBool,
    pub(crate) closed: AtomicBool,
    /// client key the offer was signed with
    pub(crate) identity: Option<String>,
}

impl Handler {
    pub(crate) fn emit(&self, e: NetEvent) {
        let _ = self.ev_tx.lock().unwrap().send(e);
    }
    pub(crate) fn close(&self, reason: &str) {
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
            RTCPeerConnectionState::Failed => self.0.close("connection failed"),
            RTCPeerConnectionState::Closed => self.0.close("connection closed"),
            RTCPeerConnectionState::Disconnected => self.0.close("connection lost"),
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
                if !h.opened.swap(true, Ordering::SeqCst) {
                    println!("[NetherNet] {} opened a session ({})", h.peer, h.sid);
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

pub(crate) async fn negotiate(
    sid: SessionId,
    offer: String,
    peer: SocketAddr,
    sessions: Sessions,
    ev_tx: std_mpsc::Sender<NetEvent>,
) -> Result<String, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let identity = identity::client_identity(&offer, now).map_err(|e| format!("invalid identity: {e}"))?;
    if identity.is_none() && REQUIRE_IDENTITY.load(Ordering::Relaxed) {
        return Err("the offer carries no identity (nethernet-require-identity=off allows it)".into());
    }
    let (gather_tx, mut gather_rx) = mpsc::unbounded_channel();
    let handler = Arc::new(Handler {
        sid,
        peer,
        sessions: sessions.clone(),
        ev_tx: Mutex::new(ev_tx),
        gathered: gather_tx,
        opened: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        identity,
    });
    let engine = SettingEngineBuilder::new()
        .with_sctp_max_message_size(rtc::peer_connection::configuration::setting_engine::SctpMaxMessageSize::Bounded(
            MAX_MESSAGE_SIZE as u32,
        ))
        .build();
    let pc = PeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .with_setting_engine(engine)
        .with_handler(Arc::new(H(handler.clone())))
        .with_udp_addrs(vec!["0.0.0.0:0".to_string(), "127.0.0.1:0".to_string()])
        .build()
        .await
        .map_err(|e| format!("peer connection: {e}"))?;
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
