// Deterministic, content-based identity. IDs never depend on wall-clock time,
// insertion order, or randomness, so export -> delete -> import preserves
// identity, ordering and source relations exactly.

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn feed(h: u64, bytes: &[u8]) -> u64 {
    let mut h = h;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h.wrapping_mul(FNV_PRIME) // separator beat, so empty parts are significant
}

/// Stable identifier from a domain prefix and ordered content parts.
pub fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let mut h = feed(FNV_OFFSET, prefix.as_bytes());
    for p in parts {
        h = feed(h, p.as_bytes());
    }
    format!("{}_{:016x}", prefix, h)
}

/// Canonical rendering of a coordinate/value inside a hash.
pub fn canon_f64(x: f64) -> String {
    if x.is_nan() {
        "nan".to_string()
    } else {
        format!("{:.9e}", x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_stable_distinct_and_order_sensitive() {
        let a = stable_id("n", &["sample", "1"]);
        let b = stable_id("n", &["sample", "1"]);
        let c = stable_id("n", &["sample", "2"]);
        let d = stable_id("n", &["1", "sample"]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert!(a.starts_with("n_"));
    }
}
