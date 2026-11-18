//! Chunk streaming, world daylight/weather ticking, survival/entity ticks and packet flushing.

use super::*;
use ceres_protocol::level_event;
use ceres_world::Weather;

/// Changed chunks are written to disk every 5 seconds.
const SAVE_INTERVAL_TICKS: u64 = 100;

impl Server {
    /// Re-queues chunks when the player crosses a chunk border.
    pub(crate) fn update_view(&mut self, sid: SessionId) {
        let Some(p) = self.players.get_mut(&sid) else { return };
        if p.radius <= 0 {
            return;
        }
        let c = ((p.pos[0].floor() as i32) >> 4, (p.pos[2].floor() as i32) >> 4);
        if p.center == Some(c) {
            return;
        }
        p.center = Some(c);
        let r = p.radius;
        p.sent.retain(|(x, z)| (x - c.0).pow(2) + (z - c.1).pow(2) <= (r + 1) * (r + 1));
        let mut want: Vec<(i32, i32)> = Vec::new();
        for dx in -r..=r {
            for dz in -r..=r {
                if dx * dx + dz * dz <= r * r && !p.sent.contains(&(c.0 + dx, c.1 + dz)) {
                    want.push((c.0 + dx, c.1 + dz));
                }
            }
        }
        want.sort_by_key(|(x, z)| (x - c.0).pow(2) + (z - c.1).pow(2));
        p.queue = want.into();
        p.out.push(protocol::network_chunk_publisher_update(
            [p.pos[0] as i32, p.pos[1] as i32, p.pos[2] as i32],
            (r * 16) as u32,
        ));
    }

    fn tick_worlds(&mut self) {
        let mut time_syncs: Vec<(usize, i32)> = Vec::new();
        let mut weather_cleared: Vec<usize> = Vec::new();

        for (wi, w) in self.worlds.worlds.iter_mut().enumerate() {
            if w.do_daylight_cycle {
                w.time = w.time.wrapping_add(1);
                if self.ticks % 200 == 0 {
                    w.mark_level_dat_dirty();
                    time_syncs.push((wi, w.time as i32));
                }
            }
            if w.weather != Weather::Clear && w.weather_ticks > 0 {
                w.weather_ticks -= 1;
                if w.weather_ticks == 0 {
                    w.weather = Weather::Clear;
                    weather_cleared.push(wi);
                }
            }
        }

        for (wi, t) in time_syncs {
            self.broadcast(&protocol::set_time(t), |_, p| p.world == wi);
        }
        for wi in weather_cleared {
            self.broadcast(&protocol::level_event(level_event::STOP_THUNDER, [0.0; 3], 0), |_, p| p.world == wi);
            self.broadcast(&protocol::level_event(level_event::STOP_RAIN, [0.0; 3], 0), |_, p| p.world == wi);
        }
    }

    pub(crate) fn tick(&mut self) {
        self.retry_pending_logins();
        let tick_start = Instant::now();
        self.ticks += 1;
        if self.ticks % SAVE_INTERVAL_TICKS == 0 {
            self.save_worlds();
        }
        self.tick_worlds();
        self.tick_players_survival();
        self.tick_furnaces();
        self.tick_items();
        self.tick_falling();

        let sids: Vec<SessionId> = self.players.keys().copied().collect();
        for sid in sids {
            let Some(p) = self.players.get_mut(&sid) else { continue };
            if p.state != State::PreSpawn && p.state != State::Spawned {
                continue;
            }
            // anything queued before the chunks must reach the client first (publisher update)
            if !p.out.is_empty() {
                let batch = protocol::encode_batch(&p.out, p.compression);
                p.out.clear();
                if let Some(ti) = self.owner.get(&sid) {
                    self.transports[*ti].send(sid, &batch);
                }
            }
            let Some(p) = self.players.get_mut(&sid) else { continue };
            let budget = if p.spawn_sent { self.cfg.chunks_per_tick } else { self.cfg.chunks_per_tick * 4 };
            let world = &mut self.worlds.worlds[p.world];
            let ti = self.owner.get(&sid).copied();
            for _ in 0..budget {
                let Some((x, z)) = p.queue.pop_front() else { break };
                let batch = world.chunk_batch(x, z, &self.gd);
                p.sent.insert((x, z));
                if let Some(ti) = ti {
                    self.transports[ti].send(sid, &batch);
                }
            }
            if p.queue.is_empty() && !p.spawn_sent {
                p.spawn_sent = true;
                p.out.push(protocol::play_status(play_status::PLAYER_SPAWN));
            }
        }
        // drop cached chunks nobody is near
        for (wi, w) in self.worlds.worlds.iter_mut().enumerate() {
            let views: Vec<((i32, i32), i32)> = self
                .players
                .values()
                .filter(|p| p.world == wi)
                .filter_map(|p| p.center.map(|c| (c, p.radius + 2)))
                .collect();
            w.evict_except(|x, z| views.iter().any(|((cx, cz), r)| (x - cx).pow(2) + (z - cz).pow(2) <= r * r));
        }

        let ms = tick_start.elapsed().as_secs_f32() * 1000.0;
        if self.tick_ms.len() >= 100 {
            self.tick_ms.pop_front();
        }
        self.tick_ms.push_back(ms);
    }

    pub(crate) fn flush_players(&mut self) {
        let mut closing = Vec::new();
        for (sid, p) in self.players.iter_mut() {
            if !p.out.is_empty() {
                if ceres_net::debug_enabled() && p.state != State::Spawned {
                    let ids: Vec<String> =
                        p.out.iter().map(|pk| format!("0x{:02x}", R::new(pk).uvarint().unwrap_or(0))).collect();
                    println!("[Debug] {sid} -> {}", ids.join(" "));
                }
                let batch = protocol::encode_batch(&p.out, p.compression);
                p.out.clear();
                if let Some(ti) = self.owner.get(sid) {
                    self.transports[*ti].send(*sid, &batch);
                }
            }
            if p.close.is_some() {
                closing.push(*sid);
            }
        }
        for sid in closing {
            for t in self.transports.iter_mut() {
                t.flush(&mut Vec::new());
            }
            if let Some(t) = self.transport(sid) {
                t.close(sid);
            }
            self.close_container(sid, false);
            if let Some(p) = self.players.remove(&sid) {
                println!("[Game] closed {} ({})", p.addr, p.close.clone().unwrap_or_default());
                self.on_left(&p);
            }
            self.owner.remove(&sid);
        }
    }

    /// Logins held until Mojang's keys are loaded: resumed when they arrive, refused after 15 s.
    fn retry_pending_logins(&mut self) {
        if self.pending_logins.is_empty() {
            return;
        }
        let ready = self.auth_keys.read().unwrap().is_some();
        let pending = std::mem::take(&mut self.pending_logins);
        for (sid, packet, since) in pending {
            if !self.players.contains_key(&sid) {
                continue;
            }
            if ready {
                if let Err(e) = self.handle_packet(sid, &packet) {
                    eprintln!("[Net] session {sid}: {e}");
                    self.kick(sid, "Pacote inválido");
                }
            } else if since.elapsed().as_secs() >= 15 {
                self.kick(sid, "§cNão foi possível verificar contas Xbox agora. Tente de novo em instantes.");
            } else {
                self.pending_logins.push((sid, packet, since));
            }
        }
    }
}
