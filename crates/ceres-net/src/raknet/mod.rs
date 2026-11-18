//! RakNet server (the subset Bedrock uses): offline handshake, reliability, ordering,
//! fragmentation, ACK/NACK and resend.
//!
//! Ported from altayofficial/Network (raknet), GPL-3.0.

mod frame;
mod server;
mod session;

pub use server::RakNetServer;

use frame::*;
use session::*;

use crate::transport::{NetEvent, SessionId, Transport};
use ceres_codec::binary::{R, W};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const MAGIC: [u8; 16] =
    [0x00, 0xff, 0xff, 0x00, 0xfe, 0xfe, 0xfe, 0xfe, 0xfd, 0xfd, 0xfd, 0xfd, 0x12, 0x34, 0x56, 0x78];
const MAX_MTU: usize = 1492;
const MIN_MTU: usize = 400;
const UDP_OVERHEAD: usize = 28;
const SESSION_TIMEOUT: Duration = Duration::from_secs(15);
const RESEND_AFTER: Duration = Duration::from_millis(500);
const MAX_SPLIT_PARTS: u32 = 8192;
const MAX_CONCURRENT_SPLITS: usize = 16;

const ID_CONNECTED_PING: u8 = 0x00;
const ID_UNCONNECTED_PING: u8 = 0x01;
const ID_UNCONNECTED_PING_OPEN: u8 = 0x02;
const ID_CONNECTED_PONG: u8 = 0x03;
const ID_OPEN_CONNECTION_REQUEST_1: u8 = 0x05;
const ID_OPEN_CONNECTION_REPLY_1: u8 = 0x06;
const ID_OPEN_CONNECTION_REQUEST_2: u8 = 0x07;
const ID_OPEN_CONNECTION_REPLY_2: u8 = 0x08;
const ID_CONNECTION_REQUEST: u8 = 0x09;
const ID_CONNECTION_REQUEST_ACCEPTED: u8 = 0x10;
const ID_NEW_INCOMING_CONNECTION: u8 = 0x13;
const ID_DISCONNECTION_NOTIFICATION: u8 = 0x15;
const ID_INCOMPATIBLE_PROTOCOL: u8 = 0x19;
const ID_UNCONNECTED_PONG: u8 = 0x1c;
const ID_GAME: u8 = 0xfe;

const RAKNET_PROTOCOL: u8 = 11;

// reliability
const UNRELIABLE: u8 = 0;
const RELIABLE: u8 = 2;
const RELIABLE_ORDERED: u8 = 3;

fn is_reliable(r: u8) -> bool {
    matches!(r, 2 | 3 | 4 | 6 | 7)
}
fn is_sequenced(r: u8) -> bool {
    matches!(r, 1 | 4)
}
fn is_sequenced_or_ordered(r: u8) -> bool {
    matches!(r, 1 | 3 | 4 | 7)
}
