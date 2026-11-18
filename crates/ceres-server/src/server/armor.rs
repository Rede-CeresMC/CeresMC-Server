//! Putting armor on from the hand: using a helmet, chestplate, leggings, boots (or an elytra,
//! a turtle shell, a carved pumpkin) puts it in its slot, swapping with what was worn there.

use super::*;

/// Armor slot (0 head, 1 chest, 2 legs, 3 feet) and equip sound of an item.
fn armor_slot(name: &str) -> Option<(usize, &'static str)> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let sound = |material: &str| match material {
        "leather" => "armor.equip_leather",
        "chainmail" => "armor.equip_chain",
        "iron" => "armor.equip_iron",
        "golden" => "armor.equip_gold",
        "diamond" => "armor.equip_diamond",
        "netherite" => "armor.equip_netherite",
        _ => "armor.equip_generic",
    };
    if n == "elytra" {
        return Some((1, "armor.equip_elytra"));
    }
    if n == "turtle_helmet" || n == "carved_pumpkin" || n.ends_with("_head") || n.ends_with("_skull") {
        return Some((0, "armor.equip_generic"));
    }
    let (material, piece) = n.rsplit_once('_')?;
    let slot = match piece {
        "helmet" => 0,
        "chestplate" => 1,
        "leggings" => 2,
        "boots" => 3,
        _ => return None,
    };
    Some((slot, sound(material)))
}

impl Server {
    /// Click in the air with armor: wear it. Returns true when the held item is armor.
    pub(crate) fn equip_held_armor(&mut self, sid: SessionId) -> bool {
        let Some(p) = self.players.get(&sid) else { return false };
        let hand = (p.inventory.held_slot as usize).min(8);
        let held = p.inventory.main[hand].clone();
        let Some(name) = self.gd.items_by_id.get(&held.id).map(|i| i.name.clone()) else { return false };
        let Some((slot, sound)) = armor_slot(&name) else { return false };
        if p.dead || p.gamemode == 6 {
            return true;
        }
        let p = self.players.get_mut(&sid).unwrap();
        let worn = std::mem::take(&mut p.inventory.armor[slot]);
        let mut piece = held.clone();
        piece.count = 1;
        piece.stack_id = p.inventory.alloc_id();
        p.inventory.armor[slot] = piece;
        // the rest of the stack (rare) stays; otherwise the hand gets what was worn
        let mut rest = held;
        rest.count -= 1;
        if rest.count > 0 {
            p.inventory.set_main(hand, rest);
            if !worn.is_empty() {
                let (_, left) = p.inventory.add_item(worn.clone(), &self.gd);
                if left > 0 {
                    let mut drop = worn;
                    drop.count = left;
                    self.drop_from_player(sid, drop);
                }
            }
        } else {
            p.inventory.set_main(hand, worn);
        }
        let p = self.players.get_mut(&sid).unwrap();
        p.inventory.dirty = true;
        let (world, pos) = (p.world, p.pos);
        self.sync_full_inventory(sid);
        self.broadcast_equipment(sid);
        self.broadcast(&protocol::level_sound_event(sound, pos, -1), |_, q| q.world == world);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots() {
        assert_eq!(armor_slot("minecraft:iron_helmet").map(|a| a.0), Some(0));
        assert_eq!(armor_slot("minecraft:diamond_chestplate"), Some((1, "armor.equip_diamond")));
        assert_eq!(armor_slot("minecraft:leather_boots").map(|a| a.0), Some(3));
        assert_eq!(armor_slot("minecraft:elytra").map(|a| a.0), Some(1));
        assert_eq!(armor_slot("minecraft:stone"), None);
        assert_eq!(armor_slot("minecraft:iron_sword"), None);
    }
}
