//! Core domain types for the SAXS scattering bench.
//!
//! Everything a reviewer needs to reconstruct a number is kept here:
//! raw q/I/error, exposure, thickness, transmission, buffer candidates,
//! instrument response and mask provenance. Quantities carry their original
//! representation and their normalized SI value (if a conversion was possible).

use serde::{Deserialize, Serialize};

/// A physical quantity that preserves the original textual representation.
///
/// Import keeps `raw` untouched. The derived layer computes `value_si`; if the
/// value cannot be converted it stays in place with a diagnostic instead of
/// being dropped to make a plot work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Qty {
    pub raw: String,
    pub value_si: Option<f64>,
    pub unit: String,
    pub si_unit: String,
    pub diagnostic: Option<String>,
}

impl Qty {
    pub fn raw(raw: impl Into<String>, unit: impl Into<String>, si_unit: impl Into<String>) -> Self {
        Qty { raw: raw.into(), value_si: None, unit: unit.into(), si_unit: si_unit.into(), diagnostic: None }
    }

    pub fn converted(
        raw: impl Into<String>,
        unit: impl Into<String>,
        si_unit: impl Into<String>,
        value_si: f64,
    ) -> Self {
        Qty { raw: raw.into(), value_si: Some(value_si), unit: unit.into(), si_unit: si_unit.into(), diagnostic: None }
    }

    pub fn ok(&self) -> bool {
        self.value_si.is_some() && self.diagnostic.is_none()
    }
}

/// One point of an imported 1D curve. The point id is stable and derived from
/// the *content* (see [`crate::ids::point_id`]) so it survives export/delete/
/// re-import as long as the underlying data does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawPoint {
    pub id: String,
    pub index: usize,
    pub q: Qty,
    pub intensity: Qty,
    pub error: Qty,
    pub masked: bool,
}

/// Role of an imported material record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Sample,
    Buffer,
}

/// A single, immutable version of an imported material record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialVersion {
    pub material_id: String,
    pub version: u32,
    pub kind: MaterialKind,
    pub label: String,
    pub points: Vec<RawPoint>,
    pub exposure: Qty,
    pub thickness: Qty,
    pub transmission: Qty,
    pub transmission_error: Qty,
    /// Instrument response version (normalization calibration source).
    pub response_ref: Option<Ref>,
    /// Mask source: where the masked flags came from.
    pub mask_ref: Option<Ref>,
    pub diagnostics: Vec<String>,
}

/// Instrument response calibration (flat-field / detector response).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub id: String,
    pub version: u32,
    pub label: String,
    /// Scaling factor applied during absolute normalization.
    pub scale: f64,
    pub scale_uncertainty: f64,
    pub source: String,
}

/// A mask source description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub id: String,
    pub version: u32,
    pub label: String,
    pub source: String,
    /// Point indices that are masked (interpreted per referencing material).
    pub masked_indexes: Vec<usize>,
}

/// An immutable reference to another stored object version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ref {
    pub id: String,
    pub version: u32,
}

impl Ref {
    pub fn new(id: impl Into<String>, version: u32) -> Self {
        Ref { id: id.into(), version }
    }
    pub fn slug(&self) -> String {
        format!("{}#v{}", self.id, self.version)
    }
}

/// Derived normalized curve at a particular q value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormPoint {
    pub id: String,
    pub q_si: f64,
    pub i_si: f64,
    /// Total 1-sigma uncertainty including normalization contributions.
    pub sigma: f64,
    /// Decomposed uncertainty budget.
    pub sigma_stat: f64,
    pub sigma_trans: f64,
    pub sigma_response: f64,
    pub source_point: String,
    pub masked: bool,
}

/// A normalized curve artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Normalized {
    pub node: String,
    pub source: Ref,
    pub points: Vec<NormPoint>,
    pub diagnostics: Vec<String>,
}

/// How the buffer scale factor was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaleMethod {
    /// Pure transmission/exposure ratio, no fitted freedom.
    Analytical,
    /// Least-squares fit over an overlap scale window, with a fitted error.
    Fitted,
}

/// One legal background subtraction candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubCandidate {
    pub id: String,
    pub method: ScaleMethod,
    pub scale: f64,
    pub scale_sigma: f64,
    pub chosen: bool,
}

/// One subtracted point. Provenance keeps both parent curves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPoint {
    pub id: String,
    pub q_si: f64,
    pub i_sub: f64,
    /// Total sigma including normalization and background-scale uncertainty.
    pub sigma: f64,
    pub sigma_bg_scale: f64,
    pub residual: f64,
    pub sample_source_point: String,
    pub buffer_source_point: String,
    pub negative: bool,
}

/// Subtraction result between a sample and one buffer version. May carry
/// multiple legal candidates; the system never forces a single answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subtracted {
    pub node: String,
    pub sample: Ref,
    pub buffer: Ref,
    /// Overlap [q_lo, q_hi] actually used for interpolation/subtraction.
    pub overlap: Option<[f64; 2]>,
    pub interpolation_used: bool,
    pub candidates: Vec<SubCandidate>,
    pub chosen_candidate: Option<String>,
    pub points: Vec<SubPoint>,
    pub diagnostics: Vec<String>,
}

/// Which interval model a candidate window belongs to. These are kept as
/// separate evidence and are never auto-stitched into one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntervalModel {
    Guinier,
    Porod,
}

/// A model fit over a window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub model: IntervalModel,
    pub q_lo: f64,
    pub q_hi: f64,
    /// Guinier: Rg; Porod: exponent n.
    pub parameter: f64,
    pub parameter_sigma: f64,
    pub intercept: f64,
    pub r_squared: f64,
    pub point_count: usize,
    pub qrguinier_max: Option<f64>,
    pub legal: bool,
    pub reason: String,
    pub chosen: bool,
}

/// Multi-interval evidence attached to a subtracted curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelEvidence {
    pub node: String,
    pub source_sub: Ref,
    pub candidates: Vec<IntervalCandidate>,
    /// True if a Guinier and Porod window overlap in q. They stay separate.
    pub cross_interval_overlap: bool,
    pub diagnostics: Vec<String>,
}

/// Machine readable judgment. The wording matches the invariants the bench is
/// built to defend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictCode {
    Imported,
    ParameterRevised,
    SourceChanged,
    QGridMismatch,
    NoOverlap,
    InterpolatedOverlap,
    BgScaleUncertain,
    NegativeIntensity,
    IntervalOverlap,
    BufferVersionSwitched,
    MultiCandidateResidual,
    Accepted,
    Rejected,
    ParamAdjusted,
    Conflict,
    Recomputed,
    StillValid,
    NeedsRecompute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    pub code: VerdictCode,
    pub detail: String,
}

/// What kind of object an event attached its result to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Material,
    Response,
    Mask,
    Normalized,
    Subtracted,
    Model,
}

/// Artifacts attached to an event. A commit is only made when the whole
/// command succeeded and the current pointer is switched atomically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Artifacts {
    pub materials: Vec<MaterialVersion>,
    pub responses: Vec<InstrumentResponse>,
    pub masks: Vec<Mask>,
    pub normalized: Vec<Normalized>,
    pub subtracted: Vec<Subtracted>,
    pub models: Vec<ModelEvidence>,
}
