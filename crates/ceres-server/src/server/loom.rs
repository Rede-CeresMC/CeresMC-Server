//! Loom (logic of PowerNukkitX's `CraftLoomActionProcessor`, LGPL-3.0): a banner and a dye, and
//! for some patterns their banner pattern item (kept, not used up), weave one more pattern onto
//! the banner (at most 6; not on the ominous banner). The client names the pattern by its code
//! (`CraftLoom` request action); the banner and dye are taken once per banner made.

use super::*;
use ceres_codec::nbt::Tag;
use ceres_protocol::ItemDesc;
use durability as dur;

pub(crate) const LOOM_UI: u8 = 0xf9;
pub(crate) const UI_LOOM_INPUT: u8 = 41;
pub(crate) const UI_LOOM_DYE: u8 = 42;
pub(crate) const UI_LOOM_MATERIAL: u8 = 43;
pub(crate) const SLOT_LOOM_INPUT: u8 = 9;
pub(crate) const SLOT_LOOM_DYE: u8 = 10;
pub(crate) const SLOT_LOOM_MATERIAL: u8 = 11;

const MAX_PATTERNS: usize = 6;

/// Every pattern code the loom knows (BannerPatternType of PowerNukkitX).
const CODES: [&str; 42] = [
    "bo", "bri", "mc", "cre", "cr", "cbo", "lud", "rd", "ld", "rud", "flo", "gra", "gru", "hh", "hhb", "vh", "vhr", "moj", "mr", "sku", "ss", "bl", "br", "tl",
    "tr", "sc", "bs", "cs", "dls", "drs", "ls", "ms", "rs", "ts", "bt", "tt", "bts", "tts", "glb", "pig", "flw", "gus",
];

/// The banner pattern item a pattern needs in the loom, if any.
fn needed_item(code: &str) -> Option<&'static str> {
    Some(match code {
        "flo" => "minecraft:flower_banner_pattern",
        "cre" => "minecraft:creeper_banner_pattern",
        "sku" => "minecraft:skull_banner_pattern",
        "moj" => "minecraft:mojang_banner_pattern",
        "glb" => "minecraft:globe_banner_pattern",
        "pig" => "minecraft:piglin_banner_pattern",
        "flw" => "minecraft:flow_banner_pattern",
        "gus" => "minecraft:guster_banner_pattern",
        "bri" => "minecraft:field_masoned_banner_pattern",
        "cbo" => "minecraft:bordure_indented_banner_pattern",
        _ => return None,
    })
}

/// Colour index of a dye item (the banner colours: black 0 ... white 15).
fn dye_colour(name: &str) -> Option<i32> {
    Some(match name.strip_prefix("minecraft:")? {
        "black_dye" | "ink_sac" => 0,
        "red_dye" => 1,
        "green_dye" => 2,
        "brown_dye" | "cocoa_beans" => 3,
        "blue_dye" | "lapis_lazuli" => 4,
        "purple_dye" => 5,
        "cyan_dye" => 6,
        "light_gray_dye" => 7,
        "gray_dye" => 8,
        "pink_dye" => 9,
        "lime_dye" => 10,
        "yellow_dye" => 11,
        "light_blue_dye" => 12,
        "magenta_dye" => 13,
        "orange_dye" => 14,
        "white_dye" | "bone_meal" => 15,
        _ => return None,
    })
}

/// The woven banners (`times` of them), or None when the loom cannot make that.
pub(crate) fn weave(banner: &ItemDesc, dye: &ItemDesc, pattern_item: &ItemDesc, code: &str, times: u8, gd: &GameData) -> Option<ItemDesc> {
    let name = |i: &ItemDesc| if i.is_empty() { "" } else { gd.items_by_id.get(&i.id).map(|x| x.name.as_str()).unwrap_or("") };
    if name(banner) != "minecraft:banner" || !CODES.contains(&code) {
        return None;
    }
    let colour = dye_colour(name(dye))?;
    // the ominous banner (Type 1) cannot be woven
    if matches!(dur::item_field(banner, "Type"), Some(Tag::Int(1))) {
        return None;
    }
    let mut patterns = match dur::item_field(banner, "Patterns") {
        Some(Tag::List(_, l)) => l,
        _ => Vec::new(),
    };
    if patterns.len() >= MAX_PATTERNS {
        return None;
    }
    // a pattern item in the loom only allows its own pattern; some patterns need theirs
    let held = name(pattern_item);
    match needed_item(code) {
        Some(item) if held != item => return None,
        None if !held.is_empty() => return None,
        _ => {}
    }
    let times = times.max(1) as u16;
    if times > banner.count || times > dye.count || times > 16 {
        return None;
    }
    patterns.push(Tag::Compound(vec![("Color".into(), Tag::Int(colour)), ("Pattern".into(), Tag::String(code.into()))]));
    let mut out = banner.clone();
    out.count = times;
    out.stack_id = 0;
    dur::set_item_field(&mut out, "Patterns", Some(Tag::List(10, patterns)));
    Some(out)
}

impl Server {
    /// After a successful weave: the loom's sound for everyone near.
    pub(crate) fn finish_loom(&mut self, sid: SessionId) {
        let Some(c) = self.players.get(&sid).and_then(|p| p.inventory.container.as_ref()) else { return };
        let (world, pos) = (c.world, c.pos);
        let center = [pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5];
        self.broadcast(&protocol::level_sound_event("block.loom.use", center, -1), |_, q| q.world == world);
    }
}
