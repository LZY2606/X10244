//! Domain types for SAXS normalization / background subtraction.
//!
//! Every domain quantity carries a unit or a coordinate basis. Raw imported
//! representations are preserved verbatim; normalization happens only in the
//! derived layer. Values that cannot be converted stay in place with a
//! diagnostic attached.

use serde::{Deserialize, Serialize};

/// Canonical units used by the derived layer.
pub const CANONICAL_Q_UNIT: &str = "1/angstrom";
pub const CANONICAL_I_UNIT: &str = "1/cm";

/// A raw quantity exactly as imported, plus its parsed/normalized form.
/// If parsing or unit conversion fails, `parsed` is `None` and `diagnostic`
/// explains why; the raw text is never discarded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawQty {
    pub raw: String,
    pub parsed: Option<Qty>,
    pub diagnostic: Option<String>,
}

/// A parsed scalar quantity with unit and optional absolute uncertainty.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Qty {
    pub value: f64,
    pub unit: String,
    pub uncertainty: Option<f64>,
}

impl RawQty {
    pub fn parse(raw: &str) -> RawQty {
        let trimmed = raw.trim();
        // Accepted forms: "0.42 +/- 0.01 cm", "0.42 ± 0.01 cm", "0.42 cm", "0.42"
        let mut value_part = trimmed;
        let mut unc: Option<f64> = None;
        for sep in ["+/-", "±"] {
            if let Some(pos) = trimmed.find(sep) {
                value_part = &trimmed[..pos];
                let rest = trimmed[pos + sep.len()..].trim();
                let unc_str: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == 'e' || *c == 'E' || *c == '-' || *c == '+')
                    .collect();
                match unc_str.parse::<f64>() {
                    Ok(u) => unc = Some(u.abs()),
                    Err(_) => {
                        return RawQty {
                            raw: raw.to_string(),
                            parsed: None,
                            diagnostic: Some(format!("unparseable uncertainty in '{raw}'")),
                        }
                    }
                }
                break;
            }
        }
        let mut tokens = value_part.trim().split_whitespace();
        let num = tokens.next().unwrap_or("");
        let unit = tokens.next().unwrap_or("").to_string();
        match num.parse::<f64>() {
            Ok(v) => RawQty {
                raw: raw.to_string(),
                parsed: Some(Qty { value: v, unit, uncertainty: unc }),
                diagnostic: None,
            },
            Err(_) => RawQty {
                raw: raw.to_string(),
                parsed: None,
                diagnostic: Some(format!("unparseable numeric value in '{raw}'")),
            },
        }
    }
}

/// Convert a length-unit quantity to centimetres. Unknown units stay
/// unconverted and produce a diagnostic (handled by the caller).
pub fn to_cm(q: &Qty) -> Option<f64> {
    match q.unit.as_str() {
        "cm" => Some(q.value),
        "mm" => Some(q.value / 10.0),
        "m" => Some(q.value * 100.0),
        _ => None,
    }
}

/// Convert a time quantity to seconds.
pub fn to_seconds(q: &Qty) -> Option<f64> {
    match q.unit.as_str() {
        "s" | "sec" | "" => Some(q.value),
        "ms" => Some(q.value / 1000.0),
        "min" => Some(q.value * 60.0),
        _ => None,
    }
}

/// Convert a q coordinate to the canonical 1/angstrom basis.
pub fn q_to_canonical(v: f64, unit: &str) -> Option<f64> {
    match unit {
        "1/angstrom" | "1/A" | "A^-1" => Some(v),
        "1/nm" | "nm^-1" => Some(v / 10.0),
        _ => None,
    }
}

/// A scattering curve on an explicit q basis. Points keep their import order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub sigma: Vec<f64>,
    pub q_unit: String,
    pub i_unit: String,
}

impl Curve {
    pub fn len(&self) -> usize {
        self.q.len()
    }
    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }
    pub fn q_range(&self) -> Option<(f64, f64)> {
        if self.q.is_empty() {
            return None;
        }
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for &v in &self.q {
            lo = lo.min(v);
            hi = hi.max(v);
        }
        Some((lo, hi))
    }
}

/// Raw imported sample record: everything from the 2D-integration export is
/// preserved, including instrument response and mask provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SampleRecord {
    pub id: String,
    pub name: String,
    pub curve: Curve,
    pub exposure: RawQty,
    pub thickness: RawQty,
    pub transmission: RawQty,
    pub instrument_response: InstrumentResponse,
    pub mask_source: MaskSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstrumentResponse {
    pub id: String,
    pub version: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MaskSource {
    pub id: String,
    pub sha: String,
    pub note: String,
}

/// A buffer (solvent) record. Multiple candidates/versions may coexist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BufferRecord {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub curve: Curve,
    pub exposure: RawQty,
    pub thickness: RawQty,
    pub transmission: RawQty,
}

/// A diagnostic attached to a derived artifact; never silently dropped.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub point_index: Option<usize>,
}

/// Normalization factors actually applied, with relative uncertainty.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormFactors {
    pub exposure_s: f64,
    pub thickness_cm: f64,
    pub transmission: f64,
    pub transmission_rel_unc: f64,
    pub factor: f64,
    pub factor_rel_unc: f64,
}

/// How two q grids were reconciled. Machine-readable judgment basis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum GridDecision {
    /// Grids identical: direct point-by-point combination is allowed.
    ExactMatch,
    /// Grids differ: buffer interpolated onto sample grid, strictly inside
    /// the overlap range. Points outside the overlap are dropped.
    InterpolatedOverlap { overlap: (f64, f64), kept: usize, dropped: usize },
    /// No overlap at all: subtraction is refused.
    NoOverlap,
}

/// Result of a background subtraction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractionResult {
    pub curve: Curve,
    pub grid_decision: GridDecision,
    pub scale: f64,
    pub scale_rel_unc: f64,
    pub negative_points: Vec<usize>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Kind of model interval candidate. Guinier and Porod candidates are never
/// auto-merged into a single model, even when their intervals overlap.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IntervalKind {
    Guinier,
    Porod,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntervalCandidate {
    pub kind: IntervalKind,
    pub q_min: f64,
    pub q_max: f64,
    pub params: Vec<(String, f64)>,
    pub residual_chi2: f64,
    pub overlaps_other: bool,
    pub status: CandidateStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CandidateStatus {
    Open,
    Accepted,
    Rejected,
}

/// Residual comparison across multiple buffer candidates. All legal
/// candidates are kept; ambiguity is reported, never collapsed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResidualComparison {
    pub entries: Vec<ResidualEntry>,
    pub ambiguous: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResidualEntry {
    pub buffer_id: String,
    pub buffer_version: u32,
    pub chi2: f64,
    pub chosen: bool,
}
