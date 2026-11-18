//! NBT in the three flavours Bedrock uses: big-endian (data files), little-endian (world storage)
//! and network (little-endian with zigzag varints).

use crate::binary::{R, Res, W};

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(String),
    List(u8, Vec<Tag>),
    /// Kept as a Vec so re-encoding preserves the original key order.
    Compound(Vec<(String, Tag)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    BigEndian,
    LittleEndian,
    Network,
}

impl Tag {
    pub fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => 1,
            Tag::Short(_) => 2,
            Tag::Int(_) => 3,
            Tag::Long(_) => 4,
            Tag::Float(_) => 5,
            Tag::Double(_) => 6,
            Tag::ByteArray(_) => 7,
            Tag::String(_) => 8,
            Tag::List(..) => 9,
            Tag::Compound(_) => 10,
            Tag::IntArray(_) => 11,
            Tag::LongArray(_) => 12,
        }
    }
    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(v) => v.iter().find(|(k, _)| k == key).map(|(_, t)| t),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        Some(match self {
            Tag::Byte(v) => *v as i64,
            Tag::Short(v) => *v as i64,
            Tag::Int(v) => *v as i64,
            Tag::Long(v) => *v,
            _ => return None,
        })
    }
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Tag::Float(v) => Some(*v),
            Tag::Double(v) => Some(*v as f32),
            _ => self.as_i64().map(|v| v as f32),
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Option<&[Tag]> {
        match self {
            Tag::List(_, v) => Some(v),
            _ => None,
        }
    }
    pub fn as_compound(&self) -> Option<&[(String, Tag)]> {
        match self {
            Tag::Compound(v) => Some(v),
            _ => None,
        }
    }
    pub fn empty_compound() -> Tag {
        Tag::Compound(Vec::new())
    }
}

// ---------------------------------------------------------------- reading

fn read_len(r: &mut R, f: Flavor) -> Res<i32> {
    match f {
        Flavor::BigEndian => r.i32_be(),
        Flavor::LittleEndian => r.i32_le(),
        Flavor::Network => r.varint(),
    }
}

fn read_str(r: &mut R, f: Flavor) -> Res<String> {
    let n = match f {
        Flavor::BigEndian => r.u16_be()? as usize,
        Flavor::LittleEndian => r.u16_le()? as usize,
        Flavor::Network => r.uvarint()? as usize,
    };
    Ok(String::from_utf8_lossy(r.take(n)?).into_owned())
}

fn read_payload(r: &mut R, id: u8, f: Flavor, depth: u32) -> Res<Tag> {
    if depth > 512 {
        return Err("NBT nested too deep".into());
    }
    let be = f == Flavor::BigEndian;
    Ok(match id {
        1 => Tag::Byte(r.u8()? as i8),
        2 => Tag::Short(if be { r.i16_be()? } else { r.i16_le()? }),
        3 => Tag::Int(match f {
            Flavor::BigEndian => r.i32_be()?,
            Flavor::LittleEndian => r.i32_le()?,
            Flavor::Network => r.varint()?,
        }),
        4 => Tag::Long(match f {
            Flavor::BigEndian => r.i64_be()?,
            Flavor::LittleEndian => r.i64_le()?,
            Flavor::Network => r.varlong()?,
        }),
        5 => Tag::Float(if be { r.f32_be()? } else { r.f32_le()? }),
        6 => Tag::Double(if be { r.f64_be()? } else { r.f64_le()? }),
        7 => {
            let n = read_len(r, f)?.max(0) as usize;
            Tag::ByteArray(r.take(n)?.to_vec())
        }
        8 => Tag::String(read_str(r, f)?),
        9 => {
            let elem = r.u8()?;
            let n = read_len(r, f)?.max(0) as usize;
            let mut v = Vec::with_capacity(n.min(4096));
            for _ in 0..n {
                v.push(read_payload(r, elem, f, depth + 1)?);
            }
            Tag::List(elem, v)
        }
        10 => {
            let mut v = Vec::new();
            loop {
                let t = r.u8()?;
                if t == 0 {
                    break;
                }
                let name = read_str(r, f)?;
                v.push((name, read_payload(r, t, f, depth + 1)?));
            }
            Tag::Compound(v)
        }
        11 => {
            let n = read_len(r, f)?.max(0) as usize;
            let mut v = Vec::with_capacity(n.min(4096));
            for _ in 0..n {
                v.push(match f {
                    Flavor::BigEndian => r.i32_be()?,
                    Flavor::LittleEndian => r.i32_le()?,
                    Flavor::Network => r.varint()?,
                });
            }
            Tag::IntArray(v)
        }
        12 => {
            let n = read_len(r, f)?.max(0) as usize;
            let mut v = Vec::with_capacity(n.min(4096));
            for _ in 0..n {
                v.push(match f {
                    Flavor::BigEndian => r.i64_be()?,
                    Flavor::LittleEndian => r.i64_le()?,
                    Flavor::Network => r.varlong()?,
                });
            }
            Tag::LongArray(v)
        }
        _ => return Err(format!("unknown NBT tag id {id}")),
    })
}

/// Reads one root tag (type + name + payload). Returns (name, tag).
pub fn read_root(r: &mut R, f: Flavor) -> Res<(String, Tag)> {
    let id = r.u8()?;
    if id == 0 {
        return Ok((String::new(), Tag::empty_compound()));
    }
    let name = read_str(r, f)?;
    Ok((name, read_payload(r, id, f, 0)?))
}

// ---------------------------------------------------------------- writing

fn write_len(w: &mut W, f: Flavor, n: i32) {
    match f {
        Flavor::BigEndian => w.bytes(&n.to_be_bytes()),
        Flavor::LittleEndian => w.i32_le(n),
        Flavor::Network => w.varint(n),
    }
}

fn write_str(w: &mut W, f: Flavor, s: &str) {
    match f {
        Flavor::BigEndian => w.u16_be(s.len() as u16),
        Flavor::LittleEndian => w.u16_le(s.len() as u16),
        Flavor::Network => w.uvarint(s.len() as u32),
    }
    w.bytes(s.as_bytes());
}

fn write_payload(w: &mut W, t: &Tag, f: Flavor) {
    let be = f == Flavor::BigEndian;
    match t {
        Tag::Byte(v) => w.u8(*v as u8),
        Tag::Short(v) => w.bytes(&if be { v.to_be_bytes() } else { v.to_le_bytes() }),
        Tag::Int(v) => match f {
            Flavor::Network => w.varint(*v),
            _ => w.bytes(&if be { v.to_be_bytes() } else { v.to_le_bytes() }),
        },
        Tag::Long(v) => match f {
            Flavor::Network => w.varlong(*v),
            _ => w.bytes(&if be { v.to_be_bytes() } else { v.to_le_bytes() }),
        },
        Tag::Float(v) => w.bytes(&if be { v.to_be_bytes() } else { v.to_le_bytes() }),
        Tag::Double(v) => w.bytes(&if be { v.to_be_bytes() } else { v.to_le_bytes() }),
        Tag::ByteArray(v) => {
            write_len(w, f, v.len() as i32);
            w.bytes(v);
        }
        Tag::String(s) => write_str(w, f, s),
        Tag::List(elem, v) => {
            let elem = if v.is_empty() { *elem } else { v[0].id() };
            w.u8(elem);
            write_len(w, f, v.len() as i32);
            for t in v {
                write_payload(w, t, f);
            }
        }
        Tag::Compound(v) => {
            for (k, t) in v {
                w.u8(t.id());
                write_str(w, f, k);
                write_payload(w, t, f);
            }
            w.u8(0);
        }
        Tag::IntArray(v) => {
            write_len(w, f, v.len() as i32);
            for x in v {
                match f {
                    Flavor::Network => w.varint(*x),
                    _ => w.bytes(&if be { x.to_be_bytes() } else { x.to_le_bytes() }),
                }
            }
        }
        Tag::LongArray(v) => {
            write_len(w, f, v.len() as i32);
            for x in v {
                match f {
                    Flavor::Network => w.varlong(*x),
                    _ => w.bytes(&if be { x.to_be_bytes() } else { x.to_le_bytes() }),
                }
            }
        }
    }
}

pub fn write_root(w: &mut W, name: &str, t: &Tag, f: Flavor) {
    w.u8(t.id());
    write_str(w, f, name);
    write_payload(w, t, f);
}

/// Network-encoded root compound with an empty name (what PHP calls CacheableNbt::getEncodedNbt()).
pub fn to_network(t: &Tag) -> Vec<u8> {
    let mut w = W::new();
    write_root(&mut w, "", t, Flavor::Network);
    w.0
}

/// Stable key for a block state ("name" + sorted "states"), used to map world palettes to network IDs.
pub fn state_key(name: &str, states: Option<&Tag>) -> String {
    let mut key = String::with_capacity(64);
    key.push_str(name);
    if let Some(Tag::Compound(v)) = states {
        let mut entries: Vec<&(String, Tag)> = v.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (k, t) in entries {
            key.push('|');
            key.push_str(k);
            key.push('=');
            match t {
                Tag::String(s) => key.push_str(s),
                other => key.push_str(&other.as_i64().unwrap_or(0).to_string()),
            }
        }
    }
    key
}
