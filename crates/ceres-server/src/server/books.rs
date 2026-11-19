//! Book and quill (BookEdit, packet 97): the client sends each page change; the server keeps the
//! pages in the item's NBT (`pages: [{text, photoname}]`, at most 50 pages of 256 characters) and
//! signing turns it into a written book with its title, author and generation 0.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::{ItemDesc, window};

const MAX_PAGES: usize = 50;
const MAX_PAGE_CHARS: usize = 256;

fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| *c == '\n' || !c.is_control()).take(max).collect()
}

fn page(text: &str, photo: &str) -> Tag {
    Tag::Compound(vec![("text".into(), Tag::String(clean(text, MAX_PAGE_CHARS))), ("photoname".into(), Tag::String(clean(photo, 64)))])
}

impl Server {
    pub(crate) fn on_book_edit(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let slot = r.varint()?;
        let action = r.uvarint()?;
        let Some(p) = self.players.get(&sid) else { return Ok(()) };
        if !(0..36).contains(&slot) || p.dead {
            return Ok(());
        }
        let slot = slot as usize;
        let mut book = p.inventory.main[slot].clone();
        let name = self.gd.items_by_id.get(&book.id).map(|i| i.name.clone()).unwrap_or_default();
        if name != "minecraft:writable_book" {
            return Ok(());
        }
        let (player_name, xuid) = (p.name.clone(), p.info.as_ref().map(|i| i.xuid.clone()).unwrap_or_default());
        let mut pages: Vec<Tag> = match durability::item_field(&book, "pages") {
            Some(Tag::List(_, l)) => l,
            _ => Vec::new(),
        };
        let mut signed = false;
        match action {
            0 | 1 => {
                let index = r.varint()?.max(0) as usize;
                let text = r.string()?;
                let photo = r.string()?;
                if index >= MAX_PAGES {
                    return Ok(());
                }
                if action == 1 && pages.len() < MAX_PAGES && index <= pages.len() {
                    pages.insert(index, page(&text, &photo));
                } else {
                    while pages.len() <= index {
                        pages.push(page("", ""));
                    }
                    pages[index] = page(&text, &photo);
                }
            }
            2 => {
                let index = r.varint()?.max(0) as usize;
                if index < pages.len() {
                    pages.remove(index);
                }
            }
            3 => {
                let (a, b) = (r.varint()?.max(0) as usize, r.varint()?.max(0) as usize);
                if a < pages.len() && b < pages.len() {
                    pages.swap(a, b);
                }
            }
            4 => {
                let title = r.string()?;
                let _author = r.string()?;
                let _xuid = r.string()?;
                let Some(info) = self.gd.items_by_name.get("minecraft:written_book") else { return Ok(()) };
                book.id = info.id;
                durability::set_item_field(&mut book, "title", Some(Tag::String(clean(&title, 16))));
                durability::set_item_field(&mut book, "author", Some(Tag::String(player_name)));
                durability::set_item_field(&mut book, "xuid", Some(Tag::String(xuid)));
                durability::set_item_field(&mut book, "generation", Some(Tag::Int(0)));
                signed = true;
            }
            _ => return Ok(()),
        }
        durability::set_item_field(&mut book, "pages", Some(Tag::List(10, pages)));
        let Some(p) = self.players.get_mut(&sid) else { return Ok(()) };
        p.inventory.set_main(slot, book);
        if signed {
            // the client keeps showing the quill until told
            p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
            if slot == p.inventory.held_slot as usize {
                self.broadcast_equipment(sid);
            }
        }
        Ok(())
    }

    /// Powder snow and buckets: an empty bucket scoops powder snow up, a powder snow bucket
    /// puts the block down. True when the held item was one of them used this way.
    pub(crate) fn use_powder_snow(&mut self, sid: SessionId, clicked: [i32; 3], face: u8) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm) = (p.world, p.gamemode);
        let held = p.inventory.held_item().clone();
        let name = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()).unwrap_or_default();
        let clicked_id = self.worlds.worlds[world].get_block(clicked, &self.gd);
        let clicked_name = self.gd.block_name(clicked_id).to_string();
        let (target, result) = match name.as_str() {
            "minecraft:bucket" if clicked_name == "minecraft:powder_snow" => {
                let air = self.gd.air_id;
                self.change_block(world, clicked, air);
                (clicked, "minecraft:powder_snow_bucket")
            }
            "minecraft:powder_snow_bucket" => {
                let at = if self.gd.is_replaceable(clicked_id) { Some(clicked) } else { blocks::side(clicked, face) };
                let Some(at) = at else { return true };
                let here = self.worlds.worlds[world].get_block(at, &self.gd);
                if !self.gd.is_replaceable(here) {
                    self.resync_held(sid);
                    return true;
                }
                let Some(snow) = self.gd.default_block("minecraft:powder_snow") else { return true };
                self.change_block(world, at, snow);
                (at, "minecraft:bucket")
            }
            _ => return false,
        };
        let center = [target[0] as f32 + 0.5, target[1] as f32 + 0.5, target[2] as f32 + 0.5];
        let sound = if result == "minecraft:bucket" { "bucket.empty.powder_snow" } else { "bucket.fill.powder_snow" };
        self.broadcast(&protocol::level_sound_event(sound, center, -1), |_, q| q.world == world);
        self.swing_arm(sid);
        if gm != 1 {
            let Some(info) = self.gd.items_by_name.get(result) else { return true };
            let item = ItemDesc { id: info.id, count: 1, meta: 0, stack_id: 0, block_runtime_id: 0, extra: ceres_world::extra_data(None) };
            if held.count > 1 {
                self.consume_held(sid, gm);
                self.give_stack(sid, item);
            } else {
                let Some(p) = self.players.get_mut(&sid) else { return true };
                let slot = (p.inventory.held_slot as usize).min(8);
                p.inventory.set_main(slot, item);
                p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
                self.broadcast_equipment(sid);
            }
        }
        true
    }
}
