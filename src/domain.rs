//! Domain entities for the scattering curve bench.
//!
//! Every quantity that has a physical meaning carries its original
//! representation (unit / coordinate basis). Normalization lives on derived
//! layers only; values that cannot be converted stay at their origin with a
//! diagnostic instead of being silently dropped.

use serde::{Deserialize, Serialize};

/// A scalar kept in its original representation.
/// `value_si`/canonical conversion is performed on derived layers and never
/// mutates the source record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scalar {
    pub value: f64,
    /// Original unit token as imported, e.g. "mm", "A^-1", "nm^-1".
    pub unit: String,
}

/// A 1-D vector (q axis, intensity, sigma) kept in its original basis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vector {
    pub values: Vec<f64>,
    pub unit: String,
}

/// Provenance for a mask: which pixels / q bins are excluded and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskInfo {
    pub source: String,
    /// Masked *raw* bin indices (into the imported vectors).
    pub excluded_bins: Vec<usize>,
    pub reason: String,
}

/// Detector / beam response information as imported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub source: String,
    /// Optional per-bin relative response factors (length 0 => flat response).
    pub factors: Vec<f64>,
    /// Free-form basis note ("flat-field corrected", "counts/photon", ...).
    pub basis: String,
}

/// One imported curve together with every piece of source information the
/// protocol requires us to retain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCurve {
    pub id: String,
    pub role: String, // "sample" | "buffer"
    pub label: String,
    /// Buffer version tag for a buffer curve, e.g. "b1", "b2".
    pub version: String,
    pub q: Vector,
    pub intensity: Vector,
    pub sigma: Vector,
    pub exposure_s: Scalar,
    pub thickness_mm: Scalar,
    pub transmission: Scalar,
    pub mask: MaskInfo,
    pub response: InstrumentResponse,
}

/// A source fact that cannot be converted / applied. It stays attached to its
/// origin node; nothing is dropped for plotting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
}

/// Per-point provenance pointer attached to every displayed derived value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointOrigin {
    /// Index on the derived (possibly interpolated) grid.
    pub derived_bin: usize,
    pub sample_node: String,
    /// Raw bins bracketing the derived bin in the sample grid.
    pub sample_bins: [usize; 2],
    pub buffer_node: String,
    /// Raw bins bracketing the derived bin in the buffer grid.
    pub buffer_bins: [usize; 2],
    pub interp_weights: [f64; 2],
}

/// Result of normalizing one raw curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedCurve {
    pub node_id: String,
    pub raw_id: String,
    pub role: String,
    pub label: String,
    pub version: String,
    /// Canonical q axis in 1/Å.
    pub q_inv_a: Vec<f64>,
    /// Normalized intensity (counts / (t * transmission * thickness)).
    pub i_norm: Vec<f64>,
    pub sigma_norm: Vec<f64>,
    /// Raw bins that survived the mask, aligned with the vectors above.
    pub source_bins: Vec<usize>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Uncertainty budget for one subtracted point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UncertaintyBudget {
    /// Normalization uncertainty of the sample branch (variance, 1/Å axis).
    pub sample_norm: f64,
    pub buffer_norm: f64,
    /// Variance contributed by the fitted background scale uncertainty.
    pub background_scale: f64,
    /// Total variance, propagated.
    pub total: f64,
}

/// A subtraction status. When grids do not overlap the subtraction is *not*
/// performed (`Rejected`); points are only produced inside the overlap range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SubtractionStatus {
    /// Full overlap (one grid fully covers the other).
    Full,
    /// Only a partial overlap exists; subtraction limited to overlap.
    Partial {
        q_min: f64,
        q_max: f64,
        dropped_low: usize,
        dropped_high: usize,
    },
    /// No overlap / non-comparable grids: no direct subtraction allowed.
    Rejected { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subtraction {
    pub node_id: String,
    pub sample_node: String,
    pub buffer_node: String,
    pub buffer_version: String,
    pub status: SubtractionStatus,
    pub q_inv_a: Vec<f64>,
    pub i_sub: Vec<f64>,
    pub sigma_sub: Vec<f64>,
    pub scale: f64,
    pub scale_sigma: f64,
    pub budgets: Vec<UncertaintyBudget>,
    pub origins: Vec<PointOrigin>,
    /// Bins (on the subtraction grid) whose intensity is <= 0.
    pub negative_bins: Vec<usize>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IntervalKind {
    Guinier,
    Porod,
}

/// A single candidate fit on one interval. Guinier and Porod candidates are
/// always kept separate: they are never auto-stitched into one model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCandidate {
    pub candidate_id: String,
    pub kind: IntervalKind,
    pub q_min: f64,
    pub q_max: f64,
    pub bins: Vec<usize>,
    /// Guinier: Rg (Å), I0; Porod: intercept, slope.
    pub param_a: f64,
    pub param_b: f64,
    pub param_a_sigma: f64,
    pub param_b_sigma: f64,
    pub chi_squared: f64,
    pub residual_runs: i64,
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelEvidence {
    pub node_id: String,
    pub subtraction_node: String,
    pub guinier: Vec<ModelCandidate>,
    pub porod: Vec<ModelCandidate>,
    /// True when at least one Guinier and one Porod interval overlap on q.
    pub interval_overlap: bool,
    pub diagnostics: Vec<Diagnostic>,
}

/// Everything addressable in the graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Raw(RawCurve),
    Normalized(NormalizedCurve),
    Subtraction(Subtraction),
    Model(ModelEvidence),
}

impl Node {
    pub fn id(&self) -> &str {
        match self {
            Node::Raw(r) => &r.id,
            Node::Normalized(n) => &n.node_id,
            Node::Subtraction(s) => &s.node_id,
            Node::Model(m) => &m.node_id,
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Node::Raw(_) => "raw",
            Node::Normalized(_) => "normalized",
            Node::Subtraction(_) => "subtraction",
            Node::Model(_) => "model",
        }
    }
}

/// Deterministic 64-bit FNV-1a hash, used to derive stable content identities.
/// Stable across processes/platforms (independent of `RandomState`).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_b325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Short, content-derived identifier with a type prefix.
pub fn content_id(prefix: &str, material: &str) -> String {
    let h = fnv1a64(material.as_bytes());
    format!("{}_{:016x}", prefix, h)
}

/// Convert a q unit token to the canonical 1/Å basis.
/// Returns None when the unit cannot be converted (the value then stays in
/// place with a diagnostic).
pub fn q_unit_to_inv_a(unit: &str) -> Option<f64> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "a^-1" | "å^-1" | "angstrom^-1" | "1/a" | "1/angstrom" => Some(1.0),
        "nm^-1" | "1/nm" => Some(0.1), // 1 nm^-1 = 0.1 Å^-1
        _ => None,
    }
}
