//! Deterministic, content-derived stable identifiers.
//!
//! IDs must stay stable after export, deleting the local database and
//! re-importing. Therefore they are derived from the content they identify,
//! not from an autoincrement row id or a random uuid. We use FNV-1a so the
//! derivation is pure, dependency free and identical on every platform.

/// 64 bit FNV-1a hash.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A short stable token for an arbitrary string payload.
pub fn token(prefix: &str, payload: &str) -> String {
    format!("{}_{:016x}", prefix, fnv1a64(payload.as_bytes()))
}

/// Stable id for a single imported point.
pub fn point_id(material_id: &str, index: usize, q_raw: &str, i_raw: &str, e_raw: &str) -> String {
    token("pt", &format!("{}|{}|{}|{}|{}", material_id, index, q_raw, i_raw, e_raw))
}

/// Stable id for a normalized node.
pub fn norm_node(material: &str, version: u32) -> String {
    token("norm", &format!("{}#v{}", material, version))
}

/// Stable id for a normalized point.
pub fn norm_point(node: &str, source_point: &str) -> String {
    token("npt", &format!("{}|{}", node, source_point))
}

/// Stable id for a subtraction node (sample version + buffer version).
pub fn sub_node(sample: &str, sv: u32, buffer: &str, bv: u32) -> String {
    token("sub", &format!("{}#v{}|{}#v{}", sample, sv, buffer, bv))
}

/// Stable id for a subtraction candidate.
pub fn candidate_id(node: &str, method: &str, scale: f64) -> String {
    token("cand", &format!("{}|{}|{:.12e}", node, method, scale))
}

/// Stable id for a subtracted point.
pub fn sub_point_id(node: &str, sample_pt: &str, buffer_pt: &str) -> String {
    token("spt", &format!("{}|{}|{}", node, sample_pt, buffer_pt))
}

/// Stable id for a model-evidence node.
pub fn model_node(sub_node: &str) -> String {
    token("model", sub_node)
}

/// Stable id for an interval candidate.
pub fn interval_id(node: &str, model: &str, lo: f64, hi: f64) -> String {
    token("iv", &format!("{}|{}|{:.10e}|{:.10e}", node, model, lo, hi))
}
