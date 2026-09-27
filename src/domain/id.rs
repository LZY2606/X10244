//! Stable, content-derived identifiers.
//!
//! Every displayed value, record version, derivation and event carries a
//! stable id that survives export -> delete -> import. Ids are derived from
//! a canonical JSON payload with SHA-256, so identical inputs on different
//! machines produce identical ids.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ID_PREFIX: &str = "saxs1";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Sid(pub String);

impl Sid {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Sid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Deterministic id of a JSON-serializable payload: `saxs1:<16 hex>`.
pub fn hid<T: Serialize>(payload: &T) -> Sid {
    hid_with("", payload)
}

/// Content id with a discriminator namespace to avoid cross-entity collisions.
pub fn hid_with<T: Serialize>(discriminator: &str, payload: &T) -> Sid {
    let bytes = canonical_json(payload);
    let mut h = Sha256::new();
    h.update(ID_PREFIX.as_bytes());
    h.update([0]);
    h.update(discriminator.as_bytes());
    h.update([0]);
    h.update(&bytes);
    let d = h.finalize();
    let mut s = String::with_capacity(ID_PREFIX.len() + 1 + 16);
    s.push_str(ID_PREFIX);
    s.push(':');
    for b in d.iter().take(8) {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    Sid(s)
}

/// Canonical byte serialization. We rely on serde_json which preserves struct
/// field declaration order; f64 values are rounded through a string to be
/// platform independent.
pub fn canonical_json<T: Serialize>(value: &T) -> Vec<u8> {
    let mut v = serde_json::to_value(value).expect("canonical serialization");
    round_numbers(&mut v);
    serde_json::to_vec(&v).expect("canonical serialization")
}

fn round_numbers(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if f.is_finite() {
                    let r = (f * 1e12).round() / 1e12;
                    *n = serde_json::Number::from_f64(r).unwrap_or_else(|| n.clone());
                }
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(round_numbers),
        serde_json::Value::Object(o) => o.values_mut().for_each(round_numbers),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_namespaced() {
        let a = hid(&vec![1u64, 2, 3]);
        let b = hid(&vec![1u64, 2, 3]);
        let c = hid_with("x", &vec![1u64, 2, 3]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.as_str().starts_with("saxs1:"));
    }
}
