use serde::{Deserialize, Serialize};

/// 规范化后的 q 坐标基准：nm^-1。原始导入层不做换算，派生层才规范化。
pub const CANON_Q_UNIT: &str = "nm^-1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub q: f64,
    pub i: f64,
    pub err: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Curve {
    pub id: String,
    pub q_unit: String,
    pub i_unit: String,
    pub points: Vec<CurvePoint>,
}

impl Curve {
    pub fn qs(&self) -> Vec<f64> {
        self.points.iter().map(|p| p.q).collect()
    }
    pub fn is_sorted(&self) -> bool {
        self.points.windows(2).all(|w| w[0].q <= w[1].q)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SampleMeta {
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub transmission: f64,
    pub transmission_rel_unc: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
}

/// 区间候选（Guinier 或 Porod）。多个候选并存，无效候选也保留并附原因。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub kind: String, // "guinier" | "porod"
    pub q_min: f64,
    pub q_max: f64,
    pub n_points: usize,
    pub valid: bool,
    pub reason: String,
    pub params: serde_json::Value,
    pub chi2_red: f64,
}
