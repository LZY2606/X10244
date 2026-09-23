//! 存储模型：原始材料（source-of-record）与派生节点（derived graph）。
//!
//! RULE_PROVENANCE_V1：原始 q/I/误差、曝光、样品厚度、透射率、缓冲液候选、
//! 仪器响应和遮罩来源全部逐字保留；派生层另存规范化结果，绝不覆写原始表示。

use serde::{Deserialize, Serialize};
use crate::units::Quant;

pub const RULE_PROVENANCE: &str = "rules/provenance@1";

/// 二维积分导入后的一条原始记录，全部字段保留原始文本表示。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CurveRow {
    pub q_text: String,
    pub q_unit: String,
    pub i_text: String,
    pub i_unit: String,
    pub err_text: String,
    pub err_unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Material {
    pub id: String,
    pub kind: MaterialKind,
    pub label: String,
    pub revision: u32,
    /// 导入时逐字保留的原始来源（文件/记录标识）。
    pub source_ref: String,
    pub exposure: Option<Quant>,
    pub thickness: Option<Quant>,
    pub transmission: Option<Quant>,
    /// 仪器响应来源（文件名、逐字元数据），归一化时使用；可为空。
    pub instrument_ref: Option<String>,
    /// 遮罩来源（文件名、逐字元数据），以及被遮罩的原始 q 区间文本。
    pub mask: Option<MaskSource>,
    /// 样品材料可用的缓冲液候选 id 列表（逐字保留候选关系）。
    pub buffer_candidates: Vec<String>,
    pub rows: Vec<CurveRow>,
    pub note: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Sample,
    Buffer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MaskSource {
    pub file: String,
    /// 被遮罩点/区间的原始描述，逐字保留，例如 ["q<0.01","q>0.3","beamstop"]。
    pub ranges: Vec<String>,
}

/// 规范化曲线上的点。point_id 在版本内稳定，导出/删除/再导入后保持。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub point_id: String,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    /// 该点是否由缓冲液曲线插值得到（样品网格上的派生点）。
    pub interpolated: bool,
    /// 可追溯的来源点 id（可能多条：例如插值来自两个相邻缓冲液点）。
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Normalized {
    pub material_id: String,
    pub material_rev: u32,
    pub q_unit: String,
    pub i_unit: String,
    pub exposure_s: Option<f64>,
    pub thickness_cm: Option<f64>,
    pub transmission: Option<f64>,
    pub instrument_ref: Option<String>,
    pub mask_applied: Option<MaskSource>,
    /// 被遮罩而剔除的原始行序号（0-based）。
    pub masked_rows: Vec<usize>,
    /// 无法单位转换而保留在原位置的行诊断。
    pub row_diagnostics: Vec<(usize, String)>,
    pub points: Vec<Point>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScaleFit {
    pub fit_range_q: [f64; 2],
    pub scale: f64,
    pub scale_sigma: f64,
    pub n_pairs: usize,
    pub degrees_of_freedom: usize,
    pub chi2: f64,
    pub method: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubtractionStatus {
    Ok,
    /// q 网格无重叠，未产生任何扣背景点。
    NoOverlap,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Subtraction {
    pub sample_node: String,
    pub buffer_node: String,
    pub sample_mat_rev: u32,
    pub buffer_mat_rev: u32,
    /// 参与拟合的重叠 q 范围 [qmin, qmax]（网格不同时仅重叠区有效）。
    pub overlap_range_q: Option<[f64; 2]>,
    pub scale_fit: Option<ScaleFit>,
    pub status: SubtractionStatus,
    pub points: Vec<Point>,
    /// 负强度点（i + sigma < 0）的 point_id。
    pub negative_points: Vec<String>,
    /// 背景缩放不确定度对总误差的贡献量级（最大相对值）。
    pub scale_uncertainty_rel_max: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegionFit {
    pub kind: RegionKind,
    pub range_q: [f64; 2],
    pub n_points: usize,
    /// Guinier: Rg, I0；Porod: C（幂律系数）、指数 p。
    pub params: Vec<NamedParam>,
    pub residual_rms: f64,
    pub chi2: f64,
    pub degrees_of_freedom: usize,
    pub residuals: Vec<Residual>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegionKind {
    Guinier,
    Porod,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamedParam {
    pub name: String,
    pub value: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Residual {
    pub point_id: String,
    pub q: f64,
    pub observed: f64,
    pub predicted: f64,
    pub weighted_residual: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelFit {
    pub subtraction_node: String,
    /// 区间候选之间的 q 重叠对（range 索引）。Guinier 与 Porod 绝不自动拼成单一模型。
    pub region_overlaps: Vec<[usize; 2]>,
    pub regions: Vec<RegionFit>,
}

/// 派生节点的版本内容。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Payload {
    Normalized(Normalized),
    Subtraction(Subtraction),
    ModelFit(ModelFit),
}

impl Payload {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Payload::Normalized(_) => "normalized",
            Payload::Subtraction(_) => "subtraction",
            Payload::ModelFit(_) => "model_fit",
        }
    }
    pub fn points(&self) -> Vec<&Point> {
        match self {
            Payload::Normalized(n) => n.points.iter().collect(),
            Payload::Subtraction(s) => s.points.iter().collect(),
            Payload::ModelFit(_) => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Normalized,
    Subtraction,
    ModelFit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeVersion {
    pub node_id: String,
    pub node_kind: NodeKind,
    pub ordinal: u32,
    pub version_id: String,
    /// 输入版本（内容寻址），顺序稳定。
    pub inputs: Vec<String>,
    /// 计算规则版本。
    pub rule: String,
    pub params_json: String,
    pub payload: Payload,
    /// 机器可读判定，例如 no_overlap、scale_uncertainty、negative_intensity…
    pub diagnostics: Vec<Diagnostic>,
    pub created_event: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    /// 可选的受影响 point_id 或区间。
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}
