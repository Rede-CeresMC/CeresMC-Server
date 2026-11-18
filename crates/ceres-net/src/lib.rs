//! Network transports. Every transport implements [`transport::Transport`] so the game layer
//! never knows whether a player came in over RakNet or NetherNet.

pub mod nethernet;
pub mod raknet;
pub mod transport;

pub use nethernet::NetherNetServer;
pub use raknet::RakNetServer;
pub use transport::{NetEvent, SessionId, Transport};

/// Where the transports send their important lines (the server's log file); stderr by default.
static LOG_SINK: std::sync::OnceLock<fn(&str)> = std::sync::OnceLock::new();

pub fn set_log_sink(sink: fn(&str)) {
    let _ = LOG_SINK.set(sink);
}

pub(crate) fn log(line: &str) {
    match LOG_SINK.get() {
        Some(sink) => sink(line),
        None => eprintln!("{line}"),
    }
}

/// `CERES_DEBUG=1` turns on verbose transport logs (handshakes, SDP dumps).
pub fn debug_enabled() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("CERES_DEBUG").is_some());
    *ON
}
