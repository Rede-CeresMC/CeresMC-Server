//! What a block needs under or behind it (as the `canPlaceOn`/`onNeighborChange` checks of
//! PowerNukkitX's blocks):
//! - flowers, saplings, grass, ferns, bushes: soil (dirt family, moss, mud...); dead bushes also
//!   sand and terracotta; mushrooms and fungi any solid block, nylium, soul soil;
//! - crops and stems: farmland; nether wart: soul sand;
//! - sugar cane: sugar cane, or soil/sand next to water; cactus: cactus or sand, nothing solid
//!   beside it; kelp and seagrass: in water, on a solid block (kelp also on kelp);
//! - carpets, rails, pressure plates, redstone dust, repeaters, comparators, snow layers: a solid
//!   block below;
//! - torches, buttons, levers, ladders, wall signs: the solid block they were placed against.
//!
//! A block is refused where it could not stay, and when a block is broken the unsupported
//! blocks it held up above pop off with their drops (a column of sugar cane, cactus or kelp
//! entirely).

use super::*;
use ceres_protocol::level_event;

/// How a block is held up.
#[derive(PartialEq, Clone, Copy)]
enum Need {
    Soil,
    DeadBushSoil,
    AnySolid,
    Farmland,
    SoulSand,
    SugarCane,
    Cactus,
    Kelp,
    Seagrass,
    SolidBelow,
    /// against the block that was clicked
    Attached,
}

fn need_of(name: &str) -> Option<Need> {
    let n = name.strip_prefix("minecraft:")?;
    let has = |s: &str| n.contains(s);
    Some(match n {
        "wheat" | "carrots" | "potatoes" | "beetroot" | "pumpkin_stem" | "melon_stem" | "torchflower_crop" | "pitcher_crop" => Need::Farmland,
        "nether_wart" => Need::SoulSand,
        "reeds" | "sugar_cane" => Need::SugarCane,
        "cactus" => Need::Cactus,
        "kelp" => Need::Kelp,
        "seagrass" => Need::Seagrass,
        "deadbush" | "dead_bush" => Need::DeadBushSoil,
        "brown_mushroom" | "red_mushroom" | "crimson_fungus" | "warped_fungus" | "crimson_roots" | "warped_roots" | "nether_sprouts" => Need::AnySolid,
        "short_grass" | "tall_grass" | "fern" | "large_fern" | "sweet_berry_bush" | "dandelion" | "poppy" | "blue_orchid" | "allium" | "azure_bluet"
        | "oxeye_daisy" | "cornflower" | "lily_of_the_valley" | "wither_rose" | "sunflower" | "lilac" | "rose_bush" | "peony" | "torchflower"
        | "pitcher_plant" | "pink_petals" | "wildflowers" | "bush" | "firefly_bush" | "short_dry_grass" | "tall_dry_grass" | "leaf_litter"
        | "open_eyeblossom" | "closed_eyeblossom" | "azalea" | "flowering_azalea" => Need::Soil,
        _ if n.ends_with("_tulip") || n.ends_with("_sapling") || n == "mangrove_propagule" => Need::Soil,
        _ if n.ends_with("_carpet") || n == "moss_carpet" || n == "pale_moss_carpet" || n == "carpet" => Need::SolidBelow,
        _ if n == "rail" || n.ends_with("_rail") || has("pressure_plate") || n == "redstone_wire" => Need::SolidBelow,
        _ if n.ends_with("_repeater") || n.ends_with("_comparator") || n == "snow_layer" => Need::SolidBelow,
        _ if has("torch") && !has("torchflower") => Need::Attached,
        _ if has("button") || n == "lever" || n == "ladder" || n.ends_with("_wall_sign") || n == "wall_sign" => Need::Attached,
        _ => return None,
    })
}

fn is_soil(n: &str) -> bool {
    matches!(
        n.strip_prefix("minecraft:").unwrap_or(n),
        "grass_block" | "dirt" | "coarse_dirt" | "podzol" | "rooted_dirt" | "moss_block" | "pale_moss_block" | "mud" | "muddy_mangrove_roots"
            | "mycelium" | "farmland" | "dirt_with_roots"
    )
}

fn is_sand(n: &str) -> bool {
    matches!(n.strip_prefix("minecraft:").unwrap_or(n), "sand" | "red_sand" | "suspicious_sand")
}

impl Server {
    fn block_name_at(&mut self, world: usize, pos: [i32; 3]) -> String {
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        self.gd.block_name(id).to_string()
    }

    fn solid_block(&mut self, world: usize, pos: [i32; 3]) -> bool {
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        self.gd.is_solid(id)
    }

    fn water_beside(&mut self, world: usize, pos: [i32; 3]) -> bool {
        [[1, 0], [-1, 0], [0, 1], [0, -1]].iter().any(|d| {
            let n = self.block_name_at(world, [pos[0] + d[0], pos[1], pos[2] + d[1]]);
            n.contains("water") || self.water_layer(world, [pos[0] + d[0], pos[1], pos[2] + d[1]])
        })
    }

    /// Whether `block` can stand at `pos`; `against` is the block it was placed against (for
    /// torches and the like), `in_water` whether `pos` held water.
    pub(crate) fn can_stay(&mut self, world: usize, pos: [i32; 3], block: i32, against: Option<[i32; 3]>, in_water: bool) -> bool {
        let name = self.gd.block_name(block).to_string();
        let Some(need) = need_of(&name) else { return true };
        let below = [pos[0], pos[1] - 1, pos[2]];
        let bn = self.block_name_at(world, below);
        match need {
            Need::Soil => is_soil(&bn),
            Need::DeadBushSoil => is_soil(&bn) || is_sand(&bn) || bn.ends_with("terracotta") || bn == "minecraft:hardened_clay",
            Need::AnySolid => self.solid_block(world, below) || bn.ends_with("nylium") || bn == "minecraft:soul_soil",
            Need::Farmland => bn == "minecraft:farmland",
            Need::SoulSand => bn == "minecraft:soul_sand",
            Need::SugarCane => bn == name || ((is_soil(&bn) || is_sand(&bn)) && self.water_beside(world, below)),
            Need::Cactus => {
                (bn == name || is_sand(&bn))
                    && ![[1, 0], [-1, 0], [0, 1], [0, -1]].iter().any(|d| self.solid_block(world, [pos[0] + d[0], pos[1], pos[2] + d[1]]))
            }
            Need::Kelp => in_water && (bn == name || bn == "minecraft:kelp" || self.solid_block(world, below)),
            Need::Seagrass => in_water && self.solid_block(world, below),
            Need::SolidBelow => self.solid_block(world, below),
            Need::Attached => against.is_none_or(|a| self.solid_block(world, a)),
        }
    }

    /// After the block at `pos` was removed: what stood on it and needs it pops off (columns of
    /// sugar cane, cactus and kelp all the way up), dropping its items.
    pub(crate) fn support_update(&mut self, world: usize, pos: [i32; 3]) {
        let mut at = [pos[0], pos[1] + 1, pos[2]];
        for _ in 0..32 {
            if at[1] > ceres_world::MAX_Y {
                return;
            }
            let id = self.worlds.worlds[world].get_block(at, &self.gd);
            let name = self.gd.block_name(id).to_string();
            let Some(need) = need_of(&name) else { return };
            // attached blocks hold on to their side; a standing torch needs the block below
            if need == Need::Attached {
                let standing = self.gd.block_states.get(&id).and_then(|s| s.states.get("torch_facing_direction")).and_then(|v| v.as_str().map(String::from));
                if standing.as_deref() != Some("top") || self.solid_block(world, pos) {
                    return;
                }
            } else {
                let in_water = name.contains("kelp") || name.contains("seagrass");
                if self.can_stay(world, at, id, None, in_water) {
                    return;
                }
            }
            self.pop_block(world, at, id, &name);
            at[1] += 1;
        }
    }

    /// A block falling off: gone (water stays for water plants), its hand-break drops on the floor.
    fn pop_block(&mut self, world: usize, pos: [i32; 3], id: i32, name: &str) {
        let after = if name.contains("kelp") || name.contains("seagrass") {
            self.gd.default_block("minecraft:water").unwrap_or(self.gd.air_id)
        } else {
            self.gd.air_id
        };
        if !self.change_block(world, pos, after) {
            return;
        }
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        self.broadcast(&protocol::level_event(level_event::PARTICLE_DESTROY, center, id), |_, q| q.world == world);
        let loot = if farming::is_crop(name) {
            let g = self.gd.block_states.get(&id).and_then(|s| s.states.get("growth")).and_then(|t| t.as_i64());
            loot::Loot::Items(farming::crop_drops(name, g.unwrap_or(0)).into_iter().map(|(n, c)| (n.to_string(), c)).collect())
        } else {
            loot::block_loot(name, 0, false, false)
        };
        if let loot::Loot::Items(items) = loot {
            for (item, count) in items {
                if let Some(it) = self.simple_item(&item, count.min(64) as u16) {
                    self.drop_at(world, center, it);
                }
            }
        }
    }
}
