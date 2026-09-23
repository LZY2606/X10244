//! Append-only event log. Events are the only thing persisted/exported.

use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventPayload {
    ProjectCreated { name: String },
    RuleConfigured { params: super::RuleParams, note: String },
    ResponseImported { response: InstrumentResponse },
    MaskImported { mask: Mask },
    RunImported { parsed: ParsedRun },
    RunAmended { run_id: String, amendment: RunAmendment },
    Normalized { curve: NormCurve },
    NormalizationBlocked { run_id: String, diagnostics: Vec<Diagnostic> },
    Subtracted { version: SubVersion },
    SubtractionBlocked {
        artifact_id: String,
        sample_run: String,
        buffer_run: String,
        diagnostics: Vec<Diagnostic>,
    },
    RegionFitted { fit: RegionFit },
    FitBlocked {
        sub_version: String,
        kind: String,
        q_min: f64,
        q_max: f64,
        diagnostic: Diagnostic,
    },
    Decision { record: DecisionRecord },
    ConflictResolved {
        base_seq: u64,
        incoming_seq: u64,
        resolution: String,
        note: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub project: String,
    pub seq: u64,
    pub timestamp: String,
    pub rule_version: String,
    pub parent_hash: String,
    pub payload: EventPayload,
}

impl Event {
    pub fn content_digest(&self) -> String {
        let body = serde_json::to_string(&self.payload).unwrap_or_default();
        crate::ids::digest(&["evt", &self.project, &self.seq.to_string(), &self.timestamp, &body, &self.parent_hash])
    }
}
