//! CeresMC Server: entry point.

mod config;
mod server;

use ceres_net::{NetherNetServer, RakNetServer, Transport};
use ceres_world::{GameData, WorldManager};
use config::Config;

fn fail(what: &str, e: impl std::fmt::Display) -> ! {
    eprintln!("{what}: {e}");
    std::process::exit(1);
}

fn main() {
    let started = std::time::Instant::now();
    let root = std::env::current_dir().expect("cwd");
    let cfg = Config::load(&root.join("server.properties"));

    println!(
        "CeresMC Server {} for Minecraft: Bedrock Edition {} (protocol {})",
        env!("CARGO_PKG_VERSION"),
        ceres_protocol::VERSION,
        ceres_protocol::PROTOCOL
    );

    let gd = GameData::load(&root.join("data")).unwrap_or_else(|e| fail("Failed to load vanilla data", e));
    println!(
        "[Data] {} block states, {} block definitions, {} creative items",
        gd.block_ids.len(),
        gd.block_definition_count,
        gd.creative_item_count
    );

    let worlds = WorldManager::load_all(&root.join("worlds"), &cfg.level_name)
        .unwrap_or_else(|e| fail("Failed to load worlds", e));

    let addr = format!("0.0.0.0:{}", cfg.port).parse().unwrap();
    let rak = RakNetServer::bind(addr, 1).unwrap_or_else(|e| fail(&format!("Failed to bind {addr}/udp"), e));
    println!("[Net] RakNet listening on {addr} (UDP)");
    let mut transports: Vec<Box<dyn Transport>> = vec![Box::new(rak)];
    // NetherNet session ids start high so they never collide with RakNet ones
    match NetherNetServer::start(addr, 1 << 40, &worlds.worlds[worlds.default].display_name, cfg.gamemode) {
        Ok(n) => {
            println!("[Net] NetherNet signalling on {addr} (TCP/HTTP)");
            transports.push(Box::new(n));
        }
        Err(e) => eprintln!("[Net] NetherNet disabled: failed to bind {addr}/tcp: {e}"),
    }
    println!("Done ({:.3}s)!", started.elapsed().as_secs_f64());

    server::Server::new(cfg, gd, worlds, transports).run();
}
