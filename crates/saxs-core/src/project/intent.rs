//! Reviewer intents. The engine turns these into zero or more events.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Intent {
    CreateProject { id: String, name: String },
    ConfigureRules { params: super::RuleParams, note: String },
    ImportResponse { response: crate::model::InstrumentResponse },
    ImportMask { mask: crate::model::Mask },
    ImportRun { input: crate::model::RunImportInput },
    AmendRun { amendment: crate::model::RunAmendment },
    Normalize { run_id: String, response_id: Option<String>, mask_id: Option<String> },
    Subtract {
        artifact_id: String,
        sample_run: String,
        buffer_run: String,
        windows: Vec<(f64, f64)>,
        manual_alpha: Option<f64>,
    },
    FitRegion {
        artifact_id: String,
        kind: String,
        q_min: f64,
        q_max: f64,
    },
    AcceptScale {
        artifact_id: String,
        sub_version_id: String,
        candidate_id: String,
        base_seq: u64,
        note: String,
    },
    ContinueBranch {
        artifact_id: String,
        chosen_version: String,
        base_seq: u64,
        note: String,
    },
    ResolveConflict {
        base_seq: u64,
        incoming_seq: u64,
        resolution: String,
        note: String,
    },
}
