//! Event-sourced domain model.
//!
//! Everything that enters the system is an immutable [`Event`] with a stable
//! id, a parent pointer (optimistic-concurrency `prev`) and the active rule
//! version. Derived artifacts are produced by folding events through the
//! dependency engine; decisions never overwrite inputs or rejected candidates.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::units::Val;

/// Current rule pack. Changing rules increments this and re-marks artifacts.
pub const RULE_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Sample,
    Buffer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CurvePoint {
    /// Stable point id from the import fixture (materialId#row).
    pub id: String,
    pub q: Val,
    pub i: Val,
    /// One-sigma error in intensity units, if provided.
    pub sigma: Val,
    /// True for points hidden by the beamstop/mask.
    #[serde(default)]
    pub masked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CurveMaterial {
    pub id: String,
    pub rev: u32,
    pub kind: MaterialKind,
    pub label: String,
    pub exposure_s: Val,
    pub transmission: Val,
    /// Only samples carry thickness.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub thickness_cm: Option<Val>,
    pub points: Vec<CurvePoint>,
    /// Identifier of the instrument response curve used (e.g. response:r1).
    pub response_id: String,
    /// Identifier of the mask definition (e.g. mask:m1).
    pub mask_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResponsePoint {
    pub id: String,
    pub q: Val,
    /// Multiplicative detector response (dimensionless).
    pub r: Val,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentResponse {
    pub id: String,
    pub rev: u32,
    pub points: Vec<ResponsePoint>,
}

/// Beamstop / detector mask with its own provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MaskDef {
    pub id: String,
    pub rev: u32,
    pub source: String,
    /// Masked q intervals in canonical Å^-1, e.g. [[0.0,0.012]].
    pub q_intervals: Vec<[f64; 2]>,
}

/// Tunable normalization/subtraction parameters, versioned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Params {
    pub rev: u32,
    /// Minimum number of points for a Guinier/Porod window.
    pub min_window: usize,
    /// q*Rg validity bound for Guinier windows.
    pub qrg_max: f64,
    /// Lowest q (Å^-1) eligible for Porod windows.
    pub porod_q_min: f64,
    /// Window sizes (number of points) tried for interval fits.
    pub window_sizes: Vec<usize>,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            rev: 0,
            min_window: 5,
            qrg_max: 1.3,
            porod_q_min: 0.08,
            window_sizes: vec![6, 8, 10],
        }
    }
}

/// Machine-readable judgment attached to a derived artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Judgment {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub evidence: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Processing proceeded but the reader should notice this.
    Info,
    /// Result exists but is not trustworthy on its own.
    Warning,
    /// The artifact cannot be computed; it is an indeterminate placeholder.
    Blocked,
}

/// A normalized point with a stable derived id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormPoint {
    pub id: String,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub source_point_id: String,
    pub masked: bool,
    /// Fractional variance contribution from exposure/transmission/thickness.
    pub norm_rel_var: f64,
    /// Kept verbatim from the source (unconvertible rows stay here).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub diag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormArtifact {
    pub id: String,
    pub material_id: String,
    pub material_rev: u32,
    pub response_id: String,
    pub mask_id: String,
    pub points: Vec<NormPoint>,
    pub valid: bool,
    pub rule_version: u32,
    #[serde(default)]
    pub judgments: Vec<Judgment>,
    /// Deterministic content id; identical inputs => identical artifact.
    pub content_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubPoint {
    pub id: String,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    /// Sample point used at this q.
    pub sample_point_id: String,
    /// Synthetic id of the interpolated buffer point (`bufferMat#q`).
    pub buffer_point_id: String,
    #[serde(default)]
    pub negative: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubArtifact {
    pub id: String,
    pub request_id: String,
    pub sample_id: String,
    pub sample_rev: u32,
    pub buffer_id: String,
    pub buffer_rev: u32,
    /// Overlap region in q actually used.
    pub overlap: Option<[f64; 2]>,
    /// Fitted multiplicative buffer scale alpha and its sigma.
    pub alpha: Option<f64>,
    pub alpha_sigma: Option<f64>,
    pub points: Vec<SubPoint>,
    pub valid: bool,
    pub rule_version: u32,
    #[serde(default)]
    pub judgments: Vec<Judgment>,
    pub content_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitKind {
    Guinier,
    Porod,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FitCandidate {
    pub id: String,
    pub kind: FitKind,
    pub q_min: f64,
    pub q_max: f64,
    pub point_ids: Vec<String>,
    /// Guinier: Rg (Å) / I0 (cm^-1); Porod: intercept + slope of lnI vs lnq.
    #[serde(flatten)]
    pub params: BTreeMap<String, f64>,
    pub param_sigma: BTreeMap<String, f64>,
    pub chi2: f64,
    pub valid: bool,
    #[serde(default)]
    pub judgments: Vec<Judgment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FitArtifact {
    pub id: String,
    pub sub_id: String,
    pub kind: FitKind,
    pub candidates: Vec<FitCandidate>,
    /// Accepted candidate id, only after an explicit human decision.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub accepted_id: Option<String>,
    /// True while multiple candidates remain statistically admissible.
    pub indeterminate: bool,
    pub rule_version: u32,
    #[serde(default)]
    pub judgments: Vec<Judgment>,
    pub content_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Artifact {
    Normalization(NormArtifact),
    Subtraction(SubArtifact),
    Fit(FitArtifact),
}

impl Artifact {
    pub fn id(&self) -> &str {
        match self {
            Artifact::Normalization(a) => &a.id,
            Artifact::Subtraction(a) => &a.id,
            Artifact::Fit(a) => &a.id,
        }
    }
    pub fn content_id(&self) -> &str {
        match self {
            Artifact::Normalization(a) => &a.content_id,
            Artifact::Subtraction(a) => &a.content_id,
            Artifact::Fit(a) => &a.content_id,
        }
    }
    pub fn valid(&self) -> bool {
        match self {
            Artifact::Normalization(a) => a.valid,
            Artifact::Subtraction(a) => a.valid,
            Artifact::Fit(a) => a.candidates.iter().any(|c| c.valid),
        }
    }
}

// ---------- Event payloads ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportMaterial {
    pub material: CurveMaterial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResponse {
    pub response: InstrumentResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportMask {
    pub mask: MaskDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviseParams {
    pub params: Params,
    pub reason: String,
}

/// Request subtraction of a sample against one specific buffer candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSubtraction {
    pub sample_id: String,
    pub buffer_id: String,
    pub branch: Option<String>,
}

/// Human adjudication of a fit candidate; rejected/unaccepted candidates stay.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecideCandidate {
    pub fit_id: String,
    pub candidate_id: String,
    pub accept: bool,
    pub reason: String,
    pub reviewer: String,
}

/// Parameter adjustment from the review page; spawns a new parameter revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustParams {
    pub qrg_max: Option<f64>,
    pub porod_q_min: Option<f64>,
    pub reason: String,
    pub reviewer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Payload {
    ImportMaterial(ImportMaterial),
    ImportResponse(ImportResponse),
    ImportMask(ImportMask),
    ReviseParams(ReviseParams),
    RequestSubtraction(RequestSubtraction),
    DecideCandidate(DecideCandidate),
    AdjustParams(AdjustParams),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub seq: i64,
    /// Stable globally unique id, retained across export/import.
    pub id: String,
    /// Parent event id; concurrent writes on the same parent conflict.
    pub prev: Option<String>,
    /// Branch label for parallel review threads.
    #[serde(default)]
    pub branch: String,
    pub actor: String,
    pub at: String,
    pub rule_version: u32,
    pub payload: Payload,
}

/// Artifact records emitted by the engine for one event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Emission {
    pub event_seq: i64,
    pub artifact: Artifact,
}

/// Whole portable archive: case pack export / import.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Archive {
    pub schema: String,
    pub case_id: String,
    pub title: String,
    pub events: Vec<Event>,
}
