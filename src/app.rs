use crate::domain::{Event, EventKind};
use crate::engine::replay;
use crate::store::{ExportBundle, ForkInput, PublishInput, Store};
use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
}

pub fn router(store: Arc<Store>) -> Router {
    let state = AppState { store };
    Router::new()
        .route("/", get(index))
        .route("/api/health", get(health))
        .route("/api/state", get(state_handler))
        .route("/api/events", get(events_handler))
        .route("/api/branches", get(branches_handler).post(fork_handler))
        .route("/api/events/publish", post(publish_handler))
        .route("/api/export", get(export_handler))
        .route("/api/import", post(import_handler))
        .route("/api/demo/reset", post(reset_handler))
        .route("/api/demo/fork-conflict", post(fork_conflict_handler))
        .with_state(state)
}

async fn index() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../web/index.html"),
    )
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok"}))
}

#[derive(Deserialize)]
struct BranchQuery {
    branch: Option<String>,
    at_event: Option<String>,
}

fn branch(query: Option<Query<BranchQuery>>) -> String {
    query
        .and_then(|Query(value)| value.branch)
        .unwrap_or_else(|| "main".to_string())
}

async fn state_handler(
    State(state): State<AppState>,
    Query(query): Query<BranchQuery>,
) -> axum::response::Response {
    let branch = query.branch.clone().unwrap_or_else(|| "main".to_string());
    let events = match state.store.events_for_branch(&branch) {
        Ok(events) => events,
        Err(error) => return (StatusCode::NOT_FOUND, error.to_string()).into_response(),
    };
    if let Some(at_event) = query.at_event {
        return match events_until(&events, &at_event) {
            Some(prefix) => {
                Json(serde_json::to_value(replay(&prefix)).unwrap_or_default()).into_response()
            }
            None => (
                StatusCode::NOT_FOUND,
                format!("event {at_event} is not on branch {branch}"),
            )
                .into_response(),
        };
    }
    match state.store.state(&branch) {
        Ok(project) => Json(serde_json::to_value(project).unwrap_or_default()).into_response(),
        Err(error) => (StatusCode::NOT_FOUND, error.to_string()).into_response(),
    }
}

async fn events_handler(
    State(state): State<AppState>,
    Query(query): Query<BranchQuery>,
) -> axum::response::Response {
    let branch = query.branch.unwrap_or_else(|| "main".to_string());
    match state.store.events_for_branch(&branch) {
        Ok(events) => Json(events).into_response(),
        Err(error) => (StatusCode::NOT_FOUND, error.to_string()).into_response(),
    }
}

fn events_until<'a>(events: &'a [Event], event_id: &str) -> Option<Vec<Event>> {
    let index = events.iter().position(|event| event.event_id == event_id)?;
    Some(events[..=index].to_vec())
}

async fn branches_handler(State(state): State<AppState>) -> axum::response::Response {
    match state.store.branches() {
        Ok(branches) => Json(branches).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn publish_handler(
    State(state): State<AppState>,
    Json(input): Json<PublishInput>,
) -> axum::response::Response {
    match state.store.publish(&input) {
        Ok(event) => (StatusCode::CREATED, Json(event)).into_response(),
        Err(crate::store::StoreError::Conflict(branch, expected, current)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error":"OPTIMISTIC_LOCK_CONFLICT",
                "branch": branch,
                "expected_parent": expected,
                "current_head": current,
                "resolution":"fork_branch_or_rebase_review",
            })),
        )
            .into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

async fn fork_handler(
    State(state): State<AppState>,
    Json(input): Json<ForkInput>,
) -> axum::response::Response {
    match state.store.fork(&input) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => (StatusCode::CONFLICT, error.to_string()).into_response(),
    }
}

async fn export_handler(State(state): State<AppState>) -> axum::response::Response {
    match state.store.export() {
        Ok(bundle) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::to_vec(&bundle).unwrap_or_default(),
        )
            .into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn import_handler(
    State(state): State<AppState>,
    Json(bundle): Json<ExportBundle>,
) -> axum::response::Response {
    match state.store.import_bundle(&bundle) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

async fn reset_handler(State(state): State<AppState>) -> axum::response::Response {
    match state.store.reset_for_demo() {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn fork_conflict_handler(State(state): State<AppState>) -> axum::response::Response {
    if state
        .store
        .branches()
        .map(|branches| branches.iter().any(|(name, _)| name == "reviewer-toru"))
        .unwrap_or(false)
    {
        return StatusCode::NO_CONTENT.into_response();
    }
    let result: std::result::Result<(), crate::store::StoreError> = (|| {
        state.store.fork(&ForkInput {
            source_branch: "main".to_string(),
            new_branch: "reviewer-toru".to_string(),
            at_event_id: Some("evt-0010".to_string()),
        })?;
        state.store.publish(&PublishInput {
            branch: "reviewer-toru".to_string(),
            parent_event_id: "evt-0010".to_string(),
            actor: "reviewer-toru".to_string(),
            kind: EventKind::CandidateDecision {
                candidate_id: "background:sample-a.buffer-b.fixed_0p95".to_string(),
                decision: "accepted".to_string(),
                actor: "reviewer-toru".to_string(),
                reason: "第二位审阅者在同一候选集合选择保守固定缩放，形成可解决冲突。".to_string(),
            },
        })?;
        Ok(())
    })();
    match result {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(error) => (StatusCode::CONFLICT, error.to_string()).into_response(),
    }
}
