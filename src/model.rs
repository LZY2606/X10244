//! 领域模型：实体、版本、来源链、事件。
use serde::{Deserialize, Serialize};

pub type EntityId = String;
pub type Version = u64;

/// 稳定身份：实体 id + 版本。导出/删除/再导入后保持不变。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeRef {
    pub id: EntityId,
    pub version: Version,
}

impl NodeRef {
    pub fn new(id: impl Into<String>, version: Version) -> Self {
        Self { id: id.into(), version }
    }
    pub fn label(&self) -> String {
        format!("{}@v{}", self.id, self.version)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityKind {
    RawCurve,        // 原始 q/I/误差曲线（导入，不修改）
    Instrument,      // 仪器响应 + 遮罩来源
    NormalizedCurve, // 归一化曲线
    Subtraction,     // 背景扣除结果（含判定）
    IntervalModel,   // Guinier / Porod 区间候选
    Decision,        // 人工裁定
}

impl EntityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityKind::RawCurve => "raw_curve",
            EntityKind::Instrument => "instrument",
            EntityKind::NormalizedCurve => "normalized_curve",
            EntityKind::Subtraction => "subtraction",
            EntityKind::IntervalModel => "interval_model",
            EntityKind::Decision => "decision",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "raw_curve" => EntityKind::RawCurve,
            "instrument" => EntityKind::Instrument,
            "normalized_curve" => EntityKind::NormalizedCurve,
            "subtraction" => EntityKind::Subtraction,
            "interval_model" => EntityKind::IntervalModel,
            "decision" => EntityKind::Decision,
            _ => return None,
        })
    }
}

/// 来源链：每个派生实体记录直接输入（含版本）与规则版本。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub inputs: Vec<NodeRef>,
    pub rule: String,        // 规则名，如 "subtract/v1"
    pub rule_params: serde_json::Value,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub node: NodeRef,
    pub kind: EntityKind,
    pub name: String,
    pub payload: serde_json::Value,
    pub source: Provenance,
    pub created_event: u64,
}

/// 机器可读取的判定事件（对应“判断依据写成机器可读取的事件”）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub code: String,           // 如 "grid_mismatch_interpolated" / "no_overlap" / "negative_intensity"
    pub status: VerdictStatus,
    pub detail: String,
    pub affected: Vec<NodeRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerdictStatus {
    Ok,
    Warning,  // 可继续但需审阅
    Blocked,  // 不产生结果（如无重叠区）
    Multi,    // 多候选，无法唯一决定
}

impl VerdictStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            VerdictStatus::Ok => "ok",
            VerdictStatus::Warning => "warning",
            VerdictStatus::Blocked => "blocked",
            VerdictStatus::Multi => "multi",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}
