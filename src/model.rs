use serde::{Deserialize, Serialize};

pub type Id = String;

pub fn new_id(prefix: &str) -> Id {
    format!("{}-{}", prefix, uuid::Uuid::new_v4().simple())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub q: f64,
    pub i: f64,
    pub err: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurveData {
    pub unit_q: String,
    pub unit_i: String,
    pub points: Vec<CurvePoint>,
}

impl CurveData {
    pub fn q_range(&self) -> Option<(f64, f64)> {
        let first = self.points.first()?;
        let last = self.points.last()?;
        Some((first.q, last.q))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub at_q: Option<f64>,
}

impl Diagnostic {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Diagnostic { code: code.to_string(), message: message.into(), at_q: None }
    }
    pub fn at(code: &str, message: impl Into<String>, q: f64) -> Self {
        Diagnostic { code: code.to_string(), message: message.into(), at_q: Some(q) }
    }
}

/// Raw import metadata: original representation is preserved verbatim;
/// normalization happens only in derived layers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawMeta {
    pub exposure_s: Option<f64>,
    pub thickness_mm: Option<f64>,
    pub transmission: Option<f64>,
    pub instrument_response: Option<String>,
    pub mask_source: Option<String>,
    pub buffer_version: Option<String>,
    pub raw_text: String,
    /// Values that could not be converted stay in place with a diagnosis.
    pub unconverted: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputRef {
    pub entity: Id,
    pub version: Id,
    pub role: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub label: String,
    /// pending | accepted | rejected
    pub status: String,
    /// Machine-readable judgment basis.
    pub basis: serde_json::Value,
    pub curve: Option<CurveData>,
    pub metrics: serde_json::Value,
}

/// Payload of a version: a node in the derivation graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub rule: String,
    pub inputs: Vec<InputRef>,
    pub params: serde_json::Value,
    pub curve: Option<CurveData>,
    pub diagnostics: Vec<Diagnostic>,
    pub candidates: Vec<Candidate>,
    pub meta: Option<RawMeta>,
}

impl Node {
    pub fn empty(rule: &str) -> Self {
        Node {
            rule: rule.to_string(),
            inputs: vec![],
            params: serde_json::json!({}),
            curve: None,
            diagnostics: vec![],
            candidates: vec![],
            meta: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: Id,
    pub seq: i64,
    pub kind: String,
    pub summary: String,
    pub inputs: Vec<InputRef>,
    pub outputs: Vec<(Id, Id)>,
    /// Machine-readable judgment basis.
    pub basis: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub id: Id,
    pub entity: Id,
    pub seq: i64,
    pub node: Node,
    pub event_id: Id,
    pub stale: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entity {
    pub id: Id,
    pub kind: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conflict {
    pub id: Id,
    pub entity: Id,
    pub base_version: Id,
    pub branch_a: Id,
    pub branch_b: Id,
    /// open | resolved
    pub status: String,
    pub resolved_event: Option<Id>,
}
