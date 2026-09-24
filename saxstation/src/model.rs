//! Persistent, content-addressed domain objects.
//!
//! Every object here is immutable once created. A change to a source produces a
//! new object with a new id rather than mutating the old one. The graph keeps
//! pointers to the *current* object; old objects remain so any past event can
//! be revisited without being overwritten by the newest computation.

use crate::quantity::Quantity;
use serde::{Deserialize, Serialize};

pub type Id = String;

/// Which side of the sample/buffer pair a bundle plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Sample,
    Buffer,
}

/// A single imported sample point. Canonical values are filled on the derived
/// layer; the raw `*_raw` quantities are always retained.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// stable id derived from (bundle, index, raw values)
    pub id: Id,
    pub index: usize,
    /// raw q, exactly as imported
    pub q_raw: Quantity,
    pub i_raw: Quantity,
    pub sigma_raw: Quantity,
    /// canonical base values; `None` if conversion failed (kept in place)
    pub q: Option<f64>,
    pub i: Option<f64>,
    pub sigma: Option<f64>,
    pub convertible: bool,
    /// true when excluded by the imported mask
    pub masked: bool,
    /// diagnostics attached to this point (unit problems etc.)
    pub diagnostics: Vec<String>,
}

/// Instrument response / calibration used to bring intensity onto an absolute
/// base. `absent` means the data stay in relative (normalized detector) units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub name: String,
    pub source: String,
    /// multiplicative calibration constant; 1.0 when absent
    pub scale: f64,
    /// relative standard uncertainty (0 if unknown)
    pub scale_rel_sigma: f64,
    pub present: bool,
    pub raw_token: String,
}

/// Provenance of the mask applied during 2D integration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskSource {
    pub name: String,
    pub source: String,
    pub raw_token: String,
}

/// Transmission measurement with its own uncertainty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transmission {
    pub value: f64,
    pub rel_sigma: f64,
    pub raw: String,
}

/// A full imported record: the 2D-integrated I(q) curve plus every context
/// quantity the reviewer must be able to audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawBundle {
    pub id: Id,
    pub label: String,
    pub role: Role,
    /// buffer family/version label, e.g. "buffer@v1"
    pub version: String,
    pub points: Vec<Point>,
    pub exposure: Quantity,
    pub thickness: Quantity,
    pub transmission: Transmission,
    pub instrument: InstrumentResponse,
    pub mask: MaskSource,
    /// original file text / rows, verbatim
    pub source_text: String,
    pub diagnostics: Vec<String>,
}

impl RawBundle {
    pub fn convertible_points(&self) -> impl Iterator<Item = &Point> {
        self.points.iter().filter(|p| p.convertible)
    }
    pub fn active_points(&self) -> impl Iterator<Item = &Point> {
        self.points.iter().filter(|p| p.convertible && !p.masked)
    }
}

/// Per-point output of the normalization layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedPoint {
    pub id: Id,
    pub source_point: Id,
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
    /// relative variance from normalization terms (T, t, exp, response)
    pub norm_rel_var: f64,
    pub norm_factor: f64,
    pub masked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedCurve {
    pub id: Id,
    pub source_bundle: Id,
    pub role: Role,
    pub points: Vec<NormalizedPoint>,
    pub q_unit: String,
    pub i_unit: String,
    pub diagnostics: Vec<String>,
}

/// Result of subtracting one interpolated buffer point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPoint {
    pub id: Id,
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub sample_point: Id,
    /// buffer points bracketing this q (one if exactly on a grid node)
    pub buffer_points: Vec<Id>,
    pub interpolated_buffer: bool,
    /// whether this point sits inside the shared overlap
    pub in_overlap: bool,
}

/// Status of a subtraction attempt. A non-ok status still carries structured
/// evidence and never silently deletes inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubStatus {
    Ok,
    NoOverlap,
    GridsMismatch,
    MissingInput,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subtraction {
    pub id: Id,
    pub sample_norm: Id,
    pub buffer_norm: Id,
    pub buffer_version: String,
    pub scale: f64,
    pub scale_rel_sigma: f64,
    pub overlap: Option<(f64, f64)>,
    pub points: Vec<SubPoint>,
    pub status: SubStatus,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegionKind {
    Guinier,
    Porod,
}

/// A linear-fit window over the subtracted curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionCandidate {
    pub id: Id,
    pub kind: RegionKind,
    pub q_min: f64,
    pub q_max: f64,
    /// Guinier: slope of ln I vs q^2; Porod: slope of log I vs log q
    pub slope: f64,
    pub intercept: f64,
    pub r_squared: f64,
    pub chi_squared: f64,
    pub residuals: Vec<PointResidual>,
    pub member_points: Vec<Id>,
    /// set when this window overlaps another accepted/considered window
    pub overlaps_other: bool,
    /// machine readable reason this candidate is accepted/rejected/held
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointResidual {
    pub point: Id,
    pub q: f64,
    pub observed: f64,
    pub predicted: f64,
    pub normalized_residual: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionState {
    Pending,
    Accepted,
    Rejected,
}

/// A set of region candidates produced from one subtraction. Guinier and Porod
/// candidates are never automatically fused into a single piecewise model;
/// when no unique choice exists all legal candidates stay pending.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionFit {
    pub id: Id,
    pub subtraction: Id,
    pub candidates: Vec<RegionCandidate>,
    pub diagnostics: Vec<String>,
}

/// A reviewer decision about a candidate/branch. Does not delete the others.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub id: Id,
    pub candidate_id: Id,
    pub region_fit: Id,
    pub state: DecisionState,
    pub note: String,
    pub base_version: Id,
}
