//! Content-addressed stable identities. IDs are sha256 over canonical JSON,
//! so exporting, deleting the local database and re-importing yields identical
//! ids, ordering and provenance links.

use serde::Serialize;
use sha2::{Digest, Sha256};

pub fn content_id<T: Serialize>(tag: &str, body: &T) -> String {
    let json = serde_json::to_string(body).expect("domain records serialize");
    let mut h = Sha256::new();
    h.update(tag.as_bytes());
    h.update(b"\0");
    h.update(json.as_bytes());
    format!("{:x}", h.finalize())
}
