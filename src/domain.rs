use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnitValue {
    pub value: f64,
    pub unit: String,
    pub raw: String,
    pub convertible: bool,
    pub diagnostic: Option<String>,
}

impl UnitValue {
    pub fn new(value: f64, unit: &str, raw: &str) -> Self {
        Self { value, unit: unit.to_string(), raw: raw.to_string(), convertible: true, diagnostic: None }
    }

    pub fn unconverted(raw: &str, intended_unit: &str, diagnostic: impl Into<String>) -> Self {
        Self {
            value: f64::NAN,
            unit: intended_unit.to_string(),
            raw: raw.to_string(),
            convertible: false,
            diagnostic: Some(diagnostic.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub id: String,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub q_unit: String,
    pub i_unit: String,
    pub mask_state: String,
    pub mask_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawCurve {
    pub id: String,
    pub label: String,
    pub role: String,
    pub version: String,
    pub q_axis: String,
    pub points: Vec<RawPoint>,
    pub exposure: UnitValue,
    pub thickness: UnitValue,
    pub transmission: UnitValue,
    pub transmission_sigma: f64,
    pub exposure_sigma: f64,
    pub thickness_sigma: f64,
    pub instrument_response: InstrumentResponse,
    pub mask_source: String,
    pub source_record: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstrumentResponse {
    pub id: String,
    pub gain: f64,
    pub gain_sigma: f64,
    pub unit: String,
    pub source: String,
    pub raw_record: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedPoint {
    pub id: String,
    pub q: f64,
    pub q_unit: String,
    pub intensity: f64,
    pub sigma: f64,
    pub sample_source_point: String,
    pub buffer_source_point_a: Option<String>,
    pub buffer_source_point_b: Option<String>,
    pub interpolation_weight: Option<f64>,
    pub in_overlap: bool,
    pub negative: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntervalCandidate {
    pub id: String,
    pub kind: String,
    pub q_min: f64,
    pub q_max: f64,
    pub status: String,
    pub score: f64,
    pub residual_rms: f64,
    pub evidence: String,
    pub overlaps: Vec<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResidualPoint {
    pub id: String,
    pub q: f64,
    pub residual: f64,
    pub sigma: f64,
    pub normalized_residual: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractionResult {
    pub id: String,
    pub version: String,
    pub sample_id: String,
    pub buffer_id: String,
    pub scale: f64,
    pub scale_sigma: f64,
    pub overlap_q_min: Option<f64>,
    pub overlap_q_max: Option<f64>,
    pub grid_mismatch: bool,
    pub interpolated: bool,
    pub status: String,
    pub diagnostics: Vec<String>,
    pub points: Vec<DerivedPoint>,
    pub residuals: Vec<ResidualPoint>,
    pub intervals: Vec<IntervalCandidate>,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub seq: i64,
    pub event_type: String,
    pub actor: String,
    pub base_event_id: Option<String>,
    pub summary: String,
    pub payload: serde_json::Value,
    pub created_at: String,
    pub status: String,
    pub affects: Vec<String>,
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub status: String,
    pub reason: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidateComparison {
    pub id: String,
    pub label: String,
    pub result_id: String,
    pub buffer_id: String,
    pub residual_rms: f64,
    pub chi_square: f64,
    pub scale_sigma: f64,
    pub legal: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppState {
    pub events: Vec<Event>,
    pub raw_curves: Vec<RawCurve>,
    pub results: Vec<SubtractionResult>,
    pub graph_nodes: Vec<GraphNode>,
    pub graph_edges: Vec<GraphEdge>,
    pub candidates: Vec<CandidateComparison>,
    pub current_event_id: String,
    pub current_result_ids: Vec<String>,
    pub accepted_result_id: Option<String>,
    pub rejected_result_ids: Vec<String>,
    pub counters: BTreeMap<String, i64>,
}

pub fn finite_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}
