//! Moderation lists: banned players, banned IPs and whitelist, persisted as plain text files.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;

const BANNED_PLAYERS: &str = "banned-players.txt";
const BANNED_IPS: &str = "banned-ips.txt";
const WHITELIST: &str = "white-list.txt";

pub struct Moderation {
    /// lowercase player name -> reason
    pub banned_players: HashMap<String, String>,
    /// ip string -> reason
    pub banned_ips: HashMap<String, String>,
    /// lowercase whitelisted player names
    pub whitelist: HashSet<String>,
}

fn load_map(path: &str) -> HashMap<String, String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once('|') {
            Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim().to_string()),
            None => (l.to_ascii_lowercase(), "Banido por um operador".to_string()),
        })
        .collect()
}

fn save_map(path: &str, map: &HashMap<String, String>) {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    let text: String = keys.into_iter().map(|k| format!("{k}|{}\n", map[k])).collect();
    let _ = std::fs::write(path, text);
}

impl Moderation {
    pub fn load() -> Self {
        let whitelist = std::fs::read_to_string(WHITELIST)
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_ascii_lowercase())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        Moderation { banned_players: load_map(BANNED_PLAYERS), banned_ips: load_map(BANNED_IPS), whitelist }
    }

    pub fn save_bans(&self) {
        save_map(BANNED_PLAYERS, &self.banned_players);
        save_map(BANNED_IPS, &self.banned_ips);
    }

    pub fn save_whitelist(&self) {
        let mut names: Vec<&String> = self.whitelist.iter().collect();
        names.sort();
        let text: String = names.into_iter().map(|n| format!("{n}\n")).collect();
        let _ = std::fs::write(WHITELIST, text);
    }

    pub fn check_login(&self, name: &str, ip: IpAddr, whitelist_enabled: bool, is_op: bool) -> Result<(), String> {
        let ip_str = ip.to_string().to_ascii_lowercase();
        if let Some(reason) = self.banned_ips.get(&ip_str) {
            return Err(format!("§cSeu IP está banido deste servidor.\n§7Motivo: {reason}"));
        }
        let lower = name.to_ascii_lowercase();
        if let Some(reason) = self.banned_players.get(&lower) {
            return Err(format!("§cVocê está banido deste servidor.\n§7Motivo: {reason}"));
        }
        if whitelist_enabled && !is_op && !self.whitelist.contains(&lower) {
            return Err("§cVocê não está na whitelist deste servidor.".into());
        }
        Ok(())
    }
}
