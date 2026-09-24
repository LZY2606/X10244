use crate::domain::{CaseBundle, IntervalCandidate, ReductionRun};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    CaseImported,
    ParameterRevised,
    SourcePointPatched,
    ReductionRecomputed,
    CandidateDecided,
    RunSelected,
    ConflictDetected,
    ConflictResolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourcePointPatch {
    pub curve_id: String,
    pub point_id: String,
    pub field: String,
    pub before: serde_json::Value,
    pub after: serde_json::Value,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    CaseImported {
        bundle: CaseBundle,
    },
    ParameterRevised {
        curve_id: String,
        field: String,
        before: serde_json::Value,
        after: serde_json::Value,
        reason: String,
    },
    SourcePointPatched {
        patch: SourcePointPatch,
    },
    ReductionRecomputed {
        run: ReductionRun,
    },
    CandidateDecided {
        run_id: String,
        run_key: String,
        interval_id: String,
        accepted: bool,
        rationale: String,
    },
    RunSelected {
        run_id: String,
        rejected_run_id: Option<String>,
        rationale: String,
    },
    ConflictDetected {
        fork_id: String,
        expected_parent: String,
        actual_parent: String,
        actor: String,
        summary: String,
    },
    ConflictResolved {
        fork_id: String,
        winning_event_id: String,
        losing_event_id: String,
        rationale: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub parent_ids: Vec<String>,
    pub kind: EventKind,
    pub timestamp: String,
    pub actor: String,
    pub revision: i64,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventRecord {
    pub event: Event,
    pub prev_revision: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PointTrace {
    pub derived_point_id: String,
    pub q: f64,
    pub sample: SourcePointView,
    pub background: SourcePointView,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourcePointView {
    pub curve_id: String,
    pub curve_version: String,
    pub point_id: String,
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub masked: bool,
    pub interpolation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventView {
    pub event: Event,
    pub heads: Vec<String>,
    pub changed_inputs: Vec<String>,
    pub affected_runs: Vec<String>,
    pub validity: Vec<ValidityStatement>,
    pub current_bundle_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidityStatement {
    pub run_id: String,
    pub valid: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntervalView {
    pub run_id: String,
    pub run_key: String,
    pub candidate: IntervalCandidate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplayState {
    pub at_event_id: Option<String>,
    pub heads: Vec<String>,
    pub bundle: Option<CaseBundle>,
    pub runs: Vec<ReductionRun>,
    pub events: Vec<Event>,
    pub current_run_key: Option<String>,
    pub selected_run_id: Option<String>,
    pub rejected_run_ids: Vec<String>,
    pub unresolved_conflict: Option<Event>,
    pub resolved_conflicts: Vec<Event>,
    pub changed_inputs: Vec<String>,
    pub affected_run_keys: Vec<String>,
    pub interval_decisions: Vec<(String, String, bool)>,
}

impl ReplayState {
    pub fn run_by_id(&self, run_id: &str) -> Option<&ReductionRun> {
        self.runs.iter().find(|run| run.id == run_id)
    }

    pub fn run_by_key(&self, key: &str) -> Option<&ReductionRun> {
        self.runs.iter().rev().find(|run| run.key == key)
    }

    pub fn current_runs(&self) -> Vec<&ReductionRun> {
        let mut seen = std::collections::BTreeSet::new();
        let mut result = Vec::new();
        for run in self.runs.iter().rev() {
            if seen.insert(run.key.clone()) {
                result.push(run);
            }
        }
        result
    }

    pub fn interval_views(&self) -> Vec<IntervalView> {
        self.runs
            .iter()
            .flat_map(|run| {
                run.intervals.iter().map(move |candidate| IntervalView {
                    run_id: run.id.clone(),
                    run_key: run.key.clone(),
                    candidate: candidate.clone(),
                })
            })
            .collect()
    }
}
