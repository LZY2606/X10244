use crate::domain::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppendEvent {
    pub id: String,
    pub event_type: String,
    pub author: String,
    pub created_at: String,
    pub basis_revision: Option<i64>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppendResult {
    pub state: TimelineState,
    pub appended_event_id: String,
    pub created_object_ids: Vec<String>,
    pub retained_object_ids: Vec<String>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConflictError {
    pub kind: String,
    pub message: String,
    pub current_revision: i64,
    pub basis_revision: i64,
    pub conflicting_event_id: String,
    pub resolvable_commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EngineError {
    Conflict(ConflictError),
    Invalid(String),
}

pub fn initial_state(package: CasePackage) -> TimelineState {
    let mut state = TimelineState {
        revision: 0,
        at_event_seq: None,
        package,
        events: Vec::new(),
        objects: Vec::new(),
        edges: Vec::new(),
        current: BTreeMap::new(),
        normalized: Vec::new(),
        subtractions: Vec::new(),
        intervals: Vec::new(),
    };
    let import = serde_json::json!({
        "package_id": state.package.package_id,
        "title": state.package.title,
        "preserved_representation": state.package.imported_representation,
        "coordinate_basis": state.package.coordinate_basis,
        "curve_count": state.package.curves.len(),
        "measurement_count": state.package.measurements.len(),
        "buffer_candidate_count": state.package.buffer_candidates.len(),
        "response_count": state.package.responses.len(),
        "mask_count": state.package.masks.len()
    });
    let event = make_event(1, "package.import", "saxs-team", "T00:00:00Z", None, import);
    apply_import(&mut state, &event);
    state
}

pub(crate) fn make_event(
    seq: i64,
    event_type: &str,
    author: &str,
    created_at: &str,
    basis_revision: Option<i64>,
    payload: serde_json::Value,
) -> EventRecord {
    let suffix = match event_type.rsplit_once('.') {
        Some((_, suffix)) => suffix,
        None => event_type,
    };
    EventRecord {
        seq,
        id: format!("ev-{seq:04}-{suffix}"),
        event_type: event_type.to_string(),
        author: author.to_string(),
        created_at: created_at.to_string(),
        basis_revision,
        payload,
        supersedes_event_id: None,
    }
}

pub fn append(state: TimelineState, event: AppendEvent) -> Result<AppendResult, EngineError> {
    validate_event(&event)?;
    let current_revision = state.revision;
    if let Some(basis) = event.basis_revision {
        if basis != current_revision {
            let conflicting = state
                .events
                .iter()
                .find(|item| item.seq == current_revision)
                .map(|item| item.id.clone())
                .unwrap_or_else(|| "unknown".to_string());
            return Err(EngineError::Conflict(ConflictError {
                kind: "stale_revision".to_string(),
                message: "Two editors started from the same revision; reload, compare branches, then append a resolution event.".to_string(),
                current_revision,
                basis_revision: basis,
                conflicting_event_id: conflicting,
                resolvable_commands: vec![
                    "compare_branches".to_string(),
                    "append_decision_with_basis_current".to_string(),
                ],
            }));
        }
    }
    let mut next = state.clone();
    let seq = next.revision + 1;
    let record = EventRecord {
        seq,
        id: event.id,
        event_type: event.event_type,
        author: event.author,
        created_at: event.created_at,
        basis_revision: event.basis_revision,
        payload: event.payload,
        supersedes_event_id: None,
    };
    let before: BTreeSet<String> = next.objects.iter().map(|obj| obj.id.clone()).collect();
    apply_event(&mut next, &record)?;
    let after: BTreeSet<String> = next.objects.iter().map(|obj| obj.id.clone()).collect();
    next.revision = seq;
    next.at_event_seq = Some(seq);
    next.events.push(record.clone());
    Ok(AppendResult {
        created_object_ids: after.difference(&before).cloned().collect(),
        retained_object_ids: before.intersection(&after).cloned().collect(),
        diagnostics: collect_recent_diagnostics(&next, &record.id),
        state: next,
        appended_event_id: record.id,
    })
}

pub fn at_revision(original: &TimelineState, revision: i64) -> Result<TimelineState, EngineError> {
    if revision < 0 || revision > original.revision {
        return Err(EngineError::Invalid(format!("revision {revision} is outside 0..{}", original.revision)));
    }
    if revision == original.at_event_seq.unwrap_or(0) && revision == original.revision {
        return Ok(original.clone());
    }
    let mut rebuilt = initial_state(original.package.clone());
    for event in original.events.iter().skip(1).filter(|event| event.seq <= revision).cloned().collect::<Vec<_>>() {
        apply_event(&mut rebuilt, &event)?;
        rebuilt.revision = event.seq;
        rebuilt.at_event_seq = Some(event.seq);
    }
    if revision == 0 {
        let mut empty = rebuilt;
        empty.events.clear();
        empty.objects.clear();
        empty.edges.clear();
        empty.current.clear();
        empty.normalized.clear();
        empty.subtractions.clear();
        empty.intervals.clear();
        empty.revision = 0;
        empty.at_event_seq = Some(0);
        return Ok(empty);
    }
    Ok(rebuilt)
}

fn validate_event(event: &AppendEvent) -> Result<(), EngineError> {
    if event.id.trim().is_empty() {
        return Err(EngineError::Invalid("event id is required".to_string()));
    }
    if event.event_type.trim().is_empty() {
        return Err(EngineError::Invalid("event type is required".to_string()));
    }
    Ok(())
}

fn collect_recent_diagnostics(state: &TimelineState, event_id: &str) -> Vec<String> {
    let mut values = Vec::new();
    for object in &state.objects {
        if object.event_id == event_id {
            for key in ["diagnostics", "diagnostic"] {
                if let Some(items) = object.payload.get(key).and_then(|value| value.as_array()) {
                    for item in items {
                        if let Some(text) = item.as_str() {
                            values.push(format!("{}: {text}", object.logical_key));
                        }
                    }
                }
            }
        }
    }
    values
}
