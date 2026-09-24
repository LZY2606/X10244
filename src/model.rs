//! Domain model. Everything serialises to JSON so it can live in the store
//! with a stable identity; raw representations are kept verbatim.

use serde::{Deserialize, Serialize};

/// A scattering curve as imported (raw representation, original units).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCurve {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub sigma: Vec<f64>,
    pub q_unit: String,
    pub i_unit: String,
    pub source_file: String,
}

/// Canonical curve used by the derived layer (q in 1/nm).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Curve {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub sigma: Vec<f64>,
    pub i_unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormParams {
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub transmission: f64,
    pub exposure_rel_unc: f64,
    pub thickness_rel_unc: f64,
    pub transmission_rel_unc: f64,
}

/// One point of a background-subtracted curve, with full provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubPoint {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub sample_idx: usize,
    pub sample_i: f64,
    pub sample_sigma: f64,
    /// Buffer intensity interpolated onto the sample grid (overlap only).
    pub buffer_i: f64,
    pub buffer_sigma: f64,
    /// Bracketing buffer rows used by the linear interpolation.
    pub buffer_bracket: (usize, usize),
    pub residual: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtraction {
    pub scale: f64,
    pub scale_sigma: f64,
    pub overlap: (f64, f64),
    pub interpolated: bool,
    pub points: Vec<SubPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleCandidate {
    pub scale: f64,
    pub loss: f64,
    pub rank: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub kind: String, // "guinier" | "porod"
    pub qmin: f64,
    pub qmax: f64,
    pub params: serde_json::Value,
    pub quality: f64, // R^2 for guinier, relative flatness for porod
}

/// Machine-readable judgement attached to events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Judgement {
    pub rule: String,
    pub detail: serde_json::Value,
}
