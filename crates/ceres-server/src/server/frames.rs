//! Item frames and lecterns (logic as in df-mc/dragonfly's item_frame.go and lectern.go, MIT):
//! - item frame (and glow frame): using it puts one of the held items in, using it again turns
//!   the item (8 steps); hitting it takes the item out; breaking it drops the item too.
//! - lectern: using it with a book and quill or a written book puts the book on it (the client
//!   then reads it there); turning pages (LecternUpdate, packet 125) is kept for everyone;
//!   hitting it takes the book back.

use super::*;
use ceres_codec::nbt::Tag;

const SOUND_ADD_ITEM: i32 = 1040;
const SOUND_FRAME_REMOVE_ITEM: i32 = 1043;
const SOUND_FRAME_ROTATE_ITEM: i32 = 1044;

fn is_frame(name: &str) -> bool {
    name == "minecraft:frame" || name == "minecraft:glow_frame"
}

impl Server {
    fn holder_tag(&mut self, world: usize, pos: [i32; 3], name: &str) -> Tag {
        self.worlds.worlds[world].get_block_entity(pos, &self.gd).unwrap_or_else(|| {
            let id = match name {
                "minecraft:glow_frame" => "GlowItemFrame",
                "minecraft:lectern" => "Lectern",
                _ => "ItemFrame",
            };
            let mut t = Tag::Compound(vec![
                ("id".into(), Tag::String(id.into())),
                ("x".into(), Tag::Int(pos[0])),
                ("y".into(), Tag::Int(pos[1])),
                ("z".into(), Tag::Int(pos[2])),
                ("isMovable".into(), Tag::Byte(1)),
            ]);
            if name == "minecraft:lectern" {
                t.set("hasBook", Tag::Byte(0));
                t.set("page", Tag::Int(0));
            } else {
                t.set("ItemDropChance", Tag::Float(1.0));
                t.set("ItemRotation", Tag::Float(0.0));
            }
            t
        })
    }

    fn store_holder(&mut self, world: usize, pos: [i32; 3], tag: Tag) {
        self.worlds.worlds[world].set_block_entity(pos, tag.clone(), &self.gd);
        self.broadcast(&protocol::block_actor_data(pos, &tag), |_, q| q.world == world);
    }

    fn holder_sound(&mut self, world: usize, pos: [i32; 3], event: i32) {
        let at = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        self.broadcast(&protocol::level_event(event, at, 0), |_, q| q.world == world);
    }

    /// Use button on an item frame or a lectern. True when it was one of them.
    pub(crate) fn use_holder(&mut self, sid: SessionId, clicked: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm) = (p.world, p.gamemode);
        let held = p.inventory.held_item().clone();
        let block = self.worlds.worlds[world].get_block(clicked, &self.gd);
        let name = self.gd.block_name(block).to_string();
        if !is_frame(&name) && name != "minecraft:lectern" {
            return false;
        }
        let mut tag = self.holder_tag(world, clicked, &name);
        if is_frame(&name) {
            if tag.get("Item").is_some() {
                let rot = tag.get("ItemRotation").and_then(|t| t.as_f32()).unwrap_or(0.0);
                tag.set("ItemRotation", Tag::Float((rot + 45.0) % 360.0));
                self.holder_sound(world, clicked, SOUND_FRAME_ROTATE_ITEM);
            } else if !held.is_empty() {
                let mut one = held.clone();
                one.count = 1;
                let Some(item) = containers::item_to_vanilla(&one, None, &self.gd) else { return true };
                tag.set("Item", item);
                tag.set("ItemRotation", Tag::Float(0.0));
                if gm != 1 {
                    self.consume_held(sid, gm);
                }
                self.holder_sound(world, clicked, SOUND_ADD_ITEM);
            } else {
                return true;
            }
            self.store_holder(world, clicked, tag);
            self.swing_arm(sid);
            return true;
        }
        // lectern: an empty one takes a book
        if tag.get("hasBook").and_then(|t| t.as_i64()).unwrap_or(0) != 0 {
            return true; // the client opens the book by itself
        }
        let held_name = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()).unwrap_or_default();
        if held_name != "minecraft:writable_book" && held_name != "minecraft:written_book" {
            return false; // other items are placed against the lectern as usual
        }
        let mut one = held.clone();
        one.count = 1;
        let pages = match durability::item_field(&one, "pages") {
            Some(Tag::List(_, l)) => l.len().max(1),
            _ => 1,
        };
        let Some(book) = containers::item_to_vanilla(&one, None, &self.gd) else { return true };
        tag.set("book", book);
        tag.set("hasBook", Tag::Byte(1));
        tag.set("page", Tag::Int(0));
        tag.set("totalPages", Tag::Int(pages as i32));
        self.store_holder(world, clicked, tag);
        if gm != 1 {
            self.consume_held(sid, gm);
        }
        let at = [clicked[0] as f32 + 0.5, clicked[1] as f32 + 1.0, clicked[2] as f32 + 0.5];
        self.broadcast(&protocol::level_sound_event("item.book.put", at, -1), |_, q| q.world == world);
        self.swing_arm(sid);
        true
    }

    /// Hitting an item frame or a lectern takes out what it holds.
    pub(crate) fn punch_holder(&mut self, sid: SessionId, pos: [i32; 3]) {
        let Some(p) = self.players.get(&sid) else { return };
        let (world, creative) = (p.world, p.gamemode == 1);
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        let name = self.gd.block_name(block).to_string();
        if !is_frame(&name) && name != "minecraft:lectern" {
            return;
        }
        let Some(mut tag) = self.worlds.worlds[world].get_block_entity(pos, &self.gd) else { return };
        let key = if is_frame(&name) { "Item" } else { "book" };
        let Some(item) = tag.get(key).and_then(|t| containers::item_from_vanilla(t, &self.gd)) else { return };
        if let Tag::Compound(fields) = &mut tag {
            fields.retain(|(k, _)| k != key);
        }
        if is_frame(&name) {
            tag.set("ItemRotation", Tag::Float(0.0));
            self.holder_sound(world, pos, SOUND_FRAME_REMOVE_ITEM);
        } else {
            tag.set("hasBook", Tag::Byte(0));
            tag.set("page", Tag::Int(0));
        }
        self.store_holder(world, pos, tag);
        let drop = !is_frame(&name) || !creative;
        if drop {
            let at = [pos[0] as f32 + 0.5, pos[1] as f32 + if is_frame(&name) { 0.5 } else { 1.0 }, pos[2] as f32 + 0.5];
            self.spawn_item(world, at, [0.0, 0.1, 0.0], item, 10);
        }
    }

    /// What an item frame or lectern holds, taken out when the block is broken.
    pub(crate) fn take_holder_contents(&mut self, world: usize, pos: [i32; 3]) -> Vec<ceres_protocol::ItemDesc> {
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        let name = self.gd.block_name(block).to_string();
        if !is_frame(&name) && name != "minecraft:lectern" {
            return Vec::new();
        }
        let Some(tag) = self.worlds.worlds[world].get_block_entity(pos, &self.gd) else { return Vec::new() };
        let key = if is_frame(&name) { "Item" } else { "book" };
        tag.get(key).and_then(|t| containers::item_from_vanilla(t, &self.gd)).into_iter().collect()
    }

    /// LecternUpdate (125): a player turned a page of the book on a lectern.
    pub(crate) fn on_lectern_update(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let page = r.u8()? as i32;
        let _total = r.u8()?;
        let pos = [r.varint()?, r.uvarint()? as i32, r.varint()?];
        let Some(p) = self.players.get(&sid) else { return Ok(()) };
        let world = p.world;
        if !Self::in_reach(p, pos) {
            return Ok(());
        }
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        if self.gd.block_name(block) != "minecraft:lectern" {
            return Ok(());
        }
        let Some(mut tag) = self.worlds.worlds[world].get_block_entity(pos, &self.gd) else { return Ok(()) };
        let total = tag.get("totalPages").and_then(|t| t.as_i64()).unwrap_or(1) as i32;
        if tag.get("hasBook").and_then(|t| t.as_i64()).unwrap_or(0) == 0 || page < 0 || page >= total.max(1) {
            return Ok(());
        }
        tag.set("page", Tag::Int(page));
        self.store_holder(world, pos, tag);
        Ok(())
    }
}
