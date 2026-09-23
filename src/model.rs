//! 领域模型：所有领域量携带单位或坐标基准；原始表示保留在 payload.raw_text。
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type Id = String;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub err: Vec<f64>,
}

impl Curve {
    pub fn len(&self) -> usize {
        self.q.len()
    }
    pub fn q_range(&self) -> Option<(f64, f64)> {
        if self.q.is_empty() {
            None
        } else {
            Some((self.q[0], self.q[self.q.len() - 1]))
        }
    }
}

/// 带单位/坐标基准的领域量。无法转换的值留在原位置并附诊断。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
    #[serde(default)]
    pub uncertainty: Option<f64>,
}

/// 机器可读取的判定事件。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: Id,
    pub seq: i64,
    pub kind: String,
    pub summary: String,
    pub data: Value,
    /// 受影响对象（记录 id 列表）。
    pub affects: Vec<Id>,
}

impl Event {
    pub fn new(kind: &str, summary: impl Into<String>, data: Value) -> Event {
        Event {
            id: String::new(),
            seq: 0,
            kind: kind.to_string(),
            summary: summary.into(),
            data,
            affects: Vec::new(),
        }
    }
}

/// 一条不可变记录（原始导入或派生版本）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub id: Id,
    pub node: String,
    pub version: u64,
    pub kind: String, // "source" | "derived"
    pub deps: Vec<Id>,
    pub rule: Option<Value>,
    pub payload: Value,
    pub created_seq: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conflict {
    pub id: Id,
    pub node: String,
    pub base_record: Id,
    pub branch_a: Id,
    pub branch_b: Id,
    pub status: String, // "open" | "resolved"
    pub resolution: Option<Id>,
    pub created_seq: i64,
}

// ---- 事件 kind 常量（机器可读取判定的词汇表）----
pub mod kinds {
    pub const IMPORT: &str = "import";
    pub const UNIT_CONVERTED: &str = "unit_converted";
    pub const UNIT_UNCONVERTIBLE: &str = "unit_unconvertible";
    pub const NORMALIZATION_UNCERTAINTY: &str = "normalization_uncertainty";
    pub const GRID_MISMATCH: &str = "grid_mismatch";
    pub const NO_OVERLAP: &str = "no_overlap_region";
    pub const BG_SCALE_UNCERTAINTY: &str = "background_scale_uncertainty";
    pub const NEGATIVE_INTENSITY: &str = "negative_intensity";
    pub const INTERVAL_OVERLAP: &str = "interval_overlap";
    pub const MODELS_NOT_MERGED: &str = "models_not_merged";
    pub const BUFFER_SWITCH: &str = "buffer_version_switch";
    pub const MULTI_CANDIDATE: &str = "multi_candidate_residuals";
    pub const RECOMPUTE: &str = "recompute";
    pub const DECISION: &str = "decision";
    pub const CONFLICT_DETECTED: &str = "conflict_detected";
    pub const CONFLICT_RESOLVED: &str = "conflict_resolved";
    pub const PARAM_REVISION: &str = "param_revision";
    pub const SOURCE_REVISION: &str = "source_revision";
}
