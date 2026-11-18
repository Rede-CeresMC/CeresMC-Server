//! Breaking, placing, orienting, interacting with blocks and editing signs.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::{ItemDesc, UseItem, block_action, level_event, use_item_action, window};

/// Block position next to `pos` on `face` (0 down, 1 up, 2 north, 3 south, 4 west, 5 east).
pub(crate) fn side(pos: [i32; 3], face: u8) -> Option<[i32; 3]> {
    let [x, y, z] = pos;
    Some(match face {
        0 => [x, y - 1, z],
        1 => [x, y + 1, z],
        2 => [x, y, z - 1],
        3 => [x, y, z + 1],
        4 => [x - 1, y, z],
        5 => [x + 1, y, z],
        _ => return None,
    })
}

/// Players may only touch blocks within this distance of their eyes.
const REACH: f32 = 12.0;
const EYE_HEIGHT: f32 = 1.62;

fn default_sign_nbt(pos: [i32; 3]) -> Tag {
    let text_side = || {
        Tag::Compound(vec![
            ("Text".into(), Tag::String(String::new())),
            ("SignTextColor".into(), Tag::Int(-16777216)),
            ("IgnoreLighting".into(), Tag::Byte(0)),
            ("PersistFormatting".into(), Tag::Byte(1)),
            ("HideGlowOutline".into(), Tag::Byte(0)),
            ("TextOwner".into(), Tag::String(String::new())),
        ])
    };
    Tag::Compound(vec![
        ("id".into(), Tag::String("Sign".into())),
        ("x".into(), Tag::Int(pos[0])),
        ("y".into(), Tag::Int(pos[1])),
        ("z".into(), Tag::Int(pos[2])),
        ("isMovable".into(), Tag::Byte(1)),
        ("FrontText".into(), text_side()),
        ("BackText".into(), text_side()),
        ("IsWaxed".into(), Tag::Byte(0)),
        ("LockedForEditingBy".into(), Tag::Long(-1)),
    ])
}

impl Server {
    pub(crate) fn in_reach(p: &Player, pos: [i32; 3]) -> bool {
        let d = |i: usize| p.pos[i] - (pos[i] as f32 + 0.5);
        d(0) * d(0) + d(1) * d(1) + d(2) * d(2) <= REACH * REACH
    }

    /// True if a full block at `pos` intersects any living non-spectator player in `world`.
    pub(crate) fn collides_with_player(&self, world: usize, pos: [i32; 3]) -> bool {
        let (bx0, bx1) = (pos[0] as f32, pos[0] as f32 + 1.0);
        let (by0, by1) = (pos[1] as f32, pos[1] as f32 + 1.0);
        let (bz0, bz1) = (pos[2] as f32, pos[2] as f32 + 1.0);
        for p in self.players.values() {
            // spectators and players with no-clip do not block placement: blocks go right where they stand
            if p.state != State::Spawned || p.world != world || p.dead || p.gamemode == 6 || p.noclip {
                continue;
            }
            let feet_y = p.pos[1] - EYE_HEIGHT;
            let (px0, px1) = (p.pos[0] - 0.29, p.pos[0] + 0.29);
            let (py0, py1) = (feet_y + 0.01, feet_y + p.actor_state.hitbox_height() - 0.01);
            let (pz0, pz1) = (p.pos[2] - 0.29, p.pos[2] + 0.29);
            if bx0 < px1 && bx1 > px0 && by0 < py1 && by1 > py0 && bz0 < pz1 && bz1 > pz0 {
                return true;
            }
        }
        false
    }

    /// Tells one player what is really at `pos` (undoes a client prediction we rejected).
    fn resync(&mut self, sid: SessionId, pos: [i32; 3]) {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return };
        let id = self.worlds.worlds[world].get_block(pos, &self.gd);
        let be = self.worlds.worlds[world].get_block_entity(pos, &self.gd);
        if let Some(p) = self.players.get_mut(&sid) {
            p.out.push(protocol::update_block(pos, id));
            if let Some(tag) = be {
                p.out.push(protocol::block_actor_data(pos, &tag));
            }
        }
    }

    /// Changes a block and shows it to everyone in that world.
    pub(crate) fn change_block(&mut self, world: usize, pos: [i32; 3], id: i32) -> bool {
        if !self.worlds.worlds[world].set_block(pos, id, &self.gd) {
            return false;
        }
        let pk = protocol::update_block(pos, id);
        self.broadcast(&pk, |_, p| p.world == world);
        self.queue_gravity(world, pos);
        self.queue_liquid(world, pos);
        true
    }

    pub(crate) fn break_block(&mut self, sid: SessionId, pos: [i32; 3]) {
        let Some(p) = self.players.get(&sid) else { return };
        let (world, gm, dead) = (p.world, p.gamemode, p.dead);
        if dead || gm == 2 || gm == 6 || !Self::in_reach(p, pos) {
            self.resync(sid, pos);
            return;
        }
        let old = self.worlds.worlds[world].get_block(pos, &self.gd);
        let air = self.gd.air_id;
        if old == air {
            return;
        }
        // Bedrock is unbreakable in survival
        if gm == 0 && self.gd.block_name(old) == "minecraft:bedrock" {
            self.resync(sid, pos);
            return;
        }
        let ev = self.block_event_json("block_break", sid, world, pos, old);
        if !self.plugins_allow(ev) {
            self.resync(sid, pos);
            return;
        }
        let mut contents = if containers::container_kind(self.gd.block_name(old)).is_some() {
            self.take_container_items(world, pos)
        } else {
            Vec::new()
        };
        if self.gd.block_name(old) == "minecraft:jukebox" {
            contents.extend(self.eject_jukebox(world, pos));
        }
        let bed_color = if self.gd.block_name(old) == "minecraft:bed" {
            self.worlds.worlds[world]
                .get_block_entity(pos, &self.gd)
                .and_then(|t| t.get("color").and_then(|v| v.as_i64()))
                .unwrap_or(0) as u32
        } else {
            0
        };
        if !self.change_block(world, pos, air) {
            self.resync(sid, pos);
            return;
        }
        self.break_partner(world, pos, old);
        self.swing_arm(sid);
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        // a broken container spills what it held, in every game mode (like vanilla)
        for item in contents {
            let rx = (rand::random::<f32>() - 0.5) * 0.2;
            let rz = (rand::random::<f32>() - 0.5) * 0.2;
            self.spawn_item(world, center, [rx, 0.2, rz], item, 10);
        }
        // break particles and sound, for everyone (the breaker too, like vanilla servers do)
        let destroy_fx = protocol::level_event(level_event::PARTICLE_DESTROY, center, old);
        self.broadcast(&destroy_fx, |_, q| q.world == world);

        // Survival block drops (nothing when the tool is not good enough, like stone by hand); the
        // tool is checked before it wears, so a tool breaking on its last block still drops it
        let held_item = self.players.get(&sid).map(|p| p.inventory.held_item().clone()).unwrap_or_default();
        let held = held_item.id;
        let (fortune, silk_touch) = loot::loot_enchants(&held_item);
        if self.gd.break_info(old).is_some_and(|b| b.hardness > 0.0) {
            self.wear_held_item(sid, false);
        }
        self.exhaust(sid, food::exhaustion::MINE);
        if gm == 0 && self.gd.can_harvest(old, held) {
            let xp = if silk_touch { 0 } else { xp::block_xp(self.gd.block_name(old)) };
            if xp > 0 {
                self.spawn_xp(world, center, xp);
            }
            let crop_name = self.gd.block_name(old).to_string();
            let loot = if farming::is_crop(&crop_name) {
                let g = self.gd.block_states.get(&old).and_then(|s| s.states.get("growth")).and_then(|t| t.as_i64());
                loot::Loot::Items(
                    farming::crop_drops(&crop_name, g.unwrap_or(0)).into_iter().map(|(n, c)| (n.to_string(), c)).collect(),
                )
            } else {
                let shears = self.gd.items_by_id.get(&held).is_some_and(|i| i.name == "minecraft:shears");
                loot::block_loot(&crop_name, fortune, silk_touch, shears)
            };
            if let loot::Loot::Items(items) = loot {
                for (name, count) in items {
                    let Some(info) = self.gd.items_by_name.get(&name) else { continue };
                    let drop = ItemDesc {
                        id: info.id,
                        count: count.min(64) as u16,
                        meta: 0,
                        stack_id: 0,
                        block_runtime_id: info.block_runtime_id,
                        extra: ceres_world::extra_data(None),
                    };
                    let rx = (rand::random::<f32>() - 0.5) * 0.15;
                    let rz = (rand::random::<f32>() - 0.5) * 0.15;
                    self.spawn_item(world, center, [rx, 0.15, rz], drop, 10);
                }
            } else if let Some(ce) = self.gd.block_to_item.get(&old).cloned() {
                let count = if self.gd.block_name(old).contains("_double_slab") { 2 } else { 1 };
                let drop = ItemDesc {
                    id: ce.id,
                    count,
                    meta: if bed_color != 0 { bed_color } else { ce.meta },
                    stack_id: 0,
                    block_runtime_id: ce.block_runtime_id,
                    extra: ce.extra,
                };
                let rx = (rand::random::<f32>() - 0.5) * 0.15;
                let rz = (rand::random::<f32>() - 0.5) * 0.15;
                self.spawn_item(world, center, [rx, 0.15, rz], drop, 10);
            }
        }
    }

    pub(crate) fn place_block(&mut self, sid: SessionId, u: UseItem) {
        let clicked = u.block_pos;
        let face = u.face;
        let Some(p) = self.players.get(&sid) else { return };
        if p.dead {
            return;
        }
        let (world, gm, sneaking, pitch, yaw) = (p.world, p.gamemode, p.actor_state.sneaking, p.rot[0], p.rot[1]);
        if gm == 2 || gm == 6 || !Self::in_reach(p, clicked) {
            self.resync(sid, clicked);
            if let Some(t) = side(clicked, face) {
                self.resync(sid, t);
            }
            return;
        }

        // Sync hotbar selection if the client sent a valid hotbar slot
        if (0..9).contains(&u.hotbar_slot) {
            if let Some(p) = self.players.get_mut(&sid) {
                if p.inventory.held_slot != u.hotbar_slot as u8 {
                    p.inventory.held_slot = u.hotbar_slot as u8;
                    self.broadcast_equipment(sid);
                }
            }
        }

        let clicked_id = self.worlds.worlds[world].get_block(clicked, &self.gd);

        // 1. Containers (chest, barrel, shulker box) and interactive blocks (doors, trapdoors,
        //    fence gates, levers) when not sneaking
        if !sneaking && self.use_music_block(sid, clicked) {
            if let Some(t) = side(clicked, face) {
                self.resync(sid, t);
            }
            return;
        }
        if !sneaking && self.use_bed(sid, clicked) {
            self.swing_arm(sid);
            if let Some(t) = side(clicked, face) {
                self.resync(sid, t);
            }
            return;
        }
        if !sneaking && self.open_container(sid, clicked) {
            self.swing_arm(sid);
            if let Some(t) = side(clicked, face) {
                self.resync(sid, t);
            }
            return;
        }
        if !sneaking {
            // iron doors and trapdoors only open with redstone
            let iron = self.gd.block_name(clicked_id).starts_with("minecraft:iron_");
            if let Some(toggled) = self.gd.toggle_interactive(clicked_id).filter(|_| !iron) {
                self.change_block(world, clicked, toggled);
                self.toggle_partner(world, clicked, toggled);
                let center = [clicked[0] as f32 + 0.5, clicked[1] as f32 + 0.5, clicked[2] as f32 + 0.5];
                let name = self.gd.block_name(clicked_id).to_string();
                let opened = self
                    .gd
                    .block_states
                    .get(&toggled)
                    .and_then(|s| s.states.get("open_bit"))
                    .and_then(|t| t.as_i64())
                    == Some(1);
                let sound = if name.ends_with("trapdoor") {
                    Some(if opened { "trapdoor.open" } else { "trapdoor.close" })
                } else if name.ends_with("door") {
                    Some(if opened { "door.open" } else { "door.close" })
                } else {
                    None
                };
                match sound {
                    Some(s) => self.broadcast(&protocol::level_sound_event(s, center, toggled), |_, q| q.world == world),
                    None => {
                        let ev = if name.ends_with("lever") { level_event::SOUND_CLICK } else { level_event::SOUND_DOOR };
                        self.broadcast(&protocol::level_event(ev, center, 0), |_, q| q.world == world);
                    }
                }
                self.swing_arm(sid);
                if let Some(t) = side(clicked, face) {
                    self.resync(sid, t);
                }
                return;
            }
        }

        // cauldrons take and give buckets and bottles
        if self.use_cauldron(sid, clicked) {
            if let Some(t) = side(clicked, face) {
                self.resync(sid, t);
            }
            return;
        }
        // buckets fill from or empty into the world instead of placing a block
        if self.use_bucket(sid, clicked, side(clicked, face)) {
            return;
        }
        // a rocket used on a block takes off from the clicked face
        if let Some(t) = side(clicked, face) {
            let from = [t[0] as f32 + 0.5, t[1] as f32 + 0.1, t[2] as f32 + 0.5];
            if self.launch_firework_at(sid, from) {
                self.resync(sid, t);
                return;
            }
        }
        // armor used on a block is put on, like in the air
        if self.equip_held_armor(sid) {
            if let Some(t) = side(clicked, face) {
                self.resync(sid, t);
            }
            return;
        }
        // tools on blocks (hoe, shovel, axe, flint and steel), seeds and bone meal
        if self.use_item_on_block(sid, clicked, face) {
            if let Some(t) = side(clicked, face) {
                self.resync(sid, t);
            }
            return;
        }

        // 2. Determine which block the player is placing
        let (base_block, survival_slot): (i32, Option<u8>) = if gm == 1 {
            let mut b = u.item.block_runtime_id;
            if b == 0 && u.item.id != 0 {
                if let Some(info) = self.gd.items_by_id.get(&u.item.id) {
                    b = info.block_runtime_id;
                    if b == 0 && info.name.ends_with("_sign") {
                        let standing = info.name.replace("_sign", "_standing_sign");
                        b = self.gd.default_block(&standing).unwrap_or(0);
                    }
                    // items placed as the block of the same name (beds, doors...)
                    if b == 0 {
                        b = self.gd.default_block(&info.name).unwrap_or(0);
                    }
                }
            }
            (b, None)
        } else {
            let p = self.players.get(&sid).unwrap();
            let slot = p.inventory.held_slot;
            let held = p.inventory.held_item();
            if held.is_empty() {
                if let Some(t) = side(clicked, face) {
                    self.resync(sid, t);
                }
                self.resync(sid, clicked);
                return;
            }
            let mut b = held.block_runtime_id;
            if b == 0 {
                if let Some(info) = self.gd.items_by_id.get(&held.id) {
                    b = info.block_runtime_id;
                    if b == 0 && info.name.ends_with("_sign") {
                        let standing = info.name.replace("_sign", "_standing_sign");
                        b = self.gd.default_block(&standing).unwrap_or(0);
                    }
                    // items placed as the block of the same name (beds, doors...)
                    if b == 0 {
                        b = self.gd.default_block(&info.name).unwrap_or(0);
                    }
                }
            }
            (b, Some(slot))
        };

        if base_block == 0 || !self.gd.state_nbt.contains_key(&base_block) {
            if let Some(t) = side(clicked, face) {
                self.resync(sid, t);
            }
            self.resync(sid, clicked);
            return;
        }

        // 3. Double slab combination on the clicked slab (inner face) or target slab
        let clicked_half = self
            .gd
            .block_states
            .get(&clicked_id)
            .and_then(|s| s.states.get("minecraft:vertical_half"))
            .and_then(|t| t.as_str());
        let inner_slab_click = matches!((clicked_half, face), (Some("bottom"), 1) | (Some("top"), 0));
        if inner_slab_click {
            if let Some(double_id) = self.gd.try_merge_double_slab(clicked_id, base_block) {
                if !self.collides_with_player(world, clicked) && self.change_block(world, clicked, double_id) {
                    self.finish_placement(sid, world, clicked, double_id, survival_slot);
                    return;
                }
            }
        }

        // 4. Replaceable clicked block vs adjacent target block
        let (target, place_face) = if self.gd.is_replaceable(clicked_id) {
            (clicked, 1)
        } else {
            let Some(adj) = side(clicked, face) else { return };
            (adj, face)
        };

        let Some(p) = self.players.get(&sid) else { return };
        if !Self::in_reach(p, target) {
            self.resync(sid, target);
            self.resync(sid, clicked);
            return;
        }

        let target_existing = self.worlds.worlds[world].get_block(target, &self.gd);
        let final_block = if let Some(double_id) = self.gd.try_merge_double_slab(target_existing, base_block) {
            double_id
        } else if self.gd.is_replaceable(target_existing) {
            self.gd.orient_placement(base_block, place_face, u.click_pos, yaw, pitch)
        } else {
            self.resync(sid, target);
            self.resync(sid, clicked);
            return;
        };

        let ev = self.block_event_json("block_place", sid, world, target, final_block);
        if !self.plugins_allow(ev) {
            self.resync(sid, target);
            self.resync(sid, clicked);
            if let Some(slot) = survival_slot {
                if let Some(p) = self.players.get_mut(&sid) {
                    let item = p.inventory.main[slot as usize].clone();
                    p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &item));
                }
            }
            return;
        }
        // doors, tall plants and beds take two blocks
        let item_meta = if gm == 1 {
            u.item.meta
        } else {
            self.players.get(&sid).map(|p| p.inventory.held_item().meta).unwrap_or(0)
        };
        if let Some(placed) = self.place_two_part(world, target, final_block, yaw, item_meta) {
            if placed {
                let placed_id = self.worlds.worlds[world].get_block(target, &self.gd);
                self.finish_placement(sid, world, target, placed_id, survival_slot);
            } else {
                self.resync(sid, target);
                self.resync(sid, clicked);
            }
            return;
        }

        if self.gd.is_solid(final_block) && self.collides_with_player(world, target) {
            self.resync(sid, target);
            self.resync(sid, clicked);
            return;
        }

        if !self.change_block(world, target, final_block) {
            self.resync(sid, target);
            return;
        }
        self.finish_placement(sid, world, target, final_block, survival_slot);
    }

    /// JSON of a block event for plugins.
    pub(crate) fn block_event_json(&self, event: &str, sid: SessionId, world: usize, pos: [i32; 3], block: i32) -> serde_json::Value {
        serde_json::json!({
            "event": event,
            "player": self.players.get(&sid).map(|p| p.name.clone()).unwrap_or_default(),
            "block": self.gd.block_name(block),
            "world": self.worlds.worlds[world].folder,
            "x": pos[0], "y": pos[1], "z": pos[2],
        })
    }

    fn finish_placement(
        &mut self,
        sid: SessionId,
        world: usize,
        pos: [i32; 3],
        placed_id: i32,
        survival_slot: Option<u8>,
    ) {
        self.swing_arm(sid);
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        // the placer hears it too: the client does not play it by itself
        let snd = protocol::level_sound_event("place", center, placed_id);
        self.broadcast(&snd, |_, q| q.world == world);

        if let Some(slot) = survival_slot {
            if let Some(p) = self.players.get_mut(&sid) {
                if let Some(rem) = p.inventory.consume_slot(slot, 1) {
                    p.out.push(protocol::inventory_slot(window::INVENTORY as u32, slot as u32, &rem));
                }
            }
            self.broadcast_equipment(sid);
        }

        // containers get their (empty) block entity right away, so hoppers work before anyone opens them
        if let Some((_, tile_id, _)) = containers::container_kind(self.gd.block_name(placed_id)) {
            if !tile_id.is_empty() && self.worlds.worlds[world].get_block_entity(pos, &self.gd).is_none() {
                let tile = ceres_codec::nbt::Tag::Compound(vec![
                    ("id".into(), ceres_codec::nbt::Tag::String(tile_id.into())),
                    ("isMovable".into(), ceres_codec::nbt::Tag::Byte(1)),
                    ("Items".into(), ceres_codec::nbt::Tag::List(10, Vec::new())),
                ]);
                self.worlds.worlds[world].set_block_entity(pos, tile, &self.gd);
            }
        }
        self.pair_new_chest(world, pos);
        // If a sign was placed, create its BlockEntity NBT and open the sign editor on the placer!
        if self.gd.is_sign(placed_id) {
            let nbt = default_sign_nbt(pos);
            self.worlds.worlds[world].set_block_entity(pos, nbt.clone(), &self.gd);
            let be_pk = protocol::block_actor_data(pos, &nbt);
            self.broadcast(&be_pk, |_, q| q.world == world);
            if let Some(p) = self.players.get_mut(&sid) {
                p.out.push(protocol::open_sign(pos, true));
                p.editing_sign = Some((pos, Instant::now()));
            }
        }
    }

    /// Client edited a sign (BlockActorDataPacket 0x38).
    pub(crate) fn on_block_actor_data(&mut self, sid: SessionId, r: &mut R) -> Res<()> {
        let (pos, incoming) = protocol::read_block_actor_data(r)?;
        let Some(p) = self.players.get_mut(&sid) else { return Ok(()) };
        // only the sign whose editor we opened for this player, for a limited time
        let allowed = matches!(p.editing_sign, Some((spos, at)) if spos == pos && at.elapsed().as_secs() < 120);
        p.editing_sign = None;
        if !allowed || p.dead || p.gamemode == 2 || p.gamemode == 6 || !Self::in_reach(p, pos) {
            return Ok(());
        }
        let world = p.world;
        let bid = self.worlds.worlds[world].get_block(pos, &self.gd);
        if !self.gd.is_sign(bid) {
            return Ok(());
        }
        let mut sign =
            self.worlds.worlds[world].get_block_entity(pos, &self.gd).unwrap_or_else(|| default_sign_nbt(pos));
        if sign.get("IsWaxed").and_then(|t| t.as_i64()).unwrap_or(0) != 0 {
            return Ok(());
        }
        for side_key in ["FrontText", "BackText"] {
            if let Some(text) = incoming.get(side_key).and_then(|t| t.get("Text")).and_then(|t| t.as_str()) {
                let clean: String = text.chars().filter(|c| *c == '\n' || !c.is_control()).take(256).collect();
                let mut side_compound = sign.get(side_key).cloned().unwrap_or_else(Tag::empty_compound);
                side_compound.set("Text", Tag::String(clean));
                sign.set(side_key, side_compound);
            }
        }
        self.worlds.worlds[world].set_block_entity(pos, sign.clone(), &self.gd);
        let pk = protocol::block_actor_data(pos, &sign);
        self.broadcast(&pk, |_, q| q.world == world);
        Ok(())
    }

    /// Survival mining must take as long as vanilla says (hardness, tool, tier, Efficiency).
    /// A little faster is tolerated for lag; much faster (instant mining, nukers) is refused.
    fn mining_done(&mut self, sid: SessionId, pos: [i32; 3]) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        if p.gamemode == 1 {
            return true;
        }
        let block = self.worlds.worlds[p.world].get_block(pos, &self.gd);
        let held = p.inventory.held_item();
        let Some(needed) = self.gd.break_time(block, held.id, enchant_level(held, ENCHANT_EFFICIENCY)) else {
            return false; // unbreakable
        };
        let needed = needed / p.mining_multiplier();
        let elapsed = match p.breaking {
            Some((at, since)) if at == pos => since.elapsed().as_secs_f32(),
            _ => 0.0,
        };
        let ok = elapsed + 0.15 >= needed * 0.8;
        if !ok && ceres_net::debug_enabled() {
            println!("[Debug] {sid}: block broken too fast ({elapsed:.2}s of {needed:.2}s) at {pos:?}");
        }
        ok
    }

    /// Every 5 ticks while someone mines: hit particles on the face being mined, the hit sound and
    /// the arm swing (vanilla's punch effects).
    pub(crate) fn tick_punch_effects(&mut self) {
        if self.ticks % 5 != 0 {
            return;
        }
        let miners: Vec<(SessionId, usize, [i32; 3], u8)> = self
            .players
            .iter()
            .filter_map(|(s, p)| p.punching.map(|(pos, face)| (*s, p.world, pos, face)))
            .collect();
        for (sid, world, pos, face) in miners {
            let block = self.worlds.worlds[world].get_block(pos, &self.gd);
            if block == self.gd.air_id {
                if let Some(p) = self.players.get_mut(&sid) {
                    p.punching = None;
                }
                continue;
            }
            let at = [pos[0] as f32, pos[1] as f32, pos[2] as f32];
            let center = [at[0] + 0.5, at[1] + 0.5, at[2] + 0.5];
            let particles = protocol::level_event(level_event::PARTICLE_PUNCH_BLOCK_DOWN + face as i32, at, block);
            self.broadcast(&particles, |_, q| q.world == world);
            let sound = protocol::level_sound_event("hit", center, block);
            self.broadcast(&sound, |_, q| q.world == world);
            self.swing_arm(sid);
        }
    }

    /// Ticks a survival player needs for the block, for the crack animation others see.
    fn break_ticks(&mut self, sid: SessionId, pos: [i32; 3]) -> u32 {
        let Some(p) = self.players.get(&sid) else { return 20 };
        let block = self.worlds.worlds[p.world].get_block(pos, &self.gd);
        let held = p.inventory.held_item();
        let secs = self.gd.break_time(block, held.id, enchant_level(held, ENCHANT_EFFICIENCY)).unwrap_or(0.0)
            / p.mining_multiplier();
        ((secs * 20.0) as u32).max(1)
    }

    pub(crate) fn on_use_item(&mut self, sid: SessionId, u: UseItem) {
        match u.action {
            use_item_action::BREAK_BLOCK => {
                if self.mining_done(sid, u.block_pos) {
                    self.break_block(sid, u.block_pos);
                } else {
                    self.resync(sid, u.block_pos);
                }
            }
            use_item_action::CLICK_BLOCK => self.place_block(sid, u),
            use_item_action::CLICK_AIR => self.on_click_air(sid),
            _ => {}
        }
    }

    pub(crate) fn on_block_action(&mut self, sid: SessionId, action: i32, pos: [i32; 3], face: i32) {
        let Some(world) = self.players.get(&sid).map(|p| p.world) else { return };
        let fpos = [pos[0] as f32, pos[1] as f32, pos[2] as f32];
        match action {
            block_action::START_BREAK | block_action::CONTINUE_DESTROY_BLOCK => {
                // hitting a note block plays it
                if action == block_action::START_BREAK {
                    self.punch_note_block(world, pos);
                }
                // continuing onto a new block starts its timer
                if let Some(p) = self.players.get_mut(&sid) {
                    if p.breaking.is_none_or(|(at, _)| at != pos) {
                        p.breaking = Some((pos, Instant::now()));
                    }
                    // hit particles while mining (survival: creative breaks instantly)
                    p.punching = (p.gamemode != 1).then_some((pos, face.clamp(0, 5) as u8));
                }
                self.swing_arm(sid);
                let ticks = self.break_ticks(sid, pos);
                // the cracks: everyone, the miner too (server-authoritative breaking)
                let pk = protocol::level_event(level_event::BLOCK_START_BREAK, fpos, (65535 / ticks) as i32);
                self.broadcast(&pk, |_, q| q.world == world);
            }
            block_action::ABORT_BREAK | block_action::STOP_BREAK => {
                if let Some(p) = self.players.get_mut(&sid) {
                    p.punching = None;
                }
                if action == block_action::ABORT_BREAK {
                    if let Some(p) = self.players.get_mut(&sid) {
                        p.breaking = None;
                    }
                }
                let pk = protocol::level_event(level_event::BLOCK_STOP_BREAK, fpos, 0);
                self.broadcast(&pk, |_, q| q.world == world);
            }
            block_action::CREATIVE_PLAYER_DESTROY_BLOCK | block_action::PREDICT_DESTROY_BLOCK => {
                let stop = protocol::level_event(level_event::BLOCK_STOP_BREAK, fpos, 0);
                self.broadcast(&stop, |_, q| q.world == world);
                let creative_only = action == block_action::CREATIVE_PLAYER_DESTROY_BLOCK
                    && self.players.get(&sid).is_some_and(|p| p.gamemode != 1);
                if creative_only || !self.mining_done(sid, pos) {
                    self.resync(sid, pos);
                    return;
                }
                if let Some(p) = self.players.get_mut(&sid) {
                    p.breaking = None;
                    p.punching = None;
                }
                self.break_block(sid, pos);
            }
            _ => {}
        }
    }

    /// Writes changed chunks (and level.dat) of every world to disk, plus player states.
    pub(crate) fn save_worlds(&mut self) {
        self.save_all_players();
        self.save_mobs();
        for w in self.worlds.worlds.iter_mut() {
            match w.save(&self.gd) {
                Ok(0) => {}
                Ok(n) => {
                    if ceres_net::debug_enabled() {
                        println!("[World {}] saved {n} chunk(s)", w.folder);
                    }
                }
                Err(e) => eprintln!("[World {}] save failed: {e}", w.folder),
            }
        }
    }
}

const ENCHANT_EFFICIENCY: i16 = 15;

/// Level of an enchantment on an item (0 when it does not have it).
pub(crate) fn enchant_level(item: &ItemDesc, id: i16) -> u8 {
    use ceres_codec::nbt::{self, Flavor};
    // network extra data: 0xffff, version 1, little-endian NBT
    if item.extra.len() <= 3 || item.extra[0] != 0xff || item.extra[1] != 0xff {
        return 0;
    }
    let Ok((_, tag)) = nbt::read_root(&mut R::new(&item.extra[3..]), Flavor::LittleEndian) else { return 0 };
    tag.get("ench")
        .and_then(|l| l.as_list())
        .and_then(|l| l.iter().find(|e| e.get("id").and_then(|v| v.as_i64()) == Some(id as i64)))
        .and_then(|e| e.get("lvl").and_then(|v| v.as_i64()))
        .unwrap_or(0)
        .clamp(0, 255) as u8
}
