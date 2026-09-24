use serde::{Deserialize, Serialize};

/// 原始数据点：保留导入时的原始表示（原始单位、原始数值）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub q: f64,
    pub q_unit: String,
    pub i: f64,
    pub sigma: f64,
}

/// 原始曲线 + 元数据。透射率、曝光、厚度等全部保留。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCurve {
    pub points: Vec<RawPoint>,
    pub exposure_s: f64,
    pub thickness_cm: f64,
    pub transmission: f64,
    pub description: String,
}

/// 规范化后的点。q 统一为 1/angstrom；无法转换单位的点保留原值并标记。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormPoint {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub convertible: bool,
    pub source_index: usize,
}

/// 机器可读取的诊断/判定事件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point_index: Option<usize>,
}

impl Diagnostic {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Diagnostic { code: code.to_string(), message: message.into(), point_index: None }
    }
    pub fn at(code: &str, message: impl Into<String>, idx: usize) -> Self {
        Diagnostic { code: code.to_string(), message: message.into(), point_index: Some(idx) }
    }
}

/// 背景扣除后的点。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubPoint {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub sample_index: usize,
    pub buffer_index: Option<usize>,
}

/// 多候选背景缩放的残差检验记录：全部候选保留，不吞掉多解。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScaleCandidate {
    pub scale: f64,
    pub residual: f64,
    pub adopted: bool,
}

/// 区间拟合（Guinier / Porod）。重叠区间各自独立保留，绝不自动合并。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntervalFit {
    pub kind: String,
    pub q_min: f64,
    pub q_max: f64,
    pub params: Vec<(String, f64)>,
    pub residual: f64,
    pub n_points: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalSpec {
    pub kind: String,
    pub q_min: f64,
    pub q_max: f64,
}
