use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type Id = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub id: Id,
    pub q: f64,
    pub i: Option<f64>,
    pub sigma: Option<f64>,
    pub q_unit: String,
    pub i_unit: String,
    pub raw_q: String,
    pub raw_i: String,
    pub raw_sigma: String,
    pub mask_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawCurve {
    pub id: Id,
    pub role: String,
    pub label: String,
    pub version: u32,
    pub points: Vec<RawPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Measurement {
    pub id: Id,
    pub role: String,
    pub label: String,
    pub exposure: Quantity,
    pub thickness: Quantity,
    pub transmission: Quantity,
    pub instrument_response_id: Id,
    pub mask_source_id: Id,
    pub curve_ids: Vec<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstrumentResponse {
    pub id: Id,
    pub label: String,
    pub scale_factor: f64,
    pub uncertainty_relative: f64,
    pub coordinate_basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MaskSource {
    pub id: Id,
    pub label: String,
    pub source: String,
    pub excluded_q: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedPoint {
    pub id: Id,
    pub raw_point_id: Id,
    pub q_inv_a: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub mask_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedCurve {
    pub id: Id,
    pub measurement_id: Id,
    pub curve_id: Id,
    pub version: u32,
    pub points: Vec<NormalizedPoint>,
    pub exposure_s: f64,
    pub thickness_cm: f64,
    pub transmission: f64,
    pub response_scale: f64,
    pub basis: DerivationBasis,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScaleFit {
    pub value: f64,
    pub sigma: f64,
    pub method: String,
    pub overlap_q_min: f64,
    pub overlap_q_max: f64,
    pub overlap_points: usize,
    pub fixed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractedPoint {
    pub id: Id,
    pub q_inv_a: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub sample_point_id: Id,
    pub background_point_id: Id,
    pub interpolated_background: bool,
    pub residual_z: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractedCurve {
    pub id: Id,
    pub label: String,
    pub sample_normalized_id: Id,
    pub background_normalized_id: Id,
    pub sample_measurement_id: Id,
    pub background_measurement_id: Id,
    pub scale: ScaleFit,
    pub points: Vec<SubtractedPoint>,
    pub diagnostics: Vec<Diagnostic>,
    pub basis: DerivationBasis,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelCandidate {
    pub id: Id,
    pub subtracted_id: Id,
    pub family: String,
    pub label: String,
    pub q_min_inv_a: f64,
    pub q_max_inv_a: f64,
    pub q_r_guinier: Option<f64>,
    pub slope: Option<f64>,
    pub intercept: f64,
    pub chi_squared: f64,
    pub points_used: usize,
    pub overlap_with: Vec<Id>,
    pub accepted: Option<bool>,
    pub residual_diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub q_min_inv_a: Option<f64>,
    pub q_max_inv_a: Option<f64>,
    pub related_ids: Vec<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivationBasis {
    pub event_id: Id,
    pub input_hash: String,
    pub rule_version: Id,
    pub inputs: Vec<Id>,
    pub valid: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Decision {
    pub id: Id,
    pub event_id: Id,
    pub target_id: Id,
    pub action: String,
    pub rationale: String,
    pub accepted: bool,
    pub supersedes: Option<Id>,
    pub parent_head: Id,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Conflict {
    pub id: Id,
    pub expected_head: Id,
    pub actual_head: Id,
    pub attempted_action: String,
    pub resolution: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub seq: u64,
    pub id: Id,
    pub timestamp: String,
    pub kind: String,
    pub actor: String,
    pub parent_id: Option<Id>,
    pub head_after: Id,
    pub summary: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProjectState {
    pub seq: u64,
    pub head: Option<Id>,
    pub events: Vec<Event>,
    pub curves: BTreeMap<Id, RawCurve>,
    pub measurements: BTreeMap<Id, Measurement>,
    pub responses: BTreeMap<Id, InstrumentResponse>,
    pub masks: BTreeMap<Id, MaskSource>,
    pub normalized: BTreeMap<Id, NormalizedCurve>,
    pub subtracted: BTreeMap<Id, SubtractedCurve>,
    pub models: BTreeMap<Id, ModelCandidate>,
    pub decisions: BTreeMap<Id, Decision>,
    pub conflicts: BTreeMap<Id, Conflict>,
    pub current_pointers: BTreeMap<String, Id>,
}

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("conflict: expected head {expected}, found {actual}")]
    StaleHead { expected: Id, actual: Id },
    #[error("missing record: {0}")]
    Missing(String),
    #[error("invalid value: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselinePayload {
    pub curves: Vec<RawCurve>,
    pub measurements: Vec<Measurement>,
    pub responses: Vec<InstrumentResponse>,
    pub masks: Vec<MaskSource>,
    pub normalized: Vec<NormalizedCurve>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionPayload {
    pub measurement_id: Id,
    pub old_exposure: Quantity,
    pub new_exposure: Quantity,
    pub old_thickness: Quantity,
    pub new_thickness: Quantity,
    pub old_transmission: Quantity,
    pub new_transmission: Quantity,
    pub normalized: Vec<NormalizedCurve>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawAmendmentPayload {
    pub old_curve_id: Id,
    pub new_curve: RawCurve,
    pub normalized: Vec<NormalizedCurve>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizationPayload {
    pub normalized: NormalizedCurve,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtractionPayload {
    pub subtracted: SubtractedCurve,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsPayload {
    pub candidates: Vec<ModelCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionPayload {
    pub decision: Decision,
    pub pointer_key: Option<String>,
    pub pointer_value: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictPayload {
    pub conflict: Conflict,
}

fn stable_hash(parts: &[&str]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("sha1like-{hash:016x}")
}

fn json_digest<T: Serialize>(value: &T) -> String {
    let text = serde_json::to_string(value).expect("serializable derivation input");
    stable_hash(&[&text])
}

fn finite(value: f64) -> bool {
    value.is_finite()
}

fn convert_q(q: f64, unit: &str) -> Option<f64> {
    match unit {
        "1/A" | "Å^-1" | "A^-1" => Some(q),
        "1/nm" | "nm^-1" => Some(q * 0.1),
        _ => None,
    }
}

fn convert_to_seconds(quantity: &Quantity) -> Option<f64> {
    match quantity.unit.as_str() {
        "s" | "sec" | "second" | "seconds" => Some(quantity.value),
        "min" | "minute" | "minutes" => Some(quantity.value * 60.0),
        "ms" => Some(quantity.value / 1000.0),
        _ => None,
    }
}

fn convert_to_cm(quantity: &Quantity) -> Option<f64> {
    match quantity.unit.as_str() {
        "cm" => Some(quantity.value),
        "mm" => Some(quantity.value / 10.0),
        "m" => Some(quantity.value * 100.0),
        _ => None,
    }
}
