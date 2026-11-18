//! Packet handling: login flow, resource packs, StartGame, input and chat.

use super::*;
use base64::Engine;

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
                    p.close = Some("incompatible protocol".into());
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
                let _auth_json = blob.take(auth_len)?;
                let cd_len = blob.u32_le()? as usize;
                let client_data = blob.take(cd_len)?;
                let cd = jwt_payload(client_data).unwrap_or_default();
                let name = cd["ThirdPartyName"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("Player{sid}"));
                // TODO: verify the auth chain (xbox-auth) and enable encryption
                p.op = self.ops.contains(&name.to_ascii_lowercase());
                p.name = name;
                p.entity_id = self.next_entity_id;
                self.next_entity_id += 1;
                p.info = Some(PlayerInfo {
                    uuid: skin::wire(skin::random_uuid()),
                    entity_id: p.entity_id,
                    name: p.name.clone(),
                    xuid: String::new(),
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
                    0 => self.kick(sid, "You must accept resource packs"),
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
                let (window, ty) = (r.u8()?, r.u8()?);
                p.out.push(protocol::container_close(window, ty, false));
            }
            (State::Spawned, protocol::INVENTORY_TRANSACTION) => {
                if let Some(u) = protocol::read_inventory_transaction(&mut r)? {
                    self.on_use_item(sid, u);
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
        let Some(p) = self.players.get_mut(&sid) else { return };
        let world = &self.worlds.worlds[p.world];
        let mut spawn = world.spawn;
        if !(-64..=320).contains(&spawn[1]) {
            spawn[1] = 100;
        }
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
            time: world.time as i32,
            world_name: &self.cfg.motd,
            software: &self.software,
            block_definitions: &self.gd.block_definitions,
            block_definition_count: self.gd.block_definition_count,
        }));
        p.out.push(self.gd.item_registry.clone());
        p.out.push(self.gd.actor_identifiers.clone());
        p.out.push(self.gd.biome_definitions.clone());
        p.out.push(self.gd.creative_content.clone());
        p.out.push(protocol::update_attributes(p.entity_id));
        p.out.push(protocol::set_actor_data(p.entity_id, &p.name));
        p.out.push(protocol::empty_inventory(0, 36)); // main inventory
        p.out.push(protocol::empty_inventory(119, 1)); // off hand
        p.out.push(protocol::empty_inventory(120, 4)); // armor
        p.state = State::PreSpawn;
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
