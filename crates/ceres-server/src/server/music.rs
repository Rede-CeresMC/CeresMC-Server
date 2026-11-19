//! Note blocks and jukeboxes.
//! Note block: using it raises the note (25 pitches, kept in the "Music" block entity) and plays
//! it; hitting it plays it. The instrument depends on the block underneath, as in vanilla.
//! Jukebox: a music disc used on it plays the disc; using it again ejects the disc.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::ItemDesc;

/// Bedrock instrument of the block under a note block.
fn instrument(below: &str) -> i32 {
    let n = below.strip_prefix("minecraft:").unwrap_or(below);
    match n {
        "gold_block" => 5,
        "clay" => 6,
        "packed_ice" => 7,
        "bone_block" => 9,
        "iron_block" => 10,
        "soul_sand" => 11,
        "pumpkin" | "carved_pumpkin" | "lit_pumpkin" => 12,
        "emerald_block" => 13,
        "hay_block" => 14,
        "glowstone" => 15,
        "sand" | "red_sand" | "gravel" | "suspicious_sand" | "suspicious_gravel" => 2,
        _ if n.ends_with("_wool") => 8,
        _ if n.contains("glass") || n == "sea_lantern" || n == "beacon" => 3,
        _ if n.ends_with("_planks") || n.ends_with("_log") || n.ends_with("_wood") || n == "bookshelf"
            || n == "crafting_table" || n == "chest" || n == "note_block" || n == "jukebox" =>
        {
            4
        }
        _ if n.contains("stone") || n.contains("deepslate") || n.ends_with("_ore") || n == "obsidian"
            || n == "netherrack" || n.contains("brick") || n.contains("concrete") || n.contains("terracotta")
            || n == "bedrock" || n.contains("andesite") || n.contains("diorite") || n.contains("granite") =>
        {
            1
        }
        _ => 0, // harp
    }
}

/// The record sound of a music disc item.
fn record_sound(item: &str) -> Option<String> {
    item.strip_prefix("minecraft:music_disc_").map(|d| format!("record.{d}"))
}

impl Server {
    pub(crate) fn play_note(&mut self, world: usize, pos: [i32; 3]) {
        let below = self.worlds.worlds[world].get_block([pos[0], pos[1] - 1, pos[2]], &self.gd);
        let inst = instrument(self.gd.block_name(below));
        let pitch = self
            .worlds
            .worlds[world]
            .get_block_entity(pos, &self.gd)
            .and_then(|t| t.get("note").and_then(|v| v.as_i64()))
            .unwrap_or(0)
            .clamp(0, 24) as i32;
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        self.broadcast(&protocol::block_event(pos, inst, pitch), |_, q| q.world == world);
        self.broadcast(&protocol::level_sound_event("note", center, (inst << 8) | pitch), |_, q| q.world == world);
    }

    /// Note block hit (left click): plays its note. Returns true if it is a note block.
    pub(crate) fn punch_note_block(&mut self, world: usize, pos: [i32; 3]) -> bool {
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        if self.gd.block_name(block) != "minecraft:noteblock" && self.gd.block_name(block) != "minecraft:note_block" {
            return false;
        }
        self.play_note(world, pos);
        true
    }

    /// Note block or jukebox used (right click). Returns true when handled.
    pub(crate) fn use_music_block(&mut self, sid: SessionId, pos: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let world = p.world;
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        let name = self.gd.block_name(block).to_string();
        if name == "minecraft:noteblock" || name == "minecraft:note_block" {
            let mut tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd).unwrap_or_else(|| {
                Tag::Compound(vec![("id".into(), Tag::String("Music".into())), ("isMovable".into(), Tag::Byte(1))])
            });
            let note = (tile.get("note").and_then(|v| v.as_i64()).unwrap_or(0) + 1) % 25;
            tile.set("note", Tag::Byte(note as i8));
            self.worlds.worlds[world].set_block_entity(pos, tile, &self.gd);
            self.play_note(world, pos);
            self.swing_arm(sid);
            return true;
        }
        if name != "minecraft:jukebox" {
            return false;
        }
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        let mut tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd).unwrap_or_else(|| {
            Tag::Compound(vec![("id".into(), Tag::String("Jukebox".into())), ("isMovable".into(), Tag::Byte(1))])
        });
        // a disc inside: eject it
        if let Some(disc) = tile.get("RecordItem").and_then(|t| containers::item_from_vanilla(t, &self.gd)) {
            if let Tag::Compound(fields) = &mut tile {
                fields.retain(|(k, _)| k != "RecordItem");
            }
            self.worlds.worlds[world].set_block_entity(pos, tile, &self.gd);
            self.stop_record(world, pos);
            self.spawn_item(world, [center[0], center[1] + 0.6, center[2]], [0.0, 0.1, 0.0], disc, 10);
            self.swing_arm(sid);
            return true;
        }
        // a disc in the hand: put it in and play it
        let held = self.players[&sid].inventory.held_item().clone();
        let Some(item_name) = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()) else { return true };
        let Some(sound) = record_sound(&item_name) else { return true };
        let mut one = held.clone();
        one.count = 1;
        let Some(saved) = containers::item_to_vanilla(&one, None, &self.gd) else { return true };
        tile.set("RecordItem", saved);
        self.worlds.worlds[world].set_block_entity(pos, tile, &self.gd);
        let gm = self.players[&sid].gamemode;
        self.consume_held(sid, gm);
        // records play with a sound handle (protocol 2225) so they can be stopped
        self.stop_record(world, pos);
        let handle = self.next_sound_handle;
        self.next_sound_handle += 1;
        self.jukebox_sounds.insert((world, pos), handle);
        self.broadcast(&protocol::play_sound_with_handle(&sound, center, handle), |_, q| q.world == world);
        self.broadcast(&protocol::record_started(pos, handle), |_, q| q.world == world);
        self.swing_arm(sid);
        true
    }

    /// A jukebox being broken drops its disc and stops the music.
    pub(crate) fn eject_jukebox(&mut self, world: usize, pos: [i32; 3]) -> Option<ItemDesc> {
        let tile = self.worlds.worlds[world].get_block_entity(pos, &self.gd)?;
        self.stop_record(world, pos);
        tile.get("RecordItem").and_then(|t| containers::item_from_vanilla(t, &self.gd))
    }

    /// Stops the record a jukebox is playing, if any.
    pub(crate) fn stop_record(&mut self, world: usize, pos: [i32; 3]) {
        if let Some(handle) = self.jukebox_sounds.remove(&(world, pos)) {
            self.broadcast(&protocol::stop_sound_handle(handle), |_, q| q.world == world);
        }
        // older clients still listen to the stop-record sound
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        self.broadcast(&protocol::level_sound_event("record.null", center, -1), |_, q| q.world == world);
    }
}
