use sha2::{Digest, Sha256};

pub fn digest(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h.update(b"\u{1f}");
        }
        h.update(p.as_bytes());
    }
    let out = hex::encode(h.finalize());
    format!("id_{}", &out[..24])
}

pub fn point_id(node_id: &str, index: usize, kind: &str) -> String {
    digest(&["point", kind, node_id, &index.to_string()])
}
