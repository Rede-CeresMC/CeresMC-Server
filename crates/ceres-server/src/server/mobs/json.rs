//! Reading behaviour pack JSON: comments allowed, and the loose shapes Bedrock accepts for numbers,
//! ranges and item names.

use serde_json::Value;

/// Parses JSON that may hold `//` and `/* */` comments (outside strings).
pub(crate) fn parse_lenient(text: &str) -> Option<Value> {
    let text = text.trim_start_matches('\u{feff}');
    let mut out = String::with_capacity(text.len());
    let b = text.as_bytes();
    let (mut i, mut in_str) = (0, false);
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(c as char);
            if c == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_str = true;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    // the byte loop above copied multi-byte characters one byte at a time: rebuild them
    let bytes: Vec<u8> = out.chars().map(|c| c as u32 as u8).collect();
    let fixed = String::from_utf8(bytes).ok()?;
    serde_json::from_str(&fixed).ok()
}

/// A number field (numbers given as strings or booleans count too).
pub(crate) fn num(v: &Value, key: &str, default: f32) -> f32 {
    as_num(v.get(key)).unwrap_or(default)
}

pub(crate) fn as_num(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Number(n) => n.as_f64().map(|f| f as f32),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::String(s) => s.trim().parse().ok(),
        Value::Array(a) => as_num(a.first()),
        Value::Object(o) => as_num(o.get("value").or(o.get("min")).or(o.get("range_min"))),
        _ => None,
    }
}

pub(crate) fn boolean(v: &Value, key: &str, default: bool) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(s)) => !matches!(s.as_str(), "false" | "no" | "0" | ""),
        _ => default,
    }
}

pub(crate) fn string<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|s| s.as_str())
}

/// `[min, max]`, `{min, max}`, `{range_min, range_max}` or a single number.
pub(crate) fn range(v: Option<&Value>, default: (f32, f32)) -> (f32, f32) {
    match v {
        Some(Value::Array(a)) if a.len() >= 2 => (as_num(a.first()).unwrap_or(default.0), as_num(a.get(1)).unwrap_or(default.1)),
        Some(Value::Object(o)) => {
            let lo = as_num(o.get("min").or(o.get("range_min"))).unwrap_or(default.0);
            let hi = as_num(o.get("max").or(o.get("range_max"))).unwrap_or(lo.max(default.1));
            (lo, hi)
        }
        Some(other) => match as_num(Some(other)) {
            Some(n) => (n, n),
            None => default,
        },
        None => default,
    }
}

pub(crate) fn rand_f(lo: f32, hi: f32) -> f32 {
    if hi <= lo { lo } else { lo + rand::random::<f32>() * (hi - lo) }
}

pub(crate) fn rand_i(lo: i32, hi: i32) -> i32 {
    if hi <= lo { lo } else { lo + (rand::random::<u32>() % (hi - lo + 1) as u32) as i32 }
}

pub(crate) fn chance(p: f32) -> bool {
    p >= 1.0 || (p > 0.0 && rand::random::<f32>() < p)
}

/// "wheat" -> ("minecraft:wheat", None); "bucket:1" -> ("minecraft:bucket", Some(1)).
pub(crate) fn item_ref(s: &str) -> (String, Option<i32>) {
    let s = s.trim();
    let (body, data) = match s.rsplit_once(':') {
        Some((a, b)) if b.chars().all(|c| c.is_ascii_digit()) && !b.is_empty() => (a, b.parse().ok()),
        _ => (s, None),
    };
    let name = if body.contains(':') { body.to_string() } else { format!("minecraft:{body}") };
    (name, data)
}

/// A list of strings from a string, an array of strings, or an array of `{item: ...}` objects.
pub(crate) fn str_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|e| match e {
                Value::String(s) => Some(s.clone()),
                Value::Object(o) => o.get("item").or(o.get("name")).and_then(|s| s.as_str()).map(String::from),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Item names (normalised) from a string or list.
pub(crate) fn item_list(v: Option<&Value>) -> Vec<String> {
    str_list(v).iter().map(|s| item_ref(s).0).collect()
}

/// `{event, target}` or a bare event name.
pub(crate) fn event_ref(v: Option<&Value>) -> Option<(String, String)> {
    match v? {
        Value::String(s) => Some((s.clone(), "self".into())),
        Value::Object(o) => {
            let e = o.get("event")?.as_str()?.to_string();
            let t = o.get("target").and_then(|t| t.as_str()).unwrap_or("self").to_string();
            Some((e, t))
        }
        _ => None,
    }
}

/// Short form of an identifier ("minecraft:zombie" -> "zombie").
pub(crate) fn short(id: &str) -> &str {
    id.strip_prefix("minecraft:").unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_ranges() {
        let v = parse_lenient("{\n // hi\n \"a\": [1, 3], /* x */ \"b\": \"http://x\", \"c\": \"é\"}").unwrap();
        assert_eq!(range(v.get("a"), (0.0, 0.0)), (1.0, 3.0));
        assert_eq!(string(&v, "b"), Some("http://x"));
        assert_eq!(string(&v, "c"), Some("é"));
        assert_eq!(item_ref("bucket:1"), ("minecraft:bucket".into(), Some(1)));
        assert_eq!(item_ref("minecraft:wheat"), ("minecraft:wheat".into(), None));
    }
}
