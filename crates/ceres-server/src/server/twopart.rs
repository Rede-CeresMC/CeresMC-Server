//! Blocks made of two parts: doors and tall plants (lower + upper half) and beds (foot + head).
//! Both parts are placed together, opening a door moves both halves, and breaking one part
//! removes the other (one drop).

use super::*;
use ceres_codec::nbt::Tag;

/// Bedrock's horizontal direction from a yaw: 0 south, 1 west, 2 north, 3 east.
pub(crate) fn yaw_direction(yaw: f32) -> i32 {
    ((yaw.rem_euclid(360.0) + 45.0) / 90.0).floor() as i32 % 4
}

fn direction_offset(dir: i32) -> [i32; 3] {
    match dir.rem_euclid(4) {
        0 => [0, 0, 1],
        1 => [-1, 0, 0],
        2 => [0, 0, -1],
        _ => [1, 0, 0],
    }
}

fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn state_i64(gd: &GameData, id: i32, key: &str) -> Option<i64> {
    gd.block_states.get(&id)?.states.get(key)?.as_i64()
}

/// Where the other part of a two-part block is.
pub(crate) fn partner(gd: &GameData, pos: [i32; 3], id: i32) -> Option<[i32; 3]> {
    if let Some(upper) = state_i64(gd, id, "upper_block_bit") {
        return Some([pos[0], pos[1] + if upper != 0 { -1 } else { 1 }, pos[2]]);
    }
    if let Some(head) = state_i64(gd, id, "head_piece_bit") {
        let dir = state_i64(gd, id, "direction").unwrap_or(0) as i32;
        let off = direction_offset(dir);
        return Some(if head != 0 { [pos[0] - off[0], pos[1], pos[2] - off[2]] } else { add(pos, off) });
    }
    None
}

impl Server {
    /// Places a two-part block at `target` (the lower half / foot). Returns None when `block` is not
    /// a two-part block, Some(false) when it does not fit.
    pub(crate) fn place_two_part(
        &mut self,
        world: usize,
        target: [i32; 3],
        block: i32,
        yaw: f32,
        item_meta: u32,
    ) -> Option<bool> {
        let gd = &self.gd;
        let dir = yaw_direction(yaw);
        let (first, second, second_pos) = if state_i64(gd, block, "upper_block_bit").is_some() {
            let mut lower = gd.with_states(block, &[("upper_block_bit", Tag::Byte(0))])?;
            if gd.block_states.get(&lower)?.states.get("minecraft:cardinal_direction").is_some() {
                let look = ["south", "west", "north", "east"][dir as usize];
                lower = gd.with_states(lower, &[("minecraft:cardinal_direction", Tag::String(look.into()))])?;
            }
            let upper = gd.with_states(lower, &[("upper_block_bit", Tag::Byte(1))])?;
            (lower, upper, [target[0], target[1] + 1, target[2]])
        } else if state_i64(gd, block, "head_piece_bit").is_some() {
            let foot = gd.with_states(block, &[("head_piece_bit", Tag::Byte(0)), ("direction", Tag::Int(dir))])?;
            let head = gd.with_states(foot, &[("head_piece_bit", Tag::Byte(1))])?;
            (foot, head, add(target, direction_offset(dir)))
        } else {
            return None;
        };
        let other = self.worlds.worlds[world].get_block(second_pos, &self.gd);
        if !self.gd.is_replaceable(other) || self.collides_with_player(world, second_pos) {
            return Some(false);
        }
        if !self.change_block(world, target, first) {
            return Some(false);
        }
        self.change_block(world, second_pos, second);
        // beds keep their colour in a block entity on both parts
        if self.gd.block_name(first) == "minecraft:bed" {
            for pos in [target, second_pos] {
                let tile = Tag::Compound(vec![
                    ("id".into(), Tag::String("Bed".into())),
                    ("isMovable".into(), Tag::Byte(1)),
                    ("color".into(), Tag::Byte(item_meta as i8)),
                ]);
                self.worlds.worlds[world].set_block_entity(pos, tile.clone(), &self.gd);
                self.broadcast(&protocol::block_actor_data(pos, &tile), |_, q| q.world == world);
            }
        }
        Some(true)
    }

    /// Opening a door moves both halves.
    pub(crate) fn toggle_partner(&mut self, world: usize, pos: [i32; 3], toggled: i32) {
        let Some(other_pos) = partner(&self.gd, pos, toggled) else { return };
        let other = self.worlds.worlds[world].get_block(other_pos, &self.gd);
        if self.gd.block_name(other) != self.gd.block_name(toggled) {
            return;
        }
        let open = state_i64(&self.gd, toggled, "open_bit").unwrap_or(0);
        if let Some(new) = self.gd.with_states(other, &[("open_bit", Tag::Byte(open as i8))]) {
            self.change_block(world, other_pos, new);
        }
    }

    /// Breaking one part removes the other one (it drops nothing).
    pub(crate) fn break_partner(&mut self, world: usize, pos: [i32; 3], old: i32) {
        let Some(other_pos) = partner(&self.gd, pos, old) else { return };
        let other = self.worlds.worlds[world].get_block(other_pos, &self.gd);
        if self.gd.block_name(other) == self.gd.block_name(old) {
            let air = self.gd.air_id;
            self.change_block(world, other_pos, air);
        }
    }
}
