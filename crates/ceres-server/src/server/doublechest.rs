//! Double chests: two chests side by side, facing the same way, pair up (vanilla `pairx`,
//! `pairz`, `pairlead` in both block entities). Opening either half shows 54 slots, the half with
//! the smaller coordinates first; each half keeps its own 27 items on disk.

use super::*;
use ceres_codec::nbt::Tag;

fn is_chest(name: &str) -> bool {
    name == "minecraft:chest" || name == "minecraft:trapped_chest"
}

fn facing(gd: &GameData, id: i32) -> Option<String> {
    gd.block_states.get(&id)?.states.get("minecraft:cardinal_direction")?.as_str().map(str::to_string)
}

fn chest_tile() -> Tag {
    Tag::Compound(vec![("id".into(), Tag::String("Chest".into())), ("isMovable".into(), Tag::Byte(1))])
}

/// The two halves in their fixed order (smaller x, then smaller z, first).
pub(crate) fn ordered(a: [i32; 3], b: [i32; 3]) -> ([i32; 3], [i32; 3]) {
    if (a[0], a[2]) <= (b[0], b[2]) { (a, b) } else { (b, a) }
}

impl Server {
    /// The other half of a paired chest, when both halves agree they are paired.
    pub(crate) fn chest_pair(&mut self, world: usize, pos: [i32; 3]) -> Option<[i32; 3]> {
        let w = &mut self.worlds.worlds[world];
        let block = w.get_block(pos, &self.gd);
        let tile = w.get_block_entity(pos, &self.gd)?;
        let other = [tile.get("pairx")?.as_i64()? as i32, pos[1], tile.get("pairz")?.as_i64()? as i32];
        let dist = (other[0] - pos[0]).abs() + (other[2] - pos[2]).abs();
        if dist != 1 || w.get_block(other, &self.gd) != block {
            return None;
        }
        let back = w.get_block_entity(other, &self.gd)?;
        let points_back = back.get("pairx").and_then(|v| v.as_i64()) == Some(pos[0] as i64)
            && back.get("pairz").and_then(|v| v.as_i64()) == Some(pos[2] as i64);
        points_back.then_some(other)
    }

    fn write_pair(&mut self, world: usize, pos: [i32; 3], other: Option<[i32; 3]>, lead: bool) {
        let w = &mut self.worlds.worlds[world];
        let mut tile = w.get_block_entity(pos, &self.gd).unwrap_or_else(chest_tile);
        if let Tag::Compound(fields) = &mut tile {
            fields.retain(|(k, _)| k != "pairx" && k != "pairz" && k != "pairlead");
        }
        if let Some(o) = other {
            tile.set("pairx", Tag::Int(o[0]));
            tile.set("pairz", Tag::Int(o[2]));
            tile.set("pairlead", Tag::Byte(lead as i8));
        }
        w.set_block_entity(pos, tile.clone(), &self.gd);
        self.broadcast(&protocol::block_actor_data(pos, &tile), |_, q| q.world == world);
    }

    /// A chest was just placed: pair it with a free chest of the same kind beside it.
    pub(crate) fn pair_new_chest(&mut self, world: usize, pos: [i32; 3]) {
        let block = self.worlds.worlds[world].get_block(pos, &self.gd);
        if !is_chest(self.gd.block_name(block)) {
            return;
        }
        let Some(face) = facing(&self.gd, block) else { return };
        // halves sit side by side, across the way they face
        let sides: [[i32; 3]; 2] = if face == "north" || face == "south" { [[1, 0, 0], [-1, 0, 0]] } else { [[0, 0, 1], [0, 0, -1]] };
        for s in sides {
            let other = [pos[0] + s[0], pos[1], pos[2] + s[2]];
            if self.worlds.worlds[world].get_block(other, &self.gd) != block {
                continue;
            }
            if self.chest_pair(world, other).is_some() {
                continue; // already a double chest
            }
            // windows showing the old single chest would no longer match what is stored
            self.close_containers_at(world, &[pos, other]);
            self.write_pair(world, other, Some(pos), true);
            self.write_pair(world, pos, Some(other), false);
            return;
        }
    }

    /// A chest half is being broken: the other half becomes a single chest again.
    pub(crate) fn unpair_chest(&mut self, world: usize, pos: [i32; 3]) {
        if let Some(other) = self.chest_pair(world, pos) {
            self.close_containers_at(world, &[pos, other]);
            self.write_pair(world, other, None, false);
        }
    }

    /// Contents of a double chest (54 slots) and its two halves in order.
    pub(crate) fn double_chest_items(&mut self, world: usize, pos: [i32; 3]) -> Option<([i32; 3], [i32; 3], Vec<ceres_protocol::ItemDesc>)> {
        let other = self.chest_pair(world, pos)?;
        let (first, second) = ordered(pos, other);
        let a = self.worlds.worlds[world].get_block_entity(first, &self.gd);
        let b = self.worlds.worlds[world].get_block_entity(second, &self.gd);
        let mut items = containers::items_from_tile(a.as_ref(), &self.gd, 27);
        items.extend(containers::items_from_tile(b.as_ref(), &self.gd, 27));
        Some((first, second, items))
    }
}
