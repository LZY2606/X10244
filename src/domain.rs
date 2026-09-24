use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const Q_TOL: f64 = 1e-9;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub sigma: f64,
    pub unit: String,
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub index: usize,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub masked: bool,
    pub mask_sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CurveRecord {
    pub id: String,
    pub role: String,
    pub label: String,
    pub q_unit: String,
    pub intensity_unit: String,
    pub points: Vec<RawPoint>,
    pub exposure: Quantity,
    pub thickness: Quantity,
    pub transmission: Quantity,
    pub source_file: String,
    pub raw_representation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResponsePoint {
    pub index: usize,
    pub q: f64,
    pub response: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstrumentResponse {
    pub id: String,
    pub label: String,
    pub q_unit: String,
    pub points: Vec<ResponsePoint>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MaskRecord {
    pub id: String,
    pub label: String,
    pub source: String,
    pub q_min: f64,
    pub q_max: f64,
    pub q_unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackgroundScale {
    pub value: f64,
    pub sigma: f64,
    pub source: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelWindowConfig {
    pub guinier_q_min: f64,
    pub guinier_q_max: f64,
    pub porod_q_min: f64,
    pub porod_q_max: f64,
    pub q_unit: String,
}

impl Default for ModelWindowConfig {
    fn default() -> Self {
        Self {
            guinier_q_min: 0.010,
            guinier_q_max: 0.024,
            porod_q_min: 0.030,
            porod_q_max: 0.050,
            q_unit: "1/A".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaseMaterials {
    pub case_id: String,
    pub title: String,
    pub curves: Vec<CurveRecord>,
    pub responses: Vec<InstrumentResponse>,
    pub masks: Vec<MaskRecord>,
    pub buffer_candidates: Vec<String>,
    pub selected_buffer: String,
    pub response_id: String,
    pub background_scale: BackgroundScale,
    pub model_windows: ModelWindowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixturePack {
    pub schema: String,
    pub case: CaseMaterials,
    pub scenario: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub id: String,
    pub index: usize,
    pub q: f64,
    pub value: f64,
    pub sigma: f64,
    pub masked: bool,
    pub q_unit: String,
    pub value_unit: String,
    pub source_point_ids: Vec<String>,
    pub interpolation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedCurve {
    pub node_id: String,
    pub label: String,
    pub kind: String,
    pub status: String,
    pub q_unit: String,
    pub value_unit: String,
    pub points: Vec<Point>,
    pub diagnostics: Vec<Diagnostic>,
    pub formula: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub q_min: Option<f64>,
    pub q_max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelFit {
    pub id: String,
    pub family: String,
    pub status: String,
    pub q_min: f64,
    pub q_max: f64,
    pub q_unit: String,
    pub point_count: usize,
    pub intercept: f64,
    pub slope: f64,
    pub derived_parameter: f64,
    pub derived_parameter_unit: String,
    pub rms: f64,
    pub sse: f64,
    pub residual_point_ids: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelEvidence {
    pub node_id: String,
    pub status: String,
    pub fits: Vec<ModelFit>,
    pub diagnostics: Vec<Diagnostic>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractionCandidate {
    pub buffer_id: String,
    pub subtraction: DerivedCurve,
    pub scale: BackgroundScale,
    pub residual_rms: f64,
    pub chi_square: f64,
    pub accepted: bool,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackgroundResult {
    pub node_id: String,
    pub status: String,
    pub selected_buffer: Option<String>,
    pub candidates: Vec<SubtractionCandidate>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GraphNode {
    pub id: String,
    pub version: u32,
    pub kind: String,
    pub label: String,
    pub status: String,
    pub fingerprint: String,
    pub data: serde_json::Value,
    pub dependencies: BTreeMap<String, DependencyRef>,
    pub diagnostics: Vec<Diagnostic>,
    pub created_by_event: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DependencyRef {
    pub node_id: String,
    pub version: u32,
    pub fingerprint: String,
    pub q_min: f64,
    pub q_max: f64,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GraphState {
    pub case: Option<CaseMaterials>,
    pub nodes: BTreeMap<String, GraphNode>,
    pub events: Vec<EventRecord>,
    pub decisions: Vec<Decision>,
    pub conflicts: Vec<Conflict>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Decision {
    pub id: String,
    pub actor: String,
    pub subject: String,
    pub selected_candidate: String,
    pub status: String,
    pub base_event_id: String,
    pub note: String,
    pub event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Conflict {
    pub id: String,
    pub status: String,
    pub winner_decision_id: String,
    pub rejected_decision_id: String,
    pub resolution: Option<String>,
    pub resolver: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventRecord {
    pub event_id: String,
    pub parent_event_id: Option<String>,
    pub event_type: String,
    pub actor: String,
    pub timestamp: String,
    pub base_event_id: Option<String>,
    pub payload: serde_json::Value,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Command {
    ReviseThickness { curve_id: String, value: f64, sigma: f64, raw_text: String },
    RevisePoint { curve_id: String, index: usize, intensity: f64, sigma: f64 },
    ShiftGrid { curve_id: String, delta_q: f64 },
    SelectBuffer { buffer_id: String },
    SetBackgroundScale { value: f64, sigma: f64, source: String, state: String },
    SetModelWindows { guinier_q_min: f64, guinier_q_max: f64, porod_q_min: f64, porod_q_max: f64 },
    ProposeDecision { decision_id: String, actor: String, candidate: String, note: String },
    ResolveConflict { conflict_id: String, resolution: String, actor: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRequest {
    pub base_event_id: Option<String>,
    pub actor: String,
    pub command: Command,
}

pub fn stable_hash(value: &impl Serialize) -> String {
    let json = serde_json::to_string(value).unwrap_or_default();
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in json.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a-{:016x}", hash)
}

pub fn approx_same_grid(a: &[RawPoint], b: &[RawPoint]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| (x.q - y.q).abs() < Q_TOL)
}

pub fn overlap_range(sample: &[RawPoint], background: &[RawPoint]) -> Option<(f64, f64)> {
    let (smin, smax) = (sample.first()?.q, sample.last()?.q);
    let (bmin, bmax) = (background.first()?.q, background.last()?.q);
    let lo = smin.max(bmin);
    let hi = smin.min(smax).max(lo).min(smax.max(bmax).min(smax));
    let hi = smax.min(bmax);
    if hi - lo <= Q_TOL { None } else { Some((lo, hi)) }
}

pub fn interpolate_at(points: &[RawPoint], q: f64) -> Option<(f64, f64, usize, usize, bool)> {
    if points.len() < 2 || q < points.first()?.q - Q_TOL || q > points.last()?.q + Q_TOL {
        return None;
    }
    let pos = points.partition_point(|p| p.q < q);
    if pos == 0 {
        let p = &points[0];
        return Some((p.i, p.sigma, p.index, p.index, false));
    }
    if pos >= points.len() {
        let p = points.last()?;
        return Some((p.i, p.sigma, p.index, p.index, false));
    }
    let left = &points[pos - 1];
    let right = &points[pos];
    if (left.q - q).abs() < Q_TOL {
        return Some((left.i, left.sigma, left.index, left.index, false));
    }
    let fraction = (q - left.q) / (right.q - left.q);
    let value = left.i + fraction * (right.i - left.i);
    let variance = left.sigma.powi(2) + fraction.powi(2) * (right.sigma.powi(2) - left.sigma.powi(2));
    Some((value, variance.max(0.0).sqrt(), left.index, right.index, true))
}

pub fn linear_fit(xs: &[f64], ys: &[f64], ws: &[f64]) -> Option<(f64, f64, f64)> {
    let n = xs.len() as f64;
    if xs.len() < 2 || xs.len() != ys.len() || xs.len() != ws.len() { return None; }
    let mut sw = 0.0;
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    for ((x, y), w) in xs.iter().zip(ys).zip(ws) {
        let weight = if *w > 0.0 { 1.0 / (w * w) } else { 0.0 };
        sw += weight;
        sx += weight * x;
        sy += weight * y;
        sxx += weight * x * x;
        sxy += weight * x * y;
    }
    let delta = sw * sxx - sx * sx;
    if delta.abs() < f64::EPSILON { return None; }
    let intercept = (sy * sxx - sx * sxy) / delta;
    let slope = (sw * sxy - sx * sy) / delta;
    let mut sse = 0.0;
    for ((x, y), w) in xs.iter().zip(ys).zip(ws) {
        let residual = y - (intercept + slope * x);
        sse += residual * residual / (w * w);
    }
    Some((intercept, slope, (sse / (n - 2.0).max(1.0)).sqrt()))
}
