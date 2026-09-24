//! 领域对象：原始记录与各派生层。
//!
//! 语义约束（测试与重放案例守护）：
//! * 原始 q/I/误差、曝光、厚度、透射率、缓冲液候选、仪器响应、遮罩来源全部保留；
//! * 导入层保留原始表示与单位，规范化只发生在派生层；
//! * 无法转换的值留在原位置并附诊断，绝不丢弃；
//! * q 网格不同时不直接相减，插值只在重叠范围生效，不做外推；
//! * 误差传播同时包含归一化不确定度与背景缩放不确定度；
//! * Guinier 与 Porod 区间只产出候选，绝不自动拼成单一模型。

use serde::{Deserialize, Serialize};

pub const RULE_VERSION: &str = "saxs-rules-1.0.0";

/// 带单位与坐标基准的量。`raw_repr` 保留导入时的原始表示。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Qty {
    pub value: f64,
    pub unit: String,
    /// 坐标基准/物理含义，例如 "exposure:s"、"thickness:cm"、"transmission:fraction"。
    pub basis: String,
    pub raw_repr: String,
}

impl Qty {
    pub fn new(value: f64, unit: &str, basis: &str) -> Self {
        Qty { value, unit: unit.to_string(), basis: basis.to_string(), raw_repr: format!("{value} {unit}") }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ConversionDiagnostic {
    pub field: String,
    pub point_id: Option<String>,
    pub from_unit: String,
    pub to_unit: String,
    pub reason: String,
    pub code: String,
}

/// 仪器响应（绝对刻度来源 + 归一化相对不确定度）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct InstrumentResponse {
    pub source: String,
    /// 水标/玻璃碳等给出的绝对刻度因子；无量纲刻度。
    pub absolute_scale: Option<f64>,
    pub scale_unit: String,
    /// 归一化相对不确定度（透射率/厚度/曝光/刻度的合成相对方差），分数形式。
    pub norm_rel_uncertainty: f64,
    pub notes: String,
}

/// 遮罩来源。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Mask {
    pub source: String,
    pub kind: String,
    pub notes: String,
}

/// 原始二维积分曲线上的一个点：原始单位原样保留。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub id: String,
    pub ordinal: usize,
    pub q: f64,
    pub q_unit: String,
    pub i: f64,
    pub i_unit: String,
    pub sigma: f64,
    pub masked: bool,
    pub mask_reason: Option<String>,
}

/// 原始曲线（样品或缓冲液）及其采集元数据。
/// 采集量全部携带单位与坐标基准，导入时保留 raw_* 表示。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RawCurve {
    pub id: String,
    pub role: String, // "sample" | "buffer"
    pub name: String,
    pub points: Vec<RawPoint>,
    pub exposure: Qty,
    pub thickness: Option<Qty>,
    pub transmission: Qty,
    pub instrument: InstrumentResponse,
    pub mask: Mask,
    /// 原始导入文本片段（审计用）。
    pub source_excerpt: String,
}

/// 规范化（绝对刻度）曲线上的一个点。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NormPoint {
    pub id: String,
    pub q: f64,
    pub q_unit: String, // 规范化后统一为 1/nm
    pub i_abs: f64,
    pub sigma_stat: f64, // 统计误差（绝对刻度）
    pub sigma_norm: f64, // 归一化不确定度贡献（绝对刻度）
    pub sigma_total: f64,
    pub source_point_id: String,
    pub masked: bool,
    pub converted: bool,
    pub conversion_diagnostic: Option<ConversionDiagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NormCurve {
    pub id: String,
    pub source_id: String,
    pub factor: f64,
    pub factor_expr: String,
    pub q_unit: String,
    pub i_unit: String,
    pub norm_rel_uncertainty: f64,
    pub diagnostics: Vec<ConversionDiagnostic>,
    pub points: Vec<NormPoint>,
}

/// 背景缩放 α（在重叠区对缓冲液做最小二乘缩放）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BackgroundScale {
    pub alpha: f64,
    pub sigma_alpha: f64,
    pub method: String,
    pub q_min: f64,
    pub q_max: f64,
    pub n_pairs: usize,
}

/// 扣背景结果上的一个点。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SubPoint {
    pub id: String,
    pub ordinal: usize,
    pub q: f64,
    pub q_unit: String,
    pub i_sub: f64,
    pub sigma_stat: f64,
    pub sigma_norm: f64,
    pub sigma_scale: f64, // 背景缩放不确定度贡献
    pub sigma_total: f64,
    pub sample_point_id: String,
    /// 缓冲液点为插值得到时记录两个锚点；网格严格相等时记录单点。
    pub buffer_point_ids: Vec<String>,
    pub interpolated: bool,
    pub in_overlap: bool,
    pub negative: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Subtraction {
    pub id: String,
    pub sample_norm_id: String,
    pub buffer_norm_id: String,
    pub buffer_candidate_ids: Vec<String>,
    pub grids_match: bool,
    pub q_min: Option<f64>,
    pub q_max: Option<f64>,
    pub has_overlap: bool,
    pub scale: Option<BackgroundScale>,
    pub diagnostics: Vec<ConversionDiagnostic>,
    pub points: Vec<SubPoint>,
}

/// 区间模型种类。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Guinier,
    Porod,
}

/// 一个区间拟合候选（Guinier 或 Porod）。多个候选全部保留。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IntervalCandidate {
    pub id: String,
    pub kind: ModelKind,
    pub q_min: f64,
    pub q_max: f64,
    pub n_points: usize,
    /// Guinier: Rg (nm), I0；Porod: 指数 power、前因子。
    pub rg: Option<f64>,
    pub i0: Option<f64>,
    pub power: Option<f64>,
    pub amplitude: Option<f64>,
    pub r_squared: f64,
    pub qrg_max: Option<f64>,
    pub qrg_min: Option<f64>,
    pub rejected: bool,
    pub reject_reasons: Vec<String>,
    pub source_point_ids: Vec<String>,
}

/// 区间分析结果：Guinier / Porod 候选并存，标记重叠但绝不自动拼接。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IntervalAnalysis {
    pub id: String,
    pub subtraction_id: String,
    pub guinier: Vec<IntervalCandidate>,
    pub porod: Vec<IntervalCandidate>,
    /// 区间重叠的候选对（只报告，不选择）。
    pub overlaps: Vec<(String, String)>,
    pub no_single_model_joined: bool,
}

/// 版本化实体负载。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data")]
pub enum Payload {
    Raw(RawCurve),
    Norm(NormCurve),
    Sub(Subtraction),
    Intervals(IntervalAnalysis),
}

impl Payload {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Payload::Raw(_) => "raw",
            Payload::Norm(_) => "norm",
            Payload::Sub(_) => "sub",
            Payload::Intervals(_) => "intervals",
        }
    }
    pub fn entity_id(&self) -> &str {
        match self {
            Payload::Raw(c) => &c.id,
            Payload::Norm(c) => &c.id,
            Payload::Sub(c) => &c.id,
            Payload::Intervals(c) => &c.id,
        }
    }
}
