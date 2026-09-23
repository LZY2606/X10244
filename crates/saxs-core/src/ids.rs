//! Stable identities. Every displayed value (project, version, point, event,
//! candidate) carries an id that survives export/delete/import.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub fn digest(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h.update(b"|");
        }
        h.update(p.as_bytes());
    }
    hex::encode(h.finalize())[..16].to_string()
}

pub fn event_id(project: &str, seq: u64) -> String {
    digest(&["ev", project, &seq.to_string()])
}

/// Point ids are a digest of the provenance tuple, so the same derivation
/// reproduces the same id (export/import stability) while differing sources
/// never collide.
pub fn point_id(kind: &str, parents: &[&str], q: f64, index: usize) -> String {
    let mut parts: Vec<&str> = vec![kind];
    parts.extend(parents.iter());
    let qt = format!("{:.10}", q);
    let it = index.to_string();
    parts.push(&qt);
    parts.push(&it);
    digest(&parts.iter().copied().collect::<Vec<_>>())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OpaqueId(pub String);

impl OpaqueId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for OpaqueId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
