//! Packet handling: login flow, moderation checks, resource packs, StartGame, input, equipment,
//! combat, signs and chat.

use super::*;
use base64::Engine;
use ceres_protocol::{InvTransaction, entity_use_action, window};

impl Server {
    pub(crate) fn handle_packet(&mut self, sid: SessionId, buf: &[u8]) -> Res<()> {
        let mut r = R::new(buf);
        let pid = r.uvarint()? & 0x3ff;
        let Some(p) = self.players.get_mut(&sid) else { return Ok(()) };
        if ceres_net::debug_enabled() && pid != id::PLAYER_AUTH_INPUT {
            println!("[Debug] {sid} <- 0x{pid:02x} ({} bytes) in {:?}", buf.len(), p.state);
        }
        match (p.state, pid) {
            (State::AwaitNetworkSettings, id::REQUEST_NETWORK_SETTINGS) => {
                let proto = r.u32_be()?;
                if proto != protocol::PROTOCOL {
                    println!("[Net] {} uses protocol {proto}, we speak {}", p.addr, protocol::PROTOCOL);
                    let status = if proto < protocol::PROTOCOL {
                        play_status::LOGIN_FAILED_CLIENT
                    } else {
                        play_status::LOGIN_FAILED_SERVER
                    };
                    p.out.push(protocol::play_status(status));
                    p.close = Some("Versão incompatível do Minecraft".into());
                    return Ok(());
                }
                // NetworkSettings itself goes out uncompressed; everything after is compressed
                let batch = protocol::encode_batch(&[protocol::network_settings()], false);
                if let Some(t) = self.transport(sid) {
                    t.send(sid, &batch);
                }
                let p = self.players.get_mut(&sid).unwrap();
                p.compression = true;
                p.state = State::AwaitLogin;
            }
            (State::AwaitLogin, id::LOGIN) => {
                let _proto = r.u32_be()?;
                let blob_len = r.uvarint()? as usize;
                let mut blob = R::new(r.take(blob_len)?);
                let auth_len = blob.u32_le()? as usize;
                let auth_json = blob.take(auth_len)?;
                let cd_len = blob.u32_le()? as usize;
                let client_data = blob.take(cd_len)?;
                let cd = jwt_payload(client_data).unwrap_or_default();
                let (chain_name, mut xuid, identity_uuid) = extract_auth_claims(auth_json);
                let mut name = chain_name
                    .or_else(|| cd["ThirdPartyName"].as_str().filter(|s| !s.is_empty()).map(str::to_string))
                    .unwrap_or_else(|| format!("Player{sid}"));

                // Xbox Live verification: the token's gamertag and XUID replace whatever the client claims
                let keys = self.auth_keys.read().unwrap().clone();
                if keys.is_none() && self.cfg.xbox_auth {
                    // Mojang's keys are still downloading (server just started): hold the login
                    // one held login per session, and a bounded queue overall
                    self.pending_logins.retain(|(other, _, _)| *other != sid);
                    if self.pending_logins.len() >= 64 || buf.len() > 2 * 1024 * 1024 {
                        self.kick(sid, "§cServidor iniciando, tente de novo em instantes.");
                        return Ok(());
                    }
                    self.pending_logins.push((sid, buf.to_vec(), Instant::now()));
                    return Ok(());
                }
                let mut result = crate::auth::verify_login(auth_json, client_data, keys.as_deref());
                // NetherNet: the login must come from the key that signed the connection's offer
                if let (Ok(v), Some(bound)) = (&result, &self.players[&sid].transport_key) {
                    if v.cpk != *bound {
                        result = Err("login identity does not match the connection's identity".into());
                    }
                }
                let verified = match result {
                    Ok(v) => {
                        name = v.gamertag;
                        xuid = v.xuid;
                        true
                    }
                    Err(e) => {
                        if self.cfg.xbox_auth {
                            println!("[Auth] refused login from {} ({name}): {e}", self.players[&sid].addr);
                            self.kick(sid, &format!("§cFalha ao verificar sua conta Xbox: {e}"));
                            return Ok(());
                        }
                        if ceres_net::debug_enabled() {
                            println!("[Auth] unverified login ({name}): {e}");
                        }
                        false
                    }
                };

                // operators need a verified account (or allow-unverified-ops for local tests)
                let is_op =
                    (verified || self.cfg.allow_unverified_ops) && self.ops.contains(&name.to_ascii_lowercase());
                let ip = self.players[&sid].addr.ip();
                if let Err(reason) = self.moderation.check_login(&name, ip, self.cfg.white_list, is_op) {
                    self.kick(sid, &reason);
                    return Ok(());
                }
                let online = self.players.values().filter(|q| q.state == State::Spawned).count();
                if online >= self.cfg.max_players && !is_op {
                    self.kick(sid, "§cO servidor está cheio!");
                    return Ok(());
                }
                // Logins are not verified yet, so a duplicate name must not push out the player who is
                // online (anyone could kick anyone, and the new session would load a stale inventory).
                if self
                    .players
                    .iter()
                    .any(|(osid, q)| *osid != sid && !q.name.is_empty() && q.name.to_lowercase() == name.to_lowercase())
                {
                    self.kick(sid, "§cJá existe um jogador com esse nome online.");
                    return Ok(());
                }

                let p = self.players.get_mut(&sid).unwrap();
                p.op = is_op;
                p.name = name;
                p.entity_id = self.next_entity_id;
                self.next_entity_id += 1;
                let uuid =
                    identity_uuid.map(|u| skin::uuid_wire(&u)).unwrap_or_else(|| skin::wire(skin::random_uuid()));
                p.info = Some(PlayerInfo {
                    uuid,
                    entity_id: p.entity_id,
                    name: p.name.clone(),
                    xuid,
                    device_id: cd["DeviceId"].as_str().unwrap_or("").to_string(),
                    build_platform: cd["DeviceOS"].as_i64().unwrap_or(0) as i32,
                    skin: std::rc::Rc::new(skin::encode(&cd)),
                });
                println!("[Game] {} is logging in from {}", p.name, p.addr);
                p.out.push(protocol::play_status(play_status::LOGIN_SUCCESS));
                p.out.push(protocol::resource_packs_info());
                p.state = State::ResourcePacks;
            }
            (State::ResourcePacks, id::RESOURCE_PACK_CLIENT_RESPONSE) => {
                let status = r.uvarint()?;
                match status {
                    2 => p.out.push(protocol::resource_pack_stack()), // have all packs
                    3 => self.start_game(sid),                        // completed
                    0 => self.kick(sid, "§cVocê precisa aceitar os pacotes de recursos."),
                    _ => {}
                }
            }
            (State::PreSpawn | State::Spawned, id::REQUEST_CHUNK_RADIUS) => {
                let requested = r.varint()?;
                let radius = requested.clamp(2, self.cfg.view_distance);
                p.radius = radius;
                p.out.push(protocol::chunk_radius_updated(radius));
                p.center = None; // force a full re-queue
                self.update_view(sid);
            }
            (State::PreSpawn, id::SET_LOCAL_PLAYER_AS_INITIALIZED) => {
                p.state = State::Spawned;
                println!("[Game] {} spawned in \"{}\"", p.name, self.worlds.worlds[p.world].folder);
                self.on_spawned(sid);
                self.update_status();
            }
            (State::PreSpawn | State::Spawned, id::PLAYER_AUTH_INPUT) => self.on_input(sid, &mut r)?,
            (State::Spawned, protocol::mp::TEXT) => self.on_text(sid, &mut r)?,
            (State::PreSpawn | State::Spawned, protocol::ITEM_STACK_REQUEST) => {
                self.on_item_stack_request(sid, &mut r)?
            }
            (State::Spawned, protocol::MOB_EQUIPMENT) => self.on_mob_equipment(sid, &mut r)?,
            (State::Spawned, protocol::BLOCK_PICK_REQUEST) => self.on_block_pick(sid, &mut r)?,
            (State::Spawned, protocol::BLOCK_ACTOR_DATA) => self.on_block_actor_data(sid, &mut r)?,
            (State::Spawned, protocol::mp::ANIMATE) => self.on_animate(sid, &mut r)?,
            (State::Spawned, protocol::mp::EMOTE) => self.on_emote(sid, &mut r)?,
            (State::Spawned, protocol::mp::PLAYER_SKIN) => self.on_player_skin(sid, &mut r)?,
            (State::Spawned, protocol::mp::RESPAWN) => self.on_respawn(sid, &mut r)?,
            (State::Spawned, protocol::COMMAND_REQUEST) => {
                let line = protocol::read_command_request(&mut r)?;
                self.run_command(commands::Sender::Player(sid), &line);
            }
            (State::Spawned, protocol::INTERACT) => {
                let action = r.u8()?;
                let target = r.uvarlong()? as i64;
                if action == protocol::INTERACT_OPEN_INVENTORY && target == p.entity_id {
                    p.out.push(protocol::open_own_inventory(p.entity_id));
                }
            }
            (State::Spawned, protocol::CONTAINER_CLOSE) => {
                let (win, ty) = (r.u8()?, r.u8()?);
                let block_window = p.inventory.container.as_ref().is_some_and(|c| c.window == win);
                p.out.push(protocol::container_close(win, ty, false));
                if block_window {
                    self.close_container(sid, false);
                } else {
                    self.return_craft_grid(sid);
                }
            }
            (State::Spawned, protocol::INVENTORY_TRANSACTION) => {
                if let Some(tx) = protocol::read_inventory_transaction(&mut r)? {
                    match tx {
                        InvTransaction::UseItem(u) => self.on_use_item(sid, u),
                        InvTransaction::Release => self.on_release_item(sid),
                        InvTransaction::Drop { slot, count, expected } => {
                            let drop_opt = self.players.get_mut(&sid).and_then(|p| {
                                let cur = p.inventory.main.get(slot as usize)?;
                                if cur.id != expected.id || cur.count < count {
                                    return None;
                                }
                                let mut dropped = cur.clone();
                                dropped.count = count;
                                dropped.stack_id = 0;
                                let rem = p.inventory.consume_slot(slot, count)?;
                                p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &rem));
                                Some(dropped)
                            });
                            match drop_opt {
                                Some(dropped) => {
                                    self.broadcast_equipment(sid);
                                    self.drop_from_player(sid, dropped);
                                }
                                None => self.sync_full_inventory(sid),
                            }
                        }
                        InvTransaction::UseOnEntity { target_eid, action, hotbar_slot } => {
                            if (0..9).contains(&hotbar_slot) {
                                if let Some(p) = self.players.get_mut(&sid) {
                                    p.inventory.held_slot = hotbar_slot as u8;
                                }
                            }
                            if action == entity_use_action::ATTACK {
                                self.on_attack_entity(sid, target_eid);
                            }
                        }
                        InvTransaction::Mismatch => self.sync_full_inventory(sid),
                    }
                }
            }
            (_, id::CLIENT_CACHE_STATUS) => {}
            (_, 0x9c) => {
                // PacketViolationWarning: the client rejected something we sent
                let (ty, severity, packet) = (r.varint()?, r.varint()?, r.varint()?);
                let msg = r.string()?;
                eprintln!(
                    "[Game] {} reports a bad packet 0x{packet:02x} (type {ty}, severity {severity}): {msg}",
                    p.name
                );
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn start_game(&mut self, sid: SessionId) {
        self.load_player_state(sid);
        let Some(p_world) = self.players.get(&sid).map(|p| p.world) else { return };
        let spawn = self.worlds.worlds[p_world].safe_spawn(&self.gd);
        let time = self.worlds.worlds[p_world].time as i32;

        let nametag = self.nametag_of(&self.players[&sid]);
        let p = self.players.get_mut(&sid).unwrap();
        p.pos = [spawn[0] as f32 + 0.5, spawn[1] as f32 + 1.62, spawn[2] as f32 + 0.5];
        p.out.push(self.gd.jigsaw_structures.clone());
        p.out.push(self.gd.voxel_shapes.clone());
        p.out.push(protocol::start_game(&protocol::StartGame {
            entity_id: p.entity_id,
            gamemode: p.gamemode,
            position: p.pos,
            pitch: 0.0,
            yaw: 0.0,
            spawn,
            difficulty: self.cfg.difficulty,
            time,
            world_name: &self.cfg.motd,
            software: &self.software,
            block_definitions: &self.gd.block_definitions,
            block_definition_count: self.gd.block_definition_count,
        }));
        p.out.push(self.gd.item_registry.clone());
        p.out.push(self.gd.actor_identifiers.clone());
        p.out.push(self.gd.biome_definitions.clone());
        p.out.push(self.gd.creative_content.clone());
        p.out.push(self.gd.crafting_data.clone());
        let pk = p.vitals_packet();
        p.out.push(pk);
        p.out.push(protocol::set_actor_data_state(p.entity_id, &nametag, &p.actor_state));
        p.state = State::PreSpawn;
        self.sync_full_inventory(sid);
        self.send_pre_spawn_players(sid);
        self.send_available_commands(sid);
        let p = self.players.get_mut(&sid).unwrap();
        p.radius = self.cfg.view_distance.min(8);
        self.update_view(sid);
    }
}

pub(crate) fn jwt_payload(token: &[u8]) -> Option<serde_json::Value> {
    let s = std::str::from_utf8(token).ok()?;
    let part = s.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(part.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Extracts `(displayName, XUID, identityUUID)` from the Xbox/OpenID login auth JSON if present.
fn extract_auth_claims(auth_json: &[u8]) -> (Option<String>, String, Option<String>) {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(auth_json) else {
        return (None, String::new(), None);
    };
    // Modern Token (1.21.50+) or legacy Certificate chain
    if let Some(tok) = v["Token"].as_str().filter(|t| !t.is_empty()) {
        if let Some(claims) = jwt_payload(tok.as_bytes()) {
            let name = claims["xname"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
            let xuid = claims["xid"].as_str().unwrap_or("").to_string();
            return (name, xuid, None);
        }
    }
    if let Some(cert_str) = v["Certificate"].as_str() {
        if let Ok(cert) = serde_json::from_str::<serde_json::Value>(cert_str) {
            if let Some(chain) = cert["chain"].as_array() {
                for link in chain {
                    if let Some(s) = link.as_str() {
                        if let Some(p) = jwt_payload(s.as_bytes()) {
                            let extra = &p["extraData"];
                            if extra.is_object() {
                                let name = extra["displayName"].as_str().filter(|n| !n.is_empty()).map(str::to_string);
                                let xuid = extra["XUID"].as_str().unwrap_or("").to_string();
                                let id = extra["identity"].as_str().filter(|u| !u.is_empty()).map(str::to_string);
                                return (name, xuid, id);
                            }
                        }
                    }
                }
            }
        }
    }
    (None, String::new(), None)
}
