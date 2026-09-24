//! Domain model: curves, source metadata, node kinds, recipes.
//! All domain quantities carry units / coordinate basis in `SourceMeta`.
//! Raw representations are preserved at import; normalization happens only
//! in derived layers.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A scattering curve I(q) with per-point uncertainty.
/// Non-finite values (unconvertible imports) are kept in place as NaN and
/// serialized as null so nothing is silently dropped for plotting.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub sigma: Vec<f64>,
}

fn to_opt(v: &[f64]) -> Vec<Option<f64>> {
    v.iter().map(|x| if x.is_finite() { Some(*x) } else { None }).collect()
}
fn from_opt(v: &[Option<f64>]) -> Vec<f64> {
    v.iter().map(|x| x.unwrap_or(f64::NAN)).collect()
}

impl Serialize for Curve {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct C {
            q: Vec<Option<f64>>,
            i: Vec<Option<f64>>,
            sigma: Vec<Option<f64>>,
        }
        C { q: to_opt(&self.q), i: to_opt(&self.i), sigma: to_opt(&self.sigma) }.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Curve {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct C {
            q: Vec<Option<f64>>,
            i: Vec<Option<f64>>,
            sigma: Vec<Option<f64>>,
        }
        let c = C::deserialize(d)?;
        Ok(Curve { q: from_opt(&c.q), i: from_opt(&c.i), sigma: from_opt(&c.sigma) })
    }
}

impl Curve {
    pub fn len(&self) -> usize {
        self.q.len()
    }
    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }
    /// [min, max] of finite q values.
    pub fn finite_q_range(&self) -> Option<(f64, f64)> {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for q in &self.q {
            if q.is_finite() {
                lo = lo.min(*q);
                hi = hi.max(*q);
            }
        }
        if lo <= hi { Some((lo, hi)) } else { None }
    }
}

/// Metadata captured at import. Raw representation (`raw_csv`) is preserved
/// verbatim; derived layers normalize from these values.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceMeta {
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub transmission: f64,
    pub transmission_rel_unc: f64,
    #[serde(default)]
    pub buffer_refs: Vec<String>,
    #[serde(default)]
    pub instrument_response: Option<String>,
    #[serde(default)]
    pub mask: Option<String>,
    #[serde(default)]
    pub unit_q: String,
    #[serde(default)]
    pub unit_i: String,
    /// Original imported representation, kept verbatim.
    #[serde(default)]
    pub raw_csv: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    SourceCurve,
    Normalized,
    Subtracted,
    IntervalFit,
}

impl NodeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeKind::SourceCurve => "source_curve",
            NodeKind::Normalized => "normalized",
            NodeKind::Subtracted => "subtracted",
            NodeKind::IntervalFit => "interval_fit",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "source_curve" => Some(NodeKind::SourceCurve),
            "normalized" => Some(NodeKind::Normalized),
            "subtracted" => Some(NodeKind::Subtracted),
            "interval_fit" => Some(NodeKind::IntervalFit),
            _ => None,
        }
    }
}

/// A reproducible derivation recipe stored on every derived node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recipe {
    pub op: String, // "normalize" | "subtract" | "guinier" | "porod"
    pub inputs: Vec<String>,
    #[serde(default)]
    pub params: serde_json::Value,
    /// Candidate group (e.g. the sample a subtraction belongs to).
    #[serde(default)]
    pub group: Option<String>,
    /// Human label of this candidate (e.g. buffer version).
    #[serde(default)]
    pub candidate: Option<String>,
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
