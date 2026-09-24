use serde::{Deserialize, Serialize};

/// Stable, content-derived identity. Never reused across different content.
pub fn stable_id(prefix: &str, payload: &str) -> String {
    // FNV-1a 128-ish (two interleaved 64-bit lanes) -> hex, deterministic.
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x9e3779b97f4a7c15;
    for (i, b) in payload.bytes().enumerate() {
        if i % 2 == 0 {
            h1 ^= b as u64;
            h1 = h1.wrapping_mul(0x100000001b3);
        } else {
            h2 ^= b as u64;
            h2 = h2.wrapping_mul(0x100000001b3);
        }
    }
    format!("{}:{:016x}{:016x}", prefix, h1, h2)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    pub q_unit: String, // "1/angstrom" | "1/nm"
    pub i_unit: String, // "1/cm" | "arbitrary"
    pub points: Vec<Point>,
}

impl Curve {
    pub fn q_min(&self) -> f64 {
        self.points.iter().map(|p| p.q).fold(f64::INFINITY, f64::min)
    }
    pub fn q_max(&self) -> f64 {
        self.points.iter().map(|p| p.q).fold(f64::NEG_INFINITY, f64::max)
    }
    pub fn same_grid(&self, other: &Curve) -> bool {
        self.points.len() == other.points.len()
            && self
                .points
                .iter()
                .zip(other.points.iter())
                .all(|(a, b)| (a.q - b.q).abs() < 1e-12)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Meta {
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub transmission: f64,
    pub transmission_sigma: f64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Sample,
    Buffer,
    InstrumentResponse,
    Mask,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String, // machine readable, e.g. "negative-intensity"
    pub message: String,
    pub subject: Option<String>, // stable id of affected entity
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRecord {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    pub version: u64,
    pub meta: Option<Meta>,
    pub curve: Option<Curve>,
    /// Original imported representation, preserved verbatim.
    pub raw_repr: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Normalized,
    Subtracted,
    Residual,
    Guinier,
    Porod,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Valid,
    Stale(String),  // reason it must be recomputed
    Failed(String), // diagnostic code
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub q_lo: f64,
    pub q_hi: f64,
    pub fit: serde_json::Value, // slope/intercept/r2 etc.
    pub accepted: Option<bool>, // None = undecided; multiple may coexist
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DerivedNode {
    pub id: String,
    pub kind: NodeKind,
    /// (entity id, entity version) pins — provenance for recompute & trace-back.
    pub inputs: Vec<(String, u64)>,
    pub params: serde_json::Value,
    pub status: NodeStatus,
    pub curve: Option<Curve>,
    pub candidates: Vec<IntervalCandidate>,
    pub diagnostics: Vec<Diagnostic>,
    /// Human-readable / machine-checkable explanation of validity.
    pub validity_note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub ts: String,
    pub actor: String,
    pub kind: String, // import|revise_params|source_change|decision|conflict|resolve|replay
    pub base_version: u64,
    pub resulting_version: u64,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conflict {
    pub id: String,
    pub base_version: u64,
    pub branch_a: u64, // version produced by first save
    pub branch_b: u64, // version produced by concurrent save
    pub resolved: Option<u64>, // winning version; loser retained
}
