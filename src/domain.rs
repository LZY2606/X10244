//! Domain types for the SAXS scattering-curve bench.
//!
//! Every quantity keeps its original textual representation (`raw`) and its
//! unit. The raw layer is never rewritten; unit normalization happens only in
//! the derived layer. Values that cannot be converted stay in place and carry
//! a `UnitUnconvertible` diagnostic instead of being dropped.

use serde::{Deserialize, Serialize};

pub const Q_UNIT_A: &str = "1/A";
pub const Q_UNIT_NM: &str = "1/nm";
pub const I_UNIT_CM: &str = "1/cm";
pub const I_UNIT_AU: &str = "a.u.";

/// Machine-readable judgment codes attached to events and results.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Judgment {
    /// Raw representation (original text, units, order) is preserved.
    RawPreserved,
    /// A unit conversion was applied in the derived layer.
    UnitConverted,
    /// Value cannot be converted; left in place with this diagnostic.
    UnitUnconvertible,
    /// Error budget includes normalization uncertainty (exposure/thickness/transmission).
    NormalizationUncertainty,
    /// q grids differ: curves are NOT subtracted directly.
    GridMisaligned,
    /// Interpolation is applied only inside the overlapping q range.
    InterpOverlapOnly,
    /// Point lies outside the overlap: kept, but not background-subtracted.
    OutsideOverlapNotSubtracted,
    /// No q overlap at all: subtraction refused.
    NoQOverlap,
    /// Error budget includes background scale-factor uncertainty.
    BackgroundScaleUncertainty,
    /// Negative intensity after subtraction: value kept and flagged.
    NegativeIntensity,
    /// Candidate intervals overlap each other.
    IntervalOverlap,
    /// Guinier and Porod intervals are never auto-merged into one model.
    GuinierPorodNotMerged,
    /// Buffer version switched; old version retained.
    BufferVersionSwitch,
    /// More than one legal candidate is retained.
    MultiCandidateKept,
    /// Residual test cannot decide uniquely.
    AmbiguousResiduals,
    /// A rejected / unchosen candidate is retained, not deleted.
    CandidateRejectedKept,
    /// Two users edited from the same base version.
    ConflictDetected,
    /// Conflict resolved; losing branch retained.
    ConflictResolved,
    /// Dependency-graph propagation marked this node for recompute.
    StalePropagation,
    /// Only the affected subgraph was recomputed.
    PartialRecompute,
    /// Node is unaffected by a change and remains valid.
    StillValid,
    /// Human adjudication present; recompute requires review.
    AdjudicationNeedsReview,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: Judgment,
    pub message: String,
    #[serde(default)]
    pub subjects: Vec<String>,
    #[serde(default)]
    pub point_indices: Vec<usize>,
}

impl Diagnostic {
    pub fn new(code: Judgment, message: impl Into<String>) -> Self {
        Diagnostic { code, message: message.into(), subjects: vec![], point_indices: vec![] }
    }
}

/// A physical quantity with unit and original textual representation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
    pub raw: String,
}

impl Quantity {
    pub fn new(value: f64, unit: &str) -> Self {
        Quantity { value, unit: unit.to_string(), raw: format!("{} {}", value, unit) }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    #[serde(default)]
    pub flags: Vec<Judgment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    pub q_unit: String,
    pub i_unit: String,
    pub points: Vec<CurvePoint>,
}

impl Curve {
    pub fn q_range(&self) -> Option<(f64, f64)> {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for p in &self.points {
            lo = lo.min(p.q);
            hi = hi.max(p.q);
        }
        if lo.is_finite() { Some((lo, hi)) } else { None }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IntervalWindow {
    pub qmin: f64,
    pub qmax: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FitCandidate {
    pub id: String,
    pub method: String, // "guinier" | "porod"
    pub window: IntervalWindow,
    /// Fit parameters, e.g. {"rg": 14.9, "rg_sigma": 0.2, "i0": ...} or {"slope": -4.01, ...}
    pub params: serde_json::Value,
    pub chi2_red: f64,
    /// Weighted residuals, one per in-window point.
    pub residuals: Vec<f64>,
    /// Indices into the source curve points.
    pub point_indices: Vec<usize>,
    /// "best" | "kept" | "rejected_by_user" | "accepted_by_user"
    pub status: String,
}

/// Convert a q value between supported units. Returns None when unsupported.
pub fn convert_q_value(v: f64, from: &str, to: &str) -> Option<f64> {
    if from == to {
        return Some(v);
    }
    match (from, to) {
        (Q_UNIT_A, Q_UNIT_NM) => Some(v * 10.0),
        (Q_UNIT_NM, Q_UNIT_A) => Some(v / 10.0),
        _ => None,
    }
}

/// Convert a whole curve to the canonical q unit (1/A). Unconvertible curves
/// are returned unchanged with a diagnostic; nothing is dropped.
pub fn curve_to_canonical_q(c: &Curve) -> (Curve, Vec<Diagnostic>) {
    if c.q_unit == Q_UNIT_A {
        return (c.clone(), vec![]);
    }
    let mut out = c.clone();
    let mut ok = true;
    for p in out.points.iter_mut() {
        match convert_q_value(p.q, &c.q_unit, Q_UNIT_A) {
            Some(v) => p.q = v,
            None => {
                ok = false;
                p.flags.push(Judgment::UnitUnconvertible);
            }
        }
    }
    if ok {
        out.q_unit = Q_UNIT_A.to_string();
        (
            out,
            vec![Diagnostic::new(
                Judgment::UnitConverted,
                format!("q 单位由 {} 规范化为 1/A（仅在派生层进行，原始表示保留）", c.q_unit),
            )],
        )
    } else {
        (
            out,
            vec![Diagnostic::new(
                Judgment::UnitUnconvertible,
                format!("q 单位 {} 无法转换，值留在原位置并附诊断", c.q_unit),
            )],
        )
    }
}

/// Exposure time to seconds.
pub fn convert_time_s(v: f64, unit: &str) -> Option<f64> {
    match unit {
        "s" => Some(v),
        "ms" => Some(v / 1000.0),
        "min" => Some(v * 60.0),
        _ => None,
    }
}

/// Sample thickness to millimetres.
pub fn convert_length_mm(v: f64, unit: &str) -> Option<f64> {
    match unit {
        "mm" => Some(v),
        "cm" => Some(v * 10.0),
        "m" => Some(v * 1000.0),
        _ => None,
    }
}

/// Transmission to a dimensionless fraction.
pub fn convert_fraction(v: f64, unit: &str) -> Option<f64> {
    match unit {
        "1" | "" => Some(v),
        "%" => Some(v / 100.0),
        _ => None,
    }
}
