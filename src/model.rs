//! Immutable domain records and command inputs.

use crate::quantity::Measure;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveRole {
    Sample,
    Buffer,
}

impl CurveRole {
    pub fn as_str(self) -> &'static str {
        match self {
            CurveRole::Sample => "sample",
            CurveRole::Buffer => "buffer",
        }
    }
}

/// One measured point exactly as it came out of the 2D integration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub idx: usize,
    /// Original textual representations are preserved.
    pub q_raw: String,
    pub i_raw: String,
    pub sigma_raw: String,
    pub q: Measure,
    pub i: Measure,
    pub sigma: Measure,
    pub masked: bool,
    pub mask_reasons: Vec<String>,
}

/// Mask provenance: polygons, detector edge, beamstop, pixel thresholds ...
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MaskSource {
    pub name: String,
    pub origin: String,
    pub sha256: String,
    pub point_indexes: Vec<usize>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstrumentResponse {
    pub id: String,
    pub label: String,
    pub source: String,
    /// Per-channel gain on the integrated grid (empty = flat response).
    pub gain: Vec<f64>,
    pub gain_relative_uncertainty: f64,
    pub scale: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawCurve {
    pub id: String,
    pub name: String,
    pub role: CurveRole,
    pub payload_key: String,
    pub q_unit: String,
    pub i_unit: String,
    pub points: Vec<RawPoint>,
    pub exposure: Measure,
    pub thickness: Measure,
    pub transmission: Measure,
    pub mask_sources: Vec<MaskSource>,
    pub response_id: Option<String>,
    pub supersedes: Option<String>,
    pub imported_at: String,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Warning,
    Error,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warning => "warning",
            Status::Error => "error",
        }
    }
}

/// Machine-readable judgment event (why a branch is valid/rejected/etc).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Judgment {
    /// Stable code, e.g. `q_grid_mismatch`, `no_overlap`, `scale_uncertainty`,
    /// `negative_intensity`, `interval_overlap`, `buffer_version_switch`,
    /// `multi_candidate_residuals`, `no_automatic_join`, `point_outside_overlap`.
    pub code: String,
    pub status: String,
    pub message: String,
    pub affected: Vec<String>,
    pub evidence: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedPoint {
    pub idx: usize,
    pub id: String,
    pub q: f64,
    pub value: f64,
    pub sigma: f64,
    /// Provenance point ids contributing at this q (1 or 2 sources).
    pub source_point_ids: Vec<String>,
    /// true when the point exists but is not a legal part of the operation
    /// (e.g. sample point outside the buffer overlap range).
    pub excluded: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivationKind {
    Normalized,
    Subtracted,
    IntervalCandidates,
}

impl DerivationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DerivationKind::Normalized => "normalized",
            DerivationKind::Subtracted => "subtracted",
            DerivationKind::IntervalCandidates => "interval_candidates",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "normalized" => Some(DerivationKind::Normalized),
            "subtracted" => Some(DerivationKind::Subtracted),
            "interval_candidates" => Some(DerivationKind::IntervalCandidates),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntervalCandidate {
    pub id: String,
    pub model: String, // "guinier" | "porod"
    pub q_min: f64,
    pub q_max: f64,
    pub qrg_max: Option<f64>,
    pub r_squared: f64,
    pub chi2: f64,
    pub residual_rms: f64,
    pub point_ids: Vec<String>,
    pub residuals: Vec<f64>,
    pub params: serde_json::Value,
    pub adopted: bool,
    pub rejected: bool,
    pub reject_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Derivation {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub rule_version: String,
    pub config_sig: String,
    pub inputs: Vec<String>,
    pub points: Vec<DerivedPoint>,
    pub judgments: Vec<Judgment>,
    pub candidates: Vec<IntervalCandidate>,
    pub created_event_seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub seq: i64,
    pub event_id: String,
    pub event_type: String,
    pub at: String,
    pub actor: String,
    pub client: String,
    pub body: serde_json::Value,
    pub prev_hash: String,
    pub hash: String,
}

/// Parameters controlling normalization/subtraction. Revisions create a new
/// version object; derivations record the exact parameter signature they used.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Params {
    pub version: String,
    /// Relative normalization uncertainty (flow, detector), e.g. 0.02.
    pub u_norm_rel: f64,
    /// Minimum number of overlap points.
    pub min_overlap_points: usize,
    /// Estimate buffer scale in a high-q region instead of k=1.
    pub estimate_buffer_scale: bool,
    pub scale_region_qmin: f64,
    pub scale_region_qmax: f64,
    /// Relative prior uncertainty on the buffer scale factor.
    pub u_scale_rel: f64,
    /// Guinier q*Rg window cap.
    pub guinier_qrg_max: f64,
    pub guinier_min_points: usize,
    pub porod_qmin: f64,
    pub porod_min_points: usize,
    /// Relative intensity floor below which a subtracted point is flagged.
    pub negative_flag_rel: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            version: "params-initial".into(),
            u_norm_rel: 0.02,
            min_overlap_points: 3,
            estimate_buffer_scale: false,
            scale_region_qmin: 0.2,
            scale_region_qmax: 0.5,
            u_scale_rel: 0.0,
            guinier_qrg_max: 1.3,
            guinier_min_points: 6,
            porod_qmin: 0.15,
            porod_min_points: 8,
            negative_flag_rel: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterCurve {
    pub name: String,
    pub role: CurveRole,
    pub payload_key: String,
    pub q_unit: String,
    pub i_unit: String,
    pub rows: Vec<CurveRow>,
    pub exposure_raw: String,
    pub exposure_unit: String,
    pub thickness_raw: String,
    pub thickness_unit: String,
    pub transmission_raw: String,
    pub transmission_unit: String,
    pub mask_sources: Vec<MaskSource>,
    pub response_id: Option<String>,
    pub supersedes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurveRow {
    pub q: String,
    pub i: String,
    pub sigma: String,
}
