use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
}

pub const STATUS_OK: &str = "ok";
pub const STATUS_ANOMALY: &str = "anomaly";
pub const STATUS_BLOCKED: &str = "blocked";
pub const STATUS_MULTI: &str = "multi_candidate";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub version: i64,
    pub meta: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurveMeta {
    pub id: String,
    pub material_id: Option<String>,
    pub label: String,
    pub q_unit: String,
    pub i_unit: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Derivation {
    pub id: String,
    pub slot: String,
    pub kind: String,
    pub inputs: serde_json::Value,
    pub status: String,
    pub result: serde_json::Value,
    pub version: i64,
    pub event_id: String,
    pub stale: bool,
}
