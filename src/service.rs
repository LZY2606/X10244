use crate::domain::CaseBundle;
use crate::events::*;
use crate::fixture::case_bundle;
use crate::reductions::{reduce, ReductionRequest};
use crate::state::replay;
use crate::storage::Storage;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<Mutex<Storage>>,
}

#[derive(Debug, serde::Deserialize)]
pub struct AppendCommand {
    pub expected_heads: Option<Vec<String>>,
    pub parent_ids: Vec<String>,
    pub actor: String,
    pub timestamp: String,
    pub kind: EventKind,
    pub payload: EventPayload,
    pub id: Option<String>,
}

impl AppState {
    pub fn new(path: &str) -> Self {
        Self {
            storage: Arc::new(Mutex::new(
                Storage::open(path).expect("open sqlite storage"),
            )),
        }
    }

    pub fn in_memory() -> Self {
        Self {
            storage: Arc::new(Mutex::new(
                Storage::in_memory().expect("open memory storage"),
            )),
        }
    }

    pub fn events(&self) -> Vec<Event> {
        self.storage
            .lock()
            .unwrap()
            .load_events()
            .unwrap_or_default()
    }

    pub fn state_at(&self, event_id: Option<&str>) -> ReplayState {
        replay(&self.events(), event_id)
    }

    pub fn append(&self, command: AppendCommand, allow_conflict: bool) -> Result<Event, String> {
        let state = self.state_at(None);
        let expected = command
            .expected_heads
            .clone()
            .or_else(|| Some(state.heads.clone()));
        let event = Event {
            id: command.id.unwrap_or_else(|| next_id(&command.kind)),
            parent_ids: if command.parent_ids.is_empty() {
                state.heads.clone()
            } else {
                command.parent_ids.clone()
            },
            kind: command.kind,
            timestamp: command.timestamp,
            actor: command.actor,
            revision: 0,
            payload: command.payload,
        };
        let storage = self.storage.lock().unwrap();
        if allow_conflict {
            storage
                .append_conflicting_event(event, expected.unwrap_or_default(), "system")
                .map(|(event, _)| event)
                .map_err(|error| error.to_string())
        } else {
            storage
                .append_event(event, expected)
                .map_err(|error| error.to_string())
        }
    }

    pub fn recompute(
        &self,
        request: ReductionRequest,
        actor: &str,
        parent_ids: Vec<String>,
        timestamp: String,
    ) -> Result<Event, String> {
        let state = self.state_at(None);
        let bundle = state
            .bundle
            .ok_or("import a case before recomputing")?
            .clone();
        let run = reduce(&bundle, &request);
        let event = Event {
            id: format!("recompute:{}:{}", request.key, run.id),
            parent_ids,
            kind: EventKind::ReductionRecomputed,
            timestamp,
            actor: actor.to_string(),
            revision: 0,
            payload: EventPayload::ReductionRecomputed { run },
        };
        self.storage
            .lock()
            .unwrap()
            .append_event(event, None)
            .map_err(|error| error.to_string())
    }
}

pub fn next_id(kind: &EventKind) -> String {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| format!("{}-{}", duration.as_nanos(), duration.subsec_nanos()))
        .unwrap_or_else(|_| "manual".to_string());
    format!("{:?}-{suffix}", kind).to_lowercase()
}

pub fn initial_bundle() -> CaseBundle {
    case_bundle()
}

fn push_event(
    events: &mut Vec<Event>,
    parent: &mut Vec<String>,
    id: &str,
    kind: EventKind,
    actor: &str,
    minute: usize,
    payload: EventPayload,
) {
    let event = Event {
        id: id.to_string(),
        parent_ids: parent.clone(),
        kind,
        timestamp: format!("2026-09-24T09:{minute:02}:00+09:00"),
        actor: actor.to_string(),
        revision: events.len() as i64 + 1,
        payload,
    };
    *parent = vec![event.id.clone()];
    events.push(event);
}

fn push_recompute(
    events: &mut Vec<Event>,
    parent: &mut Vec<String>,
    minute: usize,
    key: &str,
    sample: &str,
    buffer: &str,
) {
    let state = replay(events, None);
    let run = reduce(
        state.bundle.as_ref().expect("bundle"),
        &ReductionRequest {
            key: key.to_string(),
            sample_id: sample.to_string(),
            buffer_id: buffer.to_string(),
            label: key.to_string(),
        },
    );
    push_event(
        events,
        parent,
        &format!("evt-{:03}-recompute-{}", events.len() + 1, key),
        EventKind::ReductionRecomputed,
        "automatic-dependency-runner",
        minute,
        EventPayload::ReductionRecomputed { run },
    );
}

pub fn seeded_events() -> Vec<Event> {
    let bundle = initial_bundle();
    let mut events = Vec::new();
    let mut main = Vec::new();
    push_event(
        &mut events,
        &mut main,
        "evt-001-import-offline-case",
        EventKind::CaseImported,
        "beamline-import",
        0,
        EventPayload::CaseImported { bundle },
    );

    let initial = vec![
        ("baseline-buffer-v1", "sample-baseline", "buffer-a-v1"),
        ("grid-mismatch", "sample-grid-shift", "buffer-a-v1"),
        ("no-overlap", "sample-no-overlap", "buffer-a-v1"),
        ("scale-uncertain", "sample-scale-uncertain", "buffer-noisy"),
        ("negative-result", "sample-negative", "buffer-a-v1"),
    ];
    for (index, (key, sample, buffer)) in initial.iter().enumerate() {
        push_recompute(&mut events, &mut main, index + 1, key, sample, buffer);
    }

    push_event(
        &mut events,
        &mut main,
        "evt-006-revise-baseline-thickness",
        EventKind::ParameterRevised,
        "reviewer-a",
        6,
        EventPayload::ParameterRevised {
            curve_id: "sample-baseline".to_string(),
            field: "thickness_mm".to_string(),
            before: serde_json::json!(1.0),
            after: serde_json::json!(1.04),
            reason: "Capillary center thickness correction after microscopy check.".to_string(),
        },
    );
    push_recompute(
        &mut events,
        &mut main,
        7,
        "baseline-buffer-v1",
        "sample-baseline",
        "buffer-a-v1",
    );

    push_event(
        &mut events,
        &mut main,
        "evt-008-patch-local-source-point",
        EventKind::SourcePointPatched,
        "reviewer-a",
        8,
        EventPayload::SourcePointPatched {
            patch: SourcePointPatch {
                curve_id: "sample-grid-shift".to_string(),
                point_id: "sample-grid-shift-p08".to_string(),
                field: "intensity".to_string(),
                before: serde_json::json!(792.0),
                after: serde_json::json!(748.0),
                reason: "Integration spike tied to a detector hot pixel; source row retained."
                    .to_string(),
            },
        },
    );
    push_recompute(
        &mut events,
        &mut main,
        9,
        "grid-mismatch",
        "sample-grid-shift",
        "buffer-a-v1",
    );

    push_recompute(
        &mut events,
        &mut main,
        10,
        "baseline-buffer-v2",
        "sample-baseline",
        "buffer-a-v2",
    );

    let mut decision_parent = main.clone();
    let v1_run_before_decision = run_id_for(&events, "baseline-buffer-v1");
    let v2_run_before_decision = run_id_for(&events, "baseline-buffer-v2");
    push_event(
        &mut events,
        &mut decision_parent,
        "evt-011-decline-guinier-overlap-v1",
        EventKind::CandidateDecided,
        "reviewer-a",
        11,
        EventPayload::CandidateDecided {
            run_id: run_id_for(&events, "baseline-buffer-v1"),
            run_key: "baseline-buffer-v1".to_string(),
            interval_id: "guinier-low-q".to_string(),
            accepted: false,
            rationale: "Overlap candidate is evidence only; it is not joined automatically."
                .to_string(),
        },
    );

    let mut fork_a = decision_parent.clone();
    push_event(
        &mut events,
        &mut fork_a,
        "evt-012-select-buffer-v1",
        EventKind::RunSelected,
        "reviewer-a",
        12,
        EventPayload::RunSelected {
            run_id: v1_run_before_decision.clone(),
            rejected_run_id: Some(v2_run_before_decision.clone()),
            rationale: "Reviewer A chooses v1 because residuals are smaller.".to_string(),
        },
    );

    let mut fork_b = decision_parent.clone();
    push_event(
        &mut events,
        &mut fork_b,
        "evt-013-select-buffer-v2",
        EventKind::RunSelected,
        "reviewer-b",
        12,
        EventPayload::RunSelected {
            run_id: v2_run_before_decision.clone(),
            rejected_run_id: Some(v1_run_before_decision.clone()),
            rationale: "Reviewer B chooses v2 because it matches the refill chronology."
                .to_string(),
        },
    );

    let mut parents = vec![fork_a[0].clone(), fork_b[0].clone()];
    push_event(
        &mut events,
        &mut parents,
        "evt-014-conflict-buffer-version",
        EventKind::ConflictDetected,
        "system",
        13,
        EventPayload::ConflictDetected {
            fork_id: "fork-buffer-version".to_string(),
            expected_parent: fork_a[0].clone(),
            actual_parent: fork_b[0].clone(),
            actor: "reviewer-b".to_string(),
            summary: "Two users selected different buffer versions from the same reviewed state."
                .to_string(),
        },
    );

    push_event(
        &mut events,
        &mut parents,
        "evt-015-resolve-continue-v2",
        EventKind::ConflictResolved,
        "lead-reviewer",
        14,
        EventPayload::ConflictResolved {
            fork_id: "fork-buffer-version".to_string(),
            winning_event_id: fork_b[0].clone(),
            losing_event_id: fork_a[0].clone(),
            rationale: "Continue with v2 for downstream work; v1 remains in history as an unadopted branch.".to_string(),
        },
    );

    push_recompute(
        &mut events,
        &mut parents,
        15,
        "multi-candidate-residual",
        "sample-baseline",
        "buffer-a-v2",
    );
    let multi_run_id = run_id_for(&events, "multi-candidate-residual");
    push_event(
        &mut events,
        &mut parents,
        "evt-017-accept-guinier-v2",
        EventKind::CandidateDecided,
        "lead-reviewer",
        16,
        EventPayload::CandidateDecided {
            run_id: multi_run_id.clone(),
            run_key: "multi-candidate-residual".to_string(),
            interval_id: "guinier-low-q".to_string(),
            accepted: true,
            rationale: "Low-q interval accepted independently; Porod stays a separate candidate."
                .to_string(),
        },
    );
    push_event(
        &mut events,
        &mut parents,
        "evt-018-keep-porod-separate",
        EventKind::CandidateDecided,
        "lead-reviewer",
        17,
        EventPayload::CandidateDecided {
            run_id: multi_run_id.clone(),
            run_key: "multi-candidate-residual".to_string(),
            interval_id: "porod-high-q".to_string(),
            accepted: false,
            rationale: "High-q evidence is retained but not auto-joined to Guinier.".to_string(),
        },
    );

    events
}

fn run_id_for(events: &[Event], key: &str) -> String {
    events
        .iter()
        .rev()
        .find_map(|event| match &event.payload {
            EventPayload::ReductionRecomputed { run } if run.key == key => Some(run.id.clone()),
            _ => None,
        })
        .unwrap_or_else(|| format!("missing-{key}"))
}
