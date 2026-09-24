use crate::events::{EventView, PointTrace, ReplayState, ValidityStatement};
use crate::reductions::{curve_version_id, ReductionRequest};
use crate::service::{AppState, AppendCommand};
use crate::storage::{Storage, StorageError};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Deserialize)]
struct AtQuery {
    at: Option<String>,
}

#[derive(Debug, Serialize)]
struct StateResponse {
    state: ReplayState,
    events: Vec<EventView>,
    traces: Vec<PointTrace>,
    source_versions: Vec<SourceVersion>,
}

#[derive(Debug, Serialize)]
struct SourceVersion {
    curve_id: String,
    role: String,
    label: String,
    version_id: String,
    point_count: usize,
}

#[derive(Debug, Deserialize)]
struct RecomputeBody {
    key: String,
    sample_id: String,
    buffer_id: String,
    label: Option<String>,
    actor: Option<String>,
    timestamp: Option<String>,
    parent_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ConflictBody {
    command: AppendCommand,
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/static/app.js", get(app_js))
        .route("/api/state", get(state_view))
        .route("/api/events", get(events_json))
        .route("/api/events", post(append_event))
        .route("/api/events/conflict", post(append_conflict))
        .route("/api/recompute", post(recompute))
        .route("/api/trace/{point_id}", get(trace_point))
        .route("/api/export", get(export))
        .route("/api/import", post(import_events))
        .with_state(state)
}

async fn index() -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], include_str!("../static/index.html")).into_response()
}

async fn app_js() -> Response {
    ([(header::CONTENT_TYPE, "application/javascript; charset=utf-8")], include_str!("../static/app.js")).into_response()
}

fn source_versions(state: &ReplayState) -> Vec<SourceVersion> {
    let Some(bundle) = state.bundle.as_ref() else { return Vec::new() };
    let mut result = Vec::new();
    for sample in bundle.samples.values() {
        result.push(SourceVersion {
            curve_id: sample.curve.id.clone(),
            role: "sample".to_string(),
            label: sample.curve.label.clone(),
            version_id: curve_version_id(&sample.curve),
            point_count: sample.curve.points.len(),
        });
    }
    for buffer in bundle.buffers.values() {
        result.push(SourceVersion {
            curve_id: buffer.curve.id.clone(),
            role: "buffer".to_string(),
            label: buffer.curve.label.clone(),
            version_id: curve_version_id(&buffer.curve),
            point_count: buffer.curve.points.len(),
        });
    }
    result
}

fn event_views(state: &ReplayState) -> Vec<EventView> {
    let all_events = state.events.clone();
    all_events.iter().map(|event| {
        let before = crate::state::replay(&all_events, None);
        let mut changed = Vec::new();
        let mut affected = Vec::new();
        match &event.payload {
            crate::events::EventPayload::CaseImported { .. } => changed.push("case-import".to_string()),
            crate::events::EventPayload::ParameterRevised { curve_id, field, .. } => {
                changed.push(format!("parameter:{curve_id}:{field}"));
                affected.extend(before.runs.iter().filter(|run| run.sample_id == *curve_id || run.buffer_id == *curve_id).map(|run| run.key.clone()));
            }
            crate::events::EventPayload::SourcePointPatched { patch } => {
                changed.push(format!("point:{}:{}", patch.curve_id, patch.point_id));
                affected.extend(before.runs.iter().filter(|run| run.sample_id == patch.curve_id || run.buffer_id == patch.curve_id).map(|run| run.key.clone()));
            }
            crate::events::EventPayload::ReductionRecomputed { run } => affected.push(run.key.clone()),
            _ => {}
        }
        EventView {
            event: event.clone(),
            heads: before.heads.clone(),
            changed_inputs: changed,
            affected_runs: affected,
            validity: validity_at(event, &before),
            current_bundle_hash: before.bundle.as_ref().map(crate::reductions::stable_hash).unwrap_or_else(|| "none".to_string()),
        }
    }).collect()
}

fn validity_at(event: &crate::events::Event, before: &ReplayState) -> Vec<ValidityStatement> {
    let mut result = Vec::new();
    match &event.payload {
        crate::events::EventPayload::ParameterRevised { curve_id, .. }
        | crate::events::EventPayload::SourcePointPatched { patch: crate::events::SourcePointPatch { curve_id, .. } } => {
            for run in &before.runs {
                if run.sample_id == *curve_id || run.buffer_id == *curve_id {
                    result.push(ValidityStatement { run_id: run.id.clone(), valid: false, reason: "Input hash changed; this reduction needs recomputation.".to_string() });
                } else {
                    result.push(ValidityStatement { run_id: run.id.clone(), valid: true, reason: "Dependency graph has no edge from the changed source to this reduction.".to_string() });
                }
            }
        }
        crate::events::EventPayload::ReductionRecomputed { run } => {
            result.push(ValidityStatement { run_id: run.id.clone(), valid: !matches!(run.status, crate::domain::RunStatus::Invalid), reason: "Computed as a complete new version; current pointer switched only after success.".to_string() });
        }
        crate::events::EventPayload::ConflictDetected { .. } => {
            for run in &before.runs {
                result.push(ValidityStatement { run_id: run.id.clone(), valid: true, reason: "The calculation is preserved while the human decision conflict is unresolved.".to_string() });
            }
        }
        _ => {}
    }
    result
}

async fn state_view(State(app): State<AppState>, Query(query): Query<AtQuery>) -> Json<StateResponse> {
    let state = app.state_at(query.at.as_deref());
    let traces = state.runs.iter().flat_map(|run| crate::state::trace_point(&state, &run.subtracted.first().map(|point| point.id.clone()).unwrap_or_default())).collect();
    Json(StateResponse {
        source_versions: source_versions(&state),
        events: event_views(&state),
        traces,
        state,
    })
}

async fn events_json(State(app): State<AppState>) -> Json<BTreeMap<String, Vec<crate::events::Event>>> {
    Json(BTreeMap::from([("events".to_string(), app.events())]))
}

async fn append_event(State(app): State<AppState>, Json(command): Json<AppendCommand>) -> Result<Json<crate::events::Event>, (StatusCode, Json<ErrorResponse>)> {
    app.append(command, false).map(Json).map_err(api_error)
}

async fn append_conflict(State(app): State<AppState>, Json(body): Json<ConflictBody>) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    app.append(body.command, true).map(|event| Json(serde_json::json!({ "fork_event": event }))).map_err(api_error)
}

async fn recompute(State(app): State<AppState>, Json(body): Json<RecomputeBody>) -> Result<Json<crate::events::Event>, (StatusCode, Json<ErrorResponse>)> {
    let request = ReductionRequest {
        key: body.key,
        sample_id: body.sample_id,
        buffer_id: body.buffer_id,
        label: body.label.unwrap_or_else(|| "manual recompute".to_string()),
    };
    app.recompute(
        request,
        body.actor.as_deref().unwrap_or("reviewer"),
        body.parent_ids.unwrap_or_default(),
        body.timestamp.unwrap_or_else(|| "2026-09-24T10:00:00+09:00".to_string()),
    ).map(Json).map_err(api_error)
}

async fn trace_point(State(app): State<AppState>, Path(point_id): Path<String>, Query(query): Query<AtQuery>) -> Result<Json<PointTrace>, (StatusCode, Json<ErrorResponse>)> {
    let state = app.state_at(query.at.as_deref());
    crate::state::trace_point(&state, &point_id).map(Json).ok_or((StatusCode::NOT_FOUND, Json(ErrorResponse { error: "derived point not found".to_string() })))
}

async fn export(State(app): State<AppState>) -> Response {
    match app.storage.lock().unwrap().export_json() {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json"), (header::CONTENT_DISPOSITION, "attachment; filename=\"saxs-events.json\"")], json).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: error.to_string() })).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct ImportBody {
    events: Option<Vec<crate::events::Event>>,
    envelope: Option<serde_json::Value>,
}

async fn import_events(State(app): State<AppState>, Json(body): Json<ImportBody>) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let storage = app.storage.lock().unwrap();
    let count = if let Some(events) = body.events {
        storage.replace_with_events(events).map(|_| 0)
    } else if let Some(envelope) = body.envelope {
        storage.import_json(&envelope.to_string())
    } else {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "provide events or envelope".to_string() })));
    }.map_err(|error| api_error(error.to_string()))?;
    Ok(Json(serde_json::json!({ "imported": count.max(1) })))
}

fn api_error(error: String) -> (StatusCode, Json<ErrorResponse>) {
    let status = if error.contains("expected heads") { StatusCode::CONFLICT } else { StatusCode::BAD_REQUEST };
    (status, Json(ErrorResponse { error }))
}

pub fn build_router(path: &str) -> Router {
    router(AppState::new(path))
}

pub fn build_router_from_state(state: AppState) -> Router {
    router(state)
}

#[allow(dead_code)]
fn force_storage_error_type(_error: StorageError) {}

#[allow(dead_code)]
fn force_arc_type(_state: Arc<Mutex<Storage>>) {}
