//! Deterministic FNV-1a (64 bit) content hashing over canonical JSON.
//!
//! Identities in this system are content addressed: identical inputs + rule
//! versions always produce the same uid, even after export/delete/import.

use serde_json::Value;

pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Recursively canonical JSON: object keys sorted, compact separators.
pub fn canonical(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::String(s) => out.extend_from_slice(serde_json::to_vec(s).unwrap()),
        Value::Array(a) => {
            out.push(b'[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(item, out);
            }
            out.push(b']');
        }
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(serde_json::to_string(k).unwrap().as_bytes());
                out.push(b':');
                write_canonical(&o[*k], out);
            }
            out.push(b'}');
        }
    }
}

pub fn digest16(v: &Value) -> String {
    format!("{:016x}", fnv1a(&canonical(v)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_independent() {
        assert_eq!(digest16(&json!({"a": 1, "b": 2})), digest16(&json!({"b": 2, "a": 1})));
    }

    #[test]
    fn value_sensitive() {
        assert_ne!(digest16(&json!({"a": 1})), digest16(&json!({"a": 2})));
    }
}
