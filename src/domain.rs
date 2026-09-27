use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type Json = serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    Complete,
    CompleteWithWarnings,
    Blocked,
    Candidate,
    Superseded,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAction {
    Accept,
    Reject,
    Continue,
    Revise,
    ResolveConflict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitValue {
    pub value: f64,
    pub unit: String,
    #[serde(default)]
    pub uncertainty: f64,
    #[serde(default)]
    pub raw_representation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCurve {
    pub id: String,
    pub role: String,
    pub label: String,
    pub q: Vec<f64>,
    pub intensity: Vec<f64>,
    pub error: Vec<f64>,
    pub q_unit: String,
    pub intensity_unit: String,
    pub coordinate_basis: String,
    pub exposure: UnitValue,
    pub thickness: UnitValue,
    pub transmission: UnitValue,
    pub mask_source: String,
    pub instrument_response_id: Option<String>,
    #[serde(default)]
    pub source_file: String,
    #[serde(default)]
    pub raw_representation: Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseCurve {
    pub id: String,
    pub label: String,
    pub q: Vec<f64>,
    pub response: Vec<f64>,
    pub uncertainty: Vec<f64>,
    pub q_unit: String,
    pub coordinate_basis: String,
    pub source_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskRecord {
    pub id: String,
    pub label: String,
    pub excluded_q: Vec<[f64; 2]>,
    pub q_unit: String,
    pub source: String,
    pub coordinate_basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointProvenance {
    pub point_id: String,
    pub q: f64,
    pub q_unit: String,
    pub source_point_ids: Vec<String>,
    pub operation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedCurve {
    pub id: String,
    pub source_id: String,
    pub q: Vec<f64>,
    pub intensity: Vec<f64>,
    pub error: Vec<f64>,
    pub q_unit: String,
    pub intensity_unit: String,
    pub normalization_factor: Vec<f64>,
    pub point_ids: Vec<String>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtractedCurve {
    pub id: String,
    pub sample_id: String,
    pub background_id: String,
    pub q: Vec<f64>,
    pub intensity: Vec<f64>,
    pub error: Vec<f64>,
    pub q_unit: String,
    pub intensity_unit: String,
    pub overlap_q_min: Option<f64>,
    pub overlap_q_max: Option<f64>,
    pub scale: f64,
    pub scale_uncertainty: f64,
    pub point_provenance: Vec<PointProvenance>,
    pub diagnostics: Vec<String>,
    pub status: ArtifactStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCandidate {
    pub id: String,
    pub subtracted_id: String,
    pub kind: String,
    pub q_min: f64,
    pub q_max: f64,
    pub q_unit: String,
    pub parameters: BTreeMap<String, f64>,
    pub residual_rss: f64,
    pub residual_rms: f64,
    pub point_count: usize,
    pub legal: bool,
    pub diagnostics: Vec<String>,
    pub overlaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasePack {
    pub schema_hint: String,
    pub title: String,
    pub raw_curves: Vec<RawCurve>,
    pub responses: Vec<ResponseCurve>,
    pub masks: Vec<MaskRecord>,
    pub operations: Vec<OperationSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationSpec {
    pub event_id: String,
    pub kind: String,
    pub node_id: String,
    pub label: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub background_id: Option<String>,
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default)]
    pub scale_uncertainty: Option<f64>,
    #[serde(default)]
    pub intervals: Vec<[f64; 2]>,
    #[serde(default)]
    pub actor: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub expected_seq: Option<i64>,
    #[serde(default)]
    pub chosen: Option<String>,
    #[serde(default)]
    pub rejected: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub id: String,
    pub seq: i64,
    pub kind: String,
    pub actor: String,
    pub note: String,
    pub node_id: String,
    pub version_id: Option<String>,
    pub base_seq: Option<i64>,
    pub payload: Json,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub id: String,
    pub node_id: String,
    pub version: i64,
    pub kind: String,
    pub label: String,
    pub status: ArtifactStatus,
    pub parent_id: Option<String>,
    pub payload: Json,
    pub created_seq: i64,
    pub scope_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub node_id: String,
    pub label: String,
    pub current_id: String,
    pub state: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub at_seq: i64,
    pub events: Vec<EventRecord>,
    pub artifacts: Vec<ArtifactRecord>,
    pub current: BTreeMap<String, String>,
    pub graph: Vec<GraphNode>,
}
