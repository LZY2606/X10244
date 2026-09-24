//! Domain model for the SAXS workbench.
//!
//! Every physical quantity keeps its *original* representation (as imported)
//! alongside a canonical value+unit used by the derived layer. Quantities that
//! cannot be converted to the canonical unit stay in place with a diagnostic:
//! they are never dropped just to make plotting possible.

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 2;

/// Canonical units used by the derived layer. Everything is SI-adjacent in the
/// conventions common to small-angle scattering reduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    InverseAngstrom, // q: A^-1
    InverseNm,       // q: nm^-1
    PerCm,           // absorbance: cm^-1 (informational)
    Seconds,         // exposure: s
    Milliseconds,    // exposure: ms
    Cm,              // thickness: cm
    Mm,              // thickness: mm
    Dimensionless,   // transmission / scale
    Absolute,        // I(q): cm^-1 absolute scale
    Arbitrary,       // I(q): detector count units (a.u.)
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::InverseAngstrom => "A^-1",
            Unit::InverseNm => "nm^-1",
            Unit::PerCm => "cm^-1",
            Unit::Seconds => "s",
            Unit::Milliseconds => "ms",
            Unit::Cm => "cm",
            Unit::Mm => "mm",
            Unit::Dimensionless => "1",
            Unit::Absolute => "cm^-1",
            Unit::Arbitrary => "a.u.",
        }
    }
}

/// A vector quantity (a curve axis or signal) with original and canonical form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vector {
    pub values: Vec<f64>,
    pub unit: Unit,
    /// Original text values exactly as imported; empty when not applicable.
    #[serde(default)]
    pub original_text: Vec<String>,
    /// Original unit label exactly as imported.
    #[serde(default)]
    pub original_unit: String,
    /// Canonical values; `None` entries could not be converted for that point.
    /// Length always equals `values.len()` (same positions retained).
    #[serde(default)]
    pub canonical: Vec<Option<f64>>,
    #[serde(default)]
    pub canonical_unit: Option<Unit>,
    #[serde(default)]
    pub convertible: bool,
}

impl Vector {
    pub fn from_canonical(values: Vec<f64>, unit: Unit) -> Self {
        let n = values.len();
        Vector {
            values: values.clone(),
            unit,
            original_text: Vec::new(),
            original_unit: String::new(),
            canonical: values.into_iter().map(Some).collect(),
            canonical_unit: Some(unit),
            convertible: true,
        }
    }
}

/// A scalar physical quantity preserving original representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scalar {
    pub value: f64,
    pub unit: Unit,
    #[serde(default)]
    pub original_text: String,
    #[serde(default)]
    pub original_unit: String,
    #[serde(default)]
    pub canonical: Option<f64>,
    #[serde(default)]
    pub canonical_unit: Option<Unit>,
    #[serde(default)]
    pub convertible: bool,
}

impl Scalar {
    pub fn canonical(value: f64, unit: Unit) -> Self {
        Scalar {
            value,
            unit,
            original_text: String::new(),
            original_unit: String::new(),
            canonical: Some(value),
            canonical_unit: Some(unit),
            convertible: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskSource {
    pub name: String,
    /// Origin of the mask, e.g. "beamstop", "detector-edge", "import-file".
    pub mask_type: String,
    #[serde(default)]
    pub source_uri: String,
    #[serde(default)]
    pub sha256: String,
    /// 1-based excluded q-index list documented by the integration software.
    #[serde(default)]
    pub excluded_indices: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseSource {
    pub name: String,
    #[serde(default)]
    pub source_uri: String,
    #[serde(default)]
    pub sha256: String,
}

/// A machine-readable diagnostic attached to an import or derivation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    /// Optional stable point id the diagnostic refers to.
    #[serde(default)]
    pub point_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Sample,
    Buffer,
}

/// Raw record exactly as supplied by the small-angle scattering group, after
/// parsing the 2D integration output. Nothing here is ever mutated in place;
/// revisions create new versioned records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRecord {
    /// Stable id, versioned: e.g. "raw:sample-A@v2".
    pub id: String,
    /// Stable logical key shared across revisions, e.g. "sample-A".
    pub logical_key: String,
    pub kind: RecordKind,
    pub q: Vector,
    pub intensity: Vector,
    pub error: Vector,
    pub exposure: Scalar,
    pub thickness: Scalar,
    pub transmission: Scalar,
    /// Logical keys of buffer runs proposed for subtraction (samples only).
    #[serde(default)]
    pub buffer_candidates: Vec<String>,
    #[serde(default)]
    pub response: Option<ResponseSource>,
    /// Multiplicative response scale canonical value; default 1.0.
    #[serde(default)]
    pub response_scale: Scalar,
    #[serde(default)]
    pub masks: Vec<MaskSource>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    pub version: u32,
    #[serde(default)]
    pub supersedes: Option<String>,
    #[serde(default)]
    pub source_uri: String,
    #[serde(default)]
    pub imported_at: String,
}

/// Input shape accepted by the importer. Units may be given in the original
/// instrument convention and are converted by the import layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRecordIn {
    pub logical_key: String,
    pub kind: RecordKind,
    pub q: Vec<f64>,
    pub q_unit: String,
    pub intensity: Vec<f64>,
    /// Error column in the *same* unit as intensity; text allowed so that
    /// unparseable sentinels ("N/A") survive at their original position.
    #[serde(default)]
    pub error: Vec<serde_json::Value>,
    #[serde(default = "default_err_au")]
    pub intensity_unit: String,
    #[serde(default)]
    pub error_unit: String,
    pub exposure: f64,
    pub exposure_unit: String,
    pub thickness: serde_json::Value,
    pub thickness_unit: String,
    pub transmission: serde_json::Value,
    #[serde(default)]
    pub transmission_unit: String,
    #[serde(default)]
    pub buffer_candidates: Vec<String>,
    #[serde(default)]
    pub response_name: String,
    #[serde(default)]
    pub response_scale: serde_json::Value,
    #[serde(default = "default_resp_unit")]
    pub response_scale_unit: String,
    #[serde(default)]
    pub masks: Vec<MaskSource>,
    #[serde(default)]
    pub source_uri: String,
    /// Explicit version; importer assigns next version when omitted.
    #[serde(default)]
    pub version: Option<u32>,
    #[serde(default)]
    pub supersedes: Option<String>,
}

fn default_err_au() -> String {
    "a.u.".to_string()
}
fn default_resp_unit() -> String {
    "1".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeriveStatus {
    /// All points converted and the operation applied.
    Ok,
    /// Operation completed on the usable subset; per-point diagnostics exist.
    Partial,
    /// The operation could not be applied (e.g. no overlapping q range).
    Failed,
}

/// One point of a normalized curve, with stable identity and provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormPoint {
    pub id: String,
    pub source_point_id: String,
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Normalized {
    pub id: String,
    pub record_id: String,
    pub status: DeriveStatus,
    pub points: Vec<NormPoint>,
    /// Contributions to the total variance budget, per point.
    pub variance: Vec<f64>,
    pub q_unit: String,
    pub intensity_unit: String,
    pub diagnostics: Vec<Diagnostic>,
    /// Canonical normalization constants used (kept for audit).
    pub exposure_s: f64,
    pub thickness_cm: f64,
    pub transmission: f64,
    pub response_scale: f64,
}

/// Provenance of one subtracted point back to its two source curves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPointProvenance {
    pub sample_norm_point_id: String,
    pub buffer_norm_point_id: Option<String>,
    /// "same-grid" | "interpolated"
    pub match_kind: String,
    pub q: f64,
    pub sample_q: f64,
    pub buffer_q: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPoint {
    pub id: String,
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub inside_overlap: bool,
    pub negative: bool,
    pub provenance: SubPointProvenance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subtracted {
    pub id: String,
    pub sample_record_id: String,
    pub buffer_record_id: String,
    pub sample_norm_id: String,
    pub buffer_norm_id: String,
    pub status: DeriveStatus,
    pub points: Vec<SubPoint>,
    /// Weighted background scale alpha with its propagated standard uncertainty.
    pub alpha: f64,
    pub alpha_sigma: f64,
    /// Overlapping q interval actually used for scaling and subtraction.
    pub overlap_q_min: f64,
    pub overlap_q_max: f64,
    /// Residual diagnostics over the overlap region.
    pub residual_mean: f64,
    pub residual_rms: f64,
    pub chi_squared: f64,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionKind {
    Guinier,
    Porod,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitPoint {
    pub q: f64,
    pub model: f64,
    pub observed: f64,
    pub residual: f64,
    pub source_point_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitCandidate {
    pub id: String,
    pub region: RegionKind,
    pub q_min: f64,
    pub q_max: f64,
    /// Guinier: I0 and Rg (A). Porod: scale K and exponent n.
    pub intercept: f64,
    pub intercept_sigma: f64,
    pub coefficient: f64,
    pub coefficient_sigma: f64,
    pub r_squared: f64,
    pub chi_squared: f64,
    pub n_points: usize,
    pub points: Vec<FitPoint>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fit {
    pub id: String,
    pub subtraction_id: String,
    pub status: DeriveStatus,
    pub candidates: Vec<FitCandidate>,
    pub diagnostics: Vec<Diagnostic>,
}

/// A machine-readable judgment emitted by a derivation rule. These are the
/// audit evidence behind statements such as "grids differ, do not subtract
/// directly" or "Guinier and Porod are not merged into one model".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Judgment {
    pub rule: String,
    pub decision: String,
    pub basis: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    DatasetCreated,
    RawImported,
    Normalized,
    Subtracted,
    Fitted,
    Decision,
    Branch,
    ConflictDetected,
    ConflictResolved,
    Exported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Accepted,
    Rejected,
    Adjusted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub seq: i64,
    pub dataset_id: String,
    pub event_type: EventType,
    /// Base event seq this edit was made from (optimistic concurrency).
    #[serde(default)]
    pub base_seq: i64,
    pub author: String,
    pub timestamp: String,
    /// The object (node/record) primarily created or touched.
    #[serde(default)]
    pub object_id: String,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub judgments: Vec<Judgment>,
    /// Optional branch label. Events on the main line use "main".
    #[serde(default)]
    pub branch: String,
}

/// Full dataset bundle. This is exactly what export/import round-trips, with
/// stable ids, order (event seq) and source relationships preserved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetBundle {
    pub format: String,
    pub bundle_version: u32,
    pub schema_version: u32,
    pub dataset_id: String,
    pub name: String,
    pub created_at: String,
    pub events: Vec<Event>,
    pub raw_records: Vec<RawRecord>,
    pub normalized: Vec<Normalized>,
    pub subtracted: Vec<Subtracted>,
    pub fits: Vec<Fit>,
}
