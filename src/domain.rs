//! 领域模型：原始表示（含单位/坐标基准）与派生记录。
//!
//! 规则：导入层只解析、不丢弃——无法转换为 SI/规范化单位的值留在
//! `RawCell.diagnostic` 中；派生层才做单位规范化。

use serde::{Deserialize, Serialize};
use std::fmt;

/// 带显式单位标签的标量。原始文本始终保留在 `raw_text`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub value: f64,
    pub unit: UnitLabel,
    pub raw_text: String,
    /// 规范化到派生层使用的基准单位后的值；无法转换时为 None。
    pub canonical: Option<f64>,
    pub diagnostic: Option<String>,
}

impl Quantity {
    pub fn new(value: f64, unit: UnitLabel, raw_text: impl Into<String>) -> Self {
        let raw_text = raw_text.into();
        let canonical = unit.to_canonical(value);
        Quantity { value, unit, raw_text, canonical, diagnostic: None }
    }

    pub fn invalid(raw_text: impl Into<String>, unit: UnitLabel, why: impl Into<String>) -> Self {
        Quantity {
            value: f64::NAN,
            unit,
            raw_text: raw_text.into(),
            canonical: None,
            diagnostic: Some(why.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitLabel {
    AngstromInv,
    NmInv,
    CountsPerSecond,
    Counts,
    Seconds,
    Milliseconds,
    Centimetres,
    Millimetres,
    Dimensionless,
    Arbitrary,
    Unknown,
}

impl UnitLabel {
    pub fn parse(s: &str) -> UnitLabel {
        match s.trim().to_ascii_lowercase().as_str() {
            "a^-1" | "å^-1" | "ang^-1" | "angstrom^-1" | "1/a" | "1/å" | "a-1" => {
                UnitLabel::AngstromInv
            }
            "nm^-1" | "1/nm" | "nm-1" => UnitLabel::NmInv,
            "counts/s" | "cps" | "count/s" | "count per second" => UnitLabel::CountsPerSecond,
            "counts" | "count" => UnitLabel::Counts,
            "s" | "sec" | "second" | "seconds" => UnitLabel::Seconds,
            "ms" | "millisecond" | "milliseconds" => UnitLabel::Milliseconds,
            "cm" | "centimetre" | "centimetres" => UnitLabel::Centimetres,
            "mm" | "millimetre" | "millimetres" => UnitLabel::Millimetres,
            "" | "1" | "-" | "dimensionless" => UnitLabel::Dimensionless,
            "a.u." | "au" | "arbitrary" => UnitLabel::Arbitrary,
            _ => UnitLabel::Unknown,
        }
    }

    /// 派生层规范化：q -> Å^-1；曝光 -> s；厚度 -> cm；强度列保留计数语义。
    pub fn to_canonical(&self, v: f64) -> Option<f64> {
        match self {
            UnitLabel::AngstromInv
            | UnitLabel::CountsPerSecond
            | UnitLabel::Counts
            | UnitLabel::Seconds
            | UnitLabel::Centimetres
            | UnitLabel::Dimensionless
            | UnitLabel::Arbitrary => Some(v),
            UnitLabel::NmInv => Some(v / 10.0),
            UnitLabel::Milliseconds => Some(v / 1000.0),
            UnitLabel::Millimetres => Some(v / 10.0),
            UnitLabel::Unknown => None,
        }
    }

    pub fn canonical_unit(&self) -> &'static str {
        match self {
            UnitLabel::AngstromInv | UnitLabel::NmInv => "Å^-1",
            UnitLabel::CountsPerSecond => "counts/s",
            UnitLabel::Counts => "counts",
            UnitLabel::Seconds | UnitLabel::Milliseconds => "s",
            UnitLabel::Centimetres | UnitLabel::Millimetres => "cm",
            UnitLabel::Dimensionless => "1",
            UnitLabel::Arbitrary => "a.u.",
            UnitLabel::Unknown => "?",
        }
    }
}

/// 原始表格中的一个单元格：值、原始文本、列单位与诊断。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCell {
    pub text: String,
    pub numeric: Option<f64>,
    pub unit: UnitLabel,
    pub canonical: Option<f64>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawColumn {
    pub name: String,
    pub unit: UnitLabel,
    pub cells: Vec<RawCell>,
}

/// 导入的原始 q/I/误差表，逐格保留原始表示。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawTable {
    pub columns: Vec<RawColumn>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentResponse {
    /// 平坦响应因子（1 表示已做仪器响应校正）。
    pub factor: f64,
    pub rel_uncertainty: f64,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskSource {
    pub file: String,
    pub sha256: String,
    pub masked_pixels: usize,
}

/// 一条原始材料（样品或缓冲液）的某个不可变版本。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialVersion {
    pub id: String,
    pub kind: MaterialKind,
    pub label: String,
    pub version: u32,
    pub supersedes: Option<String>,
    pub raw: RawTable,
    pub exposure: Quantity,
    pub thickness: Quantity,
    pub transmission: Quantity,
    pub instrument: InstrumentResponse,
    pub mask: MaskSource,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Sample,
    Buffer,
}

impl fmt::Display for MaterialKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MaterialKind::Sample => write!(f, "sample"),
            MaterialKind::Buffer => write!(f, "buffer"),
        }
    }
}

// ---------- 派生层 ----------

/// 派生点回溯到的源点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointRef {
    pub material_id: String,
    pub material_version: u32,
    pub row: usize,
    pub q: f64,
}

/// 归一化曲线上的点（单位：q Å^-1，I counts/s/cm/transmission 标度，保留原始标度说明）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormPoint {
    pub id: String,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub sigma_norm: f64,
    pub refs: Vec<PointRef>,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedArtifact {
    pub points: Vec<NormPoint>,
    pub exposure_s: Option<f64>,
    pub thickness_cm: Option<f64>,
    pub transmission: Option<f64>,
    pub response_factor: f64,
    pub diagnostics: Vec<String>,
}

/// 缓冲液缩放到重叠 q 网格后的点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScaledPoint {
    pub id: String,
    pub q: f64,
    pub i_buf: f64,
    pub sigma_buf: f64,
    pub refs: Vec<PointRef>,
    pub interpolated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScaledBufferArtifact {
    pub alpha: f64,
    pub alpha_sigma: f64,
    pub method: String,
    pub q_min: f64,
    pub q_max: f64,
    pub points: Vec<ScaledPoint>,
    pub diagnostics: Vec<String>,
}

/// 扣背景后的点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPoint {
    pub id: String,
    pub q: f64,
    pub i_sub: f64,
    pub sigma_sub: f64,
    pub i_sample: f64,
    pub i_buffer_scaled: f64,
    pub refs_sample: Vec<PointRef>,
    pub refs_buffer: Vec<PointRef>,
    pub interpolated_buffer: bool,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtractedArtifact {
    pub points: Vec<SubPoint>,
    pub alpha: f64,
    pub alpha_sigma: f64,
    pub n_negative: usize,
    pub diagnostics: Vec<String>,
}

/// 模型区间候选（Guinier / Porod）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowFit {
    pub id: String,
    pub model: String,
    pub q_lo: f64,
    pub q_hi: f64,
    pub params: Vec<(String, f64)>,
    pub chi2: f64,
    pub residual_rms: f64,
    pub n_points: usize,
    pub overlap_with: Vec<String>,
    pub q_values: Vec<f64>,
    pub fit_values: Vec<f64>,
    pub residuals: Vec<f64>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelFitArtifact {
    pub windows: Vec<WindowFit>,
    pub diagnostics: Vec<String>,
}

/// 派生记录的负载（成功产物或阻断诊断）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DerivedRecord {
    Normalized(NormalizedArtifact),
    ScaledBuffer(ScaledBufferArtifact),
    Subtracted(SubtractedArtifact),
    ModelFit(ModelFitArtifact),
    /// 输入不合法或 q 范围无重叠——不产生曲线，只留判定。
    Blocked { reason: String, diagnostics: Vec<String> },
}

impl DerivedRecord {
    pub fn kind_name(&self) -> &'static str {
        match self {
            DerivedRecord::Normalized(_) => "normalized",
            DerivedRecord::ScaledBuffer(_) => "scaled_buffer",
            DerivedRecord::Subtracted(_) => "subtracted",
            DerivedRecord::ModelFit(_) => "model_fit",
            DerivedRecord::Blocked { .. } => "blocked",
        }
    }
}

/// 派生节点的一个不可变版本。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivedVersion {
    pub node_id: String,
    pub version: u32,
    pub node_kind: String,
    pub inputs: Vec<String>,
    pub input_fingerprint: String,
    pub rule: String,
    pub record: DerivedRecord,
    pub supersedes: Option<String>,
}

// ---------- 事件（机器可读的判断依据） ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Import,
    Revision,
    Derived,
    RuleVerdict,
    Decision,
    Conflict,
    ConflictResolved,
    Replay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    Accept,
    Reject,
    Choose,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub event_type: EventType,
    pub actor: String,
    pub summary: String,
    /// 判断依据，例如 `{"rule":"no_direct_subtract_on_mismatched_grids",...}`。
    pub evidence: serde_json::Value,
    pub affected: Vec<String>,
    /// 事件之后相关节点的当前版本指针（历史回放按此恢复）。
    pub head_after: serde_json::Value,
    pub base_event_seq: Option<i64>,
}

/// 依赖图中一个节点为何仍有效/需重算的说明。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeValidity {
    pub node_id: String,
    pub current_version: u32,
    pub state: String,
    pub reason: String,
    pub current_fingerprint: String,
    pub stored_fingerprint: String,
    pub inputs: Vec<String>,
}
