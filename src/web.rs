use crate::domain::{CommandRequest, DependencyRef, GraphNode, Point};
use crate::engine::fixture;
use crate::storage::Store;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
}

#[derive(Deserialize)]
struct AtQuery { at: Option<String> }

#[derive(Deserialize)]
struct TraceQuery { point_id: String, at: Option<String> }

#[derive(Deserialize)]
struct DemoQuery { reset: Option<String> }

fn error(status: StatusCode, message: String) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

pub fn router(store: Arc<Store>) -> Router {
    let state = AppState { store };
    Router::new()
        .route("/api/health", get(health))
        .route("/api/fixture", get(fixture_pack))
        .route("/api/state", get(get_state))
        .route("/api/commands", post(command))
        .route("/api/trace", get(trace))
        .route("/api/export", get(export))
        .route("/api/import", post(import_bundle))
        .route("/api/demo/replay", get(demo_replay))
        .fallback_service(tower_http::services::ServeDir::new("static"))
        .with_state(state)
}

async fn health() -> Json<Value> { Json(json!({"status":"ok"})) }
async fn fixture_pack() -> Json<Value> { Json(json!(fixture())) }

async fn get_state(State(app): State<AppState>, Query(q): Query<AtQuery>) -> Response {
    match app.store.projection_at(q.at.as_deref()) {
        Ok(projection) => {
            let last = projection.state.events.last().map(|e| json!({"event_id":e.event_id,"event_type":e.event_type,"timestamp":e.timestamp,"hash":e.hash}));
            Json(json!({"current":last,"state":projection.state,"nodes":projection.nodes,"scenario":fixture().scenario})).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}

async fn command(State(app): State<AppState>, Json(req): Json<CommandRequest>) -> Response {
    match app.store.append(req) {
        Ok(applied) => Json(json!({"ok":true,"event":applied.event,"changed_nodes":applied.changed_nodes})).into_response(),
        Err((status, message)) => error(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), message),
    }
}

fn find_point(node: &GraphNode, point_id: &str) -> Option<Point> {
    let value = if node.id == "background-evidence" {
        node.data.get("candidates").and_then(|v| v.as_array()).and_then(|cs| cs.iter().find_map(|c| c.get("subtraction").and_then(|s| s.get("points")).and_then(|p| p.as_array()).and_then(|points| points.iter().find(|p| p.get("id").and_then(|x| x.as_str()) == Some(point_id)).cloned())))
    } else {
        node.data.get("points").and_then(|p| p.as_array()).and_then(|points| points.iter().find(|p| p.get("id").and_then(|x| x.as_str()) == Some(point_id)).cloned())
    };
    value.and_then(|v| serde_json::from_value(v).ok())
}

async fn trace(State(app): State<AppState>, Query(q): Query<TraceQuery>) -> Response {
    let projection = match app.store.projection_at(q.at.as_deref()) { Ok(p) => p, Err(e) => return error(StatusCode::BAD_REQUEST, e) };
    let point = projection.nodes.values().find_map(|n| find_point(n, &q.point_id));
    let Some(point) = point else { return error(StatusCode::NOT_FOUND, "point identity not found at selected event".into()); };
    let mut source_nodes = Vec::new();
    for source_id in &point.source_point_ids {
        let node_id = source_id.split(".normalized.").next().unwrap_or(source_id).split(".p").next().unwrap_or(source_id).to_string();
        let node_id = node_id.replace("..p", "");
        let direct_node = projection.nodes.get(&node_id).cloned();
        let normalized = projection.nodes.get(&format!("{node_id}.normalized")).cloned();
        if let Some(n) = direct_node { source_nodes.push(n); }
        if let Some(n) = normalized { source_nodes.push(n); }
        if source_id.contains("response/") {
            if let Some(n) = projection.nodes.get("response/pilatus-flat-field-2026-09").cloned() { source_nodes.push(n); }
        }
    }
    let dependencies: Vec<&DependencyRef> = projection.nodes.values().flat_map(|n| n.dependencies.values()).filter(|d| point.source_point_ids.iter().any(|s| s.contains(&d.node_id))).collect();
    Json(json!({"point":point,"source_nodes":source_nodes,"dependency_refs":dependencies,"hash_chain_note":"source node versions and fingerprints are sufficient to recover the exact event"})).into_response()
}

async fn export(State(app): State<AppState>) -> Response {
    match app.store.export() {
        Ok(value) => ([(header::CONTENT_TYPE, "application/json")], serde_json::to_vec_pretty(&value).unwrap_or_default()).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn import_bundle(State(app): State<AppState>, Json(bundle): Json<Value>) -> Response {
    match app.store.import_bundle(bundle) {
        Ok(count) => Json(json!({"ok":true,"events":count})).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}

async fn demo_replay(State(_app): State<AppState>, Query(q): Query<DemoQuery>) -> Response {
    let _ = q.reset;
    let _ = std::fs::remove_file("saxs-demo.sqlite");
    let _ = std::fs::remove_file("saxs-demo.sqlite-wal");
    let _ = std::fs::remove_file("saxs-demo.sqlite-shm");
    let store = match Store::open("saxs-demo.sqlite") { Ok(s) => s, Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e) };
    let mut base = match store.latest() { Ok(p) => p.state.events.last().unwrap().event_id.clone(), Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e) };
    let mk = |base: &str, actor: &str, command: crate::domain::Command| CommandRequest { base_event_id: Some(base.into()), actor: actor.into(), command };
    let commands: Vec<(&str, crate::domain::Command)> = vec![
        ("reviewer-a", crate::domain::Command::ReviseThickness { curve_id:"sample".into(), value:0.152, sigma:0.0018, raw_text:"0.152 cm ± 0.0018".into() }),
        ("reviewer-a", crate::domain::Command::RevisePoint { curve_id:"sample".into(), index:8, intensity:6.2, sigma:0.08 }),
        ("reviewer-a", crate::domain::Command::ShiftGrid { curve_id:"buffer-a".into(), delta_q:0.001 }),
        ("reviewer-a", crate::domain::Command::ShiftGrid { curve_id:"buffer-a".into(), delta_q:0.100 }),
        ("reviewer-a", crate::domain::Command::ShiftGrid { curve_id:"buffer-a".into(), delta_q:-0.100 }),
        ("reviewer-a", crate::domain::Command::SetBackgroundScale { value:0.98, sigma:0.04, source:"post-edge fit".into(), state:"uncertain".into() }),
        ("reviewer-a", crate::domain::Command::RevisePoint { curve_id:"sample".into(), index:1, intensity:-2.0, sigma:0.05 }),
        ("reviewer-a", crate::domain::Command::SetModelWindows { guinier_q_min:0.012, guinier_q_max:0.034, porod_q_min:0.030, porod_q_max:0.050 }),
        ("reviewer-a", crate::domain::Command::SelectBuffer { buffer_id:"buffer-b".into() }),
        ("reviewer-a", crate::domain::Command::SetModelWindows { guinier_q_min:0.012, guinier_q_max:0.024, porod_q_min:0.030, porod_q_max:0.050 }),
    ];
    let mut replayed = Vec::new();
    for (actor, command) in commands {
        let applied = match store.append(mk(&base, actor, command)) { Ok(a) => a, Err((_, e)) => return error(StatusCode::INTERNAL_SERVER_ERROR, e) };
        base = applied.event.event_id.clone();
        replayed.push(json!({"event_id":applied.event.event_id,"type":applied.event.event_type,"changed":applied.changed_nodes.iter().map(|n| json!({"id":n.id,"version":n.version,"status":n.status})).collect::<Vec<_>>()}));
    }
    let decision_base = base.clone();
    let first = store.append(mk(&decision_base, "reviewer-a", crate::domain::Command::ProposeDecision { decision_id:"decision.choose-buffer-a".into(), actor:"reviewer-a".into(), candidate:"buffer-a".into(), note:"switch back to batch A".into() }));
    let _ = first;
    let conflict = store.append(mk(&decision_base, "reviewer-b", crate::domain::Command::ProposeDecision { decision_id:"decision.keep-buffer-b".into(), actor:"reviewer-b".into(), candidate:"buffer-b".into(), note:"keep batch B".into() }));
    let mut conflict_events = Vec::new();
    if let Err((status, message)) = conflict {
        conflict_events.push(json!({"conflict_status":status,"message":message}));
        let latest = match store.latest() { Ok(p) => p.state.events.last().unwrap().event_id.clone(), Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e) };
        let resolved = match store.append(mk(&latest, "chair", crate::domain::Command::ResolveConflict { conflict_id:"conflict.decision.keep-buffer-b.decision.choose-buffer-a.rejected".into(), resolution:"continue reviewer-b branch; reviewer-a branch stays retained".into(), actor:"chair".into() })) { Ok(a) => a, Err((_, e)) => return error(StatusCode::INTERNAL_SERVER_ERROR, e) };
        conflict_events.push(json!({"resolved_event":resolved.event.event_id}));
        base = resolved.event.event_id.clone();
    }
    let projection = match store.latest() { Ok(p) => p, Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e) };
    Json(json!({"ok":true,"replayed_events":replayed,"conflict_events":conflict_events,"final_event":projection.state.events.last().unwrap().event_id,"final_selected_buffer":projection.state.case.as_ref().unwrap().selected_buffer})).into_response()
}
