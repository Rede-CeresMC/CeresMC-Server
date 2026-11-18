//! server.properties (same keys as PocketMine-MP / BDS).

use std::collections::HashMap;
use std::path::Path;

const DEFAULT_PROPERTIES: &str = "\
motd=CeresMC Server
server-port=19132
max-players=20
gamemode=survival
difficulty=normal
level-name=world
view-distance=10
white-list=off
pvp=on
keep-inventory=off
";

pub struct Config {
    pub motd: String,
    pub port: u16,
    pub max_players: usize,
    pub view_distance: i32,
    pub gamemode: i32,
    pub difficulty: i32,
    pub level_name: String,
    pub chunks_per_tick: usize,
    pub white_list: bool,
    pub pvp: bool,
    pub keep_inventory: bool,
    pub max_packets_per_sec: u32,
    pub max_batch_bytes: usize,
    /// Names in ops.txt get operator rights even though logins are not verified yet (anyone can
    /// pick any name). Only for local testing.
    pub allow_unverified_ops: bool,
    /// Verify Xbox Live logins (OpenID token + client data signature) and refuse the ones that fail.
    pub xbox_auth: bool,
    /// NetherNet: refuse offers without a signed client identity (`auto` = when xbox-auth is on)
    pub nethernet_require_identity: bool,
    /// NetherNet: UDP ports for the players' WebRTC media (forward them for players from outside)
    pub nethernet_udp_ports: (u16, u16),
    /// NetherNet: public IP to offer (empty: the address the player typed, when public)
    pub nethernet_public_ip: String,
}

fn flag(s: &str) -> bool {
    matches!(s.to_ascii_lowercase().as_str(), "on" | "true" | "1" | "yes")
}

impl Config {
    /// Reads `path`, creating it with defaults when missing.
    pub fn load(path: &Path) -> Config {
        if !path.exists() {
            let _ = std::fs::write(path, DEFAULT_PROPERTIES);
        }
        let text = std::fs::read_to_string(path).unwrap_or_else(|_| DEFAULT_PROPERTIES.to_string());
        let props: HashMap<String, String> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        let get = |k: &str, d: &str| props.get(k).cloned().unwrap_or_else(|| d.to_string());

        Config {
            motd: get("motd", "CeresMC Server"),
            port: get("server-port", "19132").parse().unwrap_or(19132),
            max_players: get("max-players", "20").parse().unwrap_or(20),
            view_distance: get("view-distance", "10").parse().unwrap_or(10).clamp(2, 32),
            gamemode: match get("gamemode", "survival").as_str() {
                "creative" | "1" => 1,
                "adventure" | "2" => 2,
                "spectator" | "3" => 6,
                _ => 0,
            },
            difficulty: match get("difficulty", "normal").as_str() {
                "peaceful" | "0" => 0,
                "easy" | "1" => 1,
                "hard" | "3" => 3,
                _ => 2,
            },
            level_name: get("level-name", "world"),
            chunks_per_tick: 8,
            white_list: flag(&get("white-list", "off")),
            pvp: flag(&get("pvp", "on")),
            keep_inventory: flag(&get("keep-inventory", "off")),
            max_packets_per_sec: get("max-packets-per-sec", "600").parse().unwrap_or(600).max(50),
            max_batch_bytes: get("max-batch-bytes", "2097152").parse().unwrap_or(2 * 1024 * 1024).max(65536),
            allow_unverified_ops: flag(&get("allow-unverified-ops", "off")),
            xbox_auth: flag(&get("xbox-auth", "on")),
            nethernet_udp_ports: {
                let v = get("nethernet-udp-ports", "19300-19399");
                let parsed = v.split_once('-').and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)));
                parsed.filter(|(a, b): &(u16, u16)| *a > 0 && *b >= *a).unwrap_or((19300, 19399))
            },
            nethernet_public_ip: get("nethernet-public-ip", ""),
            nethernet_require_identity: match get("nethernet-require-identity", "auto").to_ascii_lowercase().as_str() {
                "auto" => flag(&get("xbox-auth", "on")),
                v => flag(v),
            },
        }
    }
}
