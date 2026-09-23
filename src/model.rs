use serde::{Deserialize, Serialize};

/// 机器可读取诊断：code 供程序判断，message 供人阅读。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Diag {
    pub code: String,
    pub message: String,
    pub subject: Option<String>,
}

impl Diag {
    pub fn new(code: &str, message: impl Into<String>, subject: Option<String>) -> Self {
        Diag { code: code.to_string(), message: message.into(), subject }
    }
}

/// 原始曲线点：导入时保留原始表示（原始单位、原始数值），派生层才规范化。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub q: f64,
    pub i: f64,
    pub di: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Curve {
    pub id: String,
    pub name: String,
    pub kind: String, // sample | buffer | instrument_response | mask | derived
    pub q_unit: String,
    pub i_unit: String,
    pub meta: serde_json::Value,
    pub source: String,
    pub seq: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub version_id: String,
    pub kind: String, // import | normalize | subtract | guinier | porod
    pub status: String, // ok | uncertain | invalid
    pub inputs: serde_json::Value,  // {"curves":[...],"nodes":[...],"params":{...}}
    pub params: serde_json::Value,
    pub result: serde_json::Value,
    pub diag: Vec<Diag>,
    pub explain: String,
    pub adopted: bool,
    pub created_seq: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub id: String,
    pub ts_ms: i64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub version_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub id: String,
    pub parent: Option<String>,
    pub label: String,
    pub status: String, // building | complete
    pub created_seq: i64,
}
