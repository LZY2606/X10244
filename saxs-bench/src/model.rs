//! Domain model: retained raw records, derived curves, model candidates,
//! machine-readable judgments and provenance links.

use crate::units::{Field, QBasis};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Sample,
    Buffer,
}

/// Severity of a judgment. `Blocked` means the chain must not subtract/merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Ok,
    Warning,
    Blocked,
}

/// A machine-readable judgment tied to a fixed rule id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Judgment {
    /// Stable rule identifier, e.g. "grid.overlap_only".
    pub rule_id: String,
    pub severity: Severity,
    pub message: String,
    /// Structured evidence so the UI and tests can assert without parsing text.
    #[serde(default)]
    pub evidence: serde_json::Value,
}

impl Judgment {
    pub fn ok(rule_id: &str, message: impl Into<String>, evidence: serde_json::Value) -> Self {
        Self { rule_id: rule_id.into(), severity: Severity::Ok, message: message.into(), evidence }
    }
    pub fn warn(rule_id: &str, message: impl Into<String>, evidence: serde_json::Value) -> Self {
        Self { rule_id: rule_id.into(), severity: Severity::Warning, message: message.into(), evidence }
    }
    pub fn blocked(rule_id: &str, message: impl Into<String>, evidence: serde_json::Value) -> Self {
        Self { rule_id: rule_id.into(), severity: Severity::Blocked, message: message.into(), evidence }
    }
}

/// One original (q, I, σ) point, preserving raw text representations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawPoint {
    /// 0-based index in the imported file; also the stable in-profile id.
    pub index: usize,
    pub q_raw: String,
    pub i_raw: String,
    pub e_raw: String,
    pub q: Option<f64>,
    pub i: Option<f64>,
    pub e: Option<f64>,
    /// True when this point is masked out (still retained, never plotted).
    pub masked: bool,
    pub mask_reason: Option<String>,
}

/// A raw imported record: q/I/error plus exposure, thickness, transmission,
/// buffer candidacy, instrument response and mask provenance all retained.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawProfile {
    pub id: String,
    pub role: Role,
    /// Human label, e.g. "sample-A", "buffer-v1".
    pub label: String,
    /// Version tag for buffer version switching ("v1"/"v2").
    pub version: String,
    /// Original q-coordinate token; preserved even if unparseable.
    pub q_basis_raw: String,
    pub q_basis: Option<QBasis>,
    /// Original intensity unit token (retained; derived layer is unit-scaled).
    pub intensity_unit: String,
    pub exposure: Field,
    pub thickness: Field,
    pub transmission: Field,
    /// Relative uncertainty of the transmission measurement.
    pub transmission_rel_uncertainty: f64,
    pub instrument_response: InstrumentResponse,
    pub mask_source: MaskSource,
    pub points: Vec<RawPoint>,
    pub diagnostics: Vec<Judgment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub source: String,
    /// Response curve/file reference id (provenance), kept verbatim.
    pub reference: String,
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskSource {
    pub source: String,
    pub reference: String,
    /// Number of points masked; points themselves stay in the record.
    pub masked_count: usize,
}

/// A normalised point on the canonical q grid (Å⁻¹).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormPoint {
    /// Stable point id within the derived curve.
    pub id: String,
    pub q: f64,
    pub i: f64,
    /// 1σ from photon error propagation only.
    pub sigma_stat: f64,
    pub source_profile: String,
    pub source_index: usize,
    /// Original q value before basis conversion (kept for traceback).
    pub q_source_basis: f64,
}

/// A normalised (transmission·exposure corrected) curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormCurve {
    pub id: String,
    pub source_profile: String,
    pub role: Role,
    pub label: String,
    pub version: String,
    pub points: Vec<NormPoint>,
    /// Relative 1σ of the normalisation factor (transmission uncertainty).
    pub norm_rel_sigma: Option<f64>,
    pub judgments: Vec<Judgment>,
}

/// One background-subtracted point. Negative intensities are retained.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPoint {
    pub id: String,
    pub q: f64,
    pub sample_i: f64,
    /// Buffer value interpolated onto the sample q (None outside overlap).
    pub buffer_i: Option<f64>,
    pub in_overlap: bool,
    /// Subtracted intensity; None where no overlap exists.
    pub i_sub: Option<f64>,
    /// Total 1σ incl. normalisation and background-scale uncertainty.
    pub sigma_total: Option<f64>,
    pub source_sample_norm: String,
    pub source_buffer_norm: String,
    pub source_sample_index: usize,
    pub source_buffer_lo: Option<usize>,
    pub source_buffer_hi: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntervalKind {
    Guinier,
    Porod,
}

/// A legal Guinier/Porod interval candidate. Candidates are never auto-merged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub kind: IntervalKind,
    pub q_lo: f64,
    pub q_hi: f64,
    pub n_points: usize,
    pub chi2: f64,
    pub rms: f64,
    pub valid: bool,
    pub reason: String,
    pub params: serde_json::Value,
}

/// One legal background-subtraction branch (a buffer candidate + fit window).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubBranch {
    pub id: String,
    pub sample_norm: String,
    pub buffer_norm: String,
    pub buffer_version: String,
    pub buffer_label: String,
    /// Fit window [lo,hi] (Å⁻¹) used to estimate the background scale.
    pub fit_lo: f64,
    pub fit_hi: f64,
    pub scale: Option<f64>,
    /// 1σ of the fitted scale (background scaling uncertainty).
    pub scale_sigma: Option<f64>,
    pub overlap_lo: Option<f64>,
    pub overlap_hi: Option<f64>,
    pub overlap_points: usize,
    pub rms: Option<f64>,
    pub chi2: Option<f64>,
    pub points: Vec<SubPoint>,
    pub intervals: Vec<IntervalCandidate>,
    pub judgments: Vec<Judgment>,
    pub blocked: bool,
}

/// Whether a branch was chosen by a decision, rejected, or still pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    Accept,
    Reject,
    ParamAdjust,
}

/// An explicit human adjudication. Appended only; never overwrites history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub branch_id: String,
    pub kind: DecisionKind,
    pub note: String,
    /// Optional parameter adjustment (e.g. new fit window), recorded verbatim.
    #[serde(default)]
    pub params: serde_json::Value,
}
