//! Domain model: provenance-preserving SAXS records.
//!
//! Every physical quantity keeps its original textual representation and unit.
//! Normalization to canonical units happens only in the derived layer; values
//! that cannot be converted stay in place with a diagnostic instead of being
//! dropped to make a plot work.

use serde::{Deserialize, Serialize};

pub const RULES_VERSION: &str = "saxs-rules/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Sample,
    Buffer,
    Calibrant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuantityKind {
    Q,
    Intensity,
    IntensityError,
    Exposure,
    Thickness,
    Transmission,
}

impl QuantityKind {
    pub fn canonical_unit(self) -> &'static str {
        match self {
            QuantityKind::Q => "1/A",
            QuantityKind::Intensity | QuantityKind::IntensityError => "count/s",
            QuantityKind::Exposure => "s",
            QuantityKind::Thickness => "cm",
            QuantityKind::Transmission => "fraction",
        }
    }

    /// Multiplicative factor converting `unit` into the canonical unit.
    /// `None` means the unit is unknown and the value must stay unconverted.
    pub fn factor_to_canonical(self, unit: &str) -> Option<f64> {
        let u = unit.trim().to_ascii_lowercase();
        let u = u.trim_end_matches('.');
        let factor = |pairs: &[(&str, f64)]| -> Option<f64> {
            pairs.iter().find(|(name, _)| *name == u).map(|(_, f)| *f)
        };
        match self {
            QuantityKind::Q => factor(&[
                ("1/a", 1.0),
                ("a^-1", 1.0),
                ("angstrom^-1", 1.0),
                ("å^-1", 1.0),
                ("1/nm", 0.1),
                ("nm^-1", 0.1),
            ]),
            QuantityKind::Intensity | QuantityKind::IntensityError => factor(&[
                ("count/s", 1.0),
                ("counts/s", 1.0),
                ("cps", 1.0),
                ("count", 1.0),
            ]),
            QuantityKind::Exposure => factor(&[("s", 1.0), ("sec", 1.0), ("ms", 0.001)]),
            QuantityKind::Thickness => factor(&[
                ("cm", 1.0),
                ("mm", 0.1),
                ("m", 100.0),
                ("um", 0.0001),
            ]),
            QuantityKind::Transmission => factor(&[
                ("fraction", 1.0),
                ("", 1.0),
                ("1", 1.0),
                ("percent", 0.01),
                ("%", 0.01),
            ]),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDiag {
    pub index: usize,
    pub code: String,
    pub message: String,
}

/// A vector of measurements that remembers the exact imported text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawVec {
    pub label: String,
    pub raw_unit: String,
    pub canonical_unit: String,
    pub raw: Vec<String>,
    pub parsed: Vec<Option<f64>>,
    pub canonical: Vec<Option<f64>>,
    pub diagnostics: Vec<IndexDiag>,
}

impl RawVec {
    pub fn new(label: &str, kind: QuantityKind, raw_unit: &str, raw: Vec<String>) -> Self {
        let mut parsed = Vec::with_capacity(raw.len());
        let mut canonical = Vec::with_capacity(raw.len());
        let mut diagnostics = Vec::new();
        let factor = kind.factor_to_canonical(raw_unit);
        for (i, text) in raw.iter().enumerate() {
            let value = text.trim().parse::<f64>().ok().filter(|v| v.is_finite());
            parsed.push(value);
            match (value, factor) {
                (Some(v), Some(f)) => canonical.push(Some(v * f)),
                (None, _) => {
                    canonical.push(None);
                    diagnostics.push(IndexDiag {
                        index: i,
                        code: "unparsable_value".into(),
                        message: format!("{label}[{i}]={text:?} is not a finite number"),
                    });
                }
                (_, None) => {
                    canonical.push(None);
                }
            }
        }
        if factor.is_none() {
            diagnostics.insert(
                0,
                IndexDiag {
                    index: 0,
                    code: "unknown_unit".into(),
                    message: format!(
                        "{label}: unit {raw_unit:?} cannot be converted to {}",
                        kind.canonical_unit()
                    ),
                },
            );
        }
        RawVec {
            label: label.into(),
            raw_unit: raw_unit.into(),
            canonical_unit: kind.canonical_unit().into(),
            raw,
            parsed,
            canonical,
            diagnostics,
        }
    }

    pub fn canonical_values(&self) -> &[Option<f64>] {
        &self.canonical
    }
}

/// A scalar physical quantity keeping the original representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quantity {
    pub name: String,
    pub raw: String,
    pub raw_unit: String,
    pub canonical_unit: String,
    pub parsed: Option<f64>,
    pub canonical: Option<f64>,
    pub diagnostic: Option<String>,
}

impl Quantity {
    pub fn new(name: &str, kind: QuantityKind, raw: &str, raw_unit: &str) -> Self {
        let parsed = raw.trim().parse::<f64>().ok().filter(|v| v.is_finite());
        let factor = kind.factor_to_canonical(raw_unit);
        let mut diagnostic = None;
        let canonical = match (parsed, factor) {
            (Some(v), Some(f)) => Some(v * f),
            (None, _) => {
                diagnostic =
                    Some(format!("{name}={raw:?} is not a finite number; kept at original position"));
                None
            }
            (_, None) => {
                diagnostic = Some(format!(
                    "{name}: unit {raw_unit:?} cannot be converted to {}; value left in place",
                    kind.canonical_unit()
                ));
                None
            }
        };
        Quantity {
            name: name.into(),
            raw: raw.into(),
            raw_unit: raw_unit.into(),
            canonical_unit: kind.canonical_unit().into(),
            parsed,
            canonical,
            diagnostic,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub id: String,
    pub source: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskRecord {
    pub id: String,
    pub source: String,
    pub kind: String,
    /// Inclusive ranges in the *canonical* q basis (1/A).
    pub ranges: Vec<[f64; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Diagnostic { code: code.into(), severity: Severity::Error, message: message.into(), at: None }
    }
    pub fn warning(code: &str, message: impl Into<String>) -> Self {
        Diagnostic { code: code.into(), severity: Severity::Warning, message: message.into(), at: None }
    }
    pub fn info(code: &str, message: impl Into<String>) -> Self {
        Diagnostic { code: code.into(), severity: Severity::Info, message: message.into(), at: None }
    }
    pub fn at(mut self, at: impl Into<String>) -> Self {
        self.at = Some(at.into());
        self
    }
}

/// Raw imported material: 2D-integrated I(q) plus every normalization input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub rev: String,
    pub kind: MaterialKind,
    pub label: String,
    pub origin: String,
    pub q: RawVec,
    pub i: RawVec,
    pub di: RawVec,
    pub exposure: Quantity,
    pub thickness: Quantity,
    pub transmission: Quantity,
    pub response: Provenance,
    pub masks: Vec<MaskRecord>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointStatus {
    Ok,
    Masked,
    MissingInput,
    Unnormalizable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormPoint {
    pub id: String,
    pub index: usize,
    pub q: f64,
    pub i: Option<f64>,
    pub var: Option<f64>,
    pub status: PointStatus,
    pub material_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormFactors {
    pub exposure_s: Option<f64>,
    pub thickness_cm: Option<f64>,
    pub transmission: Option<f64>,
    pub response_scale: f64,
    pub combined: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Normalized {
    pub id: String,
    pub rev: String,
    pub node_id: String,
    pub rules_version: String,
    pub source_material: String,
    pub source_rev: String,
    pub q_basis: String,
    pub factors: NormFactors,
    pub points: Vec<NormPoint>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubStatusKind {
    Ok,
    OutsideOverlap,
    Interpolated,
    Negative,
    Masked,
    MissingInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubPoint {
    pub id: String,
    pub index: usize,
    pub q: f64,
    /// Sample intensity on its own grid (canonical absolute units).
    pub sample_i: Option<f64>,
    pub sample_var: Option<f64>,
    /// Buffer value *at this q*: exact grid match or interpolation.
    pub buffer_i: Option<f64>,
    pub buffer_var: Option<f64>,
    pub buffer_source_index: Option<usize>,
    pub interpolated: bool,
    /// Subtracted intensity; null outside the overlap range.
    pub i: Option<f64>,
    /// Variance including normalization errors AND background-scale uncertainty.
    pub var: Option<f64>,
    pub statuses: Vec<SubStatusKind>,
    pub sample_point_id: String,
    pub buffer_point_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleFit {
    pub factor: Option<f64>,
    pub variance: Option<f64>,
    pub q_window: [f64; 2],
    pub n_points: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtraction {
    pub id: String,
    pub rev: String,
    pub node_id: String,
    pub rules_version: String,
    pub sample_norm: String,
    pub sample_rev: String,
    pub buffer_norm: String,
    pub buffer_rev: String,
    pub q_basis: String,
    pub same_grid: bool,
    pub overlap: Option<[f64; 2]>,
    pub scale: ScaleFit,
    pub points: Vec<SubPoint>,
    pub blocked: Vec<Diagnostic>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ResidualPoint {
    pub q: f64,
    pub observed: f64,
    pub fitted: f64,
    pub residual: f64,
    pub normalized: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitCandidate {
    pub id: String,
    pub range: String,
    pub q_lo: f64,
    pub q_hi: f64,
    pub n_points: usize,
    pub legal: bool,
    pub reasons: Vec<String>,
    /// Guinier: {I0, Rg, var_I0, var_Rg}; Porod: {porod_constant, background}.
    pub params: serde_json::Value,
    pub chi2: f64,
    pub residuals: Vec<ResidualPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateComparison {
    pub a: String,
    pub b: String,
    pub delta_chi2: f64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelReport {
    pub id: String,
    pub rev: String,
    pub node_id: String,
    pub rules_version: String,
    pub subtraction: String,
    pub subtraction_rev: String,
    pub guinier: Vec<FitCandidate>,
    pub porod: Vec<FitCandidate>,
    pub comparisons: Vec<CandidateComparison>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAction {
    Accept,
    Reject,
    Params,
    Choose,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub slot: String,
    pub action: DecisionAction,
    pub chosen: Option<String>,
    pub rejected: Vec<String>,
    pub params: serde_json::Value,
    pub note: String,
    pub author: String,
}

/// Self-contained, append-only fact. Payloads are full objects so old events
/// never depend on the newest computation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub uid: String,
    pub ts: String,
    pub author: String,
    pub kind: String,
    /// Pointer version the editor started from; used for conflict detection.
    pub base_version: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub node_id: String,
    pub rev: String,
    pub kind: String,
    pub updated_seq: i64,
    pub basis_hash: String,
    pub valid: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub node_id: String,
    pub input_id: String,
    pub input_rev: String,
}
