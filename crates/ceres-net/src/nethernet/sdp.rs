//! SDP handling: remote ICE candidates and the answer shape the Bedrock client expects.

use super::*;

pub(crate) fn candidate_init(c: String) -> webrtc::peer_connection::RTCIceCandidateInit {
    webrtc::peer_connection::RTCIceCandidateInit {
        candidate: c,
        sdp_mid: Some("0".into()),
        sdp_mline_index: Some(0),
        ..Default::default()
    }
}

pub(crate) fn is_global_unicast(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_multicast()
                || v.is_unspecified()
                || o[0] >= 224
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19)))
        }
        std::net::IpAddr::V6(v) => (v.segments()[0] & 0xe000) == 0x2000,
    }
}

/// Mirrors addOfferedCandidates() + addInferredCandidates() from the PHP transport.
pub(crate) async fn add_remote_candidates(pc: &Arc<dyn PeerConnection>, offer: &str, peer: SocketAddr) {
    let mut ports = Vec::new();
    let mut only_host = true;
    for line in offer.lines().map(str::trim).filter(|l| l.starts_with("a=candidate:")).take(32) {
        let c = &line[2..];
        let f: Vec<&str> = c.split_whitespace().collect();
        if f.len() >= 8 {
            if f[7] != "host" {
                only_host = false;
            }
            if f[2].eq_ignore_ascii_case("udp") {
                if let Ok(port) = f[5].parse::<u16>() {
                    if !ports.contains(&port) && ports.len() < 8 {
                        ports.push(port);
                    }
                }
            }
        }
        if let Err(e) = pc.add_ice_candidate(candidate_init(c.to_string())).await {
            eprintln!("[NetherNet] ignoring offered candidate ({e}): {c}");
        }
    }
    let ip = match peer.ip() {
        std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped().map(std::net::IpAddr::V4).unwrap_or(std::net::IpAddr::V6(v6)),
        ip => ip,
    };
    if only_host && is_global_unicast(ip) {
        for (i, port) in ports.iter().enumerate() {
            let c = format!("candidate:{} 1 udp 1677721855 {ip} {port} typ srflx raddr 0.0.0.0 rport 0", 90000000 + i);
            if crate::debug_enabled() {
                println!("[NetherNet] guessing {ip}:{port} for {peer}");
            }
            let _ = pc.add_ice_candidate(candidate_init(c)).await;
        }
    }
}

/// Port of AnswerRewriter::conform(): the shape the Bedrock client expects.
pub(crate) fn conform_answer(sdp: &str) -> String {
    let mut lines: Vec<String> = sdp.lines().map(|l| l.trim_end_matches('\r').to_string()).collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let media = lines.iter().position(|l| l.starts_with("m="));
    // max-message-size
    if let Some(i) = lines.iter().position(|l| l.starts_with("a=max-message-size:")) {
        lines[i] = format!("a=max-message-size:{MAX_MESSAGE_SIZE}");
    } else if let Some(m) = media {
        lines.insert(m + 1, format!("a=max-message-size:{MAX_MESSAGE_SIZE}"));
    }
    // ice-options
    if !lines.iter().any(|l| l.starts_with("a=ice-options:")) {
        if let Some(anchor) = lines.iter().rposition(|l| l.starts_with("a=ice-pwd:") || l.starts_with("a=ice-ufrag:")) {
            lines.insert(anchor + 1, "a=ice-options:trickle".to_string());
        }
    }
    // extmap-allow-mixed in the session section
    let media = lines.iter().position(|l| l.starts_with("m="));
    let end = media.unwrap_or(lines.len());
    if !lines[..end].iter().any(|l| l == "a=extmap-allow-mixed") {
        match media {
            Some(m) => lines.insert(m, "a=extmap-allow-mixed".to_string()),
            None => lines.push("a=extmap-allow-mixed".to_string()),
        }
    }
    // data channels only use component 1
    lines.retain(|l| !(l.starts_with("a=candidate:") && l.split_whitespace().nth(1) == Some("2")));
    // candidates without related address
    for l in lines.iter_mut() {
        if l.starts_with("a=candidate:") {
            if let Some(p) = l.find(" raddr ") {
                let tail = &l[p + 7..];
                let after = tail.find(" rport ").map(|r| {
                    let rest = &tail[r + 7..];
                    rest.find(' ').map(|s| &rest[s..]).unwrap_or("")
                });
                *l = format!("{}{}", &l[..p], after.unwrap_or(""));
            }
        }
    }
    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    out
}
