//! CreativeContent (the creative inventory) built from creative_items.json.
//! Same layout as Altay's CreativeInventoryCache: groups, then items pointing at their group.

use super::*;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;

pub(crate) const CREATIVE_CONTENT: u32 = 0x91;

/// ItemStackExtraData: optional NBT, then empty can-place-on / can-destroy lists.
fn extra_data(nbt_le: Option<&[u8]>) -> Vec<u8> {
    let mut w = W::new();
    match nbt_le {
        Some(n) => {
            w.u16_le(0xffff);
            w.u8(1);
            w.bytes(n);
        }
        None => w.u16_le(0),
    }
    w.u32_le(0);
    w.u32_le(0);
    w.0
}

/// One creative item entry from the JSON as an ItemStack without stack id.
fn write_item(w: &mut W, item: &serde_json::Value, ids: &HashMap<String, i32>, extra_nbt: Option<Vec<u8>>) -> bool {
    let Some(&id) = item["id"].as_str().and_then(|n| ids.get(n)) else { return false };
    let block_id = item["block_state_b64"]
        .as_str()
        .and_then(|b| STANDARD.decode(b).ok())
        .and_then(|b| nbt::read_root(&mut R::new(&b), Flavor::LittleEndian).ok())
        .and_then(|(_, t)| t.get("network_id").and_then(|v| v.as_i64()))
        .unwrap_or(0) as i32;
    let nbt = extra_nbt.or_else(|| item["nbt_b64"].as_str().and_then(|b| STANDARD.decode(b).ok()));
    w.varint(id);
    w.u16_le(1);
    w.uvarint(item["damage"].as_i64().unwrap_or(0) as u32);
    w.varint(block_id);
    let extra = extra_data(nbt.as_deref());
    w.uvarint(extra.len() as u32);
    w.bytes(&extra);
    true
}

pub(crate) fn encode_creative(path: &Path, ids: &HashMap<String, i32>) -> Res<(Vec<u8>, usize)> {
    let data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("creative_items.json: {e}"))?)
            .map_err(|e| e.to_string())?;
    let groups = data["groups"].as_array().ok_or("creative_items.json: no groups")?;
    let items = data["items"].as_array().ok_or("creative_items.json: no items")?;

    let mut body = W::with_capacity(512 * 1024);
    body.uvarint(groups.len() as u32);
    for (i, g) in groups.iter().enumerate() {
        body.u8(g["creative_category"].as_u64().unwrap_or(1) as u8);
        body.string(g["name"].as_str().unwrap_or(""));
        // client glitch since 1.21.60: identical group icons merge groups, so make each one unique
        let tag = Tag::Compound(vec![("___GroupBugWorkaround___".into(), Tag::Int(i as i32))]);
        let mut nbt_w = W::new();
        nbt::write_root(&mut nbt_w, "", &tag, Flavor::LittleEndian);
        if !write_item(&mut body, &g["icon"], ids, Some(nbt_w.0)) {
            body.varint(0); // no icon
            body.u16_le(0);
            body.uvarint(0);
            body.varint(0);
            body.uvarint(0);
        }
    }

    let mut entries = W::with_capacity(512 * 1024);
    let mut count = 0usize;
    for it in items {
        let mut e = W::new();
        e.uvarint(count as u32); // creative item net id
        if !write_item(&mut e, it, ids, None) {
            continue;
        }
        e.uvarint(it["group_index"].as_u64().unwrap_or(0) as u32);
        entries.bytes(&e.0);
        count += 1;
    }
    body.uvarint(count as u32);
    body.bytes(&entries.0);
    Ok((packet(CREATIVE_CONTENT, |w| w.bytes(&body.0)), count))
}
