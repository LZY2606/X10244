//! Domain model for the SAXS normalization / background-subtraction bench.
//! Every domain quantity carries a unit or coordinate basis; raw
//! representations are preserved at import and only normalized in the
//! derived layer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Curve {
    pub points: Vec<Point>,
}

impl Curve {
    pub fn q_min(&self) -> Option<f64> {
        self.points.iter().map(|p| p.q).fold(None, |a, b| Some(a.map_or(b, |x: f64| x.min(b))))
    }
    pub fn q_max(&self) -> Option<f64> {
        self.points.iter().map(|p| p.q).fold(None, |a, b| Some(a.map_or(b, |x: f64| x.max(b))))
    }
}

/// Measurement metadata as imported. Values are kept in their raw units;
/// the derived layer is responsible for any conversion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Meta {
    pub record: String,
    pub unit: String,           // q unit, e.g. "1/angstrom", "1/nm", or anything (kept raw)
    pub intensity_unit: String, // e.g. "counts", "1/cm"
    pub exposure_s: Option<f64>,
    pub thickness_mm: Option<f64>,
    pub transmission: Option<f64>,
    pub basis: String,          // coordinate basis, e.g. calibration reference
    pub extra: std::collections::BTreeMap<String, String>,
}

/// A machine-readable judgment emitted by a rule. These are stored as
/// events so that "why" is preserved, not just "what".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Judgment {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub detail: serde_json::Value,
}

impl Judgment {
    pub fn new(code: &str, message: impl Into<String>, detail: serde_json::Value) -> Self {
        Judgment { code: code.to_string(), message: message.into(), detail }
    }
}

// Judgment codes (stable, machine readable).
pub const J_GRID_MISMATCH_INTERPOLATED: &str = "GridMismatchInterpolated";
pub const J_INTERPOLATION_CLAMPED: &str = "InterpolationClampedToOverlap";
pub const J_NO_OVERLAP: &str = "NoOverlap";
pub const J_NEGATIVE_INTENSITY: &str = "NegativeIntensity";
pub const J_SCALE_UNCERTAINTY: &str = "ScaleUncertaintyPropagated";
pub const J_INTERVALS_OVERLAP_KEPT: &str = "IntervalsOverlapKept";
pub const J_INTERVALS_NOT_MERGED: &str = "IntervalsNotMergedIntoSingleModel";
pub const J_UNIT_UNCONVERTIBLE: &str = "UnitUnconvertible";
pub const J_META_MISSING: &str = "MetadataMissing";
pub const J_BUFFER_VERSION_SWITCHED: &str = "BufferVersionSwitched";
pub const J_MULTI_CANDIDATE_RESIDUALS: &str = "MultiCandidateResiduals";

/// Reference to an input node pinned at the version that was actually used.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputRef {
    pub id: String,
    pub version: i64,
    pub role: String,
}

/// A versioned node in the dependency graph. `payload` is kind-specific JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub version: i64,
    pub kind: String,
    pub name: String,
    pub status: String, // "valid" | "diagnostic" | "superseded" | "unadopted"
    #[serde(default)]
    pub inputs: Vec<InputRef>,
    #[serde(default)]
    pub spec: Option<serde_json::Value>, // derivation spec for recompute
    pub payload: serde_json::Value,
}

/// A machine-readable event in the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub version: i64,
    pub ts: String,
    pub actor: String,
    pub kind: String,
    pub machine: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub id: i64,
    pub base_version: i64,
    pub branch_a: i64,
    pub branch_b: i64,
    pub status: String, // "open" | "resolved"
    pub resolution: Option<serde_json::Value>,
}
