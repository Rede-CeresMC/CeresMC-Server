//! All loaded worlds (multi-world).

use super::*;

pub struct WorldManager {
    pub worlds: Vec<World>,
    pub default: usize,
}

impl WorldManager {
    pub fn load_all(root: &Path, default_name: &str) -> Res<WorldManager> {
        let mut worlds = Vec::new();
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
            .map_err(|e| format!("{}: {e}", root.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join("level.dat").is_file() && p.join("db").is_dir())
            .collect();
        dirs.sort();
        for d in dirs {
            match World::open(&d) {
                Ok(w) => {
                    println!(
                        "[Worlds] loaded \"{}\" ({}) spawn={:?}{}",
                        w.folder,
                        w.display_name,
                        w.spawn,
                        if w.flat { " flat" } else { "" }
                    );
                    worlds.push(w);
                }
                Err(e) => eprintln!("[Worlds] failed to load {}: {e}", d.display()),
            }
        }
        if worlds.is_empty() {
            return Err(format!("no worlds found in {}", root.display()));
        }
        let default = worlds.iter().position(|w| w.folder == default_name).unwrap_or_else(|| {
            eprintln!("[Worlds] level-name \"{default_name}\" not found, using \"{}\"", worlds[0].folder);
            0
        });
        Ok(WorldManager { worlds, default })
    }
}
