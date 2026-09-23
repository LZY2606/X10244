//! 领域模型：曲线、来源记录、派生节点、事件、裁定。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub q: f64,
    pub i: f64,
    pub di: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Curve {
    pub points: Vec<CurvePoint>,
    /// 规范化后的坐标基准：q 恒为 Å⁻¹，I 恒为 cm⁻¹。
    pub q_unit: String,
    pub i_unit: String,
}

impl Curve {
    pub fn new(points: Vec<CurvePoint>) -> Self {
        Curve { points, q_unit: "A^-1".into(), i_unit: "cm^-1".into() }
    }
    pub fn q_min(&self) -> f64 {
        self.points.iter().map(|p| p.q).fold(f64::INFINITY, f64::min)
    }
    pub fn q_max(&self) -> f64 {
        self.points.iter().map(|p| p.q).fold(f64::NEG_INFINITY, f64::max)
    }
}

/// 机器可读取的判定/诊断码。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum DiagCode {
    QGridMismatchInterpolated,
    NoOverlap,
    NegativeIntensity,
    IntervalOverlap,
    BufferVersionSwitch,
    MultiCandidateAmbiguous,
    BackgroundScaleUncertainty,
    UnitUnconvertible,
    NormUncertainty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diag {
    pub code: DiagCode,
    pub message: String,
    /// 受影响对象（曲线点 q 值、区间、来源 id 等）。
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRecord {
    pub id: String,
    pub kind: String, // sample_curve|buffer_curve|instrument_response|mask|meta
    pub name: String,
    /// 原始表示逐字保留。
    pub raw_payload: String,
    pub raw_format: String,
    pub units: serde_json::Value,
    pub meta: serde_json::Value,
    pub seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DerivationStatus {
    Fresh,
    Stale,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Derivation {
    pub id: String,
    pub rule: String, // normalize|subtract|guinier|porod
    pub inputs: Vec<String>,
    pub params: serde_json::Value,
    pub status: DerivationStatus,
    pub diagnostics: Vec<Diag>,
    pub outputs: Vec<String>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitCandidate {
    pub id: String,
    pub kind: String, // Guinier|Porod
    pub q_min: f64,
    pub q_max: f64,
    /// Guinier: {rg, i0}; Porod: {exponent, scale}
    pub params: serde_json::Value,
    pub residual_rms: f64,
    /// None=未裁定, Some(true)=采纳, Some(false)=未采纳（保留）。
    pub accepted: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub seq: i64,
    pub version_from: i64,
    pub version_to: i64,
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub version: i64,
    pub subject: String,
    pub chosen: serde_json::Value,
    /// 未选候选全部保留，确认不是不可逆覆盖。
    pub rejected_kept: Vec<serde_json::Value>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub id: String,
    pub base_version: i64,
    pub status: String, // open|resolved
    pub payload: serde_json::Value,
    pub resolution: Option<serde_json::Value>,
}
