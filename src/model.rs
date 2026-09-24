use serde::{Deserialize, Serialize};

use crate::saxs::CurvePoint;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Curve {
    pub id: String,
    pub name: String,
    pub kind: String, // sample | buffer | instrument_response | mask
    pub q_unit: String,
    pub i_unit: String,
    pub exposure_s: Option<f64>,
    pub exposure_unc: Option<f64>,
    pub thickness_value: Option<f64>,
    pub thickness_unit: Option<String>,
    pub transmission: Option<f64>,
    pub transmission_unc: Option<f64>,
    pub raw_text: String,
    pub source: String,
    pub created_seq: i64,
    #[serde(default)]
    pub points: Vec<CurvePoint>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeDef {
    pub id: String,
    pub kind: String, // normalize | subtract | fit
    pub label: String,
    pub params: serde_json::Value,
    pub inputs: Vec<String>,
    pub created_seq: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeVersion {
    pub node_id: String,
    pub seq: i64,
    pub adopted: bool,
    pub status: String, // ok | diagnostic | refused | candidate
    pub result: serde_json::Value,
    pub diagnostics: Vec<crate::saxs::Diagnostic>,
    pub reason: String,
    pub def: NodeDef,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub id: String,
    pub kind: String,
    pub actor: String,
    pub base_seq: i64,
    pub payload: serde_json::Value,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub seq: i64,
    pub node_id: String,
    pub candidate_id: String,
    pub action: String, // accept | reject
    pub note: String,
    pub actor: String,
    pub adopted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conflict {
    pub id: String,
    pub seq: i64,
    pub kind: String,
    pub status: String, // pending | resolved
    pub base_seq: i64,
    pub branch_a: serde_json::Value,
    pub branch_b: serde_json::Value,
    pub resolved_seq: Option<i64>,
    pub resolution: Option<String>,
}
