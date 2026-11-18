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

/// Prints the line and appends it to server.log.
pub fn log(line: &str) {
    eprintln!("{line}");
    if let Some(f) = FILE.lock().unwrap().as_mut() {
        let _ = writeln!(f, "[{}] {line}", timestamp());
        let _ = f.flush();
    }
}

/// Saves a packet the server could not read, for later analysis. Returns the file name.
pub fn dump_packet(sid: u64, data: &[u8]) -> Option<String> {
    let _ = std::fs::create_dir_all("debug");
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let name = format!("debug/bad-packet-{sid}-{secs}.bin");
    std::fs::write(&name, data).ok().map(|_| name)
}
