//! SAXS 领域类型：所有领域量携带单位或坐标基准。
//! 导入层保留原始表示，派生层才做规范化；无法转换的值留在原位置并附诊断。

use serde::{Deserialize, Serialize};

/// 机器可读取的诊断/判定依据
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diag {
    pub code: String,
    pub message: String,
    /// 受影响对象（点序号、区间、对象 id 等）
    pub subject: String,
}

impl Diag {
    pub fn new(code: &str, message: impl Into<String>, subject: impl Into<String>) -> Self {
        Diag { code: code.to_string(), message: message.into(), subject: subject.into() }
    }
}

/// 原始曲线点（导入层，保留原始单位）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub q: f64,
    pub i: f64,
    pub err: f64,
}

/// 一条导入的散射曲线（原始表示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCurve {
    pub q_unit: String, // "nm^-1" | "A^-1" | 其他（不可转换 -> 诊断）
    pub i_unit: String, // "cm^-1" | "arb" | ...
    pub points: Vec<RawPoint>,
    pub origin: String, // 来源描述（文件/仪器）
}

/// 规范化后的曲线点（派生层，q 统一为 nm^-1）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub q: f64,       // nm^-1
    pub i: f64,       // 归一化强度
    pub err: f64,     // 含归一化与背景缩放不确定度
    pub flags: Vec<String>, // 如 "negative_intensity"
    /// 点级溯源：样本点序号与缓冲液插值所用点序号
    pub src: Option<PointSrc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PointSrc {
    pub sample_idx: usize,
    pub buffer_idx_lo: Option<usize>,
    pub buffer_idx_hi: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Curve {
    pub q_unit: String, // 派生层固定 "nm^-1"
    pub i_unit: String,
    pub points: Vec<Point>,
    pub diags: Vec<Diag>,
}

/// 样品/缓冲液元数据（透射率、曝光、厚度等，均带单位）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExposureMeta {
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub transmission: f64,
    pub transmission_err: f64,
    pub instrument_response_ref: String, // 仪器响应来源
    pub mask_ref: String,                // 遮罩来源
}

/// q 单位换算到 nm^-1；不可转换返回 None（值留在原位置并附诊断）
pub fn q_to_nm(q: f64, unit: &str) -> Option<f64> {
    match unit {
        "nm^-1" | "1/nm" | "nm-1" => Some(q),
        "A^-1" | "1/A" | "A-1" | "angstrom^-1" => Some(q * 10.0),
        _ => None,
    }
}

/// 区间候选（Guinier / Porod），允许多候选与重叠，绝不自动拼成单一模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub kind: String, // "guinier" | "porod"
    pub qmin: f64,
    pub qmax: f64,
    pub params: serde_json::Value, // guinier: {rg, i0}; porod: {slope, exponent}
    pub residuals: Vec<f64>,
    pub rms: f64,
    pub selected: bool,
    pub note: String,
}
