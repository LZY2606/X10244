use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
    pub raw_representation: String,
    pub conversion_diagnostic: Option<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub id: String,
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub q_unit: String,
    pub intensity_unit: String,
    pub masked: bool,
    pub mask_source: String,
    pub original_row: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    pub id: String,
    pub role: String,
    pub label: String,
    pub points: Vec<RawPoint>,
    pub exposure: Quantity,
    pub thickness: Quantity,
    pub transmission: Quantity,
    pub instrument_response_id: String,
    pub instrument_response: Quantity,
    pub mask_source: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SampleRecord {
    #[serde(flatten)]
    pub curve: Curve,
    pub buffer_candidate_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BufferRecord {
    #[serde(flatten)]
    pub curve: Curve,
    pub version_of: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstrumentResponse {
    pub id: String,
    pub label: String,
    pub scale_factor: f64,
    pub relative_uncertainty: f64,
    pub q_grid_basis: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Mask {
    pub id: String,
    pub label: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaseBundle {
    pub case_id: String,
    pub title: String,
    pub samples: BTreeMap<String, SampleRecord>,
    pub buffers: BTreeMap<String, BufferRecord>,
    pub responses: BTreeMap<String, InstrumentResponse>,
    pub masks: BTreeMap<String, Mask>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PointStatus {
    Ok,
    Warning,
    Invalid,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Valid,
    Warning,
    Rejected,
    Stale,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProvenanceRef {
    pub entity_id: String,
    pub version_id: String,
    pub point_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedPoint {
    pub id: String,
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub status: PointStatus,
    pub source: ProvenanceRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackgroundInterpolatedPoint {
    pub id: String,
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub interpolation: String,
    pub source: ProvenanceRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractedPoint {
    pub id: String,
    pub id_stable: String,
    pub q: f64,
    pub sample_intensity: f64,
    pub background_intensity: f64,
    pub scale: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub status: PointStatus,
    pub sample_source: ProvenanceRef,
    pub background_source: ProvenanceRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntervalCandidate {
    pub id: String,
    pub model: String,
    pub q_min: f64,
    pub q_max: f64,
    pub q_rg: Option<f64>,
    pub i0: Option<f64>,
    pub slope: Option<f64>,
    pub chi_squared: f64,
    pub rms_residual: f64,
    pub max_abs_residual: f64,
    pub point_count: usize,
    pub accepted: Option<bool>,
    pub diagnostic: Option<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReductionRun {
    pub id: String,
    pub key: String,
    pub sample_id: String,
    pub buffer_id: String,
    pub status: RunStatus,
    pub normalized_sample: Vec<NormalizedPoint>,
    pub interpolated_background: Vec<BackgroundInterpolatedPoint>,
    pub subtracted: Vec<SubtractedPoint>,
    pub scale: f64,
    pub scale_sigma: f64,
    pub scale_method: String,
    pub overlap_q_min: Option<f64>,
    pub overlap_q_max: Option<f64>,
    pub intervals: Vec<IntervalCandidate>,
    pub diagnostics: Vec<Diagnostic>,
    pub input_hashes: BTreeMap<String, String>,
    pub rule_version: String,
}
