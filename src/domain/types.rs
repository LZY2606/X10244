//! Core domain records. Every displayed value carries a stable identity;
//! material rows keep the original textual representation and source files.

use crate::domain::units::Quantity;
use serde::{Deserialize, Serialize};

pub const RULE_VERSION: &str = "saxs-rules-2026.09.01";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Sample,
    Buffer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub point_id: String,
    pub q: Quantity,
    pub i: Quantity,
    pub sigma: Quantity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Material {
    /// Content-addressed object id (sha256 over the canonical JSON).
    pub id: String,
    pub logical_id: String,
    pub version: String,
    pub kind: MaterialKind,
    pub points: Vec<RawPoint>,
    pub exposure: Quantity,
    pub thickness: Quantity,
    pub transmission: Quantity,
    /// Relative buffer scaling uncertainty (1 sigma), if declared.
    pub scale_uncertainty: Option<f64>,
    pub instrument_source: String,
    pub mask_source: String,
    pub source_files: Vec<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedPoint {
    pub point_id: String,
    pub source_point_id: String,
    pub q: f64,
    pub q_unit: String,
    pub i: f64,
    pub i_unit: String,
    pub sigma: f64,
    /// Fractional 1-sigma normalization uncertainty at this point.
    pub norm_uncertainty: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String, // "info" | "warning" | "error"
    pub message: String,
    pub point_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedResult {
    pub id: String,
    pub material_id: String,
    pub rule_version: String,
    /// c = exposure * thickness * transmission (all positive) when available.
    pub norm_factor: Option<f64>,
    pub points: Vec<NormalizedPoint>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PointMethod {
    Direct,
    Interpolated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractedPoint {
    pub point_id: String,
    pub sample_point_id: String,
    /// Exact buffer point for direct subtraction; None when interpolated.
    pub buffer_point_id: Option<String>,
    pub method: PointMethod,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub sigma_normalization: f64,
    pub sigma_scale: f64,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GridMode {
    Identical,
    OverlapInterpolated,
    NoOverlap,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractionResult {
    pub id: String,
    pub sample_norm_id: String,
    pub buffer_norm_id: String,
    pub rule_version: String,
    pub grid_mode: GridMode,
    /// Overlap interval [q_lo, q_hi] inside which subtraction is defined.
    pub overlap_lo: Option<f64>,
    pub overlap_hi: Option<f64>,
    /// Fitted background multiplier I_sample - alpha I_buffer.
    pub alpha: Option<f64>,
    pub alpha_sigma: Option<f64>,
    pub points: Vec<SubtractedPoint>,
    pub chi_squared: Option<f64>,
    pub n_negative: i64,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Guinier,
    Porod,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelCandidate {
    pub candidate_id: String,
    pub kind: ModelKind,
    pub q_lo: f64,
    pub q_hi: f64,
    pub point_ids: Vec<String>,
    pub n_points: usize,
    /// Guinier: I0, Rg (A); Porod: plateau, K.
    pub intercept: f64,
    pub slope: f64,
    pub i0: Option<f64>,
    pub rg: Option<f64>,
    pub plateau: Option<f64>,
    pub k_porod: Option<f64>,
    pub intercept_sigma: f64,
    pub slope_sigma: f64,
    pub chi_squared: f64,
    pub qr_min: Option<f64>,
    pub qr_max: Option<f64>,
    pub residual_rms: f64,
    pub accepted: bool,
    pub rejection: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelFitResult {
    pub id: String,
    pub subtraction_id: String,
    pub rule_version: String,
    /// Always false: Guinier and Porod are never glued into one model.
    pub merged: bool,
    pub guinier: Vec<ModelCandidate>,
    pub porod: Vec<ModelCandidate>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Machine-readable rule basis attached to derivations.
pub fn rule_basis() -> Vec<&'static str> {
    vec![
        "different_q_grids_are_never_subtracted_directly",
        "interpolation_only_inside_overlap_range",
        "error_budget_includes_normalization_and_background_scale_uncertainty",
        "guinier_and_porod_intervals_are_never_automatically_merged",
        "all_legal_candidates_retained_when_not_uniquely_determined",
    ]
}
