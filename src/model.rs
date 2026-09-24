//! 领域模型：所有领域量携带单位或坐标基准；原始表示保留在源节点，
//! 派生层才做规范化。无法转换的值留在原位置并附诊断。

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeRef {
    pub id: String,
    pub version: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diag {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub points: Vec<usize>,
    #[serde(default)]
    pub detail: serde_json::Value,
}

impl Diag {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Diag { code: code.to_string(), message: message.into(), points: vec![], detail: serde_json::Value::Null }
    }
}

/// 一条散射曲线（原始或派生）。单位以字符串保留原始表示。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Curve {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub err: Vec<f64>,
    pub q_unit: String,
    pub i_unit: String,
    #[serde(default)]
    pub q_basis: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Meta {
    pub exposure_s: f64,
    pub exposure_s_err: f64,
    pub thickness_mm: f64,
    pub thickness_mm_err: f64,
    pub transmission: f64,
    pub transmission_err: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instrument {
    pub q: Vec<f64>,
    pub factors: Vec<f64>,
    pub q_unit: String,
    /// 原始组件；不可转换的保留在原位置并附诊断
    #[serde(default)]
    pub components: Vec<serde_json::Value>,
    #[serde(default)]
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mask {
    pub source: String,
    pub excluded_ranges: Vec<(f64, f64)>,
    pub q_unit: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ScaleMode {
    Fitted,
    Fixed { value: f64 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScaleCandidate {
    pub id: String,
    pub scale: f64,
    pub scale_err: f64,
    pub i: Vec<f64>,
    pub err: Vec<f64>,
    pub residuals: Vec<f64>,
    pub chi2: f64,
    pub adopted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PointSource {
    pub sample_index: usize,
    pub buffer_indices: Vec<usize>,
    pub buffer_interpolated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subtracted {
    pub q: Vec<f64>,
    pub q_unit: String,
    pub candidates: Vec<ScaleCandidate>,
    pub excluded_points: Vec<usize>,
    pub flags: Vec<(usize, String)>,
    pub point_sources: Vec<Option<PointSource>>,
    pub overlap: Option<(f64, f64)>,
    pub blocked: bool,
    pub diagnostics: Vec<Diag>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub model: String, // "guinier" | "porod"
    pub qmin: f64,
    pub qmax: f64,
    pub params: serde_json::Value,
    pub chi2: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Intervals {
    pub guinier: Vec<IntervalCandidate>,
    pub porod: Vec<IntervalCandidate>,
    pub overlap: bool,
    pub note: String,
}
