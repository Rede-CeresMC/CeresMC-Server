//! Items used on a block that bring a mob into the world:
//! - spawn eggs (`<mob>_spawn_egg`): the mob of the behaviour pack appears on the clicked face
//!   (a name given at an anvil becomes its name tag);
//! - buckets with a mob (`cod_bucket`, `axolotl_bucket`, `tadpole_bucket`...): the water is
//!   poured and the mob swims out, the empty bucket stays in the hand.

use super::*;
use ceres_protocol::{ItemDesc, window};

/// The entity a spawn egg makes.
pub(crate) fn egg_entity(item: &str) -> Option<String> {
    let short = item.strip_prefix("minecraft:")?.strip_suffix("_spawn_egg")?;
    Some(match short {
        "villager" => "minecraft:villager_v2".into(),
        "zombie_villager" => "minecraft:zombie_villager_v2".into(),
        "npc" | "agent" => return None,
        s => format!("minecraft:{s}"),
    })
}

/// The mob a bucket holds.
fn bucket_entity(item: &str) -> Option<&'static str> {
    Some(match item {
        "minecraft:cod_bucket" => "minecraft:cod",
        "minecraft:salmon_bucket" => "minecraft:salmon",
        "minecraft:tropical_fish_bucket" => "minecraft:tropicalfish",
        "minecraft:pufferfish_bucket" => "minecraft:pufferfish",
        "minecraft:axolotl_bucket" => "minecraft:axolotl",
        "minecraft:tadpole_bucket" => "minecraft:tadpole",
        _ => return None,
    })
}

impl Server {
    /// Spawn eggs and mob buckets used on a block. True when the held item was one.
    pub(crate) fn use_mob_item_on_block(&mut self, sid: SessionId, clicked: [i32; 3], face: u8) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm) = (p.world, p.gamemode);
        let held = p.inventory.held_item().clone();
        let name = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()).unwrap_or_default();
        let egg = egg_entity(&name).or_else(|| (name == "minecraft:armor_stand").then(|| name.clone()));
        let stand = name == "minecraft:armor_stand";
        let bucket = bucket_entity(&name);
        if egg.is_none() && bucket.is_none() {
            return false;
        }
        let clicked_id = self.worlds.worlds[world].get_block(clicked, &self.gd);
        let at = if self.gd.is_replaceable(clicked_id) { Some(clicked) } else { blocks::side(clicked, face) };
        let Some(at) = at else { return true };
        let here = self.worlds.worlds[world].get_block(at, &self.gd);
        if self.gd.is_solid(here) {
            self.resync_held(sid);
            return true;
        }
        let feet = [at[0] as f32 + 0.5, at[1] as f32, at[2] as f32 + 0.5];
        if let Some(id) = egg {
            let Some(eid) = self.spawn_entity(world, &id, feet, None) else {
                self.resync_held(sid);
                return true;
            };
            if let Some(n) = mobs::items::custom_name(&held) {
                self.with_mob(eid, |_, m| {
                    m.name_tag = Some(n);
                    m.persistent = true;
                });
            }
            if stand {
                // it faces the player, in steps of 45 degrees
                let yaw = self.players.get(&sid).map(|p| p.rot[1]).unwrap_or(0.0) + 180.0;
                let yaw = (yaw / 45.0).round() * 45.0;
                self.with_mob(eid, |_, m| {
                    m.yaw = yaw;
                    m.head_yaw = yaw;
                    m.equip.drop_chance = [1.0; 6];
                });
            }
            if gm != 1 {
                self.consume_held(sid, gm);
            }
            self.swing_arm(sid);
            return true;
        }
        let id = bucket.unwrap();
        self.set_liquid(world, at, liquids::Liquid::Water, 0);
        self.liquid_sound(world, at, "bucket.empty.fish");
        if let Some(eid) = self.spawn_entity(world, id, [feet[0], feet[1] + 0.2, feet[2]], None) {
            // a mob let out of a bucket stays (vanilla marks it as from a bucket)
            self.with_mob(eid, |_, m| m.persistent = true);
        }
        self.swing_arm(sid);
        if gm != 1 {
            let Some(info) = self.gd.items_by_name.get("minecraft:bucket") else { return true };
            let empty = ItemDesc { id: info.id, count: 1, meta: 0, stack_id: 0, block_runtime_id: 0, extra: ceres_world::extra_data(None) };
            let Some(p) = self.players.get_mut(&sid) else { return true };
            let slot = (p.inventory.held_slot as usize).min(8);
            p.inventory.set_main(slot, empty);
            p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &p.inventory.main[slot]));
            self.broadcast_equipment(sid);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn egg_names() {
        assert_eq!(egg_entity("minecraft:cow_spawn_egg").as_deref(), Some("minecraft:cow"));
        assert_eq!(egg_entity("minecraft:villager_spawn_egg").as_deref(), Some("minecraft:villager_v2"));
        assert_eq!(egg_entity("minecraft:npc_spawn_egg"), None);
        assert_eq!(egg_entity("minecraft:cow"), None);
    }
}

const STAGES: [&str; 4] = ["", "exposed_", "weathered_", "oxidized_"];

/// Copper block names without the namespace: "copper_block" is the plain stage of "*_copper".
fn copper_parts(short: &str) -> Option<(bool, usize, String)> {
    if !short.contains("copper") || short.contains("ore") || short.starts_with("raw_") {
        return None;
    }
    let (waxed, rest) = match short.strip_prefix("waxed_") {
        Some(r) => (true, r),
        None => (false, short),
    };
    let rest = if rest == "copper_block" { "copper" } else { rest };
    for (i, s) in STAGES.iter().enumerate().skip(1) {
        if let Some(base) = rest.strip_prefix(s) {
            return Some((waxed, i, base.to_string()));
        }
    }
    Some((waxed, 0, rest.to_string()))
}

fn copper_name(waxed: bool, stage: usize, base: &str) -> String {
    let body = format!("{}{}", STAGES[stage], base);
    let body = if body == "copper" && !waxed { "copper_block".to_string() } else { body };
    if waxed { format!("minecraft:waxed_{body}") } else { format!("minecraft:{body}") }
}

/// Next block for an axe (wax off, else one stage of oxidation off) or honeycomb (wax on).
pub(crate) fn copper_change(block: &str, honeycomb: bool) -> Option<String> {
    let (waxed, stage, base) = copper_parts(block.strip_prefix("minecraft:")?)?;
    if honeycomb {
        return (!waxed).then(|| copper_name(true, stage, &base));
    }
    if waxed {
        return Some(copper_name(false, stage, &base));
    }
    (stage > 0).then(|| copper_name(false, stage - 1, &base))
}

/// The next oxidation stage of an unwaxed copper block (None when waxed or fully oxidized).
pub(crate) fn copper_oxidized(block: &str) -> Option<String> {
    let (waxed, stage, base) = copper_parts(block.strip_prefix("minecraft:")?)?;
    (!waxed && stage < 3).then(|| copper_name(false, stage + 1, &base))
}

impl Server {
    /// Axe or honeycomb on a copper block. True when it changed something.
    pub(crate) fn use_on_copper(&mut self, sid: SessionId, clicked: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm) = (p.world, p.gamemode);
        let held = p.inventory.held_item().clone();
        let name = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()).unwrap_or_default();
        let honeycomb = name == "minecraft:honeycomb";
        let axe = self.gd.tool_info(held.id).is_some_and(|t| t.tool == ceres_world::Tool::Axe);
        if !honeycomb && !axe {
            return false;
        }
        let block = self.worlds.worlds[world].get_block(clicked, &self.gd);
        let Some(next) = copper_change(self.gd.block_name(block), honeycomb) else { return false };
        let Some(id) = self.gd.with_name(block, &next) else { return false };
        self.change_block(world, clicked, id);
        if honeycomb {
            let center = [clicked[0] as f32 + 0.5, clicked[1] as f32 + 0.5, clicked[2] as f32 + 0.5];
            self.broadcast(&protocol::level_sound_event("copper.wax.on", center, -1), |_, q| q.world == world);
            self.swing_arm(sid);
            if gm != 1 {
                self.consume_held(sid, gm);
            }
        } else {
            self.tool_used(sid, clicked, "scrape");
        }
        true
    }
}

#[cfg(test)]
mod copper_tests {
    use super::copper_change;

    #[test]
    fn copper_steps() {
        assert_eq!(copper_change("minecraft:oxidized_cut_copper", false).as_deref(), Some("minecraft:weathered_cut_copper"));
        assert_eq!(copper_change("minecraft:exposed_copper", false).as_deref(), Some("minecraft:copper_block"));
        assert_eq!(copper_change("minecraft:copper_block", false), None);
        assert_eq!(copper_change("minecraft:copper_block", true).as_deref(), Some("minecraft:waxed_copper"));
        assert_eq!(copper_change("minecraft:waxed_copper", false).as_deref(), Some("minecraft:copper_block"));
        assert_eq!(copper_change("minecraft:waxed_weathered_copper_door", false).as_deref(), Some("minecraft:weathered_copper_door"));
        assert_eq!(copper_change("minecraft:exposed_copper_bulb", true).as_deref(), Some("minecraft:waxed_exposed_copper_bulb"));
        assert_eq!(copper_change("minecraft:copper_ore", false), None);
        assert_eq!(super::copper_oxidized("minecraft:copper_block").as_deref(), Some("minecraft:exposed_copper"));
        assert_eq!(super::copper_oxidized("minecraft:oxidized_cut_copper"), None);
        assert_eq!(super::copper_oxidized("minecraft:waxed_copper"), None);
    }
}

impl Server {
    /// Items used in the air that are not weapons: goat horns (7 s cooldown) and wind charges.
    pub(crate) fn use_item_in_air(&mut self, sid: SessionId) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        if p.dead || p.gamemode == 6 {
            return false;
        }
        let (world, pos, gm) = (p.world, p.pos, p.gamemode);
        let held = p.inventory.held_item().clone();
        let name = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()).unwrap_or_default();
        if self.use_fishing_rod(sid) {
            return true;
        }
        let now = self.ticks;
        let ready = |s: &Server| s.item_cooldowns.get(&(sid, name.clone())).is_none_or(|t| now >= *t);
        match name.as_str() {
            "minecraft:empty_map" => self.create_map(sid),
            "minecraft:goat_horn" => {
                if ready(self) {
                    self.start_cooldown(sid, &name, "goat_horn", 140);
                    let sound = format!("horn_call{}", held.meta.min(7));
                    self.broadcast(&protocol::level_sound_event(&sound, pos, -1), |_, q| q.world == world);
                }
                true
            }
            "minecraft:wind_charge" => {
                if ready(self) && self.player_throw(sid, "minecraft:wind_charge_projectile") {
                    self.start_cooldown(sid, &name, "wind_charge", 10);
                    if gm != 1 {
                        self.consume_held(sid, gm);
                    }
                    self.swing_arm(sid);
                }
                true
            }
            _ => false,
        }
    }

    /// A fire charge on a block lights the face it was used on.
    pub(crate) fn use_fire_charge(&mut self, sid: SessionId, clicked: [i32; 3], face: u8) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm) = (p.world, p.gamemode);
        if self.gd.items_by_id.get(&p.inventory.held_item().id).is_none_or(|i| i.name != "minecraft:fire_charge") {
            return false;
        }
        let Some(target) = blocks::side(clicked, face) else { return true };
        let here = self.worlds.worlds[world].get_block(target, &self.gd);
        if here == self.gd.air_id {
            if let Some(fire) = self.gd.default_block("minecraft:fire") {
                self.change_block(world, target, fire);
                let at = [target[0] as f32 + 0.5, target[1] as f32 + 0.5, target[2] as f32 + 0.5];
                self.broadcast(&protocol::level_sound_event("ignite", at, -1), |_, q| q.world == world);
                if gm != 1 {
                    self.consume_held(sid, gm);
                }
                self.swing_arm(sid);
            }
        }
        true
    }
}

/// Text colour of each dye on a sign (ARGB), in the order of the wool colours.
const DYE_ARGB: [u32; 16] = [
    0xFFF0F0F0, 0xFFF9801D, 0xFFC74EBD, 0xFF3AB3DA, 0xFFFED83D, 0xFF80C71F, 0xFFF38BAA, 0xFF474F52, 0xFF9D9D97, 0xFF169C9C,
    0xFF8932B8, 0xFF3C44AA, 0xFF835432, 0xFF5E7C16, 0xFFB02E26, 0xFF1D1D21,
];

impl Server {
    /// Dye, glow ink sac, ink sac or honeycomb on a sign: colours the side the player looks at,
    /// makes its text glow (or stop glowing), or waxes the sign so nobody can edit it.
    pub(crate) fn use_on_sign(&mut self, sid: SessionId, clicked: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm, eye) = (p.world, p.gamemode, p.pos);
        let held = p.inventory.held_item().clone();
        let name = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()).unwrap_or_default();
        let short = name.strip_prefix("minecraft:").unwrap_or("");
        let dye = short.strip_suffix("_dye").and_then(|c| mobs::WOOL.iter().position(|w| *w == c));
        if dye.is_none() && !matches!(short, "glow_ink_sac" | "ink_sac" | "honeycomb") {
            return false;
        }
        let block = self.worlds.worlds[world].get_block(clicked, &self.gd);
        if !self.gd.is_sign(block) {
            return false;
        }
        let Some(mut sign) = self.worlds.worlds[world].get_block_entity(clicked, &self.gd) else { return false };
        if sign.get("IsWaxed").and_then(|t| t.as_i64()).unwrap_or(0) != 0 {
            let at = [clicked[0] as f32 + 0.5, clicked[1] as f32 + 0.5, clicked[2] as f32 + 0.5];
            self.broadcast(&protocol::level_sound_event("block.sign.waxed_interact_fail", at, -1), |_, q| q.world == world);
            return true;
        }
        // which side faces the player
        let states = self.gd.block_states.get(&block).map(|s| s.states.clone());
        let normal = match states.as_ref().and_then(|s| s.get("ground_sign_direction")).and_then(|t| t.as_i64()) {
            Some(d) => {
                let a = (d as f32 * 22.5).to_radians();
                [-a.sin(), a.cos()]
            }
            None => match states.as_ref().and_then(|s| s.get("facing_direction")).and_then(|t| t.as_i64()) {
                Some(2) => [0.0, -1.0],
                Some(3) => [0.0, 1.0],
                Some(4) => [-1.0, 0.0],
                _ => [1.0, 0.0],
            },
        };
        let to_player = [eye[0] - (clicked[0] as f32 + 0.5), eye[2] - (clicked[2] as f32 + 0.5)];
        let side = if normal[0] * to_player[0] + normal[1] * to_player[1] >= 0.0 { "FrontText" } else { "BackText" };
        let mut text = sign.get(side).cloned().unwrap_or_else(ceres_codec::nbt::Tag::empty_compound);
        let sound = match (dye, short) {
            (Some(c), _) => {
                text.set("SignTextColor", ceres_codec::nbt::Tag::Int(DYE_ARGB[c] as i32));
                ""
            }
            (None, "glow_ink_sac") => {
                text.set("IgnoreLighting", ceres_codec::nbt::Tag::Byte(1));
                ""
            }
            (None, "ink_sac") => {
                text.set("IgnoreLighting", ceres_codec::nbt::Tag::Byte(0));
                ""
            }
            _ => {
                sign.set("IsWaxed", ceres_codec::nbt::Tag::Byte(1));
                "copper.wax.on"
            }
        };
        if short != "honeycomb" {
            sign.set(side, text);
        }
        self.worlds.worlds[world].set_block_entity(clicked, sign.clone(), &self.gd);
        self.broadcast(&protocol::block_actor_data(clicked, &sign), |_, q| q.world == world);
        let at = [clicked[0] as f32 + 0.5, clicked[1] as f32 + 0.5, clicked[2] as f32 + 0.5];
        if !sound.is_empty() {
            self.broadcast(&protocol::level_sound_event(sound, at, -1), |_, q| q.world == world);
        }
        self.swing_arm(sid);
        if gm != 1 {
            self.consume_held(sid, gm);
        }
        true
    }
}

impl Server {
    /// True while the item is cooling down for this player.
    pub(crate) fn on_cooldown(&self, sid: SessionId, item: &str) -> bool {
        self.item_cooldowns.get(&(sid, item.to_string())).is_some_and(|t| self.ticks < *t)
    }

    /// Starts an item cooldown: kept by the server and shown greyed out on the client.
    pub(crate) fn start_cooldown(&mut self, sid: SessionId, item: &str, category: &str, ticks: u32) {
        let until = self.ticks + ticks as u64;
        self.item_cooldowns.insert((sid, item.to_string()), until);
        if let Some(p) = self.players.get_mut(&sid) {
            p.out.push(protocol::start_item_cooldown(category, ticks as i32));
        }
    }
}

impl Server {
    fn frame_with_eye(&mut self, world: usize, p: [i32; 3]) -> bool {
        let id = self.worlds.worlds[world].get_block(p, &self.gd);
        self.gd.block_name(id) == "minecraft:end_portal_frame"
            && self.gd.block_states.get(&id).and_then(|s| s.states.get("end_portal_eye_bit")).and_then(|t| t.as_i64()).unwrap_or(0) != 0
    }

    /// An eye of ender in an end portal frame; when the 12 frames around a 3x3 hole all have
    /// their eye, the portal opens. True when the held item and block were those.
    pub(crate) fn use_ender_eye(&mut self, sid: SessionId, clicked: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm) = (p.world, p.gamemode);
        if self.gd.items_by_id.get(&p.inventory.held_item().id).is_none_or(|i| i.name != "minecraft:ender_eye") {
            return false;
        }
        let block = self.worlds.worlds[world].get_block(clicked, &self.gd);
        if self.gd.block_name(block) != "minecraft:end_portal_frame" {
            return false;
        }
        if self.frame_with_eye(world, clicked) {
            return true;
        }
        let Some(with_eye) = self.gd.with_states(block, &[("end_portal_eye_bit", ceres_codec::nbt::Tag::Byte(1))]) else { return true };
        self.change_block(world, clicked, with_eye);
        let at = [clicked[0] as f32 + 0.5, clicked[1] as f32 + 0.5, clicked[2] as f32 + 0.5];
        self.broadcast(&protocol::level_sound_event("block.end_portal_frame.fill", at, -1), |_, q| q.world == world);
        self.swing_arm(sid);
        if gm != 1 {
            self.consume_held(sid, gm);
        }
        // a ring around a 3x3 centre near this frame
        let y = clicked[1];
        for cx in clicked[0] - 2..=clicked[0] + 2 {
            for cz in clicked[2] - 2..=clicked[2] + 2 {
                let mut ring = Vec::new();
                for d in -1..=1 {
                    ring.extend([[cx + d, y, cz - 2], [cx + d, y, cz + 2], [cx - 2, y, cz + d], [cx + 2, y, cz + d]]);
                }
                if !ring.contains(&clicked) || !ring.iter().all(|r| self.frame_with_eye(world, *r)) {
                    continue;
                }
                let Some(portal) = self.gd.default_block("minecraft:end_portal") else { return true };
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        self.change_block(world, [cx + dx, y, cz + dz], portal);
                    }
                }
                let center = [cx as f32 + 0.5, y as f32, cz as f32 + 0.5];
                self.broadcast(&protocol::level_sound_event("block.end_portal.spawn", center, -1), |_, q| q.world == world);
                return true;
            }
        }
        true
    }
}
