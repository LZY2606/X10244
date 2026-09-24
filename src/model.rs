use serde::{Deserialize, Serialize};

pub type Seq = i64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Sample,
    Buffer,
    Instrument,
    Mask,
}

impl RecordKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordKind::Sample => "sample",
            RecordKind::Buffer => "buffer",
            RecordKind::Instrument => "instrument",
            RecordKind::Mask => "mask",
        }
    }
    pub fn parse(s: &str) -> RecordKind {
        match s {
            "sample" => RecordKind::Sample,
            "buffer" => RecordKind::Buffer,
            "instrument" => RecordKind::Instrument,
            _ => RecordKind::Mask,
        }
    }
}

/// A domain quantity always carries its raw representation plus unit/coordinate
/// basis. Unconvertible values stay in place with a diagnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quantity {
    pub name: String,
    pub raw: String,
    pub value: Option<f64>,
    pub unit: Option<String>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurveData {
    pub q_unit: String,
    pub i_unit: String,
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub err: Vec<f64>,
}

/// An imported raw record. `raw_text` is preserved verbatim; parsing into
/// `curve`/`quantities` never destroys the original representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawRecord {
    pub id: String,
    pub version: i64,
    pub kind: RecordKind,
    pub raw_text: String,
    pub quantities: Vec<Quantity>,
    pub curve: Option<CurveData>,
    pub created_event: Seq,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DepKind {
    Record,
    Node,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepRef {
    pub kind: DepKind,
    pub id: String,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Valid,
    Stale,
    Failed,
}

impl NodeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeStatus::Valid => "valid",
            NodeStatus::Stale => "stale",
            NodeStatus::Failed => "failed",
        }
    }
    pub fn parse(s: &str) -> NodeStatus {
        match s {
            "stale" => NodeStatus::Stale,
            "failed" => NodeStatus::Failed,
            _ => NodeStatus::Valid,
        }
    }
}

/// A derivation node. Every derivation produces an independent version; the
/// current pointer only moves after a fully successful commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub version: i64,
    pub kind: String,
    pub params: serde_json::Value,
    pub deps: Vec<DepRef>,
    pub status: NodeStatus,
    pub result: Option<serde_json::Value>,
    pub diagnostics: Vec<Diagnostic>,
    pub input_hash: String,
    pub created_event: Seq,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: Seq,
    pub kind: String,
    pub actor: String,
    pub base_seq: Seq,
    pub payload: serde_json::Value,
    pub created_at: String,
}
