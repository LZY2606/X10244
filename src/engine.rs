use crate::domain::*;
use crate::pipeline::rebuild_graph;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Projection {
    pub state: GraphState,
    pub nodes: BTreeMap<String, GraphNode>,
}

#[derive(Debug, Clone)]
pub struct AppliedEvent {
    pub event: EventRecord,
    pub state: GraphState,
    pub changed_nodes: Vec<GraphNode>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChangeReport {
    pub changed: Vec<NodeChange>,
    pub unchanged: Vec<NodeUnchanged>,
    pub judgments: Vec<Judgment>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeChange {
    pub node_id: String,
    pub version: u32,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeUnchanged {
    pub node_id: String,
    pub version: u32,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Judgment {
    pub code: String,
    pub status: String,
    pub scope: String,
    pub basis: String,
    pub affected_objects: Vec<String>,
}

pub fn now_timestamp() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("2026-09-24T00:00:{:03}Z", nanos % 1000)
}

fn judgments_from_graph(nodes: &BTreeMap<String, GraphNode>) -> Vec<Judgment> {
    let mut result = Vec::new();
    for node in nodes.values() {
        for d in &node.diagnostics {
            let basis = match d.code.as_str() {
                "Q_GRID_MISMATCH" => "q grids differ; direct subtraction is forbidden and interpolation is valid only in overlap".into(),
                "NO_OVERLAP" => "closed q intervals do not intersect; no subtraction is produced".into(),
                "BACKGROUND_SCALE_UNCERTAINTY" => "sigma includes normalization and background scale uncertainty".into(),
                "NEGATIVE_SUBTRACTED_INTENSITY" | "NEGATIVE_INTENSITY" => "negative observations are retained with uncertainty and never clipped".into(),
                "INTERVAL_OVERLAP" => "Guinier and Porod evidence is retained separately and never auto-stitched".into(),
                _ => d.message.clone(),
            };
            result.push(Judgment { code: d.code.clone(), status: d.severity.clone(), scope: node.id.clone(), basis, affected_objects: vec![node.id.clone()] });
        }
    }
    result
}

fn report(previous: &BTreeMap<String, GraphNode>, next: &BTreeMap<String, GraphNode>) -> ChangeReport {
    let mut changed = Vec::new();
    let mut unchanged = Vec::new();
    for (id, node) in next {
        match previous.get(id) {
            None => changed.push(NodeChange { node_id: id.clone(), version: 1, reason: "created".into() }),
            Some(old) if old.fingerprint != node.fingerprint => changed.push(NodeChange { node_id: id.clone(), version: node.version, reason: "input, dependency fingerprint, evidence, or decision changed".into() }),
            Some(old) => unchanged.push(NodeUnchanged { node_id: id.clone(), version: old.version, reason: "content fingerprint and dependency fingerprints remain valid".into() }),
        }
    }
    ChangeReport { changed, unchanged, judgments: judgments_from_graph(next) }
}

fn versioned_graph(case: &CaseMaterials, event_id: &str, decisions: &[Decision], conflicts: &[Conflict], previous: &BTreeMap<String, GraphNode>) -> BTreeMap<String, GraphNode> {
    let candidate = rebuild_graph(case, event_id, decisions, conflicts);
    candidate.into_iter().map(|(id, mut node)| {
        if let Some(old) = previous.get(&id) {
            if old.fingerprint == node.fingerprint {
                node.version = old.version;
                node.created_by_event = old.created_by_event.clone();
            } else {
                node.version = old.version + 1;
            }
        }
        (id, node)
    }).collect()
}

fn make_event(seq: usize, parent: Option<String>, req: Option<&CommandRequest>, event_type: String, state: GraphState, report: ChangeReport, timestamp: String) -> EventRecord {
    let id = format!("E{seq:04}");
    let payload = json!({
        "state": state,
        "report": report,
        "command": req.map(|r| r.command.clone()),
        "actor": req.map(|r| r.actor.clone()).unwrap_or_else(|| "fixture".into()),
    });
    let pre_hash = json!({"parent": parent, "type": event_type, "payload": payload, "timestamp": timestamp});
    EventRecord { event_id: id, parent_event_id: parent, event_type, actor: req.map(|r| r.actor.clone()).unwrap_or_else(|| "fixture".into()), timestamp, base_event_id: req.and_then(|r| r.base_event_id.clone()), payload, hash: stable_hash(&pre_hash) }
}

pub fn initial_event(fixture: &FixturePack, seq: usize) -> AppliedEvent {
    let case = fixture.case.clone();
    let state = GraphState { case: Some(case), nodes: BTreeMap::new(), events: vec![], decisions: vec![], conflicts: vec![] };
    let nodes = versioned_graph(state.case.as_ref().unwrap(), &format!("E{seq:04}"), &state.decisions, &state.conflicts, &BTreeMap::new());
    let rep = report(&BTreeMap::new(), &nodes);
    let event = make_event(seq, None, None, "case.imported".into(), state.clone(), rep.clone(), "2026-09-24T09:00:00Z".into());
    let mut state = state;
    state.events = vec![event.clone()];
    AppliedEvent { event, state, changed_nodes: rep.changed.iter().filter_map(|c| nodes.get(&c.node_id).cloned()).collect() }
}

fn require_base(prev: &GraphState, base: &Option<String>) -> Result<(), String> {
    if let Some(base) = base {
        if !prev.events.iter().any(|e| &e.event_id == base) {
            return Err("unknown base event".into());
        }
        if prev.events.last().map(|e| &e.event_id) != Some(base) {
            return Err("stale base event; rebase the edit and resolve the conflict".into());
        }
    }
    Ok(())
}

fn latest_decision_for<'a>(decisions: &'a [Decision], candidate: &str) -> Option<&'a Decision> {
    decisions.iter().rev().find(|d| d.status == "accepted" && d.selected_candidate == candidate)
}

pub fn apply_command(prev: &Projection, req: CommandRequest, seq: usize) -> Result<AppliedEvent, (u16, String, Option<AppliedEvent>)> {
    require_base(&prev.state, &req.base_event_id).map_err(|e| (409, e, None))?;
    let mut state = prev.state.clone();
    let case = state.case.as_mut().ok_or((400, "case missing".to_string(), None))?;
    let event_type;
    match &req.command {
        Command::ReviseThickness { curve_id, value, sigma, raw_text } => {
            event_type = "parameter.thickness_revised".into();
            let curve = case.curves.iter_mut().find(|c| &c.id == curve_id).ok_or((404, "curve missing".into(), None))?;
            curve.thickness = Quantity { value: *value, sigma: *sigma, unit: curve.thickness.unit.clone(), raw_text: raw_text.clone() };
        }
        Command::RevisePoint { curve_id, index, intensity, sigma } => {
            event_type = "source.point_revised".into();
            let curve = case.curves.iter_mut().find(|c| &c.id == curve_id).ok_or((404, "curve missing".into(), None))?;
            let point = curve.points.get_mut(*index).ok_or((404, "point missing".into(), None))?;
            point.i = *intensity;
            point.sigma = *sigma;
        }
        Command::ShiftGrid { curve_id, delta_q } => {
            event_type = "source.grid_shifted".into();
            let curve = case.curves.iter_mut().find(|c| &c.id == curve_id).ok_or((404, "curve missing".into(), None))?;
            for p in &mut curve.points { p.q += delta_q; }
        }
        Command::SelectBuffer { buffer_id } => {
            event_type = "decision.buffer_selected".into();
            if !case.buffer_candidates.contains(buffer_id) { return Err((400, "unknown buffer candidate".into(), None)); }
            case.selected_buffer = buffer_id.clone();
        }
        Command::SetBackgroundScale { value, sigma, source, state: scale_state } => {
            event_type = "parameter.background_scale_revised".into();
            if *value <= 0.0 { return Err((400, "background scale must be positive".into(), None)); }
            case.background_scale = BackgroundScale { value: *value, sigma: *sigma, source: source.clone(), state: scale_state.clone() };
        }
        Command::SetModelWindows { guinier_q_min, guinier_q_max, porod_q_min, porod_q_max } => {
            event_type = "parameter.model_windows_revised".into();
            case.model_windows = ModelWindowConfig { guinier_q_min: *guinier_q_min, guinier_q_max: *guinier_q_max, porod_q_min: *porod_q_min, porod_q_max: *porod_q_max, q_unit: "1/A".into() };
        }
        Command::ProposeDecision { decision_id, actor, candidate, note } => {
            event_type = "decision.proposed".into();
            if !case.buffer_candidates.contains(candidate) { return Err((400, "unknown buffer candidate".into(), None)); }
            let base = req.base_event_id.clone().unwrap_or_else(|| prev.state.events.last().unwrap().event_id.clone());
            if let Some(existing) = latest_decision_for(&state.decisions, candidate) {
                let _ = existing;
            }
            if let Some(accepted) = state.decisions.iter().rev().find(|d| d.status == "accepted") {
                let rejected_id = format!("{decision_id}.rejected");
                let event_id = format!("E{seq:04}");
                let rejected = Decision { id: rejected_id.clone(), actor: actor.clone(), subject: "selected-background".into(), selected_candidate: candidate.clone(), status: "rejected".into(), base_event_id: base.clone(), note: note.clone(), event_id: event_id.clone() };
                let conflict_id = format!("conflict.{}-{}", accepted.id, rejected_id);
                let conflict = Conflict { id: conflict_id, status: "open".into(), winner_decision_id: accepted.id.clone(), rejected_decision_id: rejected_id, resolution: None, resolver: None };
                state.decisions.push(rejected);
                state.conflicts.push(conflict.clone());
                let nodes = versioned_graph(case, &event_id, &state.decisions, &state.conflicts, &prev.nodes);
                let rep = report(&prev.nodes, &nodes);
                let event = make_event(seq, prev.state.events.last().map(|e| e.event_id.clone()), Some(&req), "decision.conflicted".into(), state.clone(), rep, now_timestamp());
                state.events.push(event.clone());
                return Err((409, "concurrent decision from the same base version".into(), Some(AppliedEvent { event, state, changed_nodes: nodes.into_values().collect() })));
            }
            let event_id = format!("E{seq:04}");
            let decision = Decision { id: decision_id.clone(), actor: actor.clone(), subject: "selected-background".into(), selected_candidate: candidate.clone(), status: "accepted".into(), base_event_id: base, note: note.clone(), event_id };
            state.decisions.push(decision);
        }
        Command::ResolveConflict { conflict_id, resolution, actor } => {
            event_type = "conflict.resolved".into();
            let conflict = state.conflicts.iter_mut().find(|c| &c.id == conflict_id).ok_or((404, "conflict missing".into(), None))?;
            conflict.status = "resolved".into();
            conflict.resolution = Some(resolution.clone());
            conflict.resolver = Some(actor.clone());
        }
    }
    let event_id = format!("E{seq:04}");
    let nodes = versioned_graph(case, &event_id, &state.decisions, &state.conflicts, &prev.nodes);
    let rep = report(&prev.nodes, &nodes);
    let event = make_event(seq, prev.state.events.last().map(|e| e.event_id.clone()), Some(&req), event_type, state.clone(), rep.clone(), now_timestamp());
    state.events.push(event.clone());
    Ok(AppliedEvent { event, state, changed_nodes: rep.changed.iter().filter_map(|c| nodes.get(&c.node_id).cloned()).collect() })
}

pub fn replay(events: &[EventRecord], target: Option<&str>) -> Result<Projection, String> {
    let mut nodes = BTreeMap::new();
    let mut state = GraphState::default();
    for event in events {
        let payload_state: GraphState = serde_json::from_value(event.payload.get("state").cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())?;
        state = payload_state;
        state.events = events.iter().take_while(|e| e.event_id != event.event_id).cloned().chain(std::iter::once(event.clone())).collect();
        nodes = versioned_graph(state.case.as_ref().unwrap(), &event.event_id, &state.decisions, &state.conflicts, &nodes);
        if target == Some(event.event_id.as_str()) { break; }
    }
    Ok(Projection { state, nodes })
}

pub fn fixture() -> FixturePack {
    serde_json::from_str(include_str!("../fixtures/saxs_case.json")).expect("valid bundled fixture")
}
