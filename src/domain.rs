use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    pub raw: String,
    pub value: Option<f64>,
    pub unit: String,
    pub uncertainty: f64,
    #[serde(default)]
    pub conversion_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub point_id: String,
    pub q_raw: String,
    pub i_raw: String,
    pub sigma_raw: String,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawCurve {
    pub logical_curve_id: String,
    pub current_version: String,
    pub source_file: String,
    pub label: Option<String>,
    #[serde(default)]
    pub candidate_id: Option<String>,
    pub exposure: Quantity,
    pub thickness: Quantity,
    pub transmission: Quantity,
    pub instrument_response: Quantity,
    pub mask_source: BTreeMap<String, String>,
    pub points: Vec<RawPoint>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaseDefinition {
    pub case_id: String,
    pub coordinate_frame: BTreeMap<String, AxisBasis>,
    pub sample: RawCurve,
    pub buffers: Vec<RawCurve>,
    pub accepted_buffer_candidate: String,
    pub scale_candidates: Vec<ScaleCandidateDefinition>,
    pub interval_windows: Vec<IntervalWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AxisBasis {
    pub axis: String,
    pub unit: String,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub grid_origin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScaleCandidateDefinition {
    pub id: String,
    pub method: String,
    pub external_sigma: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct IntervalWindow {
    pub id: String,
    pub kind: IntervalKind,
    pub q_min: f64,
    pub q_max: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntervalKind {
    Guinier,
    Porod,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedPoint {
    pub stable_id: String,
    pub source_point_id: String,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub raw_curve_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedCurve {
    pub node_id: String,
    pub raw_curve_version: String,
    pub normalization: f64,
    pub normalization_relative_sigma: f64,
    pub points: Vec<NormalizedPoint>,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractedPoint {
    pub stable_id: String,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub sample_point_id: String,
    pub buffer_low_id: Option<String>,
    pub buffer_high_id: Option<String>,
    pub interpolated: bool,
    pub negative: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScaleFit {
    pub candidate_id: String,
    pub method: String,
    pub scale: f64,
    pub scale_sigma: f64,
    pub weighted_rss: f64,
    pub reduced_chi_square: f64,
    pub accepted: bool,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractionResult {
    pub node_id: String,
    pub sample_version: String,
    pub buffer_version: String,
    pub buffer_candidate_id: String,
    pub scale_candidate_id: String,
    pub q_overlap_min: Option<f64>,
    pub q_overlap_max: Option<f64>,
    pub status: DerivationStatus,
    pub scale: Option<ScaleFit>,
    pub points: Vec<SubtractedPoint>,
    pub residuals: Vec<ResidualPoint>,
    pub diagnostics: Vec<String>,
    pub valid: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResidualPoint {
    pub q: f64,
    pub observed: f64,
    pub predicted_background: f64,
    pub residual: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DerivationStatus {
    Ready,
    BlockedNoOverlap,
    InvalidInputs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntervalCandidate {
    pub stable_id: String,
    pub window_id: String,
    pub kind: IntervalKind,
    pub q_min: f64,
    pub q_max: f64,
    pub slope: f64,
    pub intercept: f64,
    pub r_square: f64,
    pub rms_residual: f64,
    pub point_count: usize,
    pub includes_negative: bool,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelEvidence {
    pub node_id: String,
    pub subtraction_node_id: String,
    pub guinier: Vec<IntervalCandidate>,
    pub porod: Vec<IntervalCandidate>,
    pub stitched_model: Option<()>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DerivedGraph {
    pub nodes: BTreeMap<String, GraphNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    pub inputs: Vec<String>,
    pub status: String,
    pub reason: String,
    pub depends_on_event: i64,
}

pub fn parse_curve_csv(input: &str) -> Result<Vec<RawPoint>, String> {
    let mut lines = input.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next().ok_or_else(|| "empty curve CSV".to_string())?;
    if header.split(',').count() < 4 {
        return Err("curve CSV must contain point_id,q,I,sigma".to_string());
    }
    lines
        .enumerate()
        .map(|(line_no, line)| {
            let cols: Vec<_> = line.split(',').map(str::trim).collect();
            if cols.len() != 4 {
                return Err(format!("line {} has {} columns, expected 4", line_no + 2, cols.len()));
            }
            let q: f64 = parse_numeric(cols[1]).ok_or_else(|| format!("invalid q on line {}", line_no + 2))?;
            let i: f64 = parse_numeric(cols[2]).ok_or_else(|| format!("invalid I on line {}", line_no + 2))?;
            let sigma: f64 = parse_numeric(cols[3]).ok_or_else(|| format!("invalid sigma on line {}", line_no + 2))?;
            Ok(RawPoint {
                point_id: cols[0].to_string(),
                q_raw: cols[1].to_string(),
                i_raw: cols[2].to_string(),
                sigma_raw: cols[3].to_string(),
                q,
                i,
                sigma,
            })
        })
        .collect()
}

pub fn parse_numeric(raw: &str) -> Option<f64> {
    raw.split_whitespace().next().and_then(|value| value.parse::<f64>().ok())
}

pub fn validate_curve(curve: &mut RawCurve) {
    curve.diagnostics.clear();
    for quantity in [&curve.exposure, &curve.thickness, &curve.transmission, &curve.instrument_response] {
        if quantity.value.is_none_or(|value| value <= 0.0) {
            curve.diagnostics.push(format!("invalid positive quantity: {}", quantity.raw));
        }
    }
    if curve.points.is_empty() {
        curve.diagnostics.push("curve has no points".to_string());
    }
    if curve.points.windows(2).any(|w| w[1].q <= w[0].q) {
        curve.diagnostics.push("q grid is not strictly increasing".to_string());
    }
}
