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
        }
    }
}
