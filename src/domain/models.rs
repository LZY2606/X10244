//! Domain payloads: every layer keeps provenance and diagnostics.

use crate::domain::ids::{node_id, point_id, NodeId, PointId, VersionId};
use crate::domain::quantity::RawValue;
use serde::{Deserialize, Serialize};

pub type Json = serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CurveRole {
    Sample,
    Buffer,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawPoint {
    /// Zero-based row in the imported file; order is identity-bearing.
    pub row: usize,
    pub q_text: String,
    pub i_text: String,
    pub sigma_text: String,
    pub q: RawValue,
    pub i: RawValue,
    pub sigma: RawValue,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaskSource {
    pub mask_id: String,
    pub origin: String,
    /// Rows masked at integration/import time, stored as the original rows.
    pub masked_rows: Vec<usize>,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub response_id: String,
    /// Scaling constant C used to convert rate-like intensity to absolute units.
    pub c_text: String,
    pub c: RawValue,
    pub unit_target: String,
}

/// A raw imported curve: nothing is dropped for plotting. Unparseable rows are
/// retained in their original slot with diagnostics instead of being removed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawCurve {
    pub stable_key: String,
    pub label: String,
    pub role: CurveRole,
    pub source_file: String,
    pub exposure_s: RawValue,
    pub thickness: RawValue,
    pub transmission: RawValue,
    pub response: InstrumentResponse,
    pub mask: MaskSource,
    pub points: Vec<RawPoint>,
    pub diagnostics: Vec<String>,
}

impl RawCurve {
    pub fn node_id(&self) -> NodeId {
        let kind = match self.role {
            CurveRole::Sample => "raw-sample",
            CurveRole::Buffer => "raw-buffer",
        };
        node_id(kind, &self.stable_key)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormPoint {
    pub id: PointId,
    pub row: usize,
    /// q in canonical 1/nm, None when the raw row could not be converted.
    pub q_nm: Option<f64>,
    /// Normalized intensity, in response target units. None => retained gap.
    pub i_norm: Option<f64>,
    pub sigma_norm: Option<f64>,
    pub masked: bool,
    pub source_point_id: PointId,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormalizedCurve {
    pub source_raw: NodeId,
    pub source_raw_version: VersionId,
    pub q_unit: String,
    pub i_unit: String,
    pub normalization_factor: Option<f64>,
    pub norm_diagnostic: Option<String>,
    pub points: Vec<NormPoint>,
    pub diagnostics: Vec<String>,
}

/// Per-point variance contributions, kept explicit so the error-band on a
/// subtracted curve includes both normalization and background-scale terms.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubPoint {
    pub id: PointId,
    pub row: usize,
    pub q_nm: f64,
    /// I_sample(q) - beta * I_buffer(q_interp). None outside overlap.
    pub i_sub: Option<f64>,
    pub sigma_sub: Option<f64>,
    /// sigma^2 from the two normalized curves.
    pub var_norm: Option<f64>,
    /// sigma^2 contribution from uncertainty in the background scale:
    /// (sigma_beta * I_buffer)^2.
    pub var_scale: Option<f64>,
    pub i_sample: f64,
    pub i_buffer_interp: Option<f64>,
    pub sample_point_id: PointId,
    pub buffer_point_ids: Vec<PointId>,
    pub in_overlap: bool,
    pub negative: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScaleMethod {
    /// beta fixed by the analyst (parameter revision path).
    Fixed,
    /// beta fitted on the post-peak high-q window.
    Fitted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BgScale {
    pub method: ScaleMethod,
    pub beta: Option<f64>,
    pub sigma_beta: Option<f64>,
    pub fit_q_min: f64,
    pub fit_q_max: f64,
    pub fit_points: usize,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubtractedCurve {
    pub sample_norm: NodeId,
    pub sample_norm_version: VersionId,
    pub buffer_norm: NodeId,
    pub buffer_norm_version: VersionId,
    pub scale: BgScale,
    pub overlap_q_min: Option<f64>,
    pub overlap_q_max: Option<f64>,
    pub q_grid_mismatch: bool,
    pub has_overlap: bool,
    pub points: Vec<SubPoint>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelKind {
    Guinier,
    Porod,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelFit {
    pub kind: ModelKind,
    pub candidate_id: String,
    pub q_min: f64,
    pub q_max: f64,
    pub n_points: usize,
    /// Guinier: Rg (nm), I0. Porod: exponent p, log-amplitude.
    pub params: Json,
    pub param_sigma: Json,
    pub chi2: f64,
    pub residual_rms: f64,
    pub residuals: Vec<ModelResidual>,
    pub accepted: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelResidual {
    pub point_id: PointId,
    pub q_nm: f64,
    pub observed: f64,
    pub predicted: f64,
    pub weighted_residual: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelBundle {
    pub source_sub: NodeId,
    pub source_sub_version: VersionId,
    /// Guinier and Porod candidates stay independent; they are never fused
    /// into one automatically assembled model.
    pub guinier: Vec<ModelFit>,
    pub porod: Vec<ModelFit>,
    /// Pairs (guinier_idx, porod_idx) whose q-ranges overlap.
    pub range_overlaps: Vec<(usize, usize)>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeVersion {
    pub id: VersionId,
    pub seq: u32,
    pub node: NodeId,
    pub fingerprint: String,
    pub kind: String,
    pub label: String,
    pub status: VersionStatus,
    pub payload: Json,
    pub depends_on: Vec<(NodeId, VersionId)>,
    pub diagnostics: Vec<String>,
    pub created_event: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionStatus {
    /// Fully computed and eligible to be a current pointer.
    Ready,
    /// Computed but carrying domain warnings (still a valid result).
    ReadyWithWarnings,
    /// Rule blocked the computation (e.g. no overlap region); review required.
    Blocked,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProvenanceEdge {
    pub from_node: NodeId,
    pub from_version: VersionId,
    pub to_node: NodeId,
    pub to_version: VersionId,
    pub role: String,
}

pub fn norm_point_id(node: &NodeId, row: usize, q_nm: Option<f64>) -> PointId {
    let key = match q_nm {
        Some(q) => format!("r{:04}-q{:.6}", row, q),
        None => format!("r{:04}-unconvertible", row),
    };
    point_id(node, row, &key)
}

pub fn sub_point_id(node: &NodeId, row: usize, q_nm: f64) -> PointId {
    point_id(node, row, &format!("r{:04}-q{:.6}", row, q_nm))
}
