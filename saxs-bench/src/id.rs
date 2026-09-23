//! Stable, content-derived identity.
//!
//! Identities are deterministic hashes (FNV-1a 64-bit) over a canonical,
//! self-describing string. Re-importing the same offline case pack therefore
//! reproduces the *same* ids, so export/delete/re-import preserves identity,
//! ordering and provenance. A short input tag is prefixed so ids from distinct
//! namespaces never collide.

/// FNV-1a 64-bit over the bytes of `s`.
pub fn fnv1a64(s: &str) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Build a stable id of the form `prefix_<12 hex>` from canonical parts.
///
/// Parts are length-delimited so different groupings cannot collide.
pub fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let mut canon = String::new();
    for p in parts {
        canon.push_str(&p.chars().count().to_string());
        canon.push(':');
        canon.push_str(p);
        canon.push('|');
    }
    format!("{prefix}_{:016x}", fnv1a64(&canon))
}

/// Hash helper for already-canonical strings.
pub fn hash_str(s: &str) -> String {
    format!("{:016x}", fnv1a64(s))
}
