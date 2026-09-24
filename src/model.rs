use serde::{Deserialize, Serialize};

/// Canonical q unit used by the derived layer. Raw layers keep their own units.
pub const CANON_Q_UNIT: &str = "1/A";
pub const CANON_I_UNIT: &str = "1/cm";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub err: Vec<f64>,
    pub q_unit: String,
    pub i_unit: String,
}

impl Curve {
    pub fn len(&self) -> usize {
        self.q.len()
    }
    pub fn q_range(&self) -> (f64, f64) {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for &v in &self.q {
            if v < lo {
                lo = v;
            }
            if v > hi {
                hi = v;
            }
        }
        (lo, hi)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SampleMeta {
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub transmission: f64,
    /// Relative normalization uncertainty (e.g. 0.02 = 2%).
    pub norm_unc: f64,
    #[serde(default)]
    pub buffer_candidates: Vec<String>,
    #[serde(default)]
    pub instrument_response: String,
    #[serde(default)]
    pub mask_source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op")]
pub enum Recipe {
    #[serde(rename = "normalize")]
    Normalize { source: String },
    #[serde(rename = "subtract")]
    Subtract {
        sample: String,
        buffer: String,
        scale: f64,
        scale_unc: f64,
        candidates: Vec<f64>,
    },
    #[serde(rename = "intervals")]
    Intervals {
        source: String,
        guinier_lens: Vec<usize>,
        porod_lens: Vec<usize>,
    },
}

/// Provenance of one derived point: which sample point and which buffer
/// bracketing points (with interpolation coefficient) produced it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PointProv {
    pub sample_node: String,
    pub sample_idx: usize,
    pub buffer_node: String,
    pub buffer_lo: usize,
    pub buffer_hi: usize,
    /// Interpolation coefficient in [0,1]; 0 when grids matched exactly.
    pub t: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IntervalCand {
    pub family: String, // "guinier" | "porod"
    pub i0: usize,
    pub i1: usize,
    pub q_lo: f64,
    pub q_hi: f64,
    pub fit: serde_json::Value,
    #[serde(default)]
    pub overlaps: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ScaleCandidate {
    pub scale: f64,
    pub residual: f64,
    pub chosen: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<Curve>,
    /// Original representation exactly as imported (CSV text, JSON, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<SampleMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<Recipe>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<PointProv>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intervals: Vec<IntervalCand>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<ScaleCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

impl Node {
    pub fn new(id: &str, kind: &str, name: &str) -> Self {
        Node {
            id: id.to_string(),
            kind: kind.to_string(),
            name: name.to_string(),
            curve: None,
            raw: None,
            meta: None,
            recipe: None,
            provenance: vec![],
            intervals: vec![],
            candidates: vec![],
            diagnostics: vec![],
        }
    }
}

/// A machine-readable judgment produced by the pipeline.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewEvent {
    pub kind: String,
    pub payload: serde_json::Value,
}

impl NewEvent {
    pub fn new(kind: &str, payload: serde_json::Value) -> Self {
        NewEvent {
            kind: kind.to_string(),
            payload,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dep {
    pub id: String,
    pub hash: String,
}

/// FNV-1a 64 bit, hex encoded. Stable identity hash for node payloads.
pub fn content_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

pub fn node_hash(node: &Node) -> String {
    content_hash(&serde_json::to_string(node).unwrap_or_default())
}
