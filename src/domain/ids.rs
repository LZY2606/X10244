use serde_json::Value;
use sha2::{Digest, Sha256};

/// Canonical JSON: keys sorted (serde_json::Map is BTreeMap without
/// preserve_order), compact separators, deterministic float rendering.
pub fn canonical(v: &Value) -> String {
    serde_json::to_string(v).expect("json canonicalization")
}

/// Stable, content-addressed identity: `<kind>_<16 hex of sha256>`.
/// Equal content, regardless of export/delete/reimport, yields equal ids.
pub fn hid(kind: &str, payload: &Value) -> String {
    let mut h = Sha256::new();
    h.update(kind.as_bytes());
    h.update(b"|");
    h.update(canonical(payload).as_bytes());
    let d = h.finalize();
    format!("{}_{}", kind, hex::encode(d).split_at(16).0)
}
