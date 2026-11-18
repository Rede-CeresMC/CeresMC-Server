//! `server.log`: a copy of what matters for diagnosing disconnects (kicks, unreadable packets,
//! lost connections), with the time. The console still shows everything.

use std::io::Write;
use std::sync::Mutex;

static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn init() {
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open("server.log") {
        *FILE.lock().unwrap() = Some(f);
    }
}

fn timestamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // UTC time of day is enough to line entries up with the client's error screen (also UTC)
    format!("{:02}:{:02}:{:02}Z", secs / 3600 % 24, secs / 60 % 60, secs % 60)
}

/// Prints the line and appends it to server.log. Control characters (a player name or kick
/// message with line breaks) are escaped so nobody can forge log lines.
pub fn log(line: &str) {
    let line: String = line.chars().flat_map(|c| if c.is_control() { c.escape_default().collect::<Vec<_>>() } else { vec![c] }).collect();
    eprintln!("{line}");
    if let Some(f) = FILE.lock().unwrap().as_mut() {
        let _ = writeln!(f, "[{}] {line}", timestamp());
        let _ = f.flush();
    }
}

const MAX_DUMPS: usize = 20;
const MAX_DUMP_BYTES: usize = 64 * 1024;
const LOGIN_PACKET: u32 = 0x01;

/// Saves a packet the server could not read, for later analysis. Returns the file name.
/// Login packets (they carry the player's tokens) are never saved; at most 20 dumps of 64 KB.
pub fn dump_packet(sid: u64, data: &[u8]) -> Option<String> {
    let id = ceres_codec::binary::R::new(data).uvarint().ok()? & 0x3ff;
    if id == LOGIN_PACKET {
        return None;
    }
    let existing = std::fs::read_dir("debug")
        .map(|d| d.filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with("bad-packet-")).count())
        .unwrap_or(0);
    if existing >= MAX_DUMPS {
        return None;
    }
    let data = &data[..data.len().min(MAX_DUMP_BYTES)];
    let _ = std::fs::create_dir_all("debug");
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let name = format!("debug/bad-packet-{sid}-{secs}.bin");
    std::fs::write(&name, data).ok().map(|_| name)
}
