use serde::{Deserialize, Serialize};

/// 诊断信息：无法转换或需要注意的值留在原位置并附诊断。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub indices: Vec<usize>,
}

/// 散射曲线。导入层保留原始表示（raw_text 与原始单位），
/// 派生层才做单位规范化。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurveData {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub err: Vec<f64>,
    pub q_unit: String,
    pub i_unit: String,
    pub raw_text: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl CurveData {
    pub fn len(&self) -> usize {
        self.q.len()
    }
}

/// 依赖引用：稳定身份 + 版本。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InputRef {
    pub id: String,
    pub version: u64,
}

/// 派生曲线（归一化或背景扣除结果）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DerivedCurve {
    pub rule: String, // "normalize" | "subtract"
    pub inputs: Vec<InputRef>,
    pub params: serde_json::Value,
    pub curve: CurveData,
    /// 插值生效的重叠范围（q 网格错位时）。
    pub overlap: Option<(f64, f64)>,
    /// 因无重叠而被丢弃的源点数。
    pub dropped_points: usize,
    pub flags: Vec<String>,
}

/// 区间模型候选（Guinier / Porod）。多候选全部保留，不自动合并。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub kind: String, // "guinier" | "porod"
    pub q_lo: f64,
    pub q_hi: f64,
    pub params: serde_json::Value,
    pub residual: Vec<f64>,
    /// proposed | accepted | rejected | unchosen
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalModel {
    pub rule: String, // "intervals"
    pub inputs: Vec<InputRef>,
    pub candidates: Vec<IntervalCandidate>,
}

/// 样品元数据：曝光、厚度、透射率（含不确定度），raw 保留原始记录。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SampleMeta {
    pub exposure_s: f64,
    pub thickness_cm: f64,
    pub transmission: f64,
    pub transmission_err: f64,
    pub raw: serde_json::Value,
}

/// 仪器响应与遮罩来源。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub description: String,
    pub q: Vec<f64>,
    pub response: Vec<f64>,
    pub q_unit: String,
    pub raw_text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaskSource {
    pub description: String,
    pub masked_ranges: Vec<(f64, f64)>,
    pub q_unit: String,
    pub origin: String,
}

/// 缓冲液候选记录。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BufferRecord {
    pub name: String,
    pub curve_id: String,
    pub prepared: String,
    pub note: String,
}
