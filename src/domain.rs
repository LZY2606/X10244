use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const RULE_VERSION: &str = "saxs-rules-2026.1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CasePackage {
    pub package_id: String,
    pub title: String,
    pub coordinate_basis: String,
    pub imported_representation: String,
    pub case: Case,
    pub curves: Vec<RawCurve>,
    pub measurements: Vec<MeasurementRecord>,
    pub responses: Vec<InstrumentResponse>,
    pub masks: Vec<Mask>,
    pub buffer_candidates: Vec<BufferCandidate>,
    pub edge_materials: BTreeMap<String, EdgeMaterial>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Case {
    pub id: String,
    pub label: String,
    pub group: String,
    pub integration_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawCurve {
    pub id: String,
    pub role: String,
    pub label: String,
    pub source: String,
    pub q_unit: String,
    pub intensity_unit: String,
    pub coordinate_basis: String,
    pub points: Vec<RawPoint>,
    pub version: u32,
    pub predecessor_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub id: String,
    pub index: usize,
    pub q_raw: String,
    pub i_raw: String,
    pub error_raw: String,
    pub q: Option<f64>,
    pub intensity: Option<f64>,
    pub error: Option<f64>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeasurementRecord {
    pub id: String,
    pub curve_id: String,
    pub role: String,
    pub exposure: Quantity,
    pub thickness: Quantity,
    pub transmission: UncertainQuantity,
    pub response_id: String,
    pub mask_id: String,
    pub source_document: String,
    pub version: u32,
    pub predecessor_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    pub raw: String,
    pub value: Option<f64>,
    pub unit: String,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UncertainQuantity {
    pub raw: String,
    pub value: Option<f64>,
    pub error: Option<f64>,
    pub unit: String,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstrumentResponse {
    pub id: String,
    pub label: String,
    pub source: String,
    pub q_unit: String,
    pub factors: Vec<ResponsePoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResponsePoint {
    pub q: f64,
    pub factor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Mask {
    pub id: String,
    pub label: String,
    pub source: String,
    pub excluded_indices: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BufferCandidate {
    pub id: String,
    pub record_id: String,
    pub version_label: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdgeMaterial {
    pub label: String,
    pub sample_record_id: String,
    pub buffer_record_id: String,
    pub expected_signal: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventRecord {
    pub seq: i64,
    pub id: String,
    pub event_type: String,
    pub author: String,
    pub created_at: String,
    pub basis_revision: Option<i64>,
    pub payload: serde_json::Value,
    pub supersedes_event_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObjectRecord {
    pub id: String,
    pub logical_key: String,
    pub kind: String,
    pub version: u32,
    pub event_id: String,
    pub event_seq: i64,
    pub status: String,
    pub predecessor_id: Option<String>,
    pub rule_version: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Edge {
    pub from_object_id: String,
    pub to_object_id: String,
    pub relation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedCurve {
    pub id: String,
    pub record_id: String,
    pub curve_id: String,
    pub q_unit: String,
    pub intensity_unit: String,
    pub formula: String,
    pub points: Vec<NormalizedPoint>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedPoint {
    pub id: String,
    pub index: usize,
    pub source_point_id: String,
    pub q: f64,
    pub intensity: f64,
    pub error: f64,
    pub masked: bool,
    pub response_factor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractionCandidate {
    pub id: String,
    pub label: String,
    pub sample_norm_id: String,
    pub buffer_norm_id: String,
    pub buffer_candidate_id: String,
    pub method: String,
    pub scale: Option<f64>,
    pub scale_error: Option<f64>,
    pub overlap_q_min: Option<f64>,
    pub overlap_q_max: Option<f64>,
    pub interpolation_required: bool,
    pub q_unit: String,
    pub intensity_unit: String,
    pub points: Vec<SubtractionPoint>,
    pub residuals: ResidualSummary,
    pub diagnostics: Vec<String>,
    pub decision_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractionPoint {
    pub id: String,
    pub index: usize,
    pub q: f64,
    pub sample_intensity: f64,
    pub buffer_intensity: f64,
    pub intensity: f64,
    pub error: f64,
    pub residual: f64,
    pub normalized_residual: f64,
    pub interpolated_buffer: bool,
    pub sample_point_id: String,
    pub buffer_point_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ResidualSummary {
    pub points: usize,
    pub chi_square: f64,
    pub reduced_chi_square: f64,
    pub mean: f64,
    pub weighted_rms: f64,
    pub positive_count: usize,
    pub negative_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntervalCandidate {
    pub id: String,
    pub subtraction_id: String,
    pub interval_kind: String,
    pub q_start: f64,
    pub q_end: f64,
    pub point_ids: Vec<String>,
    pub slope: f64,
    pub intercept: f64,
    pub r2: f64,
    pub evidence_value: Option<f64>,
    pub evidence_label: String,
    pub residual_chi2: f64,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimelineState {
    pub revision: i64,
    pub at_event_seq: Option<i64>,
    pub package: CasePackage,
    pub events: Vec<EventRecord>,
    pub objects: Vec<ObjectRecord>,
    pub edges: Vec<Edge>,
    pub current: BTreeMap<String, String>,
    pub normalized: Vec<NormalizedCurve>,
    pub subtractions: Vec<SubtractionCandidate>,
    pub intervals: Vec<IntervalCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProvenanceResponse {
    pub at_event_seq: Option<i64>,
    pub target: Option<ObjectRecord>,
    pub chain: Vec<ObjectRecord>,
    pub sources: Vec<ObjectRecord>,
    pub edges: Vec<Edge>,
    pub validity: ValidityExplanation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ValidityExplanation {
    pub status: String,
    pub reason: String,
    pub checked_dependencies: Vec<String>,
    pub unchanged_dependencies: Vec<String>,
    pub changed_or_missing_dependencies: Vec<String>,
    pub recomputed_by_event: Option<String>,
}
