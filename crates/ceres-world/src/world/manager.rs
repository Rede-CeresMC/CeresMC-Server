//! All loaded worlds (multi-world) and dynamic world creation.

use super::*;

pub struct WorldManager {
    pub root: PathBuf,
    pub worlds: Vec<World>,
    pub default: usize,
}

impl WorldManager {
    pub fn load_all(root: &Path, default_name: &str, gd: &GameData) -> Res<WorldManager> {
        std::fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
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
                        "[Worlds] loaded \"{}\" ({}) spawn={:?}{}{}",
                        w.folder,
                        w.display_name,
                        w.spawn,
                        if w.flat { " flat" } else { "" },
                        if w.void_world { " void" } else { "" }
                    );
                    worlds.push(w);
                }
                Err(e) => eprintln!("[Worlds] failed to load {}: {e}", d.display()),
            }
        }
        if worlds.is_empty() {
            println!("[Worlds] no worlds found in {}, creating flat world \"{default_name}\"...", root.display());
            let created = World::create(&root.join(default_name), default_name, false, gd)?;
            worlds.push(created);
        }
        let default = worlds.iter().position(|w| w.folder == default_name).unwrap_or_else(|| {
            eprintln!("[Worlds] level-name \"{default_name}\" not found, using \"{}\"", worlds[0].folder);
            0
        });
        Ok(WorldManager { root: root.to_path_buf(), worlds, default })
    }

    pub fn create_world(&mut self, folder: &str, void_world: bool, gd: &GameData) -> Res<usize> {
        if self.worlds.iter().any(|w| w.folder.eq_ignore_ascii_case(folder)) {
            return Err(format!("world \"{folder}\" already exists"));
        }
        let w = World::create(&self.root.join(folder), folder, void_world, gd)?;
        self.worlds.push(w);
        Ok(self.worlds.len() - 1)
    }
}
