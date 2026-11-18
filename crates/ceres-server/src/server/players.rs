//! Multiplayer: seeing other players (player list, AddPlayer, movement), joining and leaving, chat.

use super::*;
use ceres_protocol::{Abilities, PlayerInfo};

/// Players in the same world see each other's movement and spawn.
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

    pub(crate) fn abilities_of(&self, p: &Player) -> Abilities {
        Abilities {
            entity_id: p.entity_id,
            operator: p.op,
            creative: p.gamemode == 1,
            spectator: p.gamemode == 6,
            flying: false,
        }
    }

    fn add_player_packet(&self, p: &Player) -> Option<Vec<u8>> {
        let info = p.info.as_ref()?;
        let feet = [p.pos[0], p.pos[1] - EYE_HEIGHT, p.pos[2]];
        Some(protocol::add_player(info, feet, p.rot[0], p.rot[1], p.rot[2], p.gamemode))
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

    /// The client finished loading: show it to everyone and everyone to it.
    pub(crate) fn on_spawned(&mut self, sid: SessionId) {
        let Some(me) = self.players.get(&sid) else { return };
        let Some(info) = me.info.clone() else { return };
        let world = me.world;
        let list_add = protocol::player_list_add(&[&info]);
        let add_me = self.add_player_packet(me);

        // what the newcomer has to see
        let others: Vec<Vec<u8>> = self
            .players
            .iter()
            .filter(|(other, p)| **other != sid && p.state == State::Spawned && p.world == world)
            .filter_map(|(_, p)| self.add_player_packet(p))
            .collect();

        self.broadcast(&list_add, |other, _| other != sid);
        if let Some(add_me) = add_me {
            self.broadcast(&add_me, |other, p| other != sid && p.world == world);
        }
        let me = self.players.get_mut(&sid).unwrap();
        me.out.extend(others);

        let msg = protocol::system_message(&format!("§e{} entrou no jogo", info.name));
        self.broadcast(&msg, |_, _| true);
    }

    /// AddPlayer between `sid` and everyone already in its world (used when changing worlds).
    pub(crate) fn show_players_in_world(&mut self, sid: SessionId) {
        let Some(me) = self.players.get(&sid) else { return };
        let world = me.world;
        let add_me = self.add_player_packet(me);
        let others: Vec<Vec<u8>> = self
            .players
            .iter()
            .filter(|(other, p)| **other != sid && p.state == State::Spawned && p.world == world)
            .filter_map(|(_, p)| self.add_player_packet(p))
            .collect();
        if let Some(add_me) = add_me {
            self.broadcast(&add_me, |other, p| other != sid && p.world == world);
        }
        self.players.get_mut(&sid).unwrap().out.extend(others);
    }

    /// The session is gone: remove it from everyone's list and world.
    pub(crate) fn on_left(&mut self, p: &Player) {
        let Some(info) = &p.info else { return };
        if p.state != State::Spawned {
            return;
        }
        let world = p.world;
        self.broadcast(&protocol::player_list_remove(&info.uuid), |_, _| true);
        self.broadcast(&protocol::remove_actor(info.entity_id), |_, q| q.world == world);
        self.broadcast(&protocol::system_message(&format!("§e{} saiu do jogo", info.name)), |_, _| true);
    }

    /// PlayerAuthInput: pitch, yaw, position (eyes), move vector, head yaw.
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
        let moved = p.pos != pos || p.rot != [pitch, yaw, head_yaw];
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
                if let Some(u) = tail.use_item {
                    self.on_use_item(sid, u);
                }
                for (action, pos, _face) in tail.block_actions {
                    self.on_block_action(sid, action, pos);
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

    pub(crate) fn on_text(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let (ty, msg) = protocol::read_text(r)?;
        if ty != 1 {
            return Ok(());
        }
        let msg: String = msg.chars().filter(|c| !c.is_control()).take(512).collect();
        if msg.trim().is_empty() {
            return Ok(());
        }
        let Some(name) = self.players.get(&sid).map(|p| p.name.clone()) else { return Ok(()) };
        println!("<{name}> {msg}");
        self.broadcast(&protocol::chat(&name, &msg), |_, _| true);
        Ok(())
    }
}
