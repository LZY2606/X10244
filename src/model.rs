//! Domain model for the SAXS bench. Every domain quantity carries a unit or a
//! coordinate basis; raw representations are preserved at import and only the
//! derived layer normalizes.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Unit {
    InvNm,
    InvAngstrom,
    PerCm,
    Arbitrary,
    Second,
    Millimetre,
    Dimensionless,
    Unknown(String),
}

impl Unit {
    pub fn from_label(s: &str) -> Unit {
        match s.trim() {
            "nm^-1" | "1/nm" | "nm-1" => Unit::InvNm,
            "A^-1" | "1/A" | "angstrom^-1" | "Ang^-1" => Unit::InvAngstrom,
            "cm^-1" | "1/cm" => Unit::PerCm,
            "s" | "sec" => Unit::Second,
            "mm" => Unit::Millimetre,
            "arb" | "arb." | "counts" => Unit::Arbitrary,
            "" | "1" => Unit::Dimensionless,
            other => Unit::Unknown(other.to_string()),
        }
    }
    /// Multiplicative factor converting a q value in this unit to canonical nm^-1.
    /// None means the basis cannot be converted (value must stay where it is).
    pub fn q_to_canonical(&self) -> Option<f64> {
        match self {
            Unit::InvNm => Some(1.0),
            Unit::InvAngstrom => Some(10.0),
            _ => None,
        }
    }
    pub fn label(&self) -> String {
        match self {
            Unit::InvNm => "nm^-1".into(),
            Unit::InvAngstrom => "A^-1".into(),
            Unit::PerCm => "cm^-1".into(),
            Unit::Arbitrary => "arb".into(),
            Unit::Second => "s".into(),
            Unit::Millimetre => "mm".into(),
            Unit::Dimensionless => "1".into(),
            Unit::Unknown(s) => format!("unknown({s})"),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub q: f64,
    pub i: f64,
    pub err: f64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CurveRole {
    Sample,
    Buffer,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawCurve {
    pub id: String,
    pub role: CurveRole,
    pub label: String,
    pub q_unit: Unit,
    pub i_unit: Unit,
    /// Values exactly as recorded, in the recorded units.
    pub points: Vec<CurvePoint>,
    /// Original file text, preserved verbatim at import.
    pub raw_text: String,
    pub origin: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub id: String,
    pub description: String,
    pub response_file: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaskSource {
    pub id: String,
    pub generator: String,
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BufferRef {
    pub id: String,
    pub version: u32,
    pub file: String,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExperimentMeta {
    pub experiment: String,
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub transmission: f64,
    pub transmission_err: f64,
    pub instrument_response: InstrumentResponse,
    pub mask_source: MaskSource,
    pub buffer_candidates: Vec<BufferRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub affected: Vec<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormalizedCurve {
    pub source_id: String,
    pub source_version: i64,
    /// Canonical q grid (nm^-1) when convertible; otherwise the raw basis.
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    /// Includes counting error propagated through normalization plus the
    /// normalization (transmission) uncertainty.
    pub err: Vec<f64>,
    pub q_unit: Unit,
    pub norm_factor: f64,
    pub norm_rel_err: f64,
    /// False when the q basis could not be converted: the curve stays in its
    /// raw position and is excluded from subtraction.
    pub canonical: bool,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubtractedCurve {
    pub sample_id: String,
    pub sample_version: i64,
    pub buffer_id: String,
    pub buffer_version: i64,
    pub scale: f64,
    pub scale_err: f64,
    /// Only the overlap range is populated; nothing is subtracted outside it.
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub err: Vec<f64>,
    pub overlap: (f64, f64),
    /// Sample q points outside the overlap, kept for review (never subtracted).
    pub dropped_sample_q: Vec<f64>,
    /// Indices of negative intensities: flagged, never clipped.
    pub negative_indices: Vec<usize>,
    pub grids_matched: bool,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SubtractFailure {
    NoOverlap { sample_range: (f64, f64), buffer_range: (f64, f64) },
    NotCanonical { which: String, reason: String },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum IntervalKind {
    Guinier,
    Porod,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CandidateStatus {
    Pending,
    Accepted,
    /// Rejected / not selected, but retained for audit.
    Retained,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub curve_id: String,
    pub curve_version: i64,
    pub kind: IntervalKind,
    pub q_min: f64,
    pub q_max: f64,
    /// (name, value, err) e.g. ("Rg", ..), ("I0", ..) or ("exponent", ..)
    pub params: Vec<(String, f64, f64)>,
    pub rms_residual: f64,
    pub n_points: usize,
    pub status: CandidateStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResidualCandidate {
    pub id: String,
    pub scale: f64,
    pub scale_err: f64,
    pub subtracted_node: String,
    pub subtracted_version: i64,
    pub rms: f64,
    pub status: CandidateStatus,
}

/// A machine-readable judgment emitted whenever a rule fires.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Judgment {
    pub rule: String,
    pub judgment: String,
    pub detail: String,
    pub affected: Vec<String>,
}

pub fn judgment(rule: &str, judgment: &str, detail: &str, affected: Vec<String>) -> Judgment {
    Judgment {
        rule: rule.into(),
        judgment: judgment.into(),
        detail: detail.into(),
        affected,
    }
}
