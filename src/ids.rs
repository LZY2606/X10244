//! Deterministic, content-addressed identities.
//!
//! Every id is a FNV-1a 64-bit digest over tagged parts, so exporting the
//! archive, deleting the local database and re-importing it reproduces exactly
//! the same ids, ordering and provenance links.

pub fn fnv64(parts: &[&str]) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for part in parts {
        for b in part.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(PRIME);
        }
        h ^= 0x1f;
        h = h.wrapping_mul(PRIME);
    }
    format!("{h:016x}")
}

pub fn curve_id(payload_key: &str, n: usize, first_q: &str, last_q: &str, role: &str) -> String {
    format!(
        "cv_{}",
        fnv64(&["curve", payload_key, &n.to_string(), first_q, last_q, role])
    )
}

pub fn response_id(label: &str, source: &str, n: usize) -> String {
    format!("rp_{}", fnv64(&["response", label, source, &n.to_string()]))
}

pub fn raw_point_id(curve: &str, idx: usize, q_raw: &str, i_raw: &str) -> String {
    format!(
        "rp_{}",
        fnv64(&["rawpoint", curve, &idx.to_string(), q_raw, i_raw])
    )
}

pub fn deriv_id(kind: &str, signature: &str) -> String {
    format!("dr_{}", fnv64(&["derivation", kind, signature]))
}

pub fn deriv_point_id(deriv: &str, idx: usize, q: f64) -> String {
    format!(
        "dp_{}",
        fnv64(&["derpoint", deriv, &idx.to_string(), &canon(q)])
    )
}

pub fn event_id(seq: i64, canonical_body: &str) -> String {
    format!("ev_{}", fnv64(&["event", &seq.to_string(), canonical_body]))
}

/// Canonical shortest-decimal rendering of an f64 (stable across platforms).
pub fn canon(v: f64) -> String {
    if v.is_nan() {
        "NaN".to_string()
    } else if v.is_infinite() {
        if v > 0.0 { "Inf".into() } else { "-Inf".into() }
    } else if v == 0.0 {
        "0".to_string()
    } else {
        format!("{v:.12?}")
    }
}
