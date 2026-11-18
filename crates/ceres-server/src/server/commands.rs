//! Commands typed by players (CommandRequest) or on the server console, with target selectors
//! (@s, @p, @a, @r) and full administration, gameplay, world and UI commands.

use super::*;
use ceres_protocol::{CommandSpec, ItemDesc, Param, ParamType, level_event, title_type, window};
use ceres_world::{Weather, extra_data};

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

fn parse_difficulty(s: &str) -> Option<(i32, &'static str)> {
    Some(match s.to_ascii_lowercase().as_str() {
        "peaceful" | "p" | "0" => (0, "Pacífico"),
        "easy" | "e" | "1" => (1, "Fácil"),
        "normal" | "n" | "2" => (2, "Normal"),
        "hard" | "h" | "3" => (3, "Difícil"),
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
            ("Weather".to_string(), ["clear", "rain", "thunder"].map(String::from).to_vec()),
            ("Difficulty".to_string(), ["peaceful", "easy", "normal", "hard"].map(String::from).to_vec()),
            ("TitleAction".to_string(), ["title", "subtitle", "actionbar", "clear"].map(String::from).to_vec()),
            ("WhitelistOp".to_string(), ["on", "off", "add", "remove", "list"].map(String::from).to_vec()),
            ("GameRule".to_string(), ["dodaylightcycle", "keepinventory", "pvp"].map(String::from).to_vec()),
            ("Enchant".to_string(), durability::ENCHANT_NAMES.iter().map(|n| n.to_string()).collect()),
            ("Effect".to_string(), effects::EFFECT_NAMES.iter().map(|(n, _)| n.to_string()).collect()),
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
            cmd("tps", "Mostra o TPS e estatísticas do servidor", vec![vec![]]),
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
                "give",
                "Dá um item a um jogador",
                vec![vec![
                    Param::new("jogador", T::Target),
                    Param::new("item", T::String),
                    Param::opt("quantidade", T::Int),
                    Param::opt("meta", T::Int),
                ]],
            ),
            cmd("clear", "Limpa o inventário", vec![vec![Param::opt("jogador", T::Target)]]),
            cmd("kill", "Mata um jogador", vec![vec![Param::opt("jogador", T::Target)]]),
            cmd(
                "enchant",
                "Encanta o item na mão",
                vec![vec![
                    Param::new("jogador", T::Target),
                    Param::new("encantamento", T::Enum("Enchant".into())),
                    Param::opt("nivel", T::Int),
                ]],
            ),
            cmd(
                "xp",
                "Dá experiência (pontos, ou níveis com L no fim: 5L)",
                vec![vec![Param::new("quantidade", T::String), Param::opt("jogador", T::Target)]],
            ),
            cmd(
                "effect",
                "Dá ou remove efeitos",
                vec![
                    vec![
                        Param::new("jogador", T::Target),
                        Param::new("efeito", T::Enum("Effect".into())),
                        Param::opt("segundos", T::Int),
                        Param::opt("nivel", T::Int),
                        Param::opt("esconderParticulas", T::String),
                    ],
                    vec![Param::new("jogador", T::Target), Param::new("clear", T::String)],
                ],
            ),
            cmd(
                "world",
                "Vai para outro mundo ou cria um mundo (create <nome> [flat|void])",
                vec![
                    vec![Param::new("mundo", T::Enum("World".into())), Param::opt("jogador", T::Target)],
                    vec![
                        Param::new("subcomando", T::String),
                        Param::new("nome", T::String),
                        Param::opt("tipo", T::String),
                    ],
                ],
            ),
            cmd(
                "time",
                "Muda a hora do mundo",
                vec![vec![Param::new("hora", T::Enum("TimeSpec".into()))], vec![Param::new("ticks", T::Int)]],
            ),
            cmd(
                "weather",
                "Muda o clima do mundo",
                vec![vec![Param::new("clima", T::Enum("Weather".into())), Param::opt("segundos", T::Int)]],
            ),
            cmd(
                "difficulty",
                "Muda a dificuldade do servidor",
                vec![vec![Param::new("dificuldade", T::Enum("Difficulty".into()))]],
            ),
            cmd("setworldspawn", "Define o spawn do mundo", vec![vec![Param::opt("pos", T::Position)]]),
            cmd(
                "spawnpoint",
                "Define o spawn pessoal de um jogador",
                vec![vec![Param::opt("jogador", T::Target), Param::opt("pos", T::Position)]],
            ),
            cmd(
                "gamerule",
                "Altera regras do jogo",
                vec![vec![Param::new("regra", T::Enum("GameRule".into())), Param::opt("valor", T::String)]],
            ),
            cmd(
                "title",
                "Exibe título ou actionbar na tela",
                vec![vec![
                    Param::new("jogador", T::Target),
                    Param::new("tipo", T::Enum("TitleAction".into())),
                    Param::opt("texto", T::Message),
                ]],
            ),
            cmd(
                "toast",
                "Envia uma notificação toast",
                vec![vec![
                    Param::new("jogador", T::Target),
                    Param::new("titulo", T::String),
                    Param::new("mensagem", T::Message),
                ]],
            ),
            cmd(
                "transfer",
                "Transfere jogador para outro servidor",
                vec![vec![
                    Param::new("jogador", T::Target),
                    Param::new("endereco", T::String),
                    Param::opt("porta", T::Int),
                ]],
            ),
            cmd(
                "kick",
                "Expulsa um jogador do servidor",
                vec![vec![Param::new("jogador", T::Target), Param::opt("motivo", T::Message)]],
            ),
            cmd(
                "ban",
                "Bane um jogador",
                vec![vec![Param::new("jogador", T::String), Param::opt("motivo", T::Message)]],
            ),
            cmd("pardon", "Desbane um jogador", vec![vec![Param::new("jogador", T::String)]]),
            cmd("banlist", "Lista os jogadores banidos", vec![vec![]]),
            cmd(
                "whitelist",
                "Gerencia a whitelist",
                vec![vec![Param::new("acao", T::Enum("WhitelistOp".into())), Param::opt("jogador", T::String)]],
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
        let visible: Vec<CommandSpec> = commands
            .into_iter()
            .filter(|c| op || matches!(c.name.as_str(), "help" | "list" | "worlds" | "tps"))
            .collect();
        let pk = protocol::available_commands(&visible, &enums);
        if let Some(p) = self.players.get_mut(&sid) {
            p.out.push(pk);
        }
    }

    pub(crate) fn broadcast_available_commands(&mut self) {
        let sids: Vec<SessionId> =
            self.players.iter().filter(|(_, p)| p.state == State::Spawned).map(|(&sid, _)| sid).collect();
        for sid in sids {
            self.send_available_commands(sid);
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

    /// Resolves selectors (`@s`, `@p`, `@a`, `@r`) or a player name into matching online sessions.
    fn resolve_targets(&mut self, sender: Sender, arg: Option<&str>) -> Vec<SessionId> {
        let spawned_sids = |srv: &Server| -> Vec<SessionId> {
            srv.players.iter().filter(|(_, p)| p.state == State::Spawned).map(|(&sid, _)| sid).collect()
        };
        match arg {
            None | Some("@s") => match sender {
                Sender::Player(sid) => vec![sid],
                Sender::Console => {
                    self.reply(sender, "§cO console precisa dizer qual jogador.");
                    Vec::new()
                }
            },
            Some("@a") => {
                let all = spawned_sids(self);
                if all.is_empty() {
                    self.reply(sender, "§cNenhum jogador online.");
                }
                all
            }
            Some("@r") => {
                let all = spawned_sids(self);
                if all.is_empty() {
                    self.reply(sender, "§cNenhum jogador online.");
                    Vec::new()
                } else {
                    let idx = (rand::random::<u32>() as usize) % all.len();
                    vec![all[idx]]
                }
            }
            Some("@p") => match sender {
                Sender::Player(me_sid) => {
                    let Some((world, pos)) = self.players.get(&me_sid).map(|p| (p.world, p.pos)) else {
                        return Vec::new();
                    };
                    self.players
                        .iter()
                        .filter(|(_, p)| p.state == State::Spawned && p.world == world)
                        .min_by(|(_, a), (_, b)| {
                            let da: f32 = (0..3).map(|i| (a.pos[i] - pos[i]).powi(2)).sum();
                            let db: f32 = (0..3).map(|i| (b.pos[i] - pos[i]).powi(2)).sum();
                            da.total_cmp(&db)
                        })
                        .map(|(&sid, _)| vec![sid])
                        .unwrap_or_default()
                }
                Sender::Console => spawned_sids(self).into_iter().take(1).collect(),
            },
            Some(name) => match self.find_player(name) {
                Some(sid) => vec![sid],
                None => {
                    self.reply(sender, &format!("§cJogador não encontrado: {name}"));
                    Vec::new()
                }
            },
        }
    }

    fn target(&mut self, sender: Sender, arg: Option<&str>) -> Option<SessionId> {
        self.resolve_targets(sender, arg).into_iter().next()
    }

    pub(crate) fn set_gamemode(&mut self, sid: SessionId, mode: i32) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        p.gamemode = mode;
        if mode != 1 && mode != 6 {
            p.flying = false;
        }
        p.out.push(protocol::set_player_game_type(mode));
        let abilities = protocol::update_abilities(&self.abilities_of(self.players.get(&sid).unwrap()));
        self.players.get_mut(&sid).unwrap().out.push(abilities);
    }

    /// Moves a player to `feet` in their current world.
    pub(crate) fn teleport(&mut self, sid: SessionId, feet: [f32; 3]) {
        self.close_container(sid, true);
        let Some(p) = self.players.get_mut(&sid) else { return };
        p.pos = [feet[0], feet[1] + EYE_HEIGHT, feet[2]];
        p.teleport_to = Some(p.pos);
        p.fall_start_y = None;
        let (eid, pos, rot, world) = (p.entity_id, p.pos, p.rot, p.world);
        let pk = protocol::move_player(eid, pos, rot[0], rot[1], rot[2], 2, false);
        p.out.push(pk.clone());
        self.broadcast(&pk, |other, q| other != sid && q.world == world);
        self.update_view(sid);
    }

    /// Moves a player to another loaded world, at its safe spawn.
    pub(crate) fn switch_world(&mut self, sid: SessionId, world: usize) {
        let Some(p) = self.players.get(&sid) else { return };
        let (old, eid) = (p.world, p.entity_id);
        if old == world {
            return;
        }
        self.broadcast(&protocol::remove_actor(eid), |other, q| other != sid && q.world == old);
        let mut gone: Vec<i64> = self
            .players
            .iter()
            .filter(|(o, q)| **o != sid && q.state == State::Spawned && q.world == old)
            .map(|(_, q)| q.entity_id)
            .collect();
        gone.extend(self.items.iter().filter(|it| it.world == old).map(|it| it.entity_id));

        let spawn = self.worlds.worlds[world].safe_spawn(&self.gd);
        let time = self.worlds.worlds[world].time as i32;
        let p = self.players.get_mut(&sid).unwrap();
        for e in gone {
            p.out.push(protocol::remove_actor(e));
        }
        p.out.push(protocol::level_event(level_event::STOP_THUNDER, [0.0; 3], 0));
        p.out.push(protocol::level_event(level_event::STOP_RAIN, [0.0; 3], 0));
        p.world = world;
        p.sent.clear();
        p.queue.clear();
        p.center = None;
        p.fall_start_y = None;
        p.pos = [spawn[0] as f32 + 0.5, spawn[1] as f32 + EYE_HEIGHT, spawn[2] as f32 + 0.5];
        p.out.push(protocol::set_time(time));
        p.out.push(protocol::set_world_spawn(spawn));
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
        let public = matches!(name.as_str(), "help" | "?" | "list" | "worlds" | "tps" | "status");
        if !public && !is_op {
            self.reply(sender, "§cVocê não tem permissão para usar este comando.");
            return;
        }
        match name.as_str() {
            "help" | "?" => {
                let (commands, _) = self.command_list();
                let list: Vec<String> = commands
                    .iter()
                    .filter(|c| is_op || matches!(c.name.as_str(), "help" | "list" | "worlds" | "tps"))
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
                        format!(
                            "§e{}§r ({} jogador{}, {} chunks)",
                            w.folder,
                            n,
                            if n == 1 { "" } else { "es" },
                            w.loaded_chunk_count()
                        )
                    })
                    .collect();
                self.reply(sender, &format!("§6Mundos:§r {}", list.join(", ")));
            }
            "tps" | "status" => {
                let avg_ms = if self.tick_ms.is_empty() {
                    0.0
                } else {
                    self.tick_ms.iter().sum::<f32>() / self.tick_ms.len() as f32
                };
                let tps = (1000.0 / avg_ms.max(50.0)).min(20.0);
                let chunks: usize = self.worlds.worlds.iter().map(|w| w.loaded_chunk_count()).sum();
                let online = self.players.values().filter(|p| p.state == State::Spawned).count();
                let up = self.started_at.elapsed().as_secs();
                self.reply(
                    sender,
                    &format!(
                        "§6TPS:§a {tps:.1} §7({avg_ms:.2} ms/tick) §6| Online:§f {online} §6| Chunks:§f {chunks} §6| Itens:§f {} §6| Uptime:§f {}m{}s",
                        self.items.len(),
                        up / 60,
                        up % 60
                    ),
                );
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
                let targets = self.resolve_targets(sender, args.get(1).copied());
                for target in targets {
                    self.set_gamemode(target, mode);
                    let tname = self.players[&target].name.clone();
                    self.reply(Sender::Player(target), &format!("§aSeu modo de jogo agora é {label}."));
                    if sender != Sender::Player(target) {
                        self.reply(sender, &format!("§aModo de jogo de {tname} agora é {label}."));
                    }
                }
            }
            "tp" | "teleport" => self.cmd_tp(sender, &args),
            "give" => self.cmd_give(sender, &args),
            "clear" => {
                let targets = self.resolve_targets(sender, args.first().copied());
                for target in targets {
                    let tname = if let Some(p) = self.players.get_mut(&target) {
                        p.inventory.clear();
                        p.name.clone()
                    } else {
                        continue;
                    };
                    self.sync_full_inventory(target);
                    self.broadcast_equipment(target);
                    self.reply(Sender::Player(target), "§aSeu inventário foi limpo.");
                    if sender != Sender::Player(target) {
                        self.reply(sender, &format!("§aInventário de {tname} limpo."));
                    }
                }
            }
            "enchant" => {
                let usage = "§cUso: /enchant <jogador> <encantamento> [nível 1-32767]";
                let (Some(target), Some(what)) = (args.first(), args.get(1)) else { return self.reply(sender, usage) };
                let name = what.strip_prefix("minecraft:").unwrap_or(what).to_ascii_lowercase();
                let id = durability::ENCHANT_NAMES
                    .iter()
                    .position(|n| *n == name)
                    .or_else(|| name.parse::<usize>().ok().filter(|i| *i < durability::ENCHANT_NAMES.len()));
                let Some(id) = id else { return self.reply(sender, "§cEncantamento desconhecido.") };
                let Ok(level) = args.get(2).map_or(Ok(1), |s| s.parse::<i16>()) else { return self.reply(sender, usage) };
                let level = level.max(1);
                let targets = self.resolve_targets(sender, Some(target));
                let mut done = 0;
                for t in targets {
                    let Some(p) = self.players.get_mut(&t) else { continue };
                    let slot = (p.inventory.held_slot as usize).min(8);
                    let mut item = p.inventory.main[slot].clone();
                    if item.is_empty() {
                        continue;
                    }
                    durability::set_enchant(&mut item, id as i16, level);
                    p.inventory.set_main(slot, item);
                    p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
                    self.broadcast_equipment(t);
                    done += 1;
                }
                if done == 0 {
                    return self.reply(sender, "§cNinguém com item na mão.");
                }
                self.reply(sender, &format!("§a{} {level} aplicado.", durability::ENCHANT_NAMES[id]));
            }
            "xp" => {
                let usage = "§cUso: /xp <quantidade>[L] [jogador]";
                let Some(amount) = args.first() else { return self.reply(sender, usage) };
                let (levels, number) = match amount.strip_suffix(['L', 'l']) {
                    Some(n) => (true, n),
                    None => (false, *amount),
                };
                let Ok(value) = number.parse::<i64>() else { return self.reply(sender, usage) };
                let value = value.clamp(-1_000_000, 1_000_000);
                let targets = self.resolve_targets(sender, args.get(1).copied());
                if targets.is_empty() {
                    return self.reply(sender, "§cNenhum jogador encontrado.");
                }
                for t in targets {
                    if levels {
                        self.add_xp_levels(t, value);
                    } else {
                        self.add_xp(t, value);
                    }
                }
                let what = if levels { "níveis" } else { "pontos de experiência" };
                self.reply(sender, &format!("§a{value} {what} dados."));
            }
            "effect" => {
                let usage = "§cUso: /effect <jogador> <efeito|clear> [segundos] [nível 0-255] [true]";
                let (Some(target), Some(what)) = (args.first(), args.get(1)) else { return self.reply(sender, usage) };
                let targets = self.resolve_targets(sender, Some(target));
                if targets.is_empty() {
                    return self.reply(sender, "§cNenhum jogador encontrado.");
                }
                if what.eq_ignore_ascii_case("clear") {
                    for t in targets {
                        self.clear_effects(t);
                    }
                    return self.reply(sender, "§aEfeitos removidos.");
                }
                let Some(id) = effects::effect_id(what) else { return self.reply(sender, "§cEfeito desconhecido.") };
                let seconds = match args.get(2).map(|s| s.parse::<u32>()) {
                    None => Some(30),
                    Some(Ok(0)) => {
                        for t in targets {
                            self.remove_effect(t, id);
                        }
                        return self.reply(sender, &format!("§aEfeito {} removido.", effects::effect_name(id)));
                    }
                    Some(Ok(s)) => Some(s),
                    Some(Err(_)) if args[2].eq_ignore_ascii_case("infinite") => None,
                    Some(Err(_)) => return self.reply(sender, usage),
                };
                let Ok(level) = args.get(3).map_or(Ok(0), |s| s.parse::<u8>()) else { return self.reply(sender, usage) };
                let particles = !args.get(4).is_some_and(|s| s.eq_ignore_ascii_case("true"));
                for t in targets {
                    self.add_effect(t, id, level, seconds, particles);
                }
                self.reply(sender, &format!("§aEfeito {} {} aplicado.", effects::effect_name(id), level + 1));
            }
            "kill" => {
                let targets = self.resolve_targets(sender, args.first().copied());
                for target in targets {
                    let Some(tname) = self.players.get(&target).map(|p| p.name.clone()) else { continue };
                    self.kill_player(target, &format!("§c{tname} morreu"));
                }
            }
            "world" | "mv" => {
                if args.first().is_some_and(|s| s.eq_ignore_ascii_case("create")) {
                    let Some(wname) = args.get(1) else {
                        return self.reply(sender, "§cUso: /world create <nome> [flat|void]");
                    };
                    let clean: String =
                        wname.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
                    if clean.is_empty() {
                        return self.reply(sender, "§cNome de mundo inválido.");
                    }
                    let void_w = args.get(2).is_some_and(|t| t.eq_ignore_ascii_case("void"));
                    match self.worlds.create_world(&clean, void_w, &self.gd) {
                        Ok(_) => {
                            self.broadcast_available_commands();
                            self.reply(
                                sender,
                                &format!(
                                    "§aMundo \"{clean}\" ({}) criado! Use /world {clean}",
                                    if void_w { "void" } else { "flat" }
                                ),
                            );
                        }
                        Err(e) => self.reply(sender, &format!("§cFalha ao criar mundo: {e}")),
                    }
                    return;
                }
                let Some(wname) = args.first() else {
                    return self.reply(sender, "§cUso: /world <mundo> [jogador] | /world create <nome> [flat|void]");
                };
                let Some(wi) = self.worlds.worlds.iter().position(|w| w.folder.eq_ignore_ascii_case(wname)) else {
                    return self.reply(sender, &format!("§cMundo não encontrado: {wname}. Veja /worlds"));
                };
                let targets = self.resolve_targets(sender, args.get(1).copied());
                let folder = self.worlds.worlds[wi].folder.clone();
                for target in targets {
                    self.switch_world(target, wi);
                    self.reply(Sender::Player(target), &format!("§aVocê foi para o mundo {folder}."));
                }
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
                let world = self.sender_world(sender);
                self.worlds.worlds[world].time = time;
                self.worlds.worlds[world].mark_level_dat_dirty();
                self.broadcast(&protocol::set_time(time as i32), |_, p| p.world == world);
                self.reply(sender, &format!("§aHora definida para {time}."));
            }
            "weather" => {
                let Some(kind) = args.first() else {
                    return self.reply(sender, "§cUso: /weather <clear|rain|thunder> [segundos]");
                };
                let secs: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(300).clamp(1, 86400);
                let world = self.sender_world(sender);
                let (w, label) = match kind.to_ascii_lowercase().as_str() {
                    "clear" | "sun" => (Weather::Clear, "limpo"),
                    "rain" => (Weather::Rain, "chuva"),
                    "thunder" => (Weather::Thunder, "tempestade"),
                    _ => return self.reply(sender, "§cUso: /weather <clear|rain|thunder> [segundos]"),
                };
                self.worlds.worlds[world].weather = w;
                self.worlds.worlds[world].weather_ticks = if w == Weather::Clear { 0 } else { secs * 20 };
                match w {
                    Weather::Clear => {
                        self.broadcast(&protocol::level_event(level_event::STOP_THUNDER, [0.0; 3], 0), |_, p| {
                            p.world == world
                        });
                        self.broadcast(&protocol::level_event(level_event::STOP_RAIN, [0.0; 3], 0), |_, p| {
                            p.world == world
                        });
                    }
                    Weather::Rain => {
                        self.broadcast(&protocol::level_event(level_event::STOP_THUNDER, [0.0; 3], 0), |_, p| {
                            p.world == world
                        });
                        self.broadcast(&protocol::level_event(level_event::START_RAIN, [0.0; 3], 65535), |_, p| {
                            p.world == world
                        });
                    }
                    Weather::Thunder => {
                        self.broadcast(&protocol::level_event(level_event::START_RAIN, [0.0; 3], 65535), |_, p| {
                            p.world == world
                        });
                        self.broadcast(&protocol::level_event(level_event::START_THUNDER, [0.0; 3], 65535), |_, p| {
                            p.world == world
                        });
                    }
                }
                self.reply(sender, &format!("§aClima alterado para {label}."));
            }
            "difficulty" => {
                let Some((diff, label)) = args.first().and_then(|d| parse_difficulty(d)) else {
                    return self.reply(sender, "§cUso: /difficulty <peaceful|easy|normal|hard>");
                };
                self.cfg.difficulty = diff;
                self.broadcast(&protocol::set_difficulty(diff as u32), |_, _| true);
                self.reply(sender, &format!("§aDificuldade alterada para {label}."));
            }
            "setworldspawn" => {
                let (world, pos) = match (args.len(), sender) {
                    (0, Sender::Player(sid)) => {
                        let Some(p) = self.players.get(&sid) else { return };
                        (
                            p.world,
                            [p.pos[0].floor() as i32, (p.pos[1] - EYE_HEIGHT).round() as i32, p.pos[2].floor() as i32],
                        )
                    }
                    (3, _) => {
                        let base = match sender {
                            Sender::Player(sid) => self
                                .players
                                .get(&sid)
                                .map(|p| [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]])
                                .unwrap_or_default(),
                            Sender::Console => [0.0; 3],
                        };
                        let (Some(x), Some(y), Some(z)) =
                            (coord(args[0], base[0]), coord(args[1], base[1]), coord(args[2], base[2]))
                        else {
                            return self.reply(sender, "§cUso: /setworldspawn [x y z]");
                        };
                        (self.sender_world(sender), [x.floor() as i32, y.floor() as i32, z.floor() as i32])
                    }
                    _ => return self.reply(sender, "§cUso: /setworldspawn [x y z]"),
                };
                self.worlds.worlds[world].spawn = pos;
                let _ = self.worlds.worlds[world].save_level_dat();
                self.broadcast(&protocol::set_world_spawn(pos), |_, p| p.world == world);
                self.reply(sender, &format!("§aSpawn do mundo definido para {} {} {}.", pos[0], pos[1], pos[2]));
            }
            "spawnpoint" => {
                let target = match args.len() {
                    0 => self.target(sender, None),
                    1 | 4 => self.target(sender, Some(args[0])),
                    _ => return self.reply(sender, "§cUso: /spawnpoint [jogador] [x y z]"),
                };
                let Some(target) = target else { return };
                let Some(p) = self.players.get(&target) else { return };
                let world = p.world;
                let base = [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]];
                let pos = if args.len() == 4 {
                    let (Some(x), Some(y), Some(z)) =
                        (coord(args[1], base[0]), coord(args[2], base[1]), coord(args[3], base[2]))
                    else {
                        return self.reply(sender, "§cUso: /spawnpoint [jogador] [x y z]");
                    };
                    [x.floor() as i32, y.floor() as i32, z.floor() as i32]
                } else {
                    [base[0].floor() as i32, base[1].round() as i32, base[2].floor() as i32]
                };
                let tname = if let Some(p) = self.players.get_mut(&target) {
                    p.spawn_point = Some((world, pos));
                    p.out.push(protocol::set_player_spawn(pos));
                    p.name.clone()
                } else {
                    return;
                };
                if let Some(p) = self.players.get(&target) {
                    self.save_player_state_of(p);
                }
                self.reply(sender, &format!("§aSpawnpoint de {tname} definido para {} {} {}.", pos[0], pos[1], pos[2]));
            }
            "gamerule" => {
                let Some(rule) = args.first() else {
                    return self.reply(sender, "§cUso: /gamerule <doDaylightCycle|keepInventory|pvp> [true|false]");
                };
                let world = self.sender_world(sender);
                let val_opt = args.get(1).map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "on" | "1"));
                match rule.to_ascii_lowercase().as_str() {
                    "dodaylightcycle" => {
                        if let Some(v) = val_opt {
                            self.worlds.worlds[world].do_daylight_cycle = v;
                            self.worlds.worlds[world].mark_level_dat_dirty();
                        }
                        self.reply(
                            sender,
                            &format!("§adoDaylightCycle = {}", self.worlds.worlds[world].do_daylight_cycle),
                        );
                    }
                    "keepinventory" => {
                        if let Some(v) = val_opt {
                            self.cfg.keep_inventory = v;
                        }
                        self.reply(sender, &format!("§akeepInventory = {}", self.cfg.keep_inventory));
                    }
                    "pvp" => {
                        if let Some(v) = val_opt {
                            self.cfg.pvp = v;
                        }
                        self.reply(sender, &format!("§apvp = {}", self.cfg.pvp));
                    }
                    _ => self.reply(sender, "§cRegra desconhecida. Disponíveis: doDaylightCycle, keepInventory, pvp"),
                }
            }
            "title" => {
                if args.len() < 2 {
                    return self.reply(sender, "§cUso: /title <jogador> <title|subtitle|actionbar|clear> [texto]");
                }
                let targets = self.resolve_targets(sender, Some(args[0]));
                let text = args[2..].join(" ");
                let pk = match args[1].to_ascii_lowercase().as_str() {
                    "clear" => protocol::set_title(title_type::CLEAR, "", 0, 0, 0),
                    "subtitle" => protocol::set_title(title_type::SUBTITLE, &text, 10, 70, 20),
                    "actionbar" => protocol::set_title(title_type::ACTIONBAR, &text, 10, 70, 20),
                    _ => protocol::set_title(title_type::TITLE, &text, 10, 70, 20),
                };
                for t in targets {
                    if let Some(p) = self.players.get_mut(&t) {
                        p.out.push(pk.clone());
                    }
                }
                self.reply(sender, "§aTítulo enviado.");
            }
            "toast" => {
                if args.len() < 3 {
                    return self.reply(sender, "§cUso: /toast <jogador> <titulo> <mensagem>");
                }
                let targets = self.resolve_targets(sender, Some(args[0]));
                let pk = protocol::toast_request(args[1], &args[2..].join(" "));
                for t in targets {
                    if let Some(p) = self.players.get_mut(&t) {
                        p.out.push(pk.clone());
                    }
                }
                self.reply(sender, "§aToast enviado.");
            }
            "transfer" => {
                if args.len() < 2 {
                    return self.reply(sender, "§cUso: /transfer <jogador> <endereco> [porta]");
                }
                let port: u16 = args.get(2).and_then(|p| p.parse().ok()).unwrap_or(19132);
                let targets = self.resolve_targets(sender, Some(args[0]));
                let pk = protocol::transfer(args[1], port);
                for t in targets {
                    if let Some(p) = self.players.get_mut(&t) {
                        p.out.push(pk.clone());
                    }
                }
                self.reply(sender, &format!("§aTransferindo para {}:{port}.", args[1]));
            }
            "kick" => {
                let Some(who) = args.first() else {
                    return self.reply(sender, "§cUso: /kick <jogador> [motivo]");
                };
                let reason = if args.len() > 1 { args[1..].join(" ") } else { "Expulso por um operador".into() };
                let targets = self.resolve_targets(sender, Some(who));
                for t in targets {
                    let tname = self.players.get(&t).map(|p| p.name.clone()).unwrap_or_default();
                    self.kick(t, &format!("§c{reason}"));
                    self.reply(sender, &format!("§a{tname} foi expulso ({reason})."));
                }
            }
            "ban" => {
                let Some(pname) = args.first() else {
                    return self.reply(sender, "§cUso: /ban <jogador> [motivo]");
                };
                let reason = if args.len() > 1 { args[1..].join(" ") } else { "Banido por um operador".into() };
                self.moderation.banned_players.insert(pname.to_ascii_lowercase(), reason.clone());
                self.moderation.save_bans();
                if let Some(sid) = self.find_player(pname) {
                    self.kick(sid, &format!("§cVocê foi banido: {reason}"));
                }
                self.reply(sender, &format!("§a{pname} foi banido ({reason})."));
            }
            "pardon" | "unban" => {
                let Some(pname) = args.first() else {
                    return self.reply(sender, "§cUso: /pardon <jogador>");
                };
                if self.moderation.banned_players.remove(&pname.to_ascii_lowercase()).is_some() {
                    self.moderation.save_bans();
                    self.reply(sender, &format!("§a{pname} foi desbanido."));
                } else {
                    self.reply(sender, &format!("§c{pname} não estava banido."));
                }
            }
            "banlist" => {
                let mut list: Vec<&String> = self.moderation.banned_players.keys().collect();
                list.sort();
                let joined = list.into_iter().cloned().collect::<Vec<_>>().join(", ");
                self.reply(sender, &format!("§6Banidos ({}):§r {}", self.moderation.banned_players.len(), joined));
            }
            "whitelist" => {
                let Some(op) = args.first() else {
                    return self.reply(sender, "§cUso: /whitelist <on|off|add|remove|list> [jogador]");
                };
                match op.to_ascii_lowercase().as_str() {
                    "on" => {
                        self.cfg.white_list = true;
                        self.reply(sender, "§aWhitelist ativada.");
                    }
                    "off" => {
                        self.cfg.white_list = false;
                        self.reply(sender, "§eWhitelist desativada.");
                    }
                    "add" => {
                        let Some(p) = args.get(1) else {
                            return self.reply(sender, "§cUso: /whitelist add <jogador>");
                        };
                        self.moderation.whitelist.insert(p.to_ascii_lowercase());
                        self.moderation.save_whitelist();
                        self.reply(sender, &format!("§a{p} adicionado à whitelist."));
                    }
                    "remove" => {
                        let Some(p) = args.get(1) else {
                            return self.reply(sender, "§cUso: /whitelist remove <jogador>");
                        };
                        self.moderation.whitelist.remove(&p.to_ascii_lowercase());
                        self.moderation.save_whitelist();
                        self.reply(sender, &format!("§e{p} removido da whitelist."));
                    }
                    "list" => {
                        let mut names: Vec<String> = self.moderation.whitelist.iter().cloned().collect();
                        names.sort();
                        self.reply(
                            sender,
                            &format!(
                                "§6Whitelist ({}, {}):§r {}",
                                if self.cfg.white_list { "ativa" } else { "inativa" },
                                names.len(),
                                names.join(", ")
                            ),
                        );
                    }
                    _ => self.reply(sender, "§cUso: /whitelist <on|off|add|remove|list> [jogador]"),
                }
            }
            "save-all" | "save" => {
                self.save_worlds();
                for w in self.worlds.worlds.iter_mut() {
                    let _ = w.save_level_dat();
                }
                self.reply(sender, "§aMundos e jogadores salvos.");
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
                    self.sync_actor_state(sid);
                    let msg = if give { "§aVocê agora é operador." } else { "§eVocê não é mais operador." };
                    self.reply(Sender::Player(sid), msg);
                }
                self.reply(sender, &format!("§a{pname} {} operador.", if give { "agora é" } else { "não é mais" }));
            }
            _ => self.reply(sender, &format!("§cComando desconhecido: /{name}. Use /help")),
        }
    }

    fn sender_world(&self, sender: Sender) -> usize {
        match sender {
            Sender::Player(sid) => self.players.get(&sid).map(|p| p.world).unwrap_or(self.worlds.default),
            Sender::Console => self.worlds.default,
        }
    }

    fn cmd_give(&mut self, sender: Sender, args: &[&str]) {
        if args.len() < 2 {
            return self.reply(sender, "§cUso: /give <jogador> <item> [quantidade] [meta]");
        }
        let Some(info) = self.gd.find_item(args[1]).cloned() else {
            return self.reply(sender, &format!("§cItem desconhecido: {}", args[1]));
        };
        let count: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1).clamp(1, 2304);
        let meta: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
        let targets = self.resolve_targets(sender, Some(args[0]));
        for target in targets {
            let item = ItemDesc {
                id: info.id,
                count,
                meta,
                stack_id: 0,
                block_runtime_id: info.block_runtime_id,
                extra: extra_data(None),
            };
            let Some(p) = self.players.get_mut(&target) else { continue };
            let tname = p.name.clone();
            let (changed, leftover) = p.inventory.add_item(item.clone(), &self.gd);
            for s in &changed {
                let idx = *s as usize;
                p.out.push(protocol::inventory_slot(window::INVENTORY as u32, idx as u32, &p.inventory.main[idx]));
            }
            let held_touched = changed.contains(&p.inventory.held_slot);
            if held_touched {
                self.broadcast_equipment(target);
            }
            if leftover > 0 {
                let mut rem = item;
                rem.count = leftover;
                self.drop_from_player(target, rem);
            }
            self.reply(sender, &format!("§aDado {count}x {} para {tname}.", info.name));
        }
    }

    fn cmd_tp(&mut self, sender: Sender, args: &[&str]) {
        let usage = "§cUso: /tp <x y z> | /tp <jogador> | /tp <jogador> <x y z|jogador>";
        let (targets, rest): (Vec<SessionId>, &[&str]) = match args.len() {
            1 | 3 => (self.resolve_targets(sender, None), args),
            2 | 4 => (self.resolve_targets(sender, Some(args[0])), &args[1..]),
            _ => return self.reply(sender, usage),
        };
        for who in targets {
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
