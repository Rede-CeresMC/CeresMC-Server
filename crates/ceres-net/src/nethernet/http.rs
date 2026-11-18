//! HTTP signalling endpoint: GET /v1/join (status) and POST /v1/join/{id} (SDP offer -> answer).

use super::*;

pub(crate) async fn handle_http(
    mut stream: TcpStream,
    peer: SocketAddr,
    sessions: Sessions,
    ev_tx: std_mpsc::Sender<NetEvent>,
    status: Arc<Mutex<Status>>,
    next_id: Arc<AtomicU64>,
) -> Result<(), String> {
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut tmp))
            .await
            .map_err(|_| "timed out reading request")?
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(());
        }
        if buf.is_empty() && tmp[0] == 0x16 {
            println!("[NetherNet] {peer} tried TLS on the signalling port; only plain HTTP is supported for now");
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        if buf.len() > 64 * 1024 {
            return Err("request headers too large".into());
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").split('?').next().unwrap_or("").to_string();
    let mut content_length = 0usize;
    let mut host = String::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => content_length = v.trim().parse().unwrap_or(0),
                "host" => host = v.trim().to_string(),
                _ => {}
            }
        }
    }
    if crate::debug_enabled() {
        println!("[NetherNet] {peer} {method} {path}");
    }
    if content_length > MAX_HTTP_BODY {
        return respond(&mut stream, 413, "text/plain; charset=utf-8", b"SDP offer is too large").await;
    }
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut tmp))
            .await
            .map_err(|_| "timed out reading body")?
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }

    let path = path.trim_end_matches('/');
    let Some(rest) = path.strip_prefix("/v1/join") else {
        return respond(&mut stream, 404, "text/plain; charset=utf-8", b"Not found").await;
    };
    let network_id = rest.trim_start_matches('/');
    if network_id.is_empty() && (method == "GET" || method == "HEAD") {
        let s = status.lock().unwrap().clone();
        let json = serde_json::json!({
            "dataVersion": 7,
            "name": s.motd,
            "protocol": ceres_protocol::PROTOCOL,
            "version": ceres_protocol::VERSION,
            "level": s.level,
            "players": s.online,
            "maxPlayers": s.max,
            "gameType": s.game_type,
            "editor": false,
            "hardcore": false,
            "onlineAuth": false,
            "selfSignedAuth": true,
            "nonce": s.nonce,
            "transportLayer": 2,
            "connection": 4,
        });
        return respond(&mut stream, 200, "application/json", json.to_string().as_bytes()).await;
    }
    if method != "POST" {
        return respond(&mut stream, 405, "text/plain; charset=utf-8", b"Method not allowed").await;
    }
    if network_id.is_empty() || network_id.parse::<u64>().is_err() {
        return respond(&mut stream, 400, "text/plain; charset=utf-8", b"Expected /v1/join/{networkID}").await;
    }
    let offer = String::from_utf8_lossy(&body).into_owned();
    if offer.is_empty() {
        return respond(&mut stream, 400, "text/plain; charset=utf-8", b"Missing SDP offer in request body").await;
    }
    let _ = host;
    let sid = next_id.fetch_add(1, Ordering::Relaxed);
    match negotiate(sid, offer, peer, sessions, ev_tx).await {
        Ok(answer) => respond(&mut stream, 200, "application/sdp", answer.as_bytes()).await,
        Err(e) => {
            eprintln!("[NetherNet] negotiation with {peer} failed: {e}");
            respond(&mut stream, 503, "text/plain; charset=utf-8", b"Service unavailable").await
        }
    }
}

pub(crate) async fn respond(stream: &mut TcpStream, code: u16, ctype: &str, body: &[u8]) -> Result<(), String> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Service Unavailable",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.map_err(|e| e.to_string())?;
    stream.write_all(body).await.map_err(|e| e.to_string())?;
    let _ = stream.shutdown().await;
    Ok(())
}
