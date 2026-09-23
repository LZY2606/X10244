//! 领域模型：所有领域量携带单位或坐标基准；导入层保留原始表示，派生层才规范化。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CurveKind {
    SampleRaw,
    BufferRaw,
    Normalized,
    Subtracted,
}

/// 一条散射曲线。q_unit / i_unit 是坐标基准，导入时原样保留。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Curve {
    pub id: String,
    pub kind: CurveKind,
    pub q_unit: String,
    pub i_unit: String,
    pub points: Vec<CurvePoint>,
    pub diagnostics: Vec<String>,
}

/// 原始标量：保留原始字符串表示；无法转换时值留在原位置并附诊断。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawScalar {
    pub raw: String,
    pub value: Option<f64>,
    pub unit: String,
    pub rel_unc: f64,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCurve {
    pub id: String,
    pub q_unit: String,
    pub i_unit: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<f64>>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferCandidate {
    pub id: String,
    pub label: String,
    pub curve: RawCurve,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub id: String,
    pub kind: String,
    pub source_file: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskSource {
    pub id: String,
    pub origin: String,
    pub source_file: String,
}

/// 导入记录：原始 q/I/误差、曝光、样品厚度、透射率、缓冲液候选、仪器响应和遮罩来源均保留。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRecord {
    pub id: String,
    pub sample: RawCurve,
    pub exposure: RawScalar,
    pub thickness: RawScalar,
    pub transmission: RawScalar,
    pub buffers: Vec<BufferCandidate>,
    pub instrument_response: InstrumentResponse,
    pub mask_source: MaskSource,
}

/// 背景扣除参数（一次参数修订即产生新版本）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtractionParams {
    pub buffer_id: String,
    pub scale: f64,
    pub scale_sigma: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PointFlag {
    NegativeIntensity,
    OutsideOverlap,
    InterpolatedBuffer,
}

/// 派生点回溯：每个派生点记录两条源曲线的读取位置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BufferSource {
    Direct { idx: usize },
    Interpolated { lo: usize, hi: usize, t: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedPoint {
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub flags: Vec<PointFlag>,
    pub sample_idx: usize,
    pub buffer: Option<BufferSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtractedCurve {
    pub id: String,
    pub q_unit: String,
    pub i_unit: String,
    pub params: SubtractionParams,
    pub points: Vec<DerivedPoint>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RangeKind {
    Guinier,
    Porod,
}

/// 区间候选：Guinier 与 Porod 不自动拼成单一模型；多候选全部保留。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RangeCandidate {
    pub id: String,
    pub kind: RangeKind,
    pub lo: usize,
    pub hi: usize,
    pub params: serde_json::Value,
    pub residual: f64,
    pub adopted: bool,
    pub overlaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RangeAnalysis {
    pub id: String,
    pub curve_id: String,
    pub candidates: Vec<RangeCandidate>,
    pub diagnostics: Vec<String>,
}

/// 机器可读取的判定事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub id: String,
    pub ts: String,
    pub kind: String,
    pub payload: serde_json::Value,
}
