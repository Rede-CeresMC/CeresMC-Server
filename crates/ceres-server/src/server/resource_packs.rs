//! Resource pack download during login: the client says which packs it lacks, gets their size
//! and hash, then asks for them chunk by chunk.

use super::*;
use crate::packs::CHUNK_SIZE;

impl Server {
    pub(crate) fn pack_entries(&self) -> Vec<protocol::PackEntry<'_>> {
        self.packs
            .iter()
            .map(|p| protocol::PackEntry { uuid: &p.uuid, version: &p.version, size: p.data.len() as u64 })
            .collect()
    }

    /// Index of a pack by the id the client uses ("uuid" or "uuid_version").
    fn pack_index(&self, id: &str) -> Option<usize> {
        let uuid = id.split('_').next().unwrap_or(id).to_ascii_lowercase();
        self.packs.iter().position(|p| p.uuid == uuid)
    }

    /// The client asked to download these packs: send each one's size, chunk count and hash.
    pub(crate) fn send_pack_infos(&mut self, sid: SessionId, ids: &[String]) -> Res<()> {
        let mut seen = HashSet::new();
        for id in ids {
            let Some(i) = self.pack_index(id) else {
                return Err(format!("asked for unknown resource pack {id}"));
            };
            if !seen.insert(i) {
                return Err(format!("asked twice for resource pack {id}"));
            }
            let pack = &self.packs[i];
            let pk = protocol::resource_pack_data_info(
                &pack.uuid,
                CHUNK_SIZE as u32,
                pack.chunk_count(),
                pack.data.len() as u64,
                &pack.sha256,
            );
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(pk);
            }
        }
        if let Some(p) = self.players.get(&sid) {
            println!("[Packs] {} is downloading {} resource pack(s)", p.name, ids.len());
        }
        Ok(())
    }

    /// One chunk of a pack (each chunk only once per player).
    pub(crate) fn send_pack_chunk(&mut self, sid: SessionId, id: &str, index: u32) -> Res<()> {
        let Some(i) = self.pack_index(id) else {
            return Err(format!("asked for a chunk of unknown resource pack {id}"));
        };
        let pack = &self.packs[i];
        let Some(data) = pack.chunk(index) else {
            return Err(format!("asked for chunk {index} of {} (it has {})", pack.uuid, pack.chunk_count()));
        };
        let pk = protocol::resource_pack_chunk_data(&pack.uuid, index, index as u64 * CHUNK_SIZE as u64, data);
        let Some(p) = self.players.get_mut(&sid) else { return Ok(()) };
        if !p.pack_chunks_sent.insert((i, index)) {
            return Err(format!("asked twice for chunk {index} of {}", pack.uuid));
        }
        p.out.push(pk);
        Ok(())
    }
}
