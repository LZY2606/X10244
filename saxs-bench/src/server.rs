//! Axum web layer. All pages are offline: the single HTML/JS/CSS bundle is
//! embedded at compile time and served from `/`.

use crate::events::{fold, EventKind, State, Validity};
use crate::model::{Decision, DecisionKind};
use crate::replay::Step;
use crate::store::{CommitOutcome, Conflict, Store};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const INDEX_HTML: &str = include_str!("../web/index.html");

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub head: Arc<Mutex<i64>>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Server-side view of one derived point's two-source traceback.
#[derive(Debug, Serialize)]
struct Provenance {
    point_id: String,
    q: f64,
    sample: SourcePoint,
    buffer: Option<SourcePoint>,
    in_overlap: bool,
}

#[derive(Debug, Serialize)]
struct SourcePoint {
    norm_id: String,
    profile_id: String,
    label: String,
    point_index: usize,
    q: f64,
    q_source_basis: f64,
    i: f64,
    sigma_stat: f64,
    raw_q: String,
    raw_i: String,
    raw_e: String,
    masked: bool,
    mask_reason: Option<String>,
    lo: Option<usize>,
    hi: Option<usize>,
}

#[derive(Debug, Serialize)]
struct Snapshot {
    name: String,
    rules_version: String,
    head_seq: i64,
    view_seq: i64,
    events: Vec<EventView>,
    raw_profiles: serde_json::Value,
    norms: serde_json::Value,
    branches: serde_json::Value,
    graph: serde_json::Value,
    decisions: serde_json::Value,
    scenarios: Vec<(String, String)>,
}

#[derive(Debug, Serialize)]
struct EventView {
    seq: i64,
    id: String,
    parent_seq: i64,
    at_ms: i64,
    summary: String,
    kind: String,
}

fn kind_summary(k: &EventKind) -> String {
    match k {
        EventKind::ProjectCreated { name } => format!("建立项目：{name}"),
        EventKind::ProfileAdded { profile } => format!("导入原始记录：{} ({})", profile.label, profile.id),
        EventKind::ProfileUpdated { profile } => format!("修订原始记录：{} ({})", profile.label, profile.version),
        EventKind::BufferCandidate { sample_id, buffer_id } => {
            format!("声明缓冲液候选：{buffer_id} → {sample_id}")
        }
        EventKind::FitWindow { sample_id, buffer_id, fit_lo, fit_hi } => {
            format!("参数修订(拟合窗)：{buffer_id} [{fit_lo:.3},{fit_hi:.3}] → {sample_id}")
        }
        EventKind::Decision { sample_id, decision, .. } => {
            format!("人工裁定[{:?}]：{} @ {}", decision.kind, decision.branch_id, sample_id)
        }
        EventKind::RuleActivated { version } => format!("启用规则版本：{version}"),
    }
}

fn kind_name(k: &EventKind) -> &'static str {
    match k {
        EventKind::ProjectCreated { .. } => "project_created",
        EventKind::ProfileAdded { .. } => "profile_added",
        EventKind::ProfileUpdated { .. } => "profile_updated",
        EventKind::BufferCandidate { .. } => "buffer_candidate",
        EventKind::FitWindow { .. } => "fit_window",
        EventKind::Decision { .. } => "decision",
        EventKind::RuleActivated { .. } => "rule_activated",
    }
}

impl AppState {
    fn state_at(&self, seq: Option<i64>) -> State {
        let events = self.store.load_events().unwrap_or_default();
        fold(&events, seq)
    }

    fn current_head(&self) -> i64 {
        *self.head.lock().unwrap()
    }

    fn snapshot(&self, view_seq: Option<i64>) -> Snapshot {
        let head = self.store.head_seq().unwrap_or(0);
        let view = view_seq.unwrap_or(head);
        let st = self.state_at(Some(view));

        // Annotate branches with selection status from decisions and proposal.
        let mut branches = serde_json::to_value(&st.branches).unwrap_or(serde_json::json!([]));
        if let Some(arr) = branches.as_array_mut() {
            // current proposal = last branch per buffer_norm
            let mut proposed = std::collections::HashSet::new();
            let mut seen = std::collections::HashSet::new();
            for b in arr.iter().rev() {
                let key = b["buffer_norm"].as_str().unwrap_or("").to_string();
                if seen.insert(key.clone()) {
                    proposed.insert(b["id"].as_str().unwrap_or("").to_string());
                }
            }
            for b in arr.iter_mut() {
                let bid = b["id"].as_str().unwrap_or("").to_string();
                let status = if let Some((_, d, _)) = st.decision_for(&bid) {
                    match d.kind {
                        DecisionKind::Accept => "accepted",
                        DecisionKind::Reject => "rejected",
                        DecisionKind::ParamAdjust => "param_adjusted",
                    }
                } else if proposed.contains(&bid) {
                    "proposed"
                } else {
                    "superseded"
                };
                b["selection"] = serde_json::json!(status);
            }
        }

        let events: Vec<EventView> = st
            .events
            .iter()
            .map(|e| EventView {
                seq: e.seq,
                id: e.id.clone(),
                parent_seq: e.parent_seq,
                at_ms: e.at_ms,
                summary: kind_summary(&e.kind),
                kind: kind_name(&e.kind).into(),
            })
            .collect();

        let decisions = serde_json::to_value(
            &st.decisions
                .iter()
                .map(|(bid, d, seq)| serde_json::json!({"branch_id": bid, "decision": d, "at_seq": seq}))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();

        Snapshot {
            name: st.name,
            rules_version: st.rules_version,
            head_seq: head,
            view_seq: view,
            events,
            raw_profiles: serde_json::to_value(&st.profiles).unwrap_or_default(),
            norms: serde_json::to_value(&st.norms).unwrap_or_default(),
            branches,
            graph: serde_json::to_value(&st.graph).unwrap_or_default(),
            decisions,
            scenarios: crate::replay::scenario_index()
                .into_iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        }
    }

    fn commit(&self, kind: EventKind, base_seq: i64) -> Result<CommitOutcome, Conflict> {
        let out = self.store.append(kind, now_ms(), base_seq)?;
        *self.head.lock().unwrap() = out.head_seq;
        Ok(out)
    }
}

use axum::{
    extract::{Path, Query, State as AxState},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::Deserialize;

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflict: Option<Conflict>,
}

impl IntoResponse for Conflict {
    fn into_response(self) -> axum::response::Response {
        (
            StatusCode::CONFLICT,
            Json(ApiError { error: self.message.clone(), conflict: Some(self) }),
        )
            .into_response()
    }
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

#[derive(Deserialize)]
struct ViewQ {
    seq: Option<i64>,
}

async fn snapshot(
    AxState(app): AxState<AppState>,
    Query(q): Query<ViewQ>,
) -> Json<Snapshot> {
    Json(app.snapshot(q.seq))
}

/// Trace a derived point back to its two source curves.
async fn provenance(
    AxState(app): AxState<AppState>,
    Path(point_id): Path<String>,
    Query(q): Query<ViewQ>,
) -> Result<Json<Provenance>, (StatusCode, String)> {
    let st = app.state_at(q.seq);
    for branch in &st.branches {
        if let Some(sp) = branch.points.iter().find(|p| p.id == point_id) {
            let sn = st.norm(&sp.source_sample_norm).ok_or((
                StatusCode::NOT_FOUND,
                "样品归一化曲线缺失".into(),
            ))?;
            let s_pt = sn.points.iter().find(|p| p.source_index == sp.source_sample_index);
            let s_raw = st.profile(&sn.source_profile);
            let sample = source_point(sn, s_pt, s_raw, None);

            let buffer = if let (Some(bn), Some(lo), Some(hi)) = (
                st.norm(&sp.source_buffer_norm),
                sp.source_buffer_lo,
                sp.source_buffer_hi,
            ) {
                let b_pt = bn.points.get(lo);
                let b_raw = st.profile(&bn.source_profile);
                Some(source_point(bn, b_pt, b_raw, Some((lo, hi))))
            } else {
                None
            };
            return Ok(Json(Provenance {
                point_id,
                q: sp.q,
                sample,
                buffer,
                in_overlap: sp.in_overlap,
            }));
        }
    }
    Err((StatusCode::NOT_FOUND, format!("找不到派生点 {point_id}")))
}

fn source_point(
    norm: &crate::model::NormCurve,
    pt: Option<&crate::model::NormPoint>,
    raw: Option<&crate::model::RawProfile>,
    bracket: Option<(usize, usize)>,
) -> SourcePoint {
    let (q, i, sigma, qsb, idx) = match pt {
        Some(p) => (p.q, p.i, p.sigma_stat, p.q_source_basis, p.source_index),
        None => (f64::NAN, f64::NAN, f64::NAN, f64::NAN, 0),
    };
    let raw_pt = raw.and_then(|r| r.points.iter().find(|p| p.index == idx));
    SourcePoint {
        norm_id: norm.id.clone(),
        profile_id: norm.source_profile.clone(),
        label: norm.label.clone(),
        point_index: idx,
        q,
        q_source_basis: qsb,
        i,
        sigma_stat: sigma,
        raw_q: raw_pt.map(|p| p.q_raw.clone()).unwrap_or_default(),
        raw_i: raw_pt.map(|p| p.i_raw.clone()).unwrap_or_default(),
        raw_e: raw_pt.map(|p| p.e_raw.clone()).unwrap_or_default(),
        masked: raw_pt.map(|p| p.masked).unwrap_or(false),
        mask_reason: raw_pt.and_then(|p| p.mask_reason.clone()),
        lo: bracket.map(|x| x.0),
        hi: bracket.map(|x| x.1),
    }
}

/// Explain graph validity relative to a changed node set.
#[derive(Deserialize)]
struct ValidityBody {
    changed: Vec<String>,
}

async fn validity(
    AxState(app): AxState<AppState>,
    Query(q): Query<ViewQ>,
    Json(body): Json<ValidityBody>,
) -> Json<serde_json::Value> {
    let st = app.state_at(q.seq);
    let affected = st.affected_by(&body.changed);
    let nodes: Vec<Validity> = st
        .graph
        .iter()
        .map(|n| st.validity(&n.id, &body.changed))
        .collect();
    Json(serde_json::json!({"affected": affected, "nodes": nodes}))
}

#[derive(Deserialize)]
struct DecisionBody {
    base_seq: Option<i64>,
    sample_id: String,
    branch_id: String,
    kind: String,
    note: String,
    #[serde(default)]
    params: serde_json::Value,
}

async fn decide(
    AxState(app): AxState<AppState>,
    Json(body): Json<DecisionBody>,
) -> Result<Json<CommitOutcome>, Conflict> {
    let base = body.base_seq.unwrap_or_else(|| app.current_head());
    let kind = match body.kind.as_str() {
        "accept" => DecisionKind::Accept,
        "reject" => DecisionKind::Reject,
        "param_adjust" => DecisionKind::ParamAdjust,
        _ => DecisionKind::Accept,
    };
    let decision = Decision {
        branch_id: body.branch_id.clone(),
        kind,
        note: body.note,
        params: body.params,
    };
    app.commit(
        EventKind::Decision {
            sample_id: body.sample_id,
            branch_id: body.branch_id,
            decision,
        },
        base,
    )
    .map(Json)
}

#[derive(Deserialize)]
struct FitBody {
    base_seq: Option<i64>,
    sample_id: String,
    buffer_id: String,
    fit_lo: f64,
    fit_hi: f64,
}

async fn fit_window(
    AxState(app): AxState<AppState>,
    Json(body): Json<FitBody>,
) -> Result<Json<CommitOutcome>, Conflict> {
    let base = body.base_seq.unwrap_or_else(|| app.current_head());
    app.commit(
        EventKind::FitWindow {
            sample_id: body.sample_id,
            buffer_id: body.buffer_id,
            fit_lo: body.fit_lo,
            fit_hi: body.fit_hi,
        },
        base,
    )
    .map(Json)
}

#[derive(Deserialize)]
struct ReplayBody {
    seed: Option<i64>,
    /// when present, simulate a write crash on this 0-based step
    crash_at: Option<usize>,
}

/// Run (or crash-test) the offline replay. On success the store is reset and
/// the script appended. With `crash_at`, that step is rolled back and the
/// store must remain at the prior consistent prefix.
async fn replay(
    AxState(app): AxState<AppState>,
    Json(body): Json<ReplayBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let seed = body.seed.unwrap_or(0);
    let steps: Vec<Step> = crate::replay::script(seed, 0);
    app.store.wipe().map_err(internal)?;
    *app.head.lock().unwrap() = 0;

    let crash_at = body.crash_at;
    let mut applied = 0usize;
    for (i, step) in steps.iter().enumerate() {
        let base = app.store.head_seq().map_err(internal)?;
        let do_crash = crash_at == Some(i);
        let res = app.store.append_with_crash(step.kind.clone(), 1_700_000_000_000 + i as i64, base, do_crash);
        match res {
            Ok(out) => {
                *app.head.lock().unwrap() = out.head_seq;
                applied += 1;
            }
            Err(c) if c.message == "SIMULATED_CRASH_BEFORE_COMMIT" => {
                let head = app.store.head_seq().map_err(internal)?;
                return Ok(Json(serde_json::json!({
                    "crashed": true,
                    "step": i,
                    "title": step.title,
                    "recovered_head_seq": head,
                    "applied_before_crash": applied,
                    "atomic": head == applied as i64,
                })));
            }
            Err(c) => return Err((StatusCode::CONFLICT, c.message)),
        }
    }
    let head = app.store.head_seq().map_err(internal)?;
    Ok(Json(serde_json::json!({"crashed": false, "applied": applied, "head_seq": head})))
}

async fn export(AxState(app): AxState<AppState>) -> Result<String, (StatusCode, String)> {
    app.store.export_pack().map_err(internal)
}

#[derive(Deserialize)]
struct ImportBody {
    pack: String,
}

async fn import(
    AxState(app): AxState<AppState>,
    Json(body): Json<ImportBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let n = app.store.import_pack(&body.pack).map_err(internal)?;
    let head = app.store.head_seq().map_err(internal)?;
    *app.head.lock().unwrap() = head;
    Ok(Json(serde_json::json!({"imported_events": n, "head_seq": head})))
}

async fn reset(AxState(app): AxState<AppState>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    app.store.wipe().map_err(internal)?;
    *app.head.lock().unwrap() = 0;
    Ok(Json(serde_json::json!({"head_seq": 0})))
}

fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub fn router(app: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/state", get(snapshot))
        .route("/api/provenance/{point_id}", get(provenance))
        .route("/api/validity", post(validity))
        .route("/api/decide", post(decide))
        .route("/api/fit-window", post(fit_window))
        .route("/api/replay", post(replay))
        .route("/api/export", get(export))
        .route("/api/import", post(import))
        .route("/api/reset", post(reset))
        .with_state(app)
}

/// Build an app backed by a SQLite file.
pub fn app_for_db(path: &str) -> std::io::Result<Router> {
    let store = Store::open(path).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let head = store.head_seq().unwrap_or(0);
    Ok(router(AppState { store: Arc::new(store), head: Arc::new(Mutex::new(head)) }))
}

pub async fn serve(addr: &str, path: &str) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let app = app_for_db(path)?;
    tracing_free_log(&format!("散射曲线台 已启动: http://{addr}"));
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}

fn tracing_free_log(msg: &str) {
    println!("{msg}");
}
