//! Stable, export-safe identifiers.
//!
//! IDs are derived deterministically from material identity (stable keys
//! supplied by the case pack) so that export -> delete -> import preserves
//! every identity, ordering and provenance relation.

use serde::{Deserialize, Serialize};
use std::fmt;

pub(crate) fn clean_key(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('-');
        }
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}

/// Stable identifier of a material node (raw / normalized / subtracted / model).
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub String);

/// Stable identifier of a node version.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VersionId(pub String);

/// Stable identifier of an append-only ledger event.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(pub String);

/// Stable identifier of a review branch.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BranchId(pub String);

/// Stable identifier of a single data point inside a curve.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PointId(pub String);

macro_rules! id_impls {
    ($t:ty, $prefix:literal) => {
        impl $t {
            pub fn as_str(&self) -> &str {
                &self.0
            }
            pub fn is_system(&self) -> bool {
                self.0.starts_with(concat!($prefix, "-init"))
            }
        }
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl std::str::FromStr for $t {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                if s.is_empty() {
                    Err("empty id".into())
                } else {
                    Ok(Self(s.to_string()))
                }
            }
        }
        impl From<&str> for $t {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }
        impl From<String> for $t {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
    };
}

id_impls!(NodeId, "n");
id_impls!(VersionId, "v");
id_impls!(EventId, "e");
id_impls!(BranchId, "b");
id_impls!(PointId, "p");

/// Deterministic node id, stable across export/import.
pub fn node_id(kind: &str, stable_key: &str) -> NodeId {
    NodeId(format!("n-{}-{}", kind, clean_key(stable_key)))
}

/// Deterministic version id: same inputs -> same id, so re-derivation can be
/// recognized as "still valid" rather than a new version.
pub fn version_id(node: &NodeId, seq: u32, fingerprint_hex: &str) -> VersionId {
    VersionId(format!("{}:v{:03}-{}", node.0, seq, &fingerprint_hex[..fingerprint_hex.len().min(10)]))
}

pub fn point_id(node: &NodeId, slot: usize, stable: &str) -> PointId {
    PointId(format!("{}#p{:04}-{}", node.0, slot, clean_key(stable)))
}

/// FNV-1a 64-bit fingerprint over a canonical byte feed.
#[derive(Default)]
pub struct Fingerprint {
    state: u64,
}

impl Fingerprint {
    pub fn new() -> Self {
        Fingerprint { state: 0xcbf29ce484222325 }
    }
    pub fn feed(&mut self, bytes: &[u8]) -> &mut Self {
        for &b in bytes {
            self.state ^= b as u64;
            self.state = self.state.wrapping_mul(0x100000001b3);
        }
        self
    }
    pub fn feed_str(&mut self, s: &str) -> &mut Self {
        self.feed(s.as_bytes());
        self.feed(&[0])
    }
    pub fn feed_json(&mut self, v: &serde_json::Value) -> &mut Self {
        let canonical = canonical_json(v);
        self.feed_str(&canonical)
    }
    pub fn hex(&self) -> String {
        format!("{:016x}", self.state)
    }
}

/// Canonical JSON encoding (sorted object keys, no whitespace) for fingerprints
/// and for stable export ordering.
pub fn canonical_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", serde_json::to_string(k).unwrap(), canonical_json(&map[*k])))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        serde_json::Value::Array(a) => {
            format!("[{}]", a.iter().map(canonical_json).collect::<Vec<_>>().join(","))
        }
        other => serde_json::to_string(other).unwrap(),
    }
}
