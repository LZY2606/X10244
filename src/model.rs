//! 领域模型：原始记录、派生节点、拟合候选、机器可读事件。
//! 原则：原始表示永久保留；派生结果带稳定身份与来源边；多解与不确定状态不被吞掉。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::units::{Diagnostic, Quantity};

pub const RULESET_VERSION: &str = "saxs-rules-1.0.0";

/// 稳定身份：kind/序号 组成，导出导入后保持不变。
pub fn make_id(kind: &str, seq: u64) -> String {
    format!("{kind}/{seq:06}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Sample,
    Buffer,
    Instrument,
    Mask,
    Normalized,
    Subtracted,
    FitSet,
    Decision,
}

impl NodeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeKind::Sample => "sample",
            NodeKind::Buffer => "buffer",
            NodeKind::Instrument => "instrument",
            NodeKind::Mask => "mask",
            NodeKind::Normalized => "normalized",
            NodeKind::Subtracted => "subtracted",
            NodeKind::FitSet => "fitset",
            NodeKind::Decision => "decision",
        }
    }
}

/// 原始数据点：保留导入时的原始数值与单位，不做任何转换。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub q: f64,
    pub i: f64,
    pub err: f64,
    pub q_unit: String,
    pub i_unit: String,
}

/// 原始曲线记录（样品或缓冲液），连同归一化所需的全部源信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCurve {
    pub label: String,
    pub points: Vec<RawPoint>,
    pub exposure: Quantity,
    pub transmission: Quantity,
    pub thickness: Quantity,
    /// 相对不确定度（归一化因子），进入误差传播。
    pub exposure_rel_unc: f64,
    pub transmission_rel_unc: f64,
    pub thickness_rel_unc: f64,
    /// 缓冲液候选标识（样品记录可指向多个候选缓冲液）。
    pub buffer_candidates: Vec<String>,
    pub instrument_ref: Option<String>,
    pub mask_ref: Option<String>,
    /// 导入阶段产生的诊断（不可转换的单位等），值仍留在原始层。
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub label: String,
    pub response: Vec<(f64, f64)>,
    pub q_unit: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskSource {
    pub label: String,
    pub masked_ranges: Vec<(f64, f64)>,
    pub q_unit: String,
    pub source: String,
}

/// 派生层数据点：坐标基准已规范化（q: nm^-1, I: cm^-1）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DPoint {
    pub q: f64,
    pub i: f64,
    pub err: f64,
    #[serde(default)]
    pub negative: bool,
    #[serde(default)]
    pub flags: Vec<String>,
}

/// 派生点的来源：可追回两条源曲线（或一条源曲线 + 参数）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PointOrigin {
    pub node: String,
    pub index: usize,
    pub q: f64,
    pub i: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedCurve {
    pub label: String,
    pub points: Vec<DPoint>,
    /// 与 points 等长；每个派生点记录其来源点。
    pub origins: Vec<Vec<PointOrigin>>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

/// 区间拟合候选（Guinier 或 Porod）。重叠与多解全部保留。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitCandidate {
    pub id: String,
    pub kind: String, // "guinier" | "porod"
    pub q_min: f64,
    pub q_max: f64,
    pub params: Value,
    pub residuals: Vec<f64>,
    pub residual_norm: f64,
    pub overlaps_other: bool,
    pub status: String, // "candidate" | "adopted" | "rejected"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitSet {
    pub subtracted_ref: String,
    pub candidates: Vec<FitCandidate>,
    /// 显式记录：Guinier 与 Porod 区间不自动拼成单一模型。
    pub note: String,
}

/// 一次人工裁定：从多个候选中选择，未选中的保留。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub subject: String,
    pub chosen: Vec<String>,
    pub rejected: Vec<String>,
    pub rationale: String,
    pub author: String,
}

/// 节点数据负载。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeData {
    RawCurve(RawCurve),
    Instrument(InstrumentResponse),
    Mask(MaskSource),
    DerivedCurve(DerivedCurve),
    FitSet(FitSet),
    Decision(DecisionRecord),
}

/// 机器可读事件：判定依据、状态与受影响对象。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub kind: String,
    pub payload: Value,
    pub created_at: String,
}

impl Event {
    pub fn payload(kind: &str, fields: Value) -> (String, Value) {
        let mut p = fields;
        p["ruleset"] = json!(RULESET_VERSION);
        (kind.to_string(), p)
    }
}

pub fn now_iso() -> String {
    // 简化：unix 秒即可，稳定且可排序由 seq 保证。
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}
