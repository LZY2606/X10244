use serde::{Deserialize, Serialize};

/// A physical quantity always carries a unit (or coordinate basis) and an
/// optional uncertainty. Raw representations are preserved at import time;
/// only the derived layer normalizes units.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Qty {
    pub value: f64,
    pub unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unc: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
}

/// A curve as imported: raw text kept verbatim, units as declared.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawCurve {
    pub label: String,
    pub q_unit: String,
    pub i_unit: String,
    pub points: Vec<CurvePoint>,
    /// Original file content, preserved untouched.
    pub raw: String,
}

/// A curve in the derived layer: q normalized to 1/Angstrom.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormCurve {
    pub label: String,
    pub q_unit: String,
    pub i_unit: String,
    pub points: Vec<CurvePoint>,
    pub norm_factor: f64,
    pub diagnostics: Vec<Diagnostic>,
}

/// Machine-readable judgement attached to derivations and emitted as events.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<f64>,
}

/// Per-point provenance of a subtraction: each derived point can be traced
/// back to the two source curves.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubPoint {
    pub q: f64,
    pub i_sub: f64,
    pub sigma_sub: f64,
    pub status: String,
    pub sample: CurvePoint,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buffer: Option<BufferTrace>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BufferTrace {
    pub q: f64,
    pub i_interp: f64,
    pub sigma_interp: f64,
    pub method: String,
    pub src_indices: Vec<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subtraction {
    pub scale: f64,
    pub scale_unc: f64,
    pub overlap: Option<(f64, f64)>,
    pub status: String,
    pub points: Vec<SubPoint>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub kind: String,
    pub q_min: f64,
    pub q_max: f64,
    pub params: serde_json::Value,
    pub residual: f64,
    pub chosen: bool,
    pub diagnostics: Vec<Diagnostic>,
}
