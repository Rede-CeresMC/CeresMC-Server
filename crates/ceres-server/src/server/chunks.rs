//! Chunk streaming, the game tick and flushing queued packets.

use super::*;

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

    pub(crate) fn tick(&mut self) {
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
            if let Some(p) = self.players.remove(&sid) {
                println!("[Game] closed {} ({})", p.addr, p.close.unwrap_or_default());
            }
            self.owner.remove(&sid);
        }
    }
}
