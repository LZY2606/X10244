//! Axum HTTP API + embedded single-page UI.

use std::sync::Mutex;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

use crate::casepack;
use crate::graph;
use crate::store::Store;

pub struct AppState {
    pub store: Mutex<Store>,
}

fn with_store<T>(st: &AppState2, f: impl FnOnce(&mut Store) -> T) -> T {
    let mut g = st.store.lock().unwrap();
    f(&mut g)
}

type AppState2 = std::sync::Arc<AppState>;

const INDEX_HTML: &str = include_str!("../static/index.html");

pub fn router(store: Store) -> Router {
    let state = std::sync::Arc::new(AppState { store: Mutex::new(store) });
    Router::new()
        .route("/", get(|| async { Html(INDEX_HTML) }))
        .route("/api/state", get(api_state))
        .route("/api/events/:id", get(api_event))
        .route("/api/nodes/:id", get(api_node))
        .route("/api/versions/:id", get(api_version))
        .route("/api/trace/:version/:point", get(api_trace))
        .route("/api/explain/:node", get(api_explain))
        .route("/api/decide", post(api_decide))
        .route("/api/replay", post(api_replay))
        .route("/api/export", get(api_export))
        .route("/api/import", post(api_import))
        .with_state(state)
}

async fn api_state(State(st): State<AppState2>, Query(q): Query<std::collections::HashMap<String, String>>) -> Json<Value> {
    with_store(&st, |s| {
        let heads = match q.get("at").and_then(|v| v.parse::<i64>().ok()) {
            Some(eid) => s.heads_at(eid),
            None => s.heads(),
        };
        Json(json!({
            "nodes": s.nodes(),
            "heads": heads,
            "events": s.events().iter().map(|e| json!({
                "id": e.id, "kind": e.kind, "actor": e.actor, "summary": e.summary,
            })).collect::<Vec<_>>(),
        }))
    })
}

async fn api_event(State(st): State<AppState2>, Path(id): Path<i64>) -> Json<Value> {
    with_store(&st, |s| match s.event(id) {
        Some(e) => Json(json!({
            "id": e.id, "kind": e.kind, "actor": e.actor, "summary": e.summary,
            "payload": e.payload, "heads_snapshot": e.heads_snapshot,
        })),
        None => Json(json!({ "error": "not found" })),
    })
}

async fn api_node(State(st): State<AppState2>, Path(id): Path<String>) -> Json<Value> {
    with_store(&st, |s| {
        let versions: Vec<Value> = s
            .versions(&id)
            .iter()
            .map(|v| json!({ "id": v.id, "seq": v.seq, "event_id": v.event_id, "chosen": v.chosen }))
            .collect();
        Json(json!({ "id": id, "head": s.head(&id), "versions": versions }))
    })
}

async fn api_version(State(st): State<AppState2>, Path(id): Path<String>) -> Json<Value> {
    with_store(&st, |s| match s.version(&id) {
        Some(v) => Json(json!({ "id": v.id, "node_id": v.node_id, "state": v.state, "chosen": v.chosen })),
        None => Json(json!({ "error": "not found" })),
    })
}

/// Trace a derived point back to its source points on the two source curves.
async fn api_trace(State(st): State<AppState2>, Path((vid, pid)): Path<(String, String)>) -> Json<Value> {
    with_store(&st, |s| {
        let Some(v) = s.version(&vid) else { return Json(json!({ "error": "version not found" })) };
        let curve = &v.state["curve"];
        let inputs = v.state["inputs"].clone();
        let Some(point) = curve["points"].as_array().and_then(|ps| {
            ps.iter().find(|p| p["id"].as_str() == Some(pid.as_str())).cloned()
        }) else {
            return Json(json!({ "error": "point not found" }));
        };
        let mut sources = Vec::new();
        for src in point["sources"].as_array().cloned().unwrap_or_default() {
            let node = src["curve"].as_str().unwrap_or("").replace("norm:", "norm.").replace("sub:", "sub.");
            let node = if node.starts_with("norm:") { node.replacen("norm:", "norm.", 1) } else { node };
            let src_vid = inputs.get(&node).and_then(|x| x.as_str()).map(|s| s.to_string())
                .or_else(|| s.head(&node));
            let mut src_point = json!(null);
            if let Some(sv) = src_vid.clone().and_then(|id| s.version(&id)) {
                let spid = src["point"].as_str().unwrap_or("");
                src_point = sv.state["curve"]["points"]
                    .as_array()
                    .and_then(|ps| ps.iter().find(|p| p["id"].as_str() == Some(spid)).cloned())
                    .unwrap_or(json!(null));
            }
            sources.push(json!({ "node": node, "version": src_vid, "point_id": src["point"], "point": src_point }));
        }
        Json(json!({ "version": vid, "point": point, "sources": sources }))
    })
}

async fn api_explain(State(st): State<AppState2>, Path(node): Path<String>) -> Json<Value> {
    with_store(&st, |s| Json(graph::impact(s, &node)))
}

#[derive(serde::Deserialize)]
struct DecideBody {
    node_id: String,
    version_id: String,
    actor: String,
    rationale: String,
}

async fn api_decide(State(st): State<AppState2>, Json(body): Json<DecideBody>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    with_store(&st, |s| match s.choose_version(&body.node_id, &body.version_id, &body.actor, &body.rationale) {
        Ok(eid) => Ok(Json(json!({ "event_id": eid }))),
        Err(e) => Err((StatusCode::CONFLICT, Json(json!({ "error": format!("{:?}", e) })))),
    })
}

async fn api_replay(State(st): State<AppState2>) -> Json<Value> {
    with_store(&st, |s| {
        *s = Store::open_memory().unwrap();
        let log = casepack::replay(s);
        Json(json!({ "replayed": log.len(), "log": log }))
    })
}

async fn api_export(State(st): State<AppState2>) -> Json<Value> {
    with_store(&st, |s| Json(s.export_json()))
}

async fn api_import(State(st): State<AppState2>, Json(dump): Json<Value>) -> Json<Value> {
    with_store(&st, |s| {
        *s = Store::open_memory().unwrap();
        match s.import_json(&dump) {
            Ok(()) => Json(json!({ "imported": true })),
            Err(e) => Json(json!({ "error": format!("{}", e) })),
        }
    })
}
