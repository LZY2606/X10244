//! Domain data model. Raw imported representations are preserved verbatim;
//! derived layers carry normalized values plus provenance.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunRole {
    #[serde(rename = "sample")]
    Sample,
    #[serde(rename = "buffer")]
    Buffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    #[serde(rename = "error")]
    Error,
    #[serde(rename = "warning")]
    Warning,
    #[serde(rename = "info")]
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    /// associated raw row / point index when applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
}

impl Diagnostic {
    pub fn error(code: impl Into<String>, msg: impl Into<String>, at: Option<usize>) -> Self {
        Self { code: code.into(), severity: Severity::Error, message: msg.into(), at }
    }
    pub fn warn(code: impl Into<String>, msg: impl Into<String>, at: Option<usize>) -> Self {
        Self { code: code.into(), severity: Severity::Warning, message: msg.into(), at }
    }
    pub fn info(code: impl Into<String>, msg: impl Into<String>, at: Option<usize>) -> Self {
        Self { code: code.into(), severity: Severity::Info, message: msg.into(), at }
    }
}

/// One raw 1-D row exactly as imported (text is preserved).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRow {
    pub q: String,
    pub i: String,
    pub e: String,
}

/// Scalar metadata field with its original textual representation and unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawScalar {
    pub raw: String,
    pub unit: String,
}

/// Complete import record of a 2-D-integrated I(q) run. Nothing is discarded:
/// q/I/error rows, exposure, thickness, transmission, candidate linkage,
/// instrument response and mask provenance all live here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunImportInput {
    pub id: String,
    pub role: RunRole,
    pub label: String,
    pub q_unit: String,
    pub i_unit: String,
    pub err_unit: String,
    pub exposure: RawScalar,
    pub thickness: RawScalar,
    pub transmission: RawScalar,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask_id: Option<String>,
    pub rows: Vec<RawRow>,
    pub source_file: String,
    /// sha256-ish provenance hash supplied by the importer
    pub source_hash: String,
}

/// Result of parsing a raw import into base units. Unconvertible cells stay at
/// their original index with NaN and a diagnostic; rows are never dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedRun {
    pub input: RunImportInput,
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub e: Vec<f64>,
    pub row_usable: Vec<bool>,
    pub exposure_s: Option<f64>,
    pub thickness_mm: Option<f64>,
    pub transmission: Option<f64>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Amendment of a single direct source. Original raw rows are retained; the
/// override replaces effective parsed values at listed indices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunAmendment {
    pub run_id: String,
    pub reason: String,
    /// indexed raw rows to replace (raw representation preserved)
    pub rows: Vec<(usize, RawRow)>,
    pub revision: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponsePoint {
    pub q: f64,
    pub factor: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub id: String,
    pub label: String,
    pub source: String,
    pub q_unit: String,
    /// normalized to A^-1
    pub points: Vec<ResponsePoint>,
    pub rule_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub id: String,
    pub label: String,
    pub source: String,
    pub q_unit: String,
    /// masked q intervals in A^-1
    pub q_ranges: Vec<(f64, f64)>,
    /// masked beamstop / detector indices
    pub index_masks: Vec<usize>,
    /// original textual representation retained
    pub raw_repr: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormPoint {
    pub id: String,
    pub index: usize,
    pub q: f64,
    pub i: f64,
    pub sigma_i: f64,
    pub raw_row: usize,
    pub masked: bool,
    pub usable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizationMeta {
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub transmission: f64,
    pub response_id: Option<String>,
    pub mask_id: Option<String>,
    pub rule_version: String,
}

/// One immutable normalized version of a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormCurve {
    pub id: String,
    pub run_id: String,
    pub revision: u32,
    pub q_basis: String,
    pub intensity_unit: String,
    pub meta: NormalizationMeta,
    pub points: Vec<NormPoint>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScaleCandidate {
    pub id: String,
    pub method: String,
    pub alpha: f64,
    pub sigma_alpha: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit_window: Option<(f64, f64)>,
    pub n_points: usize,
    pub chi2: f64,
    pub rms_residual: f64,
    pub uncertain: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPoint {
    pub id: String,
    pub index: usize,
    pub q: f64,
    pub i: f64,
    pub sigma_i: f64,
    pub sample_point_id: String,
    pub buffer_point_id: String,
    pub interpolated: bool,
    pub residual: f64,
    pub negative: bool,
}

/// Immutable background-subtracted version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubVersion {
    pub id: String,
    pub artifact_id: String,
    pub sample_run: String,
    pub buffer_run: String,
    pub sample_norm: String,
    pub buffer_norm: String,
    pub q_basis: String,
    /// true when the two grids coincide (no interpolation needed)
    pub direct_grids: bool,
    pub overlap: Option<(f64, f64)>,
    pub points: Vec<SubPoint>,
    pub candidates: Vec<ScaleCandidate>,
    pub windows: Vec<(f64, f64)>,
    pub diagnostics: Vec<Diagnostic>,
    pub flags: Vec<String>,
    pub rule_version: String,
    pub created_seq: u64,
}

/// Independent Guinier / Porod interval evidence. Never stitched
/// automatically into a single model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionFit {
    pub id: String,
    pub artifact_id: String,
    pub sub_version: String,
    pub kind: String,
    pub q_min: f64,
    pub q_max: f64,
    pub n_points: usize,
    pub slope: f64,
    pub slope_sigma: f64,
    pub intercept: f64,
    pub intercept_sigma: f64,
    pub chi2: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rg_sigma: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i0: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i0_sigma: Option<f64>,
    pub qrg_min: Option<f64>,
    pub qrg_max: Option<f64>,
    pub overlap_with: Vec<String>,
    pub flags: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub rule_version: String,
    pub created_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub event_id: String,
    pub seq: u64,
    pub kind: String,
    pub target_artifact: String,
    pub chosen: Option<String>,
    pub rejected: Vec<String>,
    pub params_adjusted: serde_json::Value,
    pub note: String,
    pub base_seq: u64,
}
