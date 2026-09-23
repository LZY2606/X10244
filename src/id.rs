//! Deterministic stable identities (FNV-1a 64 bit, hex12).

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn id_hex(prefix: &str, material: &str) -> String {
    let h = fnv1a64(material.as_bytes());
    format!("{}{:012x?}", prefix, h % 0x01000000000000)
}

pub fn artifact_id(kind: &str, material: &str) -> String {
    id_hex(&format!("a_{}_", kind), material)
}

pub fn point_id(artifact_id: &str, index: usize, q: f64) -> String {
    id_hex("p_", &format!("{}|{}|{}", artifact_id, index, fmt_f(q)))
}

pub fn event_id(case_id: &str, branch: &str, seq: i64, command: &str) -> String {
    id_hex(
        "e_",
        &format!("{}|{}|{}|{}", case_id, branch, seq, command),
    )
}

/// Deterministic float rendering so identities are stable across replay/export.
pub fn fmt_f(x: f64) -> String {
    if x == 0.0 {
        "0".to_string()
    } else if x.is_finite() {
        let s = format!("{:.10e}", x);
        s
    } else {
        "<nan>".to_string()
    }
}
