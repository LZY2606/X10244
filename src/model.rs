//! 领域模型：SAXS 曲线、诊断、区间候选、事件。
//! 所有领域量都携带单位或坐标基准；导入层保留原始表示，不丢失。

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub indices: Vec<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recipe {
    pub rule: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// 一条散射曲线（原始或派生）。q 单位 Å^-1，I 单位见 i_unit。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Curve {
    pub id: String,
    pub record_id: String,
    pub kind: String, // raw_sample|raw_buffer|normalized|subtracted|instrument_response|mask|note
    pub q_unit: String,
    pub i_unit: String,
    #[serde(default)]
    pub q: Vec<f64>,
    #[serde(default)]
    pub i: Vec<f64>,
    #[serde(default)]
    pub sigma: Vec<f64>,
    /// 曝光、厚度、透射率等；导入时保留原始表示（含无法转换的值）。
    #[serde(default)]
    pub meta: serde_json::Value,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub recipe: Option<Recipe>,
}

impl Curve {
    pub fn diag(&mut self, code: &str, message: &str, indices: Vec<usize>) {
        self.diagnostics.push(Diagnostic {
            code: code.to_string(),
            message: message.to_string(),
            indices,
        });
    }
}

/// Guinier / Porod 区间候选。重叠区间不自动合并为单一模型。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub curve_id: String,
    pub family: String, // guinier | porod
    pub qmin: f64,
    pub qmax: f64,
    pub params: serde_json::Value,
    pub residual: f64,
    pub quality: f64,
    pub status: String, // candidate | accepted | rejected
}

/// 机器可读取的判定事件。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub seq: i64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}
