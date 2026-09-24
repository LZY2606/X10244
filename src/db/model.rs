use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scalar {
    pub value: f64,
    pub raw: String,
    pub unit: String,
    pub canonical: Option<f64>,
    pub canonical_unit: Option<String>,
    pub convertible: bool,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointIn {
    pub ordinal: u32,
    pub q: Scalar,
    pub i: Scalar,
    pub sig: Scalar,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordIn {
    pub role: String,
    pub name: String,
    pub points: Vec<PointIn>,
    pub exposure: Scalar,
    pub transmission: Scalar,
    #[serde(default)]
    pub thickness: Option<Scalar>,
    #[serde(default)]
    pub transmission_sigma: f64,
    #[serde(default)]
    pub thickness_sigma: f64,
    pub source_file: String,
    pub instrument: InstrumentIn,
    pub mask: MaskIn,
    #[serde(default)]
    pub revision_of: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentIn {
    pub name: String,
    /// Coordinate basis + response metadata, retained verbatim.
    pub spec: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskIn {
    pub name: String,
    pub origin: String,
    pub locator: String,
    pub checksum: String,
}

/// Deterministic identity so re-imported archives keep every stable id,
/// ordering and source relation.
pub fn deterministic_id(ns: &str, parts: &[&str]) -> String {
    use uuid::Uuid;
    let space = Uuid::NAMESPACE_URL;
    let key = std::iter::once(ns).chain(parts.iter().copied()).collect::<Vec<_>>().join("|");
    Uuid::new_v5(&space, key.as_bytes()).to_string()
}

pub fn checksum(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in parts {
        for b in p.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    format!("{h:016x}")
}
