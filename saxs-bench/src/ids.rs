//! Stable, content-derived identities.
//!
//! Identities are intrinsic (derived from the content and upstream identities),
//! not database row ids. This is what keeps point/curve identity, ordering and
//! provenance stable after export -> delete local DB -> re-import.

/// FNV-1a 64 bit hash, rendered as 16 hex characters.
pub fn fnv1a(parts: &[&str]) -> String {
    const OFF: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFF;
    for part in parts {
        for b in part.as_bytes() {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(PRIME);
        }
        // mix a separator so two part boundaries cannot collide
        hash ^= 0x1f;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

pub fn curve_raw_id(logical: &str, version: &str, body: &str) -> String {
    fnv1a(&["raw", logical, version, body])
}
pub fn norm_id(raw_id: &str, response_id: &str, mask_id: &str, body: &str) -> String {
    fnv1a(&["norm", raw_id, response_id, mask_id, body])
}
pub fn sub_id(sample: &str, buffer: &str, body: &str) -> String {
    fnv1a(&["sub", sample, buffer, body])
}
pub fn model_id(sub: &str, body: &str) -> String {
    fnv1a(&["model", sub, body])
}
pub fn point_id(parent: &str, index: usize, q: &str) -> String {
    fnv1a(&["pt", parent, &index.to_string(), q])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_and_distinct() {
        assert_eq!(fnv1a(&["a", "b"]), fnv1a(&["a", "b"]));
        assert_ne!(fnv1a(&["a", "b"]), fnv1a(&["ab"]));
        assert_eq!(fnv1a(&["a", "b"]).len(), 16);
    }
}
