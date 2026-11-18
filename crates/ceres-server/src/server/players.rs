//! Multiplayer: seeing other players (player list, AddPlayer, equipment, movement, animations,
//! sneaking/sprinting/swimming/gliding, emotes, skin changes), joining/leaving and chat.

use super::*;
use ceres_protocol::{Abilities, PlayerInfo, input_flag, level_event};
use ceres_world::Weather;

const EYE_HEIGHT: f32 = 1.62;

impl Server {
    /// Queues a packet for every spawned player matching `filter`.
    pub(crate) fn broadcast(&mut self, pk: &[u8], filter: impl Fn(SessionId, &Player) -> bool) {
        for (sid, p) in self.players.iter_mut() {
            if p.state == State::Spawned && filter(*sid, p) {
                p.out.push(pk.to_vec());
            }
        }
    }

    pub(crate) fn nametag_of(&self, p: &Player) -> String {
        if p.op { format!("§c[OP] §f{}", p.name) } else { p.name.clone() }
    }

    pub(crate) fn abilities_of(&self, p: &Player) -> Abilities {
        Abilities {
            entity_id: p.entity_id,
            operator: p.op,
            creative: p.gamemode == 1,
            spectator: p.gamemode == 6,
            flying: p.flying && (p.gamemode == 1 || p.gamemode == 6),
        }
    }

    pub(crate) fn sync_actor_state(&mut self, sid: SessionId) {
        let Some(p) = self.players.get(&sid) else { return };
        let (world, eid, state) = (p.world, p.entity_id, p.actor_state);
        let tag = self.nametag_of(p);
        let pk = protocol::set_actor_data_state(eid, &tag, &state);
        if let Some(me) = self.players.get_mut(&sid) {
            me.out.push(pk.clone());
        }
        self.broadcast(&pk, |other, q| other != sid && q.world == world);
    }

    pub(crate) fn swing_arm(&mut self, sid: SessionId) {
        let tick = self.ticks;
        let Some(p) = self.players.get_mut(&sid) else { return };
        if p.state != State::Spawned || p.last_swing_tick == tick {
            return;
        }
        p.last_swing_tick = tick;
        let (world, eid) = (p.world, p.entity_id);
        let pk = protocol::animate_swing(eid);
        self.broadcast(&pk, |other, q| other != sid && q.world == world);
    }

    fn add_player_bundle(&self, p: &Player) -> Option<Vec<Vec<u8>>> {
        let info = p.info.as_ref()?;
        let feet = [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]];
        let nametag = self.nametag_of(p);
        let abilities = Abilities {
            entity_id: p.entity_id,
            operator: false,
            creative: false,
            spectator: p.gamemode == 6,
            flying: false,
        };
        let mut out = vec![protocol::add_player(
            info,
            feet,
            p.rot[0],
            p.rot[1],
            p.rot[2],
            p.gamemode,
            p.inventory.held_item(),
            &nametag,
            &p.actor_state,
            &abilities,
        )];
        out.extend(self.equipment_packets_of(p));
        Some(out)
    }

    /// Sends current world weather state to `sid`.
    pub(crate) fn send_weather_to(&mut self, sid: SessionId) {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return };
        let weather = self.worlds.worlds[world].weather;
        let Some(p) = self.players.get_mut(&sid) else { return };
        match weather {
            Weather::Clear => {}
            Weather::Rain => {
                p.out.push(protocol::level_event(level_event::START_RAIN, [0.0; 3], 65535));
            }
            Weather::Thunder => {
                p.out.push(protocol::level_event(level_event::START_RAIN, [0.0; 3], 65535));
                p.out.push(protocol::level_event(level_event::START_THUNDER, [0.0; 3], 65535));
            }
        }
    }

    /// Sent during StartGame: abilities plus the player list (everyone online and ourselves).
    pub(crate) fn send_pre_spawn_players(&mut self, sid: SessionId) {
        let Some(me) = self.players.get(&sid) else { return };
        let abilities = protocol::update_abilities(&self.abilities_of(me));
        let infos: Vec<&PlayerInfo> = self
            .players
            .iter()
            .filter(|(other, p)| **other == sid || p.state == State::Spawned)
            .filter_map(|(_, p)| p.info.as_ref())
            .collect();
        let list = protocol::player_list_add(&infos);
        let me = self.players.get_mut(&sid).unwrap();
        me.out.push(abilities);
        me.out.push(list);
    }

    /// The client finished loading: show it to everyone and everyone (+ world items + weather) to it.
    pub(crate) fn on_spawned(&mut self, sid: SessionId) {
        let Some(me) = self.players.get(&sid) else { return };
        let Some(info) = me.info.clone() else { return };
        let world = me.world;
        let list_add = protocol::player_list_add(&[&info]);
        let my_bundle = self.add_player_bundle(me);

        let others: Vec<Vec<u8>> = self
            .players
            .iter()
            .filter(|(other, p)| **other != sid && p.state == State::Spawned && p.world == world)
            .filter_map(|(_, p)| self.add_player_bundle(p))
            .flatten()
            .collect();

        self.broadcast(&list_add, |other, _| other != sid);
        if let Some(bundle) = my_bundle {
            for pk in bundle {
                self.broadcast(&pk, |other, p| other != sid && p.world == world);
            }
        }
        let me = self.players.get_mut(&sid).unwrap();
        me.out.extend(others);

        self.send_world_items(sid);
        self.send_weather_to(sid);

        let msg = protocol::system_message(&format!("§e{} entrou no jogo", info.name));
        self.broadcast(&msg, |_, _| true);
    }

    /// AddPlayer + equipment between `sid` and everyone already in its world (used when changing worlds).
    pub(crate) fn show_players_in_world(&mut self, sid: SessionId) {
        let Some(me) = self.players.get(&sid) else { return };
        let world = me.world;
        let my_bundle = self.add_player_bundle(me);
        let others: Vec<Vec<u8>> = self
            .players
            .iter()
            .filter(|(other, p)| **other != sid && p.state == State::Spawned && p.world == world)
            .filter_map(|(_, p)| self.add_player_bundle(p))
            .flatten()
            .collect();
        if let Some(bundle) = my_bundle {
            for pk in bundle {
                self.broadcast(&pk, |other, p| other != sid && p.world == world);
            }
        }
        self.players.get_mut(&sid).unwrap().out.extend(others);
        self.send_world_items(sid);
        self.send_weather_to(sid);
    }

    /// The session is gone: save its data and remove it from everyone's list and world.
    pub(crate) fn on_left(&mut self, p: &Player) {
        self.save_player_state_of(p);
        let Some(info) = &p.info else { return };
        if p.state != State::Spawned {
            return;
        }
        let world = p.world;
        self.broadcast(&protocol::player_list_remove(&info.uuid), |_, _| true);
        self.broadcast(&protocol::remove_actor(info.entity_id), |_, q| q.world == world);
        self.broadcast(&protocol::system_message(&format!("§e{} saiu do jogo", info.name)), |_, _| true);
    }

    /// PlayerAuthInput: pitch, yaw, position (eyes), move vector, head yaw + flags/actions.
    pub(crate) fn on_input(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let pitch = r.f32_le()?;
        let yaw = r.f32_le()?;
        let pos = [r.f32_le()?, r.f32_le()?, r.f32_le()?];
        let _move_x = r.f32_le()?;
        let _move_z = r.f32_le()?;
        let head_yaw = r.f32_le()?;
        if !pos.iter().chain([pitch, yaw, head_yaw].iter()).all(|v| v.is_finite()) {
            return Ok(());
        }
        let Some(p) = self.players.get_mut(&sid) else { return Ok(()) };
        if p.dead {
            return Ok(());
        }
        if let Some(t) = p.teleport_to {
            let d2: f32 = (0..3).map(|i| (pos[i] - t[i]).powi(2)).sum();
            if d2 > 1.0 {
                return Ok(()); // still moving from before the teleport
            }
            p.teleport_to = None;
        }
        let moved = p.pos != pos || p.rot != [pitch, yaw, head_yaw];
        if p.actor_state.sprinting && (p.gamemode == 0 || p.gamemode == 2) {
            let d = ((pos[0] - p.pos[0]).powi(2) + (pos[2] - p.pos[2]).powi(2)).sqrt();
            if d < 2.0 {
                p.exhaustion = (p.exhaustion + d * food::exhaustion::SPRINT_PER_BLOCK).min(40.0);
            }
        }
        p.pos = pos;
        p.rot = [pitch, yaw, head_yaw];
        let (world, eid, spawned) = (p.world, p.entity_id, p.state == State::Spawned);
        if moved && spawned {
            let pk = protocol::move_player(eid, pos, pitch, yaw, head_yaw, 0, true);
            self.broadcast(&pk, |other, q| other != sid && q.world == world);
        }
        self.update_view(sid);
        if !spawned {
            return Ok(());
        }
        match protocol::read_auth_input_tail(r) {
            Ok(tail) => {
                self.apply_input_flags(sid, &tail);
                if let Some(u) = tail.use_item {
                    self.on_use_item(sid, u);
                }
                if let Some(req) = tail.stack_request {
                    self.handle_single_stack_request(sid, req);
                }
                for (action, bpos, _face) in tail.block_actions {
                    self.on_block_action(sid, action, bpos);
                }
            }
            Err(e) => {
                if ceres_net::debug_enabled() {
                    eprintln!("[Debug] {sid}: could not read PlayerAuthInput tail: {e}");
                }
            }
        }
        Ok(())
    }

    fn apply_input_flags(&mut self, sid: SessionId, tail: &protocol::AuthInputTail) {
        if tail.flags.is_empty() {
            return;
        }
        if tail.has_flag(input_flag::MISSED_SWING) {
            self.swing_arm(sid);
        }
        if tail.has_flag(input_flag::START_JUMPING) {
            let sprinting = self.players.get(&sid).is_some_and(|p| p.actor_state.sprinting);
            self.exhaust(sid, if sprinting { food::exhaustion::SPRINT_JUMP } else { food::exhaustion::JUMP });
        }
        let Some(p) = self.players.get_mut(&sid) else { return };
        let old = p.actor_state;
        if tail.has_flag(input_flag::START_SNEAKING) {
            p.actor_state.sneaking = true;
        } else if tail.has_flag(input_flag::STOP_SNEAKING) {
            p.actor_state.sneaking = false;
        }
        if tail.has_flag(input_flag::START_SPRINTING) {
            p.actor_state.sprinting = true;
        } else if tail.has_flag(input_flag::STOP_SPRINTING) {
            p.actor_state.sprinting = false;
        }
        if tail.has_flag(input_flag::START_SWIMMING) {
            p.actor_state.swimming = true;
        } else if tail.has_flag(input_flag::STOP_SWIMMING) {
            p.actor_state.swimming = false;
        }
        if tail.has_flag(input_flag::START_GLIDING) {
            p.actor_state.gliding = true;
        } else if tail.has_flag(input_flag::STOP_GLIDING) {
            p.actor_state.gliding = false;
        }
        if tail.has_flag(input_flag::START_CRAWLING) {
            p.actor_state.crawling = true;
        } else if tail.has_flag(input_flag::STOP_CRAWLING) {
            p.actor_state.crawling = false;
        }
        if tail.has_flag(input_flag::START_FLYING) && (p.gamemode == 1 || p.gamemode == 6) {
            p.flying = true;
            p.fall_start_y = None;
        } else if tail.has_flag(input_flag::STOP_FLYING) {
            p.flying = false;
        }
        if p.actor_state != old {
            self.sync_actor_state(sid);
        }
    }

    pub(crate) fn on_animate(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let (action, _eid) = protocol::read_animate(r)?;
        if action == 1 {
            self.swing_arm(sid);
        }
        Ok(())
    }

    pub(crate) fn on_emote(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let (emote_id, ticks, flags) = protocol::read_emote(r)?;
        if emote_id.len() > 128 {
            return Ok(());
        }
        let Some(p) = self.players.get(&sid) else { return Ok(()) };
        let (world, eid) = (p.world, p.entity_id);
        let xuid = p.info.as_ref().map(|i| i.xuid.clone()).unwrap_or_default();
        let pk = protocol::emote(eid, &emote_id, ticks, &xuid, "", flags | 1);
        self.broadcast(&pk, |other, q| other != sid && q.world == world);
        Ok(())
    }

    pub(crate) fn on_player_skin(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let (skin_bytes, old_name, new_name) = protocol::read_player_skin(r)?;
        if skin_bytes.len() > 1024 * 1024 || old_name.len() > 64 || new_name.len() > 64 {
            return Ok(());
        }
        let Some(p) = self.players.get_mut(&sid) else { return Ok(()) };
        // every change is sent to everyone: at most one every 10 seconds
        if p.last_skin_change.is_some_and(|t| t.elapsed().as_secs() < 10) {
            return Ok(());
        }
        p.last_skin_change = Some(Instant::now());
        let Some(info) = p.info.as_mut() else { return Ok(()) };
        info.skin = std::rc::Rc::new(skin_bytes.clone());
        let uuid = info.uuid;
        let pk = protocol::player_skin(&uuid, &skin_bytes, &old_name, &new_name);
        self.broadcast(&pk, |_, _| true);
        Ok(())
    }

    pub(crate) fn on_text(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let (ty, msg) = protocol::read_text(r)?;
        if ty != 1 {
            return Ok(());
        }
        let msg: String = msg.chars().filter(|c| !c.is_control()).take(512).collect();
        if msg.trim().is_empty() {
            return Ok(());
        }
        let Some((name, op)) = self.players.get(&sid).map(|p| (p.name.clone(), p.op)) else { return Ok(()) };
        println!("<{name}> {msg}");
        let display = if op { format!("§c[OP] §f{name}") } else { name };
        self.broadcast(&protocol::chat(&display, &msg), |_, _| true);
        Ok(())
    }
}
