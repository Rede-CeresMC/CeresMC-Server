//! Resource packs sent to the players (textures, sounds, models, UI...).
//!
//! Put them in `resource_packs/`, as `.zip`/`.mcpack` files or as folders holding a
//! `manifest.json`. With `force-resource-packs=on` (the default) the players must accept them to
//! join. Behaviour packs are never sent to clients in this protocol, so only resource packs count.

use rsa::sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

pub const DIR: &str = "resource_packs";
/// bytes per ResourcePackChunkData
pub const CHUNK_SIZE: usize = 256 * 1024;
/// biggest pack accepted (the client downloads all of it on join)
const MAX_PACK_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 20_000;

pub struct Pack {
    pub uuid: String,
    pub version: String,
    pub name: String,
    pub data: Vec<u8>,
    pub sha256: Vec<u8>,
}

impl Pack {
    pub fn chunk_count(&self) -> u32 {
        self.data.len().div_ceil(CHUNK_SIZE) as u32
    }

    pub fn chunk(&self, index: u32) -> Option<&[u8]> {
        let start = index as usize * CHUNK_SIZE;
        (start < self.data.len()).then(|| &self.data[start..(start + CHUNK_SIZE).min(self.data.len())])
    }
}

/// Loads every pack of `resource_packs/` (creating the folder if needed). Broken ones are skipped
/// with a console line.
pub fn load_all() -> Vec<Pack> {
    let dir = Path::new(DIR);
    if !dir.exists() {
        let _ = std::fs::create_dir_all(dir);
        return Vec::new();
    }
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
        Err(e) => {
            eprintln!("[Packs] cannot read {DIR}/: {e}");
            return Vec::new();
        }
    };
    entries.sort();
    let mut packs: Vec<Pack> = Vec::new();
    for path in entries {
        let shown = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let result = if path.is_dir() {
            zip_folder(&path)
        } else if ext == "zip" || ext == "mcpack" {
            std::fs::metadata(&path)
                .map_err(|e| e.to_string())
                .and_then(|m| if m.len() > MAX_PACK_BYTES { Err("too big".into()) } else { Ok(()) })
                .and_then(|_| std::fs::read(&path).map_err(|e| e.to_string()))
                .and_then(|bytes| normalize_zip(bytes))
        } else {
            continue;
        };
        match result.and_then(Pack::from_zip) {
            Ok(pack) => {
                if packs.iter().any(|p| p.uuid == pack.uuid) {
                    eprintln!("[Packs] {shown}: same uuid as another pack ({}), skipped", pack.uuid);
                    continue;
                }
                println!(
                    "[Packs] {} v{} ({shown}, {:.1} MB)",
                    pack.name,
                    pack.version,
                    pack.data.len() as f64 / (1024.0 * 1024.0)
                );
                packs.push(pack);
            }
            Err(e) => eprintln!("[Packs] {shown}: {e}"),
        }
    }
    packs
}

impl Pack {
    fn from_zip(data: Vec<u8>) -> Result<Pack, String> {
        let files = read_zip(&data)?;
        let manifest = files
            .iter()
            .find(|(name, _)| name == "manifest.json")
            .ok_or("no manifest.json at the root of the pack")?;
        let m = parse_manifest(&manifest.1)?;
        let header = &m["header"];
        let uuid = header["uuid"].as_str().ok_or("manifest.json has no header.uuid")?.to_ascii_lowercase();
        if uuid.len() != 36 || uuid.chars().filter(|c| *c == '-').count() != 4 {
            return Err(format!("bad uuid {uuid}"));
        }
        let version = match &header["version"] {
            serde_json::Value::Array(v) => v.iter().map(|n| n.as_u64().unwrap_or(0).to_string()).collect::<Vec<_>>().join("."),
            serde_json::Value::String(s) => s.clone(),
            _ => return Err("manifest.json has no header.version".into()),
        };
        let name = header["name"].as_str().unwrap_or("pack").to_string();
        let modules = m["modules"].as_array().cloned().unwrap_or_default();
        if !modules.iter().any(|md| md["type"].as_str() == Some("resources")) {
            return Err("not a resource pack (no \"resources\" module); behaviour packs cannot be sent to clients".into());
        }
        let sha256 = Sha256::digest(&data).to_vec();
        Ok(Pack { uuid, version, name, data, sha256 })
    }
}

/// manifest.json, tolerating the `//` comments some packs have.
fn parse_manifest(bytes: &[u8]) -> Result<serde_json::Value, String> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}');
    serde_json::from_str(text).or_else(|_| {
        let cleaned: String = text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        serde_json::from_str(&cleaned).map_err(|e| format!("manifest.json: {e}"))
    })
}

/// The files of a zip archive: (path, contents).
fn read_zip(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16_at = |o: usize| data.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |o: usize| data.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    // end of central directory: the last "PK\x05\x06" (a comment may follow it)
    let eocd = (0..data.len().saturating_sub(21))
        .rev()
        .take(65_557)
        .find(|&i| data[i..].starts_with(b"PK\x05\x06"))
        .ok_or("not a zip file")?;
    let count = u16_at(eocd + 10).ok_or("broken zip")?;
    let mut pos = u32_at(eocd + 16).ok_or("broken zip")?;
    if count > MAX_FILES {
        return Err("too many files".into());
    }
    let mut out = Vec::with_capacity(count);
    let mut total = 0u64;
    for _ in 0..count {
        if !data.get(pos..).is_some_and(|d| d.starts_with(b"PK\x01\x02")) {
            return Err("broken zip central directory".into());
        }
        let method = u16_at(pos + 10).ok_or("broken zip")?;
        let csize = u32_at(pos + 20).ok_or("broken zip")?;
        let usize_ = u32_at(pos + 24).ok_or("broken zip")?;
        let (nlen, elen, clen) =
            (u16_at(pos + 28).ok_or("broken zip")?, u16_at(pos + 30).ok_or("broken zip")?, u16_at(pos + 32).ok_or("broken zip")?);
        let local = u32_at(pos + 42).ok_or("broken zip")?;
        let name = String::from_utf8_lossy(data.get(pos + 46..pos + 46 + nlen).ok_or("broken zip")?).replace('\\', "/");
        pos += 46 + nlen + elen + clen;
        if name.ends_with('/') {
            continue;
        }
        if name.starts_with('/') || name.split('/').any(|p| p == "..") {
            return Err(format!("unsafe path {name}"));
        }
        total += usize_ as u64;
        if total > MAX_PACK_BYTES {
            return Err("too big when unpacked".into());
        }
        if !data.get(local..).is_some_and(|d| d.starts_with(b"PK\x03\x04")) {
            return Err("broken zip local header".into());
        }
        let start = local + 30 + u16_at(local + 26).ok_or("broken zip")? + u16_at(local + 28).ok_or("broken zip")?;
        let raw = data.get(start..start + csize).ok_or("broken zip data")?;
        let contents = match method {
            0 => raw.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(usize_);
                flate2::read::DeflateDecoder::new(raw)
                    .take(usize_ as u64 + 1)
                    .read_to_end(&mut v)
                    .map_err(|e| format!("{name}: {e}"))?;
                v
            }
            m => return Err(format!("{name}: unsupported compression {m}")),
        };
        out.push((name, contents));
    }
    Ok(out)
}

/// Packs zipped with their folder inside (`Pack/manifest.json`) are rebuilt with the files at the
/// root, where the client looks for them.
fn normalize_zip(data: Vec<u8>) -> Result<Vec<u8>, String> {
    let files = read_zip(&data)?;
    if files.iter().any(|(n, _)| n == "manifest.json") {
        return Ok(data);
    }
    let prefix = files
        .iter()
        .find_map(|(n, _)| n.strip_suffix("manifest.json").filter(|p| p.matches('/').count() == 1).map(str::to_string))
        .ok_or("no manifest.json in the pack")?;
    let inner: Vec<(String, Vec<u8>)> =
        files.into_iter().filter_map(|(n, d)| n.strip_prefix(prefix.as_str()).map(|s| (s.to_string(), d))).collect();
    Ok(write_zip(&inner))
}

/// A folder pack, zipped in memory.
fn zip_folder(root: &Path) -> Result<Vec<u8>, String> {
    if !root.join("manifest.json").is_file() {
        return Err("folder without manifest.json".into());
    }
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut total = 0u64;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
            let bytes = std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))?;
            total += bytes.len() as u64;
            if total > MAX_PACK_BYTES || files.len() >= MAX_FILES {
                return Err("too big".into());
            }
            files.push((rel, bytes));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(write_zip(&files))
}

/// A zip archive with the files deflated.
fn write_zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    use std::io::Write;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, contents) in files {
        let mut crc = flate2::Crc::new();
        crc.update(contents);
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        let _ = enc.write_all(contents);
        let packed = enc.finish().unwrap_or_default();
        let offset = out.len() as u32;
        let head = |sig: &[u8], central: bool| {
            let mut h = Vec::new();
            h.extend_from_slice(sig);
            if central {
                h.extend_from_slice(&20u16.to_le_bytes()); // made by
            }
            h.extend_from_slice(&20u16.to_le_bytes()); // version needed
            h.extend_from_slice(&0x0800u16.to_le_bytes()); // flags: utf-8 names
            h.extend_from_slice(&8u16.to_le_bytes()); // deflate
            h.extend_from_slice(&0u32.to_le_bytes()); // time/date
            h.extend_from_slice(&crc.sum().to_le_bytes());
            h.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            h.extend_from_slice(&(contents.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes()); // extra
            if central {
                h.extend_from_slice(&0u16.to_le_bytes()); // comment
                h.extend_from_slice(&0u16.to_le_bytes()); // disk
                h.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
                h.extend_from_slice(&0u32.to_le_bytes()); // external attributes
                h.extend_from_slice(&offset.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend(head(b"PK\x03\x04", false));
        out.extend_from_slice(&packed);
        central.extend(head(b"PK\x01\x02", true));
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&[0; 4]); // disk numbers
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_round_trip() {
        let files = vec![
            ("manifest.json".to_string(), br#"{"header":{"uuid":"11111111-2222-3333-4444-555555555555","version":[1,2,3],"name":"T"},"modules":[{"type":"resources"}]}"#.to_vec()),
            ("textures/a.png".to_string(), vec![7u8; 5000]),
        ];
        let zip = write_zip(&files);
        assert_eq!(read_zip(&zip).unwrap(), files);
        let pack = Pack::from_zip(zip).unwrap();
        assert_eq!(pack.version, "1.2.3");
        assert_eq!(pack.chunk_count(), 1);
    }

    #[test]
    fn nested_folder_is_moved_to_the_root() {
        let files = vec![
            ("P/manifest.json".to_string(), br#"{"header":{"uuid":"11111111-2222-3333-4444-555555555555","version":[1,0,0]},"modules":[{"type":"resources"}]}"#.to_vec()),
            ("P/x.json".to_string(), b"{}".to_vec()),
        ];
        let fixed = normalize_zip(write_zip(&files)).unwrap();
        let names: Vec<String> = read_zip(&fixed).unwrap().into_iter().map(|f| f.0).collect();
        assert_eq!(names, vec!["manifest.json", "x.json"]);
    }
}
