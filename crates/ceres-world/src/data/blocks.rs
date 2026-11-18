//! Block state metadata, orientation on placement, double slabs, replaceable/solid checks and
//! interactive toggling (doors, trapdoors, gates, levers).

use super::*;

#[derive(Clone, Debug)]
pub struct BlockStateInfo {
    pub name: String,
    pub states: Tag,
}

/// Normalizes a yaw angle to 0..360 degrees.
fn norm_yaw(yaw: f32) -> f32 {
    yaw.rem_euclid(360.0)
}

/// Cardinal direction the player is looking towards: south (0°), west (90°), north (180°), east (270°).
fn player_cardinal(yaw: f32) -> &'static str {
    match ((norm_yaw(yaw) + 45.0) / 90.0).floor() as i32 & 3 {
        0 => "south",
        1 => "west",
        2 => "north",
        _ => "east",
    }
}

fn opposite_cardinal(c: &str) -> &'static str {
    match c {
        "north" => "south",
        "south" => "north",
        "west" => "east",
        _ => "west",
    }
}

fn cardinal_to_facing(c: &str) -> i32 {
    match c {
        "north" => 2,
        "south" => 3,
        "west" => 4,
        _ => 5,
    }
}

impl GameData {
    pub fn block_name(&self, id: i32) -> &str {
        self.block_states.get(&id).map(|b| b.name.as_str()).unwrap_or("minecraft:air")
    }

    /// Modifies a block state's NBT properties and looks up the resulting runtime ID.
    pub fn with_states(&self, id: i32, updates: &[(&str, Tag)]) -> Option<i32> {
        let info = self.block_states.get(&id)?;
        let mut st = info.states.clone();
        for (k, v) in updates {
            st.set(k, v.clone());
        }
        self.block_ids.get(&nbt::state_key(&info.name, Some(&st))).copied()
    }

    /// Finds the default state of a block by name ("stone" or "minecraft:stone").
    pub fn default_block(&self, name: &str) -> Option<i32> {
        let full = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
        self.default_blocks.get(&full).copied()
    }

    /// True if placing a block on/into `id` should replace it instead of building against it.
    pub fn is_replaceable(&self, id: i32) -> bool {
        if id == self.air_id {
            return true;
        }
        let Some(info) = self.block_states.get(&id) else { return false };
        let short = info.name.strip_prefix("minecraft:").unwrap_or(&info.name);
        if short == "snow_layer" {
            return info.states.get("height").and_then(|t| t.as_i64()).unwrap_or(0) == 0;
        }
        matches!(
            short,
            "air"
                | "water"
                | "flowing_water"
                | "lava"
                | "flowing_lava"
                | "short_grass"
                | "tallgrass"
                | "fern"
                | "deadbush"
                | "vine"
                | "glow_lichen"
                | "fire"
                | "soul_fire"
                | "structure_void"
                | "light_block_0"
                | "light_block_1"
                | "light_block_2"
                | "light_block_3"
                | "light_block_4"
                | "light_block_5"
                | "light_block_6"
                | "light_block_7"
                | "light_block_8"
                | "light_block_9"
                | "light_block_10"
                | "light_block_11"
                | "light_block_12"
                | "light_block_13"
                | "light_block_14"
                | "light_block_15"
        )
    }

    /// True if entities/players collide with this block (used for safe spawn, item physics and AABB check).
    pub fn is_solid(&self, id: i32) -> bool {
        if self.is_replaceable(id) {
            return false;
        }
        let short = self.block_name(id).strip_prefix("minecraft:").unwrap_or("");
        if short.ends_with("_sign")
            || short.ends_with("_hanging_sign")
            || short.ends_with("_button")
            || short.ends_with("_pressure_plate")
            || short.ends_with("_sapling")
            || short.ends_with("_flower")
            || short.ends_with("_tulip")
            || short.ends_with("_carpet")
            || short.ends_with("_torch")
            || short.ends_with("_banner")
            || short.contains("rail")
        {
            return false;
        }
        !matches!(
            short,
            "torch"
                | "soul_torch"
                | "redstone_torch"
                | "lever"
                | "ladder"
                | "scaffolding"
                | "cobweb"
                | "sugar_cane"
                | "reeds"
                | "wheat"
                | "carrots"
                | "potatoes"
                | "beetroot"
                | "dandelion"
                | "poppy"
                | "blue_orchid"
                | "allium"
                | "azure_bluet"
                | "oxeye_daisy"
                | "cornflower"
                | "lily_of_the_valley"
                | "TORCH"
                | "brown_mushroom"
                | "red_mushroom"
                | "portal"
                | "end_portal"
                | "open_eyeblossom"
                | "closed_eyeblossom"
        )
    }

    pub fn is_sign(&self, id: i32) -> bool {
        let s = self.block_name(id);
        s.ends_with("_sign") && !s.contains("hanging")
    }

    /// Attempts to combine a slab being placed with an existing matching single slab into a double slab.
    pub fn try_merge_double_slab(&self, existing_id: i32, placing_id: i32) -> Option<i32> {
        let ex = self.block_states.get(&existing_id)?;
        let pl = self.block_states.get(&placing_id)?;
        if ex.name != pl.name || !ex.name.ends_with("_slab") || ex.name.contains("double") {
            return None;
        }
        let double_name = if let Some(rest) = ex.name.strip_prefix("minecraft:") {
            format!("minecraft:{}_double_slab", rest.trim_end_matches("_slab"))
        } else {
            return None;
        };
        let mut st = ex.states.clone();
        st.set("minecraft:vertical_half", Tag::String("bottom".into()));
        self.block_ids
            .get(&nbt::state_key(&double_name, Some(&st)))
            .or_else(|| self.default_blocks.get(&double_name))
            .copied()
    }

    /// Computes the oriented block runtime ID when a player places `base_id` against `face` at `click_pos`.
    pub fn orient_placement(&self, base_id: i32, face: u8, click_pos: [f32; 3], yaw: f32, pitch: f32) -> i32 {
        let Some(info) = self.block_states.get(&base_id) else { return base_id };
        let st = &info.states;
        let short = info.name.strip_prefix("minecraft:").unwrap_or(&info.name);
        let look = player_cardinal(yaw);
        let facing_player = opposite_cardinal(look);

        // Standing vs wall sign when placing a sign item
        if short.ends_with("_standing_sign") && face >= 2 {
            let wall_name = info.name.replace("_standing_sign", "_wall_sign");
            let wall_state = Tag::Compound(vec![("facing_direction".into(), Tag::Int(face as i32))]);
            if let Some(&wid) = self.block_ids.get(&nbt::state_key(&wall_name, Some(&wall_state))) {
                return wid;
            }
        }
        if short.ends_with("_standing_sign") {
            let dir = (((norm_yaw(yaw) + 180.0 + 11.25) / 22.5).floor() as i32) & 15;
            if let Some(id) = self.with_states(base_id, &[("ground_sign_direction", Tag::Int(dir))]) {
                return id;
            }
        }

        // Logs, pillars, basalt, quartz pillars, hay, bone block
        if st.get("pillar_axis").is_some() {
            let axis = match face {
                0 | 1 => "y",
                2 | 3 => "z",
                _ => "x",
            };
            if let Some(id) = self.with_states(base_id, &[("pillar_axis", Tag::String(axis.into()))]) {
                return id;
            }
        }

        // Stairs: weirdo_direction (0=east, 1=west, 2=south, 3=north) + upside_down_bit
        if st.get("weirdo_direction").is_some() {
            let dir = match look {
                "east" => 0,
                "west" => 1,
                "south" => 2,
                _ => 3,
            };
            let top = face == 0 || (face != 1 && click_pos[1] > 0.5);
            if let Some(id) = self
                .with_states(base_id, &[("weirdo_direction", Tag::Int(dir)), ("upside_down_bit", Tag::Byte(top as i8))])
            {
                return id;
            }
        }

        // Slabs: minecraft:vertical_half = "bottom" | "top"
        if st.get("minecraft:vertical_half").is_some() {
            let half = if face == 0 || (face != 1 && click_pos[1] > 0.5) { "top" } else { "bottom" };
            if let Some(id) = self.with_states(base_id, &[("minecraft:vertical_half", Tag::String(half.into()))]) {
                return id;
            }
        }

        // Trapdoors: direction (0=east, 1=west, 2=south, 3=north) + upside_down_bit
        if short.ends_with("trapdoor") && st.get("direction").is_some() {
            let dir = match facing_player {
                "east" => 0,
                "west" => 1,
                "south" => 2,
                _ => 3,
            };
            let top = face == 0 || (face != 1 && click_pos[1] > 0.5);
            if let Some(id) =
                self.with_states(base_id, &[("direction", Tag::Int(dir)), ("upside_down_bit", Tag::Byte(top as i8))])
            {
                return id;
            }
        }

        // Torches: torch_facing_direction
        if st.get("torch_facing_direction").is_some() {
            let dir = match face {
                1 => "top",
                2 => "south",
                3 => "north",
                4 => "east",
                5 => "west",
                _ => "top",
            };
            if let Some(id) = self.with_states(base_id, &[("torch_facing_direction", Tag::String(dir.into()))]) {
                return id;
            }
        }

        // Cardinal horizontal blocks (furnaces, chests, gates, pumpkins, stonecutter, etc.)
        if st.get("minecraft:cardinal_direction").is_some() {
            let c = if short.ends_with("fence_gate") { look } else { facing_player };
            if let Some(id) = self.with_states(base_id, &[("minecraft:cardinal_direction", Tag::String(c.into()))]) {
                return id;
            }
        }

        // Blocks with `minecraft:facing_direction` (string: down, up, north, south, west, east)
        if st.get("minecraft:facing_direction").is_some() {
            let f = if pitch > 60.0 {
                "up"
            } else if pitch < -60.0 {
                "down"
            } else {
                facing_player
            };
            if let Some(id) = self.with_states(base_id, &[("minecraft:facing_direction", Tag::String(f.into()))]) {
                return id;
            }
        }

        // Blocks with integer `facing_direction`
        if st.get("facing_direction").is_some() {
            let dir = if short.ends_with("_button") || short == "end_rod" || short == "lightning_rod" {
                face as i32
            } else if matches!(short, "piston" | "sticky_piston" | "dispenser" | "dropper" | "barrel" | "observer") {
                let raw = if pitch > 60.0 {
                    1
                } else if pitch < -60.0 {
                    0
                } else {
                    cardinal_to_facing(facing_player)
                };
                if short == "observer" { raw ^ 1 } else { raw }
            } else {
                cardinal_to_facing(facing_player)
            };
            if let Some(id) = self.with_states(base_id, &[("facing_direction", Tag::Int(dir))]) {
                return id;
            }
        }

        base_id
    }

    /// Toggles interactive blocks (doors, trapdoors, fence gates, levers) when right-clicked.
    pub fn toggle_interactive(&self, id: i32) -> Option<i32> {
        let info = self.block_states.get(&id)?;
        let open = info.states.get("open_bit")?.as_i64()?;
        self.with_states(id, &[("open_bit", Tag::Byte((open == 0) as i8))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_data() -> GameData {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        GameData::load(&root).expect("load vanilla data")
    }

    #[test]
    fn orient_logs_stairs_slabs_and_gates() {
        let gd = test_data();

        // Oak log on east face -> pillar_axis = "x"
        let log = gd.default_block("minecraft:oak_log").unwrap();
        let log_x = gd.orient_placement(log, 5, [1.0, 0.5, 0.5], 0.0, 0.0);
        assert_eq!(gd.block_states[&log_x].states.get("pillar_axis").unwrap().as_str(), Some("x"));

        // Stone stairs clicked on bottom face -> upside_down_bit = 1
        let stairs = gd.default_block("minecraft:stone_stairs").unwrap();
        let inv = gd.orient_placement(stairs, 0, [0.5, 0.0, 0.5], 180.0, 0.0);
        assert_eq!(gd.block_states[&inv].states.get("upside_down_bit").unwrap().as_i64(), Some(1));
        assert_eq!(gd.block_states[&inv].states.get("weirdo_direction").unwrap().as_i64(), Some(3));

        // Slab merge into double slab
        if let Some(slab) =
            gd.default_block("minecraft:stone_block_slab").or_else(|| gd.default_block("minecraft:oak_slab"))
        {
            let top = gd.orient_placement(slab, 0, [0.5, 0.0, 0.5], 0.0, 0.0);
            assert_eq!(gd.block_states[&top].states.get("minecraft:vertical_half").unwrap().as_str(), Some("top"));
            if let Some(double) = gd.try_merge_double_slab(slab, slab) {
                assert!(gd.block_name(double).contains("double_slab"));
            }
        }

        // Interactive toggle on trapdoor
        let trap = gd.default_block("minecraft:trapdoor").unwrap();
        let opened = gd.toggle_interactive(trap).expect("toggle trapdoor");
        assert_ne!(trap, opened);
        assert_eq!(gd.toggle_interactive(opened), Some(trap));

        // Stack sizes
        let sword = gd.find_item("diamond_sword").unwrap();
        assert_eq!(sword.max_stack, 1);
        let pearl = gd.find_item("ender_pearl").unwrap();
        assert_eq!(pearl.max_stack, 16);
        let stone = gd.find_item("stone").unwrap();
        assert_eq!(stone.max_stack, 64);
    }
}
