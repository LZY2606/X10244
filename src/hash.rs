use serde::Serialize;
use sha2::{Digest, Sha256};

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Deterministic content hash. serde_json uses declaration-ordered maps
/// (BTreeMap unless `preserve_order` is enabled), so output is stable.
pub fn hash_canonical<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("serialize canonical content");
    sha256_hex(&bytes)
}

pub fn short(id: &str) -> String {
    id.chars().take(10).collect()
}
