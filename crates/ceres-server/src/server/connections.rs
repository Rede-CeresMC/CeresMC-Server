//! Fences, panes, iron bars and walls connect to their neighbours. In Bedrock the server works
//! the connections out and sends them as block states (`minecraft:connection_*`,
//! `wall_connection_type_*`, `wall_post_bit`), so without this they stand alone (logic of
//! df-mc/dragonfly's fence/pane/wall models, MIT):
//! - a fence joins fences of its family (wooden ones together, nether brick ones together),
//!   fence gates and solid blocks;
//! - a pane or bars join panes, bars, walls and solid blocks;
//! - a wall joins walls, panes, bars, fence gates and solid blocks; a side is "tall" when a
//!   block sits on top of the wall, and the post shows unless the wall runs straight through.
//!
//! Whenever a block changes, the block itself and its four neighbours are worked out again.

use super::*;
use ceres_codec::nbt::Tag;

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    WoodFence,
    NetherFence,
    Pane,
    Wall,
}

fn kind_of(name: &str) -> Option<Kind> {
    let n = name.strip_prefix("minecraft:")?;
    if n == "nether_brick_fence" {
        Some(Kind::NetherFence)
    } else if n.ends_with("_fence") || n == "fence" {
        Some(Kind::WoodFence)
    } else if n.ends_with("glass_pane") || n == "iron_bars" || n.ends_with("_bars") {
        Some(Kind::Pane)
    } else if n.ends_with("_wall") && !n.ends_with("_sign") {
        Some(Kind::Wall)
    } else {
        None
    }
}

fn is_gate(name: &str) -> bool {
    name.ends_with("fence_gate")
}

/// north, east, south, west as (dx, dz) and the state suffix
const SIDES: [([i32; 2], &str); 4] = [([0, -1], "north"), ([1, 0], "east"), ([0, 1], "south"), ([-1, 0], "west")];

impl Server {
    /// Works out the connections of the block at `pos` and of its four neighbours.
    pub(crate) fn update_connections(&mut self, world: usize, pos: [i32; 3]) {
        self.connect_one(world, pos);
        for (d, _) in SIDES {
            self.connect_one(world, [pos[0] + d[0], pos[1], pos[2] + d[1]]);
        }
        // a block put on (or taken from) a wall changes its height
        self.connect_one(world, [pos[0], pos[1] - 1, pos[2]]);
    }

    fn connect_one(&mut self, world: usize, pos: [i32; 3]) {
        if !self.worlds.worlds[world].is_loaded(pos) {
            return;
        }
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        let Some(kind) = kind_of(self.gd.block_name(id)) else { return };
        let mut joined = [false; 4];
        for (i, (d, _)) in SIDES.iter().enumerate() {
            let n = [pos[0] + d[0], pos[1], pos[2] + d[1]];
            let nid = self.worlds.worlds[world].get_block(n, &self.gd);
            let nname = self.gd.block_name(nid).to_string();
            let nkind = kind_of(&nname);
            let solid = self.gd.is_solid(nid) && nkind.is_none() && !is_gate(&nname);
            joined[i] = solid
                || match kind {
                    Kind::WoodFence => nkind == Some(Kind::WoodFence) || is_gate(&nname),
                    Kind::NetherFence => nkind == Some(Kind::NetherFence) || is_gate(&nname),
                    Kind::Pane => matches!(nkind, Some(Kind::Pane | Kind::Wall)),
                    Kind::Wall => matches!(nkind, Some(Kind::Pane | Kind::Wall)) || is_gate(&nname),
                };
        }
        let mut states: Vec<(String, Tag)> = Vec::new();
        if kind == Kind::Wall {
            let above = self.worlds.worlds[world].get_block([pos[0], pos[1] + 1, pos[2]], &self.gd);
            let tall = self.gd.is_solid(above);
            for (i, (_, side)) in SIDES.iter().enumerate() {
                let v = if !joined[i] { "none" } else if tall { "tall" } else { "short" };
                states.push((format!("wall_connection_type_{side}"), Tag::String(v.into())));
            }
            let straight = (joined == [true, false, true, false]) || (joined == [false, true, false, true]);
            states.push(("wall_post_bit".into(), Tag::Byte((!straight || tall) as i8)));
        } else {
            for (i, (_, side)) in SIDES.iter().enumerate() {
                states.push((format!("minecraft:connection_{side}"), Tag::Byte(joined[i] as i8)));
            }
        }
        let refs: Vec<(&str, Tag)> = states.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let Some(new) = self.gd.with_states(id, &refs) else { return };
        if new != id && self.worlds.worlds[world].set_block(pos, new, &self.gd) {
            self.broadcast(&protocol::update_block(pos, new), |_, p| p.world == world);
        }
    }
}
