//! Domain model for the SAXS workbench.
//! Every displayed value carries a stable, content-addressed identity so that
//! export / delete / re-import preserves identity, order and provenance.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type NodeId = String;

/// Compute a stable content-addressed id: `prefix_` + 16 hex chars of sha256.
pub fn content_id<T: Serialize>(prefix: &str, value: &T) -> NodeId {
    let bytes = serde_json::to_vec(value).expect("serialize for id");
    let digest = Sha256::digest(&bytes);
    format!("{}_{}", prefix, hex16(&digest))
}

fn hex16(digest: &[u8]) -> String {
    digest.iter().take(8).map(|b| format!("{:02x}", b)).collect()
}

/// A physical quantity that keeps its raw representation and unit.
/// Import preserves `raw`; only the derived layer normalizes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
    #[serde(default)]
    pub sigma: Option<f64>,
    #[serde(default)]
    pub raw: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    /// Set when a derived point is negative (kept, not dropped).
    #[serde(default)]
    pub negative: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    pub id: NodeId,
    pub kind: String,
    pub q_unit: String,
    pub i_unit: String,
    pub points: Vec<CurvePoint>,
    /// Raw representation exactly as imported; never rewritten by derivations.
    #[serde(default)]
    pub raw_text: String,
    #[serde(default)]
    pub meta: serde_json::Value,
}

impl Curve {
    pub fn q_range(&self) -> Option<(f64, f64)> {
        let first = self.points.first()?;
        let last = self.points.last()?;
        Some((first.q.min(last.q), first.q.max(last.q)))
    }
}

/// Machine-readable diagnostic / judgment attached to events and nodes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub affected: Vec<NodeId>,
}

/// Judgment about how two q grids were reconciled before subtraction.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "rule")]
pub enum GridJudgment {
    /// Grids identical point-by-point: direct subtraction is allowed.
    #[serde(rename = "grid_identical_direct_subtract")]
    Identical,
    /// Grids differ: no direct subtraction; buffer interpolated onto the
    /// sample grid, restricted to the overlapping q range only.
    #[serde(rename = "grid_misaligned_interpolate_overlap_only")]
    Misaligned {
        overlap_min: f64,
        overlap_max: f64,
        n_used: usize,
        n_dropped: usize,
    },
    /// No overlapping q range: subtraction refused.
    #[serde(rename = "grid_no_overlap_refused")]
    NoOverlap,
}

/// A fit over a q interval. Guinier and Porod intervals may overlap;
/// they are kept as separate candidates and never auto-merged.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IntervalCandidate {
    pub id: NodeId,
    pub model: String, // "guinier" | "porod"
    pub q_min: f64,
    pub q_max: f64,
    pub params: serde_json::Value,
    pub residuals: Vec<f64>,
    pub rms_residual: f64,
    pub adopted: bool,
}

/// A node in the dependency graph; every derivation is an independent version.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub version: u64,
    pub kind: String,
    /// Event sequence number that produced this version.
    pub event_seq: i64,
    pub state: NodeState,
    /// Direct dependency ids (sources of this derivation).
    pub deps: Vec<NodeId>,
    pub rule_version: u32,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", content = "reason")]
pub enum NodeState {
    #[serde(rename = "valid")]
    Valid,
    #[serde(rename = "dirty")]
    Dirty(String),
    #[serde(rename = "diagnostic")]
    Diagnostic(String),
}

/// An append-only event. Returning to an old event restores the dependency
/// versions and visible conclusions as of that event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}
