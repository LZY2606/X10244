//! Immutable domain objects. Every object is content-addressed (stable id),
//! so export/delete/import preserves identity, ordering and provenance.

use serde::{Deserialize, Serialize};

use crate::units::RawField;

fn d0() -> bool {
    false
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point3 {
    pub q: f64,
    pub i: f64,
    pub e: f64,
}

/// One imported row of the 2D-integrated 1D curve, exactly as on disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawPoint {
    pub idx: usize,
    pub q_text: String,
    pub i_text: String,
    pub e_text: String,
    pub q_unit: String,
    pub i_unit: String,
    /// canonical numeric triplet (q in nm^-1), if all three fields parse
    pub canonical: Option<Point3>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub source: String,
    pub description: String,
    /// multiplicative gain per q-bin, canonical q in nm^-1
    pub gain: Vec<PointQ>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointQ {
    pub q: f64,
    pub g: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub source: String,
    pub description: String,
    /// excluded q ranges [lo, hi] in nm^-1
    pub excluded_ranges: Vec<[f64; 2]>,
    /// number of detector pixels masked (raw metadata, retained)
    pub pixels_masked: Option<i64>,
}

/// A fully retained import record. Raw representation is kept for every field;
/// canonical values exist only where unit conversion succeeded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRecord {
    /// content-addressed stable identity, computed at import
    pub id: String,
    pub kind: CurveKind,
    pub label: String,
    pub path: String,
    pub points: Vec<RawPoint>,
    pub q_unit: String,
    pub i_unit: String,
    pub exposure: RawField,
    pub thickness: RawField,
    pub transmission: RawField,
    pub transmission_sigma: RawField,
    pub buffer_candidate_of: Option<String>,
    pub instrument_response: Option<InstrumentResponse>,
    pub mask: Option<Mask>,
    /// diagnostics produced at import; a value here never removes raw data
    pub diagnostics: Vec<String>,
    /// format/version line kept for reproducibility
    pub format_tag: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveKind {
    Sample,
    Buffer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormParams {
    pub exposure_s: f64,
    pub thickness_nm: f64,
    pub transmission: f64,
    /// sigma of the transmission measurement, propagated into normalization
    pub transmission_sigma: f64,
    pub gain_applied: bool,
    pub mask_applied: bool,
    pub rule_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormPoint {
    /// stable per-curve identity: "r<record_id>#<idx>" style, set by caller
    pub id: String,
    pub idx: usize,
    pub q: f64,
    pub i: f64,
    pub e: f64,
    /// variance contribution from normalization uncertainty
    pub e_norm: f64,
    pub excluded_by_mask: bool,
}

#[derive(Debug,Clone, PartialEq, Serialize, Deserialize)]
pub struct NormCurve {
    pub params: NormParams,
    pub points: Vec<NormPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPoint {
    pub id: String,
    pub idx: usize,
    pub q: f64,
    /// background-subtracted intensity
    pub i: f64,
    /// total sigma: counting/normalization of both curves + scale uncertainty
    pub e: f64,
    pub e_sample: f64,
    pub e_buffer: f64,
    pub e_scale: f64,
    /// whether both sample and buffer cover this q on their native grids
    pub covered: bool,
    pub sample_i: f64,
    pub buffer_i_interp: Option<f64>,
    pub negative: bool,
    /// excluded by a mask carried on either source curve
    pub excluded: bool,
    /// direct provenance: normalized sample point and the two buffer
    /// points used by linear interpolation (identical grid -> one element)
    pub source_sample_pid: String,
    pub source_buffer_pids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubParams {
    /// fitted background scale k (Isub = Is - k * Ib)
    pub scale: f64,
    pub scale_sigma: f64,
    /// shared [q_lo, q_hi] used for the scale fit
    pub fit_range: [f64; 2],
    pub rule_version: String,
    /// interpolation method used inside the shared overlap only
    pub interpolation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubStatus {
    Ok,
    /// native q grids do not coincide
    GridMismatch,
    /// native grids share no overlap at all
    NoOverlap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubCurve {
    pub params: Option<SubParams>,
    pub status: SubStatus,
    /// overlap of the two native q grids [lo, hi]; empty if none
    pub overlap: Option<[f64; 2]>,
    pub grids_identical: bool,
    pub points: Vec<SubPoint>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntervalKind {
    Guinier,
    Porod,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitStats {
    pub chi2: f64,
    pub r2: f64,
    pub n: usize,
    /// log-likelihood against the stated errors, for multi-candidate tests
    pub ll: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCandidate {
    pub id: String,
    pub kind: IntervalKind,
    pub q_lo: f64,
    pub q_hi: f64,
    /// Guinier: I0 and Rg (nm). Porod: exponent alpha and prefactor.
    pub parameters: serde_json::Value,
    pub stats: FitStats,
    /// id of the subtraction curve whose points were fitted
    pub source_sub: String,
    pub decision: CandidateDecision,
    /// true when this window overlaps a kept candidate of the other kind
    #[serde(default = "d0")]
    pub interval_overlap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDecision {
    Proposed,
    Adopted,
    Rejected,
}

/// Provenance triple attached to every derived object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// record/object ids actually consumed
    pub inputs: Vec<String>,
    /// rule version applied
    pub rule_version: String,
    /// event id under which this object was created
    pub created_by_event: String,
}

/// Envelope persisted in the objects table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Obj {
    Raw {
        id: String,
        record: RawRecord,
    },
    Norm {
        id: String,
        #[serde(rename = "source_raw")]
        source: String,
        curve: NormCurve,
        provenance: Provenance,
    },
    Sub {
        id: String,
        sample_norm: String,
        buffer_norm: String,
        curve: SubCurve,
        provenance: Provenance,
    },
    Candidates {
        id: String,
        source_sub: String,
        candidates: Vec<ModelCandidate>,
        provenance: Provenance,
    },
}

impl Obj {
    pub fn source_inputs(&self) -> Vec<String> {
        match self {
            Obj::Raw { id, .. } => vec![id.clone()],
            Obj::Norm { source, .. } => vec![source.clone()],
            Obj::Sub {
                sample_norm,
                buffer_norm,
                ..
            } => vec![sample_norm.clone(), buffer_norm.clone()],
            Obj::Candidates { source_sub, .. } => vec![source_sub.clone()],
        }
    }

    pub fn object_id_ref(&self) -> &str {
        match self {
            Obj::Raw { id, .. }
            | Obj::Norm { id, .. }
            | Obj::Sub { id, .. }
            | Obj::Candidates { id, .. } => id,
        }
    }
}

/// An append-only ledger event. Events are content-addressed and never mutate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    /// previous event id on this chain (the review base)
    pub parent: Option<String>,
    pub chain: String,
    pub seq: i64,
    pub kind: EventKind,
    pub detail: serde_json::Value,
    /// machine-readable judgment / semantic guard outcome
    pub judgment: Judgment,
    /// object produced (Raw/Norm/Sub/Candidates), if any
    pub object_id: Option<String>,
    pub actor: String,
    /// ms since unix epoch; excluded from the content hash
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Import,
    Normalize,
    Subtract,
    FitModels,
    Decision,
    ChainCreate,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Judgment {
    pub status: JudgmentStatus,
    /// short machine-readable codes, e.g. "grid_mismatch", "negative_points"
    pub codes: Vec<String>,
    /// ids of derived objects invalidated/created downstream
    pub affected: Vec<String>,
    /// human and machine readable explanation
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgmentStatus {
    Ok,
    Guarded,
    Multi,
    Error,
    Conflict,
}
