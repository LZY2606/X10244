//! Domain types for the SAXS scattering-curve station.
//!
//! Design invariants encoded here:
//! * every imported value keeps its original textual representation and a
//!   declared unit/coordinate basis; parsing into SI-like canonical units only
//!   happens on the derivation layer (`rules`);
//! * a value that cannot be converted stays in place (`Cell::Unparsed`) with a
//!   diagnostic instead of being dropped or silently coerced;
//! * all derived points are arrays of points that retain explicit identities
//!   and provenance so a plotted point can be traced back to both source curves.

use serde::{Deserialize, Serialize};

/// Instrument role of an imported one-dimensional curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Sample,
    Buffer,
    /// multiplicative instrument response (arbitrary/canonical, applied in norm)
    Response,
    /// mask / shadow-flag channel; values act as a 0/1 weight after parse
    Mask,
}

/// Axis coordinate basis kept alongside the original representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AxisUnit {
    /// inverse angstrom, the canonical SAXS q basis
    #[default]
    InvAngstrom,
    /// inverse nanometre, converted to inverse angstrom by /10
    InvNanometre,
    /// pixel index / channel number, no physical conversion available
    Channel,
}

/// Units a scalar measurement may arrive in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarUnit {
    Seconds,
    Milliseconds,
    Millimetre,
    Centimetre,
    Dimensionless,
    DimensionlessTransmission,
    Unknown,
}

/// A single imported scalar: nothing is thrown away.
///
/// `raw` is the verbatim text from the import file. `value` exists only when
/// the unit is known and convertible; otherwise the cell is `Unparsed` with a
/// diagnostic attached at import time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cell {
    Parsed {
        raw: String,
        value: f64,
        unit: ScalarUnit,
    },
    Unparsed {
        raw: String,
        reason: String,
    },
}

impl Cell {
    pub fn raw(&self) -> &str {
        match self {
            Cell::Parsed { raw, .. } | Cell::Unparsed { raw, .. } => raw,
        }
    }
    pub fn canonical(&self) -> Option<f64> {
        match self {
            Cell::Parsed { value, unit, .. } => {
                use ScalarUnit::*;
                let f = match unit {
                    Seconds => 1.0,
                    Milliseconds => 1e-3,
                    Millimetre => 1.0,
                    Centimetre => 10.0,
                    Dimensionless | DimensionlessTransmission => 1.0,
                    Unknown => return None,
                };
                Some(value * f)
            }
            Cell::Unparsed { .. } => None,
        }
    }
    pub fn is_unparsed(&self) -> bool {
        matches!(self, Cell::Unparsed { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Measurement {
    pub exposure: Cell,
    pub thickness_mm: Cell,
    pub transmission: Cell,
}

impl RawCurve {
    /// stable identity shared by all versions of one physical record
    pub fn lineage_id(&self) -> String {
        self.id.rsplit_once(':')
            .filter(|(head, tail)| tail.starts_with('v') && tail[1..].chars().all(|c| c.is_ascii_digit()))
            .map(|(head, _)| head.to_string())
            .unwrap_or_else(|| self.id.clone())
    }
}

/// A row of the imported curve. `q_raw`/`i_raw`/`e_raw` are verbatim strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub index: usize,
    pub q_raw: String,
    pub i_raw: String,
    pub e_raw: String,
    pub q: Option<f64>,
    pub i: Option<f64>,
    pub e: Option<f64>,
}

/// Verbatim imported record plus parse results. Nothing is coerced away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCurve {
    pub id: String,
    pub role: Role,
    pub label: String,
    pub q_unit: AxisUnit,
    pub points: Vec<CurvePoint>,
    pub meta: Measurement,
    /// e.g. "beamline export v3 row 12"; free form provenance text
    pub source: String,
    /// masks/responses are linked to a sample/buffer by this optional key
    pub applies_to: Option<String>,
    pub import_seq: i64,
}

/// Canonical (q, I, sigma) parsed view used by the derivation layer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct QIE {
    pub q: f64,
    pub i: f64,
    pub e: f64,
}

/// A point on any derived curve with stable provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivedPoint {
    /// stable per-artifact point identity (`<artifact>#p<n>`), fixed on export
    pub point_id: String,
    pub q: f64,
    pub value: f64,
    pub sigma: f64,
    /// negative resulting intensity is kept, never clipped
    pub negative: bool,
    /// human-readable explanation of how value/sigma were obtained
    pub trace: String,
    /// which source point indices fed this derived point, per parent artifact
    pub parents: Vec<ParentRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParentRef {
    pub artifact_id: String,
    /// point ids on the parent curve (two for interpolated subtraction points)
    pub point_ids: Vec<String>,
    pub interpolated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Normalized {
    pub artifact_id: String,
    pub source_raw_id: String,
    pub response_raw_id: Option<String>,
    pub mask_raw_id: Option<String>,
    pub rule_version: i64,
    pub points: Vec<DerivedPoint>,
    /// transmission factor actually applied: T_s/T_r style canonical number
    pub transmission: f64,
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtractionStatus {
    Ok,
    /// grids do not overlap at all; subtraction is refused, not faked
    NoOverlap,
    /// a required input is unparsed/blocking
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtractionCandidate {
    pub candidate_id: String,
    pub buffer_raw_id: String,
    pub buffer_label: String,
    pub scale: f64,
    pub scale_sigma: f64,
    pub chi2: f64,
    pub residual_mean: f64,
    pub n_negative: usize,
    pub score: f64,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Subtraction {
    pub artifact_id: String,
    pub sample_norm_id: String,
    pub status: SubtractionStatus,
    pub overlap: Option<OverlapRange>,
    pub points: Vec<DerivedPoint>,
    pub candidates: Vec<SubtractionCandidate>,
    /// candidate chosen by a human decision; None means the reviewer has not
    /// uniquely decided and every legal candidate stays live
    pub chosen_candidate_id: Option<String>,
    pub rule_version: i64,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OverlapRange {
    pub q_min: f64,
    pub q_max: f64,
    pub n_sample_points: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntervalKind {
    Guinier,
    Porod,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitCandidate {
    pub candidate_id: String,
    pub interval: IntervalKind,
    pub q_min: f64,
    pub q_max: f64,
    /// Guinier: Rg, I0; Porod: power-law exponent, prefactor
    pub params: Vec<f64>,
    pub param_names: Vec<String>,
    pub param_sigmas: Vec<f64>,
    pub chi2: f64,
    pub n_points: usize,
    pub residuals: Vec<f64>,
    /// Guinier q_max*Rg criterion etc.
    pub quality_flags: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelFit {
    pub artifact_id: String,
    pub subtraction_id: String,
    /// guinier/porod candidates are kept separate; there is deliberately no
    /// single auto-stitched model
    pub candidates: Vec<FitCandidate>,
    pub chosen: Vec<String>,
    pub rule_version: i64,
    pub diagnostics: Vec<Diagnostic>,
}

/// Human adjudication of a multi-candidate derivation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub decided_candidate_id: String,
    pub note: String,
    pub event_seq: i64,
}

/// Tunable rule parameters. A revision of these bumps `rule_version` and is
/// tracked per key so only the dependent part of the graph goes stale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleParams {
    /// fraction of local curvature flux added as interpolation-grid sigma
    pub interpolation_tau: f64,
    /// relative uncertainty of the matched background scale, propagated additively
    pub background_scale_rel_sigma: f64,
    /// relative normalization uncertainty floor (transmission/response chain)
    pub norm_rel_sigma: f64,
    /// Rg criterion q_max*Rg upper bound for accepting a Guinier window
    pub guinier_qrg_max: f64,
    /// how many overlapping windows to enumerate (fraction steps of the range)
    pub window_steps: usize,
}

impl Default for RuleParams {
    fn default() -> Self {
        RuleParams {
            interpolation_tau: 0.02,
            background_scale_rel_sigma: 0.05,
            norm_rel_sigma: 0.01,
            guinier_qrg_max: 1.3,
            window_steps: 4,
        }
    }
}

/// Append-only log entry. The database state is a fold over events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    RawImported {
        raw: RawCurve,
    },
    /// a raw curve is superseded by a new version (buffer version switch,
    /// corrected exposure...); old version is retained
    RawSuperseded {
        old_raw_id: String,
        new_raw: RawCurve,
    },
    RawRedactedMask {
        raw_id: String,
    },
    RuleParamsRevised {
        new_params: RuleParams,
        note: String,
    },
    NormalizedProduced {
        norm: Normalized,
    },
    SubtractionProduced {
        sub: Subtraction,
    },
    ModelFitProduced {
        fit: ModelFit,
    },
    /// pointer move (which artifact version is "current"); derivations
    /// themselves are immutable, this only moves the current pointer
    PointerMoved {
        slot: String,
        from_artifact_id: Option<String>,
        to_artifact_id: String,
    },
    CandidateDecided {
        slot: String,
        artifact_id: String,
        candidate_id: String,
        note: String,
    },
    /// a reviewer explicitly rejected a candidate; it stays stored but inactive
    CandidateRejected {
        slot: String,
        artifact_id: String,
        candidate_id: String,
        note: String,
    },
    NoteAdded {
        target_id: String,
        note: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredEvent {
    pub seq: i64,
    pub event: Event,
}
