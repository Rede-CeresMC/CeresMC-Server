//! Network transports. Every transport implements [`transport::Transport`] so the game layer
//! never knows whether a player came in over RakNet or NetherNet.

pub mod nethernet;
pub mod raknet;
pub mod transport;

pub use nethernet::NetherNetServer;
pub use raknet::RakNetServer;
pub use transport::{NetEvent, SessionId, Transport};

/// `CERES_DEBUG=1` turns on verbose transport logs (handshakes, SDP dumps).
pub fn debug_enabled() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("CERES_DEBUG").is_some());
    *ON
}
