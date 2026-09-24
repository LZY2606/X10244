//! Stable, content-addressed identifiers.
//!
//! Identities must survive `export -> delete local DB -> import`, so they are
//! derived deterministically from content rather than from autoincrement row
//! ids or random UUIDs. Two objects with identical content and identical
//! lineage always get the same id.

use serde::Serialize;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn fnv1a(seed: u64, bytes: &[u8]) -> u64 {
    let mut hash = seed;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// 128 bit content hash rendered as 32 lowercase hex characters.
///
/// Two independent 64 bit FNV-1a hashes with different offset seeds are used
/// so that distinct structures do not collapse onto a single 64 bit value.
pub fn content_id(prefix: &str, value: &impl Serialize) -> String {
    let json = serde_json::to_string(value).expect("content id serialization");
    let bytes = json.as_bytes();
    let lo = fnv1a(FNV_OFFSET, bytes);
    let hi = fnv1a(FNV_OFFSET ^ 0x9e3779b97f4a7c15, bytes);
    format!("{}:{:016x}{:016x}", prefix, hi, lo)
}

/// Convenience helper used by tests and ad-hoc hashing.
pub fn content_id_from_bytes(prefix: &str, bytes: &[u8]) -> String {
    let lo = fnv1a(FNV_OFFSET, bytes);
    let hi = fnv1a(FNV_OFFSET ^ 0x9e3779b97f4a7c15, bytes);
    format!("{}:{:016x}{:016x}", prefix, hi, lo)
}

/// Short human-friendly correlation id embedded in event records. It does not
/// carry identity semantics; identity always lives on the content id.
pub fn short_tag(input: &str) -> String {
    let lo = fnv1a(FNV_OFFSET, input.as_bytes());
    format!("{:08x}", lo >> 32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize)]
    struct Thing {
        a: i64,
        b: String,
    }

    #[test]
    fn ids_are_deterministic() {
        let t = Thing { a: 3, b: "x".into() };
        assert_eq!(content_id("o", &t), content_id("o", &t));
    }

    #[test]
    fn ids_change_with_content_or_prefix() {
        let t1 = Thing { a: 3, b: "x".into() };
        let t2 = Thing { a: 4, b: "x".into() };
        assert_ne!(content_id("o", &t1), content_id("o", &t2));
        assert_ne!(content_id("a", &t1), content_id("b", &t1));
        assert!(content_id("o", &t1).starts_with("o:"));
    }
}
