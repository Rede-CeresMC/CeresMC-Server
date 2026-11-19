//! Boats and minecarts: the behaviour pack defines them (`minecraft:rideable`, `minecraft:buoyant`
//! ...) without health, so they load as entities with 4 health (as PowerNukkitX's boat). Placing
//! (logic of PowerNukkitX's ItemBoat): a boat item used on the top of a block or on water puts
//! a boat of its wood there, facing the player; a minecart item goes on a rail. Riding is the
//! rideable code (ride.rs). Broken, a vehicle drops its item, except to a creative player.

use super::*;

/// Boat variants (wood) by item, as the client draws them.
const WOODS: [&str; 10] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "bamboo", "cherry", "pale_oak"];

/// (entity, variant) of a vehicle item.
pub(crate) fn vehicle_of(item: &str) -> Option<(&'static str, Option<i32>)> {
    let n = item.strip_prefix("minecraft:")?;
    match n {
        "minecart" => return Some(("minecraft:minecart", None)),
        "chest_minecart" => return Some(("minecraft:chest_minecart", None)),
        "hopper_minecart" => return Some(("minecraft:hopper_minecart", None)),
        "tnt_minecart" => return Some(("minecraft:tnt_minecart", None)),
        "bamboo_raft" => return Some(("minecraft:boat", Some(7))),
        "bamboo_chest_raft" => return Some(("minecraft:chest_boat", Some(7))),
        _ => {}
    }
    let (wood, entity) = if let Some(w) = n.strip_suffix("_chest_boat") {
        (w, "minecraft:chest_boat")
    } else {
        (n.strip_suffix("_boat")?, "minecraft:boat")
    };
    let v = WOODS.iter().position(|w| *w == wood)?;
    Some((entity, Some(v as i32)))
}

/// The item a vehicle drops.
fn item_of(entity: &str, variant: Option<i32>) -> Option<String> {
    let wood = WOODS.get(variant.unwrap_or(0).max(0) as usize).copied().unwrap_or("oak");
    Some(match entity {
        "minecraft:boat" if wood == "bamboo" => "minecraft:bamboo_raft".into(),
        "minecraft:chest_boat" if wood == "bamboo" => "minecraft:bamboo_chest_raft".into(),
        "minecraft:boat" => format!("minecraft:{wood}_boat"),
        "minecraft:chest_boat" => format!("minecraft:{wood}_chest_boat"),
        "minecraft:minecart" | "minecraft:chest_minecart" | "minecraft:hopper_minecart" | "minecraft:tnt_minecart" => entity.to_string(),
        _ => return None,
    })
}

impl Server {
    /// A boat or minecart item used on a block. True when it was placed.
    pub(crate) fn place_vehicle(&mut self, sid: SessionId, clicked: [i32; 3], face: u8) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let (world, gm, yaw) = (p.world, p.gamemode, p.rot[1]);
        let held = p.inventory.held_item().clone();
        let Some(name) = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()) else { return false };
        let Some((entity, variant)) = vehicle_of(&name) else { return false };
        let block = self.worlds.worlds[world].get_block(clicked, &self.gd);
        let bname = self.gd.block_name(block).to_string();
        let pos = if entity.ends_with("minecart") {
            if !bname.contains("rail") {
                return false;
            }
            [clicked[0] as f32 + 0.5, clicked[1] as f32, clicked[2] as f32 + 0.5]
        } else if bname.contains("water") {
            [clicked[0] as f32 + 0.5, clicked[1] as f32 + 0.625, clicked[2] as f32 + 0.5]
        } else if face == 1 {
            [clicked[0] as f32 + 0.5, clicked[1] as f32 + 1.0, clicked[2] as f32 + 0.5]
        } else {
            return false;
        };
        let Some(eid) = self.spawn_entity(world, entity, pos, None) else { return false };
        self.with_mob(eid, |s, m| {
            m.item_variant = variant;
            m.persistent = true;
            m.yaw = (yaw + 90.0).rem_euclid(360.0);
            m.head_yaw = m.yaw;
            s.sync_mob(m);
        });
        if gm != 1 {
            self.consume_held(sid, gm);
        }
        self.swing_arm(sid);
        true
    }

    /// A vehicle broken: its item, unless a creative player broke it.
    pub(crate) fn vehicle_drop(&mut self, m: &Mob, attacker: Option<Ent>) {
        if let Some(Ent::Player(s)) = attacker {
            if self.players.get(&s).is_some_and(|p| p.gamemode == 1) {
                return;
            }
        }
        let Some(name) = item_of(m.id(), m.item_variant) else { return };
        if let Some(it) = self.simple_item(&name, 1) {
            self.drop_at(m.world, [m.pos[0], m.pos[1] + 0.3, m.pos[2]], it);
        }
    }
}
