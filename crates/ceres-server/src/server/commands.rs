//! Commands typed by players (CommandRequest) or on the server console.

use super::*;
use ceres_protocol::{CommandSpec, Param, ParamType};

const EYE_HEIGHT: f32 = 1.62;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sender {
    Player(SessionId),
    Console,
}

fn parse_gamemode(s: &str) -> Option<(i32, &'static str)> {
    Some(match s.to_ascii_lowercase().as_str() {
        "survival" | "s" | "0" => (0, "Sobrevivência"),
        "creative" | "c" | "1" => (1, "Criativo"),
        "adventure" | "a" | "2" => (2, "Aventura"),
        "spectator" | "sp" | "3" | "6" => (6, "Espectador"),
        _ => return None,
    })
}

/// "~", "~5", "-3.5" relative to `base`.
fn coord(s: &str, base: f32) -> Option<f32> {
    match s.strip_prefix('~') {
        Some("") => Some(base),
        Some(rest) => rest.parse::<f32>().ok().map(|v| base + v),
        None => s.parse::<f32>().ok().filter(|v| v.is_finite()),
    }
}

impl Server {
    fn command_list(&self) -> (Vec<CommandSpec>, Vec<(String, Vec<String>)>) {
        use ParamType as T;
        let worlds: Vec<String> = self.worlds.worlds.iter().map(|w| w.folder.clone()).collect();
        let enums = vec![
            (
                "GameMode".to_string(),
                ["survival", "creative", "adventure", "spectator", "s", "c", "a", "sp"].map(String::from).to_vec(),
            ),
            ("World".to_string(), worlds),
            ("TimeSpec".to_string(), ["day", "noon", "night", "midnight"].map(String::from).to_vec()),
        ];
        let cmd = |name: &str, description: &str, overloads: Vec<Vec<Param>>| CommandSpec {
            name: name.into(),
            description: description.into(),
            overloads,
        };
        let commands = vec![
            cmd("help", "Lista os comandos", vec![vec![]]),
            cmd("list", "Mostra quem está online", vec![vec![]]),
            cmd("worlds", "Lista os mundos carregados", vec![vec![]]),
            cmd("say", "Manda uma mensagem para todos", vec![vec![Param::new("mensagem", T::Message)]]),
            cmd(
                "gamemode",
                "Muda o modo de jogo",
                vec![vec![Param::new("modo", T::Enum("GameMode".into())), Param::opt("jogador", T::Target)]],
            ),
            cmd(
                "tp",
                "Teleporta",
                vec![
                    vec![Param::new("destino", T::Position)],
                    vec![Param::new("destino", T::Target)],
                    vec![Param::new("jogador", T::Target), Param::new("destino", T::Position)],
                    vec![Param::new("jogador", T::Target), Param::new("destino", T::Target)],
                ],
            ),
            cmd(
                "world",
                "Vai para outro mundo",
                vec![vec![Param::new("mundo", T::Enum("World".into())), Param::opt("jogador", T::Target)]],
            ),
            cmd(
                "time",
                "Muda a hora do mundo",
                vec![vec![Param::new("hora", T::Enum("TimeSpec".into()))], vec![Param::new("ticks", T::Int)]],
            ),
            cmd("save-all", "Salva todos os mundos", vec![vec![]]),
            cmd("stop", "Salva e desliga o servidor", vec![vec![]]),
            cmd("op", "Dá permissão de operador", vec![vec![Param::new("jogador", T::Target)]]),
            cmd("deop", "Tira a permissão de operador", vec![vec![Param::new("jogador", T::Target)]]),
        ];
        (commands, enums)
    }

    pub(crate) fn send_available_commands(&mut self, sid: SessionId) {
        let (commands, enums) = self.command_list();
        let op = self.players.get(&sid).is_some_and(|p| p.op);
        let visible: Vec<CommandSpec> =
            commands.into_iter().filter(|c| op || matches!(c.name.as_str(), "help" | "list" | "worlds")).collect();
        let pk = protocol::available_commands(&visible, &enums);
        if let Some(p) = self.players.get_mut(&sid) {
            p.out.push(pk);
        }
    }

    pub(crate) fn reply(&mut self, to: Sender, msg: &str) {
        match to {
            Sender::Console => println!("{}", strip_colors(msg)),
            Sender::Player(sid) => {
                if let Some(p) = self.players.get_mut(&sid) {
                    p.out.push(protocol::system_message(msg));
                }
            }
        }
    }

    fn find_player(&self, name: &str) -> Option<SessionId> {
        let name = name.trim_start_matches('@').to_ascii_lowercase();
        let spawned = || self.players.iter().filter(|(_, p)| p.state == State::Spawned);
        spawned()
            .find(|(_, p)| p.name.to_ascii_lowercase() == name)
            .or_else(|| spawned().find(|(_, p)| p.name.to_ascii_lowercase().starts_with(&name)))
            .map(|(sid, _)| *sid)
    }

    /// A player argument, or the sender when it is omitted ("@s" too).
    fn target(&mut self, sender: Sender, arg: Option<&str>) -> Option<SessionId> {
        match (arg, sender) {
            (None | Some("@s"), Sender::Player(sid)) => Some(sid),
            (None | Some("@s"), Sender::Console) => {
                self.reply(sender, "§cO console precisa dizer qual jogador.");
                None
            }
            (Some(name), _) => {
                let found = self.find_player(name);
                if found.is_none() {
                    self.reply(sender, &format!("§cJogador não encontrado: {name}"));
                }
                found
            }
        }
    }

    pub(crate) fn set_gamemode(&mut self, sid: SessionId, mode: i32) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        p.gamemode = mode;
        p.out.push(protocol::set_player_game_type(mode));
        let abilities = protocol::update_abilities(&self.abilities_of(self.players.get(&sid).unwrap()));
        self.players.get_mut(&sid).unwrap().out.push(abilities);
    }

    /// Moves a player to `feet` in their current world.
    pub(crate) fn teleport(&mut self, sid: SessionId, feet: [f32; 3]) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        p.pos = [feet[0], feet[1] + EYE_HEIGHT, feet[2]];
        p.teleport_to = Some(p.pos);
        let (eid, pos, rot, world) = (p.entity_id, p.pos, p.rot, p.world);
        let pk = protocol::move_player(eid, pos, rot[0], rot[1], rot[2], 2, false);
        p.out.push(pk.clone());
        self.broadcast(&pk, |other, q| other != sid && q.world == world);
        self.update_view(sid);
    }

    /// Moves a player to another loaded world, at its spawn.
    pub(crate) fn switch_world(&mut self, sid: SessionId, world: usize) {
        let Some(p) = self.players.get(&sid) else { return };
        let (old, eid) = (p.world, p.entity_id);
        if old == world {
            return;
        }
        // forget the old world: its players for us, us for them
        self.broadcast(&protocol::remove_actor(eid), |other, q| other != sid && q.world == old);
        let gone: Vec<i64> = self
            .players
            .iter()
            .filter(|(o, q)| **o != sid && q.state == State::Spawned && q.world == old)
            .map(|(_, q)| q.entity_id)
            .collect();
        let mut spawn = self.worlds.worlds[world].spawn;
        if !(-64..=320).contains(&spawn[1]) {
            spawn[1] = 100;
        }
        let time = self.worlds.worlds[world].time as i32;
        let p = self.players.get_mut(&sid).unwrap();
        for e in gone {
            p.out.push(protocol::remove_actor(e));
        }
        p.world = world;
        p.sent.clear();
        p.queue.clear();
        p.center = None;
        p.pos = [spawn[0] as f32 + 0.5, spawn[1] as f32 + EYE_HEIGHT, spawn[2] as f32 + 0.5];
        p.out.push(protocol::set_time(time));
        self.show_players_in_world(sid);
        self.teleport(sid, [spawn[0] as f32 + 0.5, spawn[1] as f32, spawn[2] as f32 + 0.5]);
    }

    pub(crate) fn run_command(&mut self, sender: Sender, line: &str) {
        let line = line.trim().trim_start_matches('/');
        let mut args = line.split_whitespace();
        let Some(name) = args.next().map(|s| s.to_ascii_lowercase()) else { return };
        let args: Vec<&str> = args.collect();
        let is_op = match sender {
            Sender::Console => true,
            Sender::Player(sid) => self.players.get(&sid).is_some_and(|p| p.op),
        };
        if let Sender::Player(sid) = sender {
            if let Some(p) = self.players.get(&sid) {
                println!("[Command] {}: /{line}", p.name);
            }
        }
        let public = matches!(name.as_str(), "help" | "list" | "worlds");
        if !public && !is_op {
            self.reply(sender, "§cVocê não tem permissão para usar este comando.");
            return;
        }
        match name.as_str() {
            "help" | "?" => {
                let (commands, _) = self.command_list();
                let list: Vec<String> = commands
                    .iter()
                    .filter(|c| is_op || matches!(c.name.as_str(), "help" | "list" | "worlds"))
                    .map(|c| format!("§e/{}§r - {}", c.name, c.description))
                    .collect();
                self.reply(sender, &format!("§6Comandos:\n{}", list.join("\n")));
            }
            "list" => {
                let names: Vec<String> =
                    self.players.values().filter(|p| p.state == State::Spawned).map(|p| p.name.clone()).collect();
                self.reply(
                    sender,
                    &format!("§6Online ({}/{}):§r {}", names.len(), self.cfg.max_players, names.join(", ")),
                );
            }
            "worlds" => {
                let list: Vec<String> = self
                    .worlds
                    .worlds
                    .iter()
                    .enumerate()
                    .map(|(i, w)| {
                        let n = self.players.values().filter(|p| p.world == i && p.state == State::Spawned).count();
                        format!("§e{}§r ({} jogador{})", w.folder, n, if n == 1 { "" } else { "es" })
                    })
                    .collect();
                self.reply(sender, &format!("§6Mundos:§r {}", list.join(", ")));
            }
            "say" => {
                let who = match sender {
                    Sender::Console => "Servidor".to_string(),
                    Sender::Player(sid) => self.players.get(&sid).map(|p| p.name.clone()).unwrap_or_default(),
                };
                let msg = protocol::system_message(&format!("§d[{who}] {}", args.join(" ")));
                self.broadcast(&msg, |_, _| true);
                println!("[{who}] {}", args.join(" "));
            }
            "gamemode" | "gm" => {
                let Some((mode, label)) = args.first().and_then(|m| parse_gamemode(m)) else {
                    return self.reply(sender, "§cUso: /gamemode <survival|creative|adventure|spectator> [jogador]");
                };
                let Some(target) = self.target(sender, args.get(1).copied()) else { return };
                self.set_gamemode(target, mode);
                let name = self.players[&target].name.clone();
                self.reply(Sender::Player(target), &format!("§aSeu modo de jogo agora é {label}."));
                if sender != Sender::Player(target) {
                    self.reply(sender, &format!("§aModo de jogo de {name} agora é {label}."));
                }
            }
            "tp" | "teleport" => self.cmd_tp(sender, &args),
            "world" | "mv" => {
                let Some(wname) = args.first() else { return self.reply(sender, "§cUso: /world <mundo> [jogador]") };
                let Some(wi) = self.worlds.worlds.iter().position(|w| w.folder.eq_ignore_ascii_case(wname)) else {
                    return self.reply(sender, &format!("§cMundo não encontrado: {wname}. Veja /worlds"));
                };
                let Some(target) = self.target(sender, args.get(1).copied()) else { return };
                let folder = self.worlds.worlds[wi].folder.clone();
                self.switch_world(target, wi);
                self.reply(Sender::Player(target), &format!("§aVocê foi para o mundo {folder}."));
            }
            "time" => {
                let spec = if args.first() == Some(&"set") { args.get(1) } else { args.first() };
                let time = match spec.copied() {
                    Some("day") => 1000,
                    Some("noon") => 6000,
                    Some("night") => 13000,
                    Some("midnight") => 18000,
                    Some(n) => match n.parse::<i64>() {
                        Ok(v) => v,
                        Err(_) => return self.reply(sender, "§cUso: /time <day|noon|night|midnight|ticks>"),
                    },
                    None => return self.reply(sender, "§cUso: /time <day|noon|night|midnight|ticks>"),
                };
                let world = match sender {
                    Sender::Player(sid) => self.players.get(&sid).map(|p| p.world).unwrap_or(self.worlds.default),
                    Sender::Console => self.worlds.default,
                };
                self.worlds.worlds[world].time = time;
                self.broadcast(&protocol::set_time(time as i32), |_, p| p.world == world);
                self.reply(sender, &format!("§aHora definida para {time}."));
            }
            "save-all" | "save" => {
                self.save_worlds();
                self.reply(sender, "§aMundos salvos.");
            }
            "stop" => {
                self.reply(sender, "§cDesligando o servidor...");
                self.shutdown("O servidor foi desligado");
            }
            "op" | "deop" => {
                let Some(pname) = args.first() else {
                    return self.reply(sender, &format!("§cUso: /{name} <jogador>"));
                };
                let give = name == "op";
                let lower = pname.to_ascii_lowercase();
                if give {
                    self.ops.insert(lower.clone());
                } else {
                    self.ops.remove(&lower);
                }
                self.save_ops();
                let effective = give && self.cfg.allow_unverified_ops;
                if give && !effective {
                    self.reply(
                        sender,
                        "§eSalvo no ops.txt, mas só vale depois da verificação Xbox (ou com allow-unverified-ops=on).",
                    );
                }
                if let Some(sid) = self.find_player(pname) {
                    if let Some(p) = self.players.get_mut(&sid) {
                        p.op = effective;
                    }
                    let abilities = protocol::update_abilities(&self.abilities_of(self.players.get(&sid).unwrap()));
                    self.players.get_mut(&sid).unwrap().out.push(abilities);
                    self.send_available_commands(sid);
                    let msg = if give { "§aVocê agora é operador." } else { "§eVocê não é mais operador." };
                    self.reply(Sender::Player(sid), msg);
                }
                self.reply(sender, &format!("§a{pname} {} operador.", if give { "agora é" } else { "não é mais" }));
            }
            _ => self.reply(sender, &format!("§cComando desconhecido: /{name}. Use /help")),
        }
    }

    fn cmd_tp(&mut self, sender: Sender, args: &[&str]) {
        let usage = "§cUso: /tp <x y z> | /tp <jogador> | /tp <jogador> <x y z|jogador>";
        // who moves, and where the rest of the arguments start
        let (who, rest): (Option<SessionId>, &[&str]) = match args.len() {
            1 | 3 => (self.target(sender, None), args),
            2 | 4 => (self.target(sender, Some(args[0])), &args[1..]),
            _ => return self.reply(sender, usage),
        };
        let Some(who) = who else { return };
        let base = self.players.get(&who).map(|p| [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]).unwrap_or_default();
        let feet = if rest.len() == 3 {
            match (coord(rest[0], base[0]), coord(rest[1], base[1]), coord(rest[2], base[2])) {
                (Some(x), Some(y), Some(z)) => [x, y, z],
                _ => return self.reply(sender, usage),
            }
        } else {
            let Some(dest) = self.target(sender, Some(rest[0])) else { return };
            let (dest_world, dpos) = match self.players.get(&dest) {
                Some(p) => (p.world, [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]]),
                None => return,
            };
            if self.players.get(&who).is_some_and(|p| p.world != dest_world) {
                self.switch_world(who, dest_world);
            }
            dpos
        };
        self.teleport(who, feet);
        let name = self.players.get(&who).map(|p| p.name.clone()).unwrap_or_default();
        self.reply(sender, &format!("§aTeleportado {name} para {:.1} {:.1} {:.1}.", feet[0], feet[1], feet[2]));
    }
}

/// Removes § colour codes for the console.
pub(crate) fn strip_colors(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut skip = false;
    for c in s.chars() {
        if skip {
            skip = false;
        } else if c == '§' {
            skip = true;
        } else {
            out.push(c);
        }
    }
    out
}
