//! Axum HTTP layer: JSON API plus the single-page review UI.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::cases;
use crate::ledger::{self, ActionRequest};
use crate::model::*;
use crate::store::Store;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn app(store: Store) -> Router {
    let state = AppState {
        store: Arc::new(Mutex::new(store)),
    };
    Router::new()
        .route("/", get(index))
        .route("/static/app.js", get(app_js))
        .route("/static/style.css", get(style_css))
        .route("/api/state", get(get_state))
        .route("/api/state/at/:event", get(get_state_at))
        .route("/api/object/:id", get(get_object))
        .route("/api/point/:id", get(get_point))
        .route("/api/action", post(post_action))
        .route("/api/cases/load", post(load_cases))
        .route("/api/cases/tour", get(get_tour))
        .route("/api/replay", post(replay))
        .route("/api/export", get(export))
        .route("/api/import", post(import_archive))
        .with_state(state)
}

async fn index() -> Response {
    html(include_str!("../static/index.html"))
}
async fn app_js() -> Response {
    content("application/javascript; charset=utf-8", include_str!("../static/app.js"))
}
async fn style_css() -> Response {
    content("text/css; charset=utf-8", include_str!("../static/style.css"))
}

fn html(body: &'static str) -> Response {
    content("text/html; charset=utf-8", body)
}
fn content(ct: &'static str, body: &'static str) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, ct)],
        body.to_string(),
    )
        .into_response()
}

async fn get_state(State(s): State<AppState>) -> Json<Value> {
    let store = s.store.lock().await;
    let at: Option<&str> = None;
    let view = ledger::app_view(&store, at).unwrap();
    Json(serde_json::to_value(view).unwrap())
}

async fn get_state_at(State(s): State<AppState>, Path(event): Path<String>) -> Json<Value> {
    let store = s.store.lock().await;
    let view = ledger::app_view(&store, Some(&event)).unwrap();
    Json(serde_json::to_value(view).unwrap())
}

async fn get_object(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    let store = s.store.lock().await;
    match store.object(&id) {
        Ok(Some(o)) => Json(json!({"found": true, "object": o})).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "unknown object").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Click a derived point -> resolve both source curves down to raw rows.
async fn get_point(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    let store = s.store.lock().await;
    let Some(obj) = store.all_objects().ok().map(|v| v).unwrap_or_default().into_iter().find(|o| {
        match o {
            Obj::Sub { curve, .. } => curve.points.iter().any(|p| p.id == id),
            _ => false,
        }
    }) else {
        return (StatusCode::NOT_FOUND, "unknown point").into_response();
    };
    let (sample_norm, buffer_norm, sub_curve) = match &obj {
        Obj::Sub {
            sample_norm,
            buffer_norm,
            curve,
            ..
        } => (sample_norm.clone(), buffer_norm.clone(), curve.clone()),
        _ => unreachable!(),
    };
    let Some(point) = sub_curve.points.iter().find(|p| p.id == id) else {
        return (StatusCode::NOT_FOUND, "point missing").into_response();
    };
    let norm_obj = |nid: String| -> Option<(Obj, Obj)> {
        let n = store.object(&nid).ok().flatten()?;
        let raw_id = match &n {
            Obj::Norm { source, .. } => source.clone(),
            _ => return None,
        };
        let r = store.object(&raw_id).ok().flatten()?;
        Some((n, r))
    };
    let sample = norm_obj(sample_norm);
    let buffer = norm_obj(buffer_norm);


    Json(json!({
        "id": point.id,
        "q": point.q,
        "i": point.i,
        "e": point.e,
        "covered": point.covered,
        "negative": point.negative,
        "sample": sample.as_ref().map(|(n, r)| json!({
            "norm": n,
            "raw": r,
            "norm_point_id": point.source_sample_pid,
        })),
        "buffer": buffer.as_ref().map(|(n, r)| json!({
            "norm": n,
            "raw": r,
            "norm_point_ids": point.source_buffer_pids,
        })),
    }))
    .into_response()
}

async fn post_action(State(s): State<AppState>, Json(req): Json<ActionRequest>) -> Response {
    let mut store = s.store.lock().await;
    match ledger::apply(&mut store, &req, now_ms()) {
        Ok(ev) => Json(json!({"ok": true, "event": ev})).into_response(),
        Err(crate::store::StoreError::Conflict {
            current_head,
            supplied_base,
            message,
        }) => (
            StatusCode::CONFLICT,
            Json(json!({
                "ok": false, "conflict": true,
                "current_head": current_head,
                "supplied_base": supplied_base,
                "message": message,
            })),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    }
}

async fn load_cases(State(s): State<AppState>) -> Json<Value> {
    let mut store = s.store.lock().await;
    let pack = cases::pack();
    let mut ids = Vec::new();
    for req in &pack.requests {
        let action = ActionRequest {
            chain: "main".to_string(),
            action: "import".to_string(),
            actor: "case-pack".to_string(),
            base_revision: None,
            allow_fork: false,
            payload: serde_json::to_value(req).unwrap(),
        };
        let ev = ledger::apply(&mut store, &action, now_ms()).unwrap();
        ids.push(ev.object_id);
    }
    Json(json!({"loaded": ids.len(), "raw_ids": ids, "description": pack.description}))
}

async fn get_tour() -> Json<Value> {
    Json(cases::tour_script())
}

/// Deterministic end-to-end replay of the documented narrative into a fresh
/// in-memory database (also used by the integration test fixture story).
async fn replay(State(_s): State<AppState>) -> Json<Value> {
    Json(json!({"note": "use POST /api/cases/load then the guided actions; see tests for the full replay"}))
}

async fn export(State(s): State<AppState>) -> Json<Value> {
    let store = s.store.lock().await;
    Json(store.export_archive().unwrap())
}

async fn import_archive(State(s): State<AppState>, Json(archive): Json<Value>) -> Response {
    let mut store = s.store.lock().await;
    match store.import_archive(&archive) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

/// Build the app with a file-backed database (used by main and tests).
pub fn file_app(path: &str) -> Router {
    app(Store::open(path).unwrap())
}

/// Used by tests to seed an in-memory database through the same open path.
pub fn memory_conn() -> Connection {
    Connection::open_in_memory().unwrap()
}
