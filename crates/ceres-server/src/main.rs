//! CeresMC Server: entry point.

// println!/eprintln! of this crate go through the shared console (colours, log file)
#[allow(unused_macros)]
macro_rules! println {
    () => { ceres_codec::console::line("", false) };
    ($($t:tt)*) => { ceres_codec::console::line(&format!($($t)*), false) };
}
#[allow(unused_macros)]
macro_rules! eprintln {
    () => { ceres_codec::console::line("", true) };
    ($($t:tt)*) => { ceres_codec::console::line(&format!($($t)*), true) };
}

mod auth;
mod colors;
mod config;
mod logfile;
mod packs;
mod server;

use ceres_net::{NetherNetServer, RakNetServer, Transport};
use ceres_world::{GameData, WorldManager};
use config::Config;

fn fail(what: &str, e: impl std::fmt::Display) -> ! {
    eprintln!("{what}: {e}");
    std::process::exit(1);
}

fn main() {
    colors::init();
    ceres_codec::console::set_sink(colors::print);
    logfile::init();
    let started = std::time::Instant::now();
    let root = std::env::current_dir().expect("cwd");
    let cfg = Config::load(&root.join("server.properties"));

    println!(
        "CeresMC Server {} for Minecraft: Bedrock Edition {} (protocol {})",
        env!("CARGO_PKG_VERSION"),
        ceres_protocol::VERSION,
        ceres_protocol::PROTOCOL
    );

    if cfg.allow_unverified_ops {
        eprintln!(
            "[Server] WARNING: allow-unverified-ops is on: anyone who joins with a name from ops.txt is an operator"
        );
    }

    let gd = GameData::load(&root.join("data")).unwrap_or_else(|e| fail("Failed to load vanilla data", e));
    println!(
        "[Data] {} block states, {} block definitions, {} items, {} creative items",
        gd.block_ids.len(),
        gd.block_definition_count,
        gd.items_by_id.len(),
        gd.creative_item_count
    );

    let worlds = WorldManager::load_all(&root.join("worlds"), &cfg.level_name, &gd)
        .unwrap_or_else(|e| fail("Failed to load worlds", e));

    let addr = format!("0.0.0.0:{}", cfg.port).parse().unwrap();
    let rak = RakNetServer::bind(addr, 1).unwrap_or_else(|e| fail(&format!("Failed to bind {addr}/udp"), e));
    println!("[Net] RakNet listening on {addr} (UDP)");
    let mut transports: Vec<Box<dyn Transport>> = vec![Box::new(rak)];
    // NetherNet session ids start high so they never collide with RakNet ones
    ceres_net::nethernet::set_require_identity(cfg.nethernet_require_identity);
    ceres_net::nethernet::set_udp_ports(cfg.nethernet_udp_ports.0, cfg.nethernet_udp_ports.1);
    ceres_net::nethernet::set_public_ip(&cfg.nethernet_public_ip);
    ceres_net::set_log_sink(logfile::log);
    match NetherNetServer::start(addr, 1 << 40, &worlds.worlds[worlds.default].display_name, cfg.gamemode) {
        Ok(n) => {
            println!("[Net] NetherNet signalling on {addr} (TCP/HTTP), media on UDP {}-{}", cfg.nethernet_udp_ports.0, cfg.nethernet_udp_ports.1);
            transports.push(Box::new(n));
        }
        Err(e) => eprintln!("[Net] NetherNet disabled: failed to bind {addr}/tcp: {e}"),
    }
    ctrlc::set_handler(|| server::SHUTDOWN.store(true, std::sync::atomic::Ordering::SeqCst))
        .unwrap_or_else(|e| eprintln!("[Server] Ctrl+C handler not installed: {e}"));
    println!("Done ({:.3}s)! Type \"help\" for commands.", started.elapsed().as_secs_f64());

    // Mojang's login keys are fetched in the background; logins verify once they are loaded
    let auth_keys = auth::spawn_key_fetcher(ceres_protocol::VERSION);
    if !cfg.xbox_auth {
        eprintln!("[Server] WARNING: xbox-auth is off: anyone can join with any name");
    }
    server::Server::new(cfg, gd, worlds, transports, auth_keys).run();
}
