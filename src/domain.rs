// Domain model for a small-angle scattering bench.
//
// Layering rule (project invariant):
//   raw layer   - imported exactly as received, original representation kept,
//                 units/coordinate basis attached, unconvertible values stay
//                 in place with diagnostics rather than being dropped.
//   derived layer - normalized curves, background subtractions, interval fits.
//
// A raw 2D-integrated SAXS record carries q, I(q), sigma(I), exposure, sample
// thickness, transmission, instrument response and mask provenance.

use serde::{Deserialize, Serialize};

pub const ALGO_VERSION: &str = "saxs-bench-1.0";

/// Physical unit / coordinate basis. Stored as text on every quantity so that
/// a value can never be plotted without its basis being recoverable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit {
    pub name: String,
    pub basis: String,
}

impl Unit {
    pub fn new(name: impl Into<String>, basis: impl Into<String>) -> Self {
        Self { name: name.into(), basis: basis.into() }
    }
}

/// A value that may fail unit conversion. It is never silently dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Valued {
    pub value: Option<f64>,
    pub raw: String,
    pub unit: Unit,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointRole {
    Sample,
    Buffer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub i: usize,
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub masked: Option<bool>,
}

/// Immutable raw material. `id` is content derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCurve {
    pub id: String,
    pub label: String,
    pub role: PointRole,
    pub points: Vec<CurvePoint>,
    pub q_unit: Unit,
    pub i_unit: Unit,
    pub exposure_s: Valued,
    pub thickness_mm: Valued,
    pub transmission: Valued,
    pub response_id: String,
    pub mask_id: String,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub id: String,
    pub label: String,
    pub scale: f64,
    pub scale_unit: Unit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskSource {
    pub id: String,
    pub label: String,
    /// indices declared masked in the original pipeline
    pub masked_indices: Vec<usize>,
    pub origin: String,
}

/// One point of a derived curve, with its own stable identity so the review UI
/// can click a derived point and trace back to the two source curves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivedPoint {
    pub id: String,
    pub i: usize,
    pub q: f64,
    pub value: f64,
    pub sigma: f64,
    /// indices into the two source (normalized) curves
    pub source_a: Option<usize>,
    pub source_b: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedCurve {
    pub id: String,
    pub raw_id: String,
    pub points: Vec<DerivedPoint>,
    /// normalization factor actually applied (response * exposure / (t * d))
    pub norm_factor: f64,
    /// relative 1-sigma normalization uncertainty (transmission/exposure/...)
    pub norm_rel_sigma: f64,
    pub q_unit: Unit,
    pub i_unit: Unit,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScaleFitMode {
    /// scale fixed from transmission ratio
    Fixed,
    /// scale fitted by least squares on the overlap window
    Fitted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtractedCurve {
    pub id: String,
    pub sample_norm_id: String,
    pub buffer_norm_id: String,
    pub scale: f64,
    pub scale_sigma: f64,
    pub mode: ScaleFitMode,
    pub overlap_min_q: f64,
    pub overlap_max_q: f64,
    pub points: Vec<DerivedPoint>,
    pub q_unit: Unit,
    pub i_unit: Unit,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntervalKind {
    Guinier,
    Porod,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub kind: IntervalKind,
    pub q_min: f64,
    pub q_max: f64,
    pub point_count: usize,
    /// Guinier: Rg (angstrom) and I(0); Porod: exponent n
    pub param_1: f64,
    pub param_1_sigma: f64,
    pub param_2: f64,
    pub residual_rms: f64,
    pub chi_square: f64,
    pub rank_score: f64,
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelFitSet {
    pub id: String,
    pub subtracted_id: String,
    pub guinier: Vec<IntervalCandidate>,
    pub porod: Vec<IntervalCandidate>,
    pub note: String,
}

/// Why a node is still valid or why it must be recomputed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidityExplanation {
    pub node_id: String,
    pub valid: bool,
    pub reasons: Vec<String>,
}
