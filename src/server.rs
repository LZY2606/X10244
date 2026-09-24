use crate::engine::apply;
use crate::state::{build_state, provenance, replay};
use crate::store::{Event, Store};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::sync::Arc;

pub struct AppState {
    pub store: Store,
}

const INDEX: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE: &str = include_str!("../web/style.css");
const CASE_PACK: &str = include_str!("../fixtures/case_pack.json");

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(|| async { Html(INDEX) }))
        .route("/app.js", get(|| async { ([("content-type", "text/javascript")], APP_JS) }))
        .route("/style.css", get(|| async { ([("content-type", "text/css")], STYLE) }))
        .route("/api/state", get(api_state))
        .route("/api/events", get(api_events))
        .route("/api/event", post(api_event))
        .route("/api/provenance/:id", get(api_provenance))
        .route("/api/replay_case", post(api_replay_case))
        .route("/api/export", get(api_export))
        .route("/api/import", post(api_import))
        .route("/api/sample_path", get(api_sample_path))
        .with_state(state)
}

fn err(status: StatusCode, msg: String) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error": msg})))
}

async fn api_state(
    State(st): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let at = q.get("at_seq").and_then(|s| s.parse::<i64>().ok());
    build_state(&st.store, at).map(Json).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))
}

async fn api_events(State(st): State<Arc<AppState>>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let events = st.store.events().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let v: Vec<Value> = events
        .iter()
        .map(|e| json!({"seq": e.seq, "id": e.id, "actor": e.actor, "kind": e.kind, "payload": e.payload, "base_seq": e.base_seq, "ts": e.ts}))
        .collect();
    Ok(Json(json!(v)))
}

#[derive(serde::Deserialize)]
struct EventReq {
    actor: Option<String>,
    kind: String,
    payload: Value,
    base_seq: Option<i64>,
}

async fn api_event(
    State(st): State<Arc<AppState>>,
    Json(req): Json<EventReq>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let actor = req.actor.unwrap_or_else(|| "reviewer".into());
    match apply(&st.store, &actor, &req.kind, &req.payload, req.base_seq) {
        Ok(ev) => Ok(Json(json!({"seq": ev.seq, "id": ev.id}))),
        Err(e) => {
            if e.contains("\"conflict\"") {
                let v: Value = serde_json::from_str(&e).unwrap_or(json!({"error": e}));
                Err((StatusCode::CONFLICT, Json(v)))
            } else {
                Err(err(StatusCode::BAD_REQUEST, e))
            }
        }
    }
}

async fn api_provenance(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    provenance(&st.store, &id).map(Json).map_err(|e| err(StatusCode::NOT_FOUND, e))
}

async fn api_replay_case(State(st): State<Arc<AppState>>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let pack: Value = serde_json::from_str(CASE_PACK).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let events: Vec<Event> = pack
        .get("events")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, e)| Event {
            seq: (i + 1) as i64,
            id: String::new(),
            ts: String::new(),
            actor: e.get("actor").and_then(|x| x.as_str()).unwrap_or("case").to_string(),
            kind: e.get("kind").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            payload: e.get("payload").cloned().unwrap_or(json!({})),
            base_seq: None,
        })
        .collect();
    replay(&st.store, &events).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({"replayed": events.len(), "case": pack.get("title")})))
}

async fn api_export(State(st): State<Arc<AppState>>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let events = st.store.events().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let v: Vec<Value> = events
        .iter()
        .map(|e| json!({"actor": e.actor, "kind": e.kind, "payload": e.payload, "base_seq": e.base_seq}))
        .collect();
    Ok(Json(json!({"format": "saxs-station-events", "version": 2, "events": v})))
}

async fn api_import(
    State(st): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let events: Vec<Event> = body
        .get("events")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, e)| Event {
            seq: (i + 1) as i64,
            id: String::new(),
            ts: String::new(),
            actor: e.get("actor").and_then(|x| x.as_str()).unwrap_or("import").to_string(),
            kind: e.get("kind").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            payload: e.get("payload").cloned().unwrap_or(json!({})),
            base_seq: e.get("base_seq").and_then(|x| x.as_i64()),
        })
        .collect();
    replay(&st.store, &events).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(json!({"imported": events.len()})))
}

async fn api_sample_path() -> Json<Value> {
    Json(json!({
        "title": "90 秒样例路径",
        "steps": [
            "1. 点击「重放案例包」：导入样品/缓冲液/仪器响应/遮罩，完成归一化与背景扣除",
            "2. 在「派生曲线」面板点击扣除曲线的任意点，右侧显示两条源曲线与来源链",
            "3. 在「诊断事件」中查看 q 网格错位、无重叠、负强度、区间重叠、多候选残差",
            "4. 在「事件时间线」点击旧事件，界面恢复到当时的依赖版本与结论",
            "5. 在「裁定」面板对候选做接受/否定/调整，观察冲突与解决"
        ]
    }))
}
