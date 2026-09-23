//! 领域模型：SAXS 归一化、背景扣除、区间候选与版本化实体。
use serde::{Deserialize, Serialize};

/// 稳定身份：所有实体/事件 id 均为单调序号，导出/导入后保持不变。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QUnit {
    #[serde(rename = "1/A")]
    InvAngstrom,
    #[serde(rename = "1/nm")]
    InvNm,
}

impl QUnit {
    /// 转换到 Å^-1；无法识别的单位返回 None（值留在原位置并附诊断）。
    pub fn to_angstrom(&self, q: f64) -> f64 {
        match self {
            QUnit::InvAngstrom => q,
            QUnit::InvNm => q / 10.0,
        }
    }
    pub fn parse(s: &str) -> Option<QUnit> {
        match s.trim() {
            "1/A" | "A^-1" | "angstrom^-1" => Some(QUnit::InvAngstrom),
            "1/nm" | "nm^-1" => Some(QUnit::InvNm),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurvePoint {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
}

/// 原始曲线：保留原始文本表示，派生层才做单位规范化。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCurve {
    pub name: String,
    pub q_unit_raw: String,
    pub i_unit_raw: String,
    pub q_unit: Option<QUnit>,
    pub points: Vec<CurvePoint>,
    /// 无法转换/解析的行留在原位置，附诊断。
    pub diagnostics: Vec<String>,
    pub raw_text: String,
    pub source_file: String,
}

/// 样品/缓冲液元数据：曝光、厚度、透射率，均带原始表示与相对不确定度。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExposureMeta {
    pub label: String,
    pub exposure_s: f64,
    pub exposure_raw: String,
    pub thickness_cm: f64,
    pub thickness_raw: String,
    pub transmission: f64,
    pub transmission_raw: String,
    pub transmission_rel_unc: f64,
    pub thickness_rel_unc: f64,
    pub exposure_rel_unc: f64,
}

/// 仪器响应与遮罩来源（溯源记录，原样保留）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceRecord {
    pub kind: String, // "instrument_response" | "mask_source"
    pub label: String,
    pub detail: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferCandidate {
    pub label: String,
    pub raw_curve_stream: String, // 指向 raw curve 流
    pub version_note: String,
}

/// 归一化曲线点：携带来源索引。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormPoint {
    pub q: f64, // Å^-1（派生层规范化坐标基准）
    pub i: f64,
    pub sigma: f64, // 含归一化不确定度
    pub src_idx: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedCurve {
    pub deps: Vec<String>,
    pub norm_factor: f64,
    pub norm_rel_unc: f64,
    pub points: Vec<NormPoint>,
}

/// 背景点对齐方式：精确匹配或重叠区内线性插值。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum BufferAlign {
    #[serde(rename = "exact")]
    Exact { idx: usize },
    #[serde(rename = "interp")]
    Interp { lo: usize, hi: usize, t: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubPoint {
    pub q: f64,
    pub i: f64,
    pub sigma: f64, // 含归一化 + 背景缩放不确定度
    pub flag: Option<String>, // "negative_intensity"
    pub sample_idx: usize,
    pub buffer: BufferAlign,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExcludedPoint {
    pub q: f64,
    pub reason: String, // "no_overlap"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtractedCurve {
    pub deps: Vec<String>,
    pub scale: f64,
    pub scale_unc: f64,
    pub points: Vec<SubPoint>,
    pub excluded: Vec<ExcludedPoint>,
    pub residual_metric: f64,
}

/// Guinier / Porod 区间候选：互不自动合并，允许多候选与区间重叠。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub deps: Vec<String>,
    pub model: String, // "guinier" | "porod"
    pub qmin: f64,
    pub qmax: f64,
    pub params: serde_json::Value,
    pub residuals: Vec<f64>,
    pub rms_residual: f64,
    pub valid: bool,
    pub notes: Vec<String>,
}

/// 人工裁定：保留未选候选，确认不是不可逆覆盖。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub deps: Vec<String>,
    pub chosen: String,
    pub rejected_kept: Vec<String>,
    pub rationale: String,
    pub author: String,
}

/// 通用实体信封：所有版本化对象。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub stream: String,
    pub kind: String,
    pub seq: i64,
    pub payload: serde_json::Value,
}

/// 机器可读取的判定事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub seq: i64,
    pub kind: String, // "judgment" | "import" | "derive" | "decision" | "conflict" | "replay"
    pub rule: Option<String>,
    pub summary: String,
    pub payload: serde_json::Value,
}
