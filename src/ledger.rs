//! Action application and view reconstruction.
//! The ledger is the only source of truth: a "view" is the deterministic
//! fold of a chain's event ancestry over the content-addressed object store.
//! Revisiting any old event replays THAT ancestry, never the latest results.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::{self, ImportRequest};
use crate::graph::Graph;
use crate::hash::hash_canonical;
use crate::model::*;
use crate::store::{Store, StoreError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ActionRequest {
    pub chain: String,
    pub action: String,
    #[serde(default)]
    pub actor: String,
    #[serde(default)]
    pub base_revision: Option<i64>,
    /// stale-base edits fork a resolvable chain instead of 409
    #[serde(default)]
    pub allow_fork: bool,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChainView {
    pub chain: String,
    pub head: Option<String>,
    pub revision: i64,
    pub events: Vec<Event>,
    /// per kind, the object id selected at this point of the fold
    pub current: BTreeMap<String, String>,
    /// raw label/kind/id listing
    pub raws: Vec<RawSummary>,
    pub norm_of: BTreeMap<String, String>,
    /// subtractions grouped by sample norm id; newest fold order first
    pub subtractions: BTreeMap<String, Vec<String>>,
    pub selected_sub: BTreeMap<String, String>,
    pub candidates_of: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RawSummary {
    pub id: String,
    pub kind: CurveKind,
    pub label: String,
    pub path: String,
    pub diagnostics: Vec<String>,
    pub convertible: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppView {
    pub main: ChainView,
    pub forks: Vec<ChainView>,
    pub chain: ChainView,
    pub graph: Value,
    pub validity: Vec<String>,
}

/// Reconstruct a chain view by walking parent links from `head`.
pub fn view_at(store: &Store, chain: &str, head_override: Option<&str>) -> Result<ChainView, StoreError> {
    let mut events = Vec::new();
    let mut seen = BTreeSet::new();
    let mut cur = match head_override {
        Some(h) => Some(h.to_string()),
        None => store.chain_head(chain)?,
    };
    while let Some(id) = cur {
        if !seen.insert(id.clone()) {
            break;
        }
        let ev = store.event(&id)?.expect("dangling event pointer");
        cur = ev.parent.clone();
        events.push(ev);
    }
    events.reverse();

    let mut view = ChainView {
        chain: chain.to_string(),
        head: events.last().map(|e| e.id.clone()),
        revision: store.revision(chain).unwrap_or(0),
        events: events.clone(),
        current: BTreeMap::new(),
        raws: Vec::new(),
        norm_of: BTreeMap::new(),
        subtractions: BTreeMap::new(),
        selected_sub: BTreeMap::new(),
        candidates_of: BTreeMap::new(),
    };

    // fold
    for ev in &events {
        match ev.kind {
            EventKind::Import => {
                if let Some(oid) = &ev.object_id {
                    if let Some(Obj::Raw { record, .. }) = store.object(oid)? {
                        view.raws.push(RawSummary {
                            id: record.id.clone(),
                            kind: record.kind,
                            label: record.label.clone(),
                            path: record.path.clone(),
                            convertible: record.exposure.convertible
                                && record.thickness.convertible
                                && record.transmission.convertible,
                            diagnostics: record.diagnostics.clone(),
                        });
                    }
                }
            }
            EventKind::Normalize => {
                if let Some(oid) = &ev.object_id {
                    if let Some(Obj::Norm { source, .. }) = store.object(oid)? {
                        view.norm_of.insert(source, oid.clone());
                    }
                }
            }
            EventKind::Subtract => {
                if let Some(oid) = &ev.object_id {
                    if let Some(Obj::Sub {
                        sample_norm,
                        buffer_norm,
                        ..
                    }) = store.object(oid)?
                    {
                        view.subtractions
                            .entry(sample_norm.clone())
                            .or_default()
                            .push(oid.clone());
                        view.current
                            .insert(format!("buffer_for:{}", sample_norm), buffer_norm);
                    }
                }
            }
            EventKind::FitModels => {
                if let Some(oid) = &ev.object_id {
                    if let Some(Obj::Candidates { source_sub, .. }) = store.object(oid)? {
                        view.candidates_of.insert(source_sub, oid.clone());
                    }
                }
            }
            EventKind::Decision => {
                // decision events may select the current subtraction version
                if let Some(sub_id) = ev.detail.get("select_sub").and_then(|v| v.as_str()) {
                    if let Some(Obj::Sub { sample_norm, .. }) = store.object(sub_id)? {
                        view.selected_sub.insert(sample_norm, sub_id.to_string());
                    }
                }
                if let Some(cid) = ev.detail.get("candidates_id").and_then(|v| v.as_str()) {
                    if let Some(sub) = ev.detail.get("source_sub").and_then(|v| v.as_str()) {
                        view.candidates_of.insert(sub.to_string(), cid.to_string());
                    }
                }
            }
            EventKind::ChainCreate | EventKind::Conflict => {}
        }
    }
    Ok(view)
}

pub fn main_chain(store: &Store) -> Result<ChainView, StoreError> {
    view_at(store, "main", None)
}

pub fn app_view(store: &Store, at_event: Option<&str>) -> Result<AppView, StoreError> {
    let main = view_at(store, "main", at_event)?;
    let mut forks = Vec::new();
    for (chain, _head, fp) in store.chains()? {
        if chain == "main" {
            continue;
        }
        let mut v = view_at(store, &chain, None)?;
        v.head = v.head.or(fp);
        forks.push(v);
    }
    let objs = store.all_objects()?;
    let g = Graph::from_objects(&objs);
    let validity = explain_current_validity(&main, &g);
    let graph_json = json!({
        "nodes": g.nodes.values().map(|n| json!({
            "id": n.id, "kind": n.kind,
            "inputs": n.inputs, "rule_version": n.rule_version,
        })).collect::<Vec<_>>(),
    });
    Ok(AppView {
        chain: main.clone(),
        main,
        forks,
        graph: graph_json,
        validity,
    })
}

/// Why each object visible in the current fold is valid: it exists in the
/// content-addressed store and its inputs are unchanged, so recomputation is
/// unnecessary.
fn explain_current_validity(view: &ChainView, g: &Graph) -> Vec<String> {
    let mut ids = Vec::new();
    ids.extend(view.norm_of.values().cloned());
    for v in view.subtractions.values() {
        ids.extend(v.iter().cloned());
    }
    ids.extend(view.candidates_of.values().cloned());
    let mut out = Vec::new();
    for id in ids {
        if let Some(n) = g.nodes.get(&id) {
            let inputs_ok = n
                .inputs
                .iter()
                .all(|i| g.nodes.contains_key(i));
            out.push(if inputs_ok {
                format!(
                    "STILL_VALID {id} [{}]: rule {} with unchanged input ids {:?}",
                    n.kind, n.rule_version, n.inputs
                )
            } else {
                format!("RECOMPUTE {id}: missing input(s) {:?}", n.inputs)
            });
        }
    }
    out
}

pub fn find_obj<'a>(store: &'a Store, id: &str) -> Option<Obj> {
    store.object(id).ok().flatten()
}

// ---------------------------------------------------------------------------
// Apply: each action derives a new independent version; only a fully built
// object is appended, and the chain head moves in the same transaction.
// ---------------------------------------------------------------------------

fn require_str(p: &Value, key: &str) -> Result<String, String> {
    p.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing '{key}'"))
}

pub fn apply(store: &mut Store, req: &ActionRequest, ts_ms: i64) -> Result<Event, StoreError> {
    let chain = if req.chain.is_empty() {
        "main".to_string()
    } else {
        req.chain.clone()
    };
    let actor = if req.actor.is_empty() {
        "reviewer".to_string()
    } else {
        req.actor.clone()
    };
    match req.action.as_str() {
        "import" => {
            let ir: ImportRequest = serde_json::from_value(req.payload.clone())
                .map_err(|e| bad_request(e.to_string()))?;
            let obj = engine::import_raw(&ir);
            let codes = match &obj {
                Obj::Raw { record, .. } => {
                    let mut c = Vec::new();
                    if !record.diagnostics.is_empty() {
                        c.push("import_diagnostics".to_string());
                    }
                    c
                }
                _ => vec![],
            };
            commit(
                store, &chain, &actor, req.base_revision, req.allow_fork,
                "import",
                json!({"label": ir.label, "path": ir.path, "kind": format!("{:?}", ir.kind)}),
                judgment(JudgmentStatus::Ok, codes, vec![obj.object_id_ref().to_string()],
                    "raw q/I/error, exposure, thickness, transmission, buffer candidates, response and mask retained"),
                Some(obj),
                ts_ms,
            )
        }
        "normalize" => {
            let raw_id = pstr(&req.payload, "raw_id")?;
            let raw = find_obj(store, &raw_id).ok_or_else(|| bad_request("unknown raw_id"))?;
            match engine::normalize(&raw) {
                Ok(obj) => {
                    let id = obj.object_id_ref().to_string();
                    commit(store, &chain, &actor, req.base_revision, req.allow_fork,
                        "normalize",
                        json!({"raw_id": raw_id, "norm_id": id}),
                        judgment(JudgmentStatus::Ok, vec![], vec![id.clone()],
                            "normalization with propagated transmission uncertainty"),
                        Some(obj), ts_ms)
                }
                Err(e) => commit(store, &chain, &actor, req.base_revision, req.allow_fork,
                    "normalize",
                    json!({"raw_id": raw_id, "error": e.why}),
                    judgment(JudgmentStatus::Error,
                        e.diagnostics.clone(),
                        vec![], &e.why),
                    None, ts_ms),
            }
        }
        "subtract" => {
            let sample_norm_id = pstr(&req.payload, "sample_norm_id")?;
            let buffer_norm_id = pstr(&req.payload, "buffer_norm_id")?;
            let (sn, bn) = (find_obj(store, &sample_norm_id), find_obj(store, &buffer_norm_id));
            let (sc, bsrc, bc) = match (&sn, &bn) {
                (Some(Obj::Norm { curve: sc, .. }),
                 Some(Obj::Norm { curve: bc, source: bsrc, .. })) => (sc.clone(), bsrc.clone(), bc.clone()),
                _ => return Err(bad_request("subtract needs two normalized curves")),
            };
            let ranges = match find_obj(store, &bsrc) {
                Some(Obj::Raw { record, .. }) => record
                    .mask
                    .as_ref()
                    .map(|m| m.excluded_ranges.clone())
                    .unwrap_or_default(),
                _ => vec![],
            };
            let obj = engine::subtract(engine::SubInputs {
                sample_id: &sample_norm_id,
                buffer_id: &buffer_norm_id,
                sample: &sc,
                buffer: &bc,
                buffer_mask_ranges: ranges,
            });
            let (status, codes, multi_like) = match &obj {
                Obj::Sub { curve, .. } => {
                    let mut c = Vec::new();
                    if !curve.grids_identical {
                        c.push("grid_mismatch".to_string());
                    }
                    if curve.status == SubStatus::NoOverlap {
                        c.push("no_overlap".to_string());
                    }
                    for d in &curve.diagnostics {
                        if d.contains("SCALE_UNCERTAIN") {
                            c.push("background_scale_uncertainty".to_string());
                        }
                        if d.contains("NEGATIVE_POINTS") {
                            c.push("negative_intensity".to_string());
                        }
                    }
                    let st = match curve.status {
                        SubStatus::NoOverlap => JudgmentStatus::Error,
                        _ if c.iter().any(|x| x == "background_scale_uncertainty") =>
                            JudgmentStatus::Guarded,
                        _ => JudgmentStatus::Ok,
                    };
                    (st, c, curve.status)
                }
                _ => unreachable!(),
            };
            let id = obj.object_id_ref().to_string();
            commit(store, &chain, &actor, req.base_revision, req.allow_fork,
                "subtract",
                json!({"sample_norm_id": sample_norm_id, "buffer_norm_id": buffer_norm_id,
                       "sub_id": id, "sub_status": format!("{multi_like:?}")}),
                judgment(status, codes, vec![id.clone()],
                    "q grids differing are never directly subtracted; interpolation is valid inside overlap only; errors include normalization and scale uncertainty"),
                Some(obj), ts_ms)
        }
        "fit_models" => {
            let sub_id = pstr(&req.payload, "sub_id")?;
            let sub = find_obj(store, &sub_id).ok_or_else(|| bad_request("unknown sub_id"))?;
            let Obj::Sub { curve, .. } = &sub else {
                return Err(bad_request("fit_models needs a subtraction"));
            };
            let obj = engine::fit_models(&sub_id, curve);
            let n = match &obj {
                Obj::Candidates { candidates, .. } => candidates.len(),
                _ => 0,
            };
            let overlap = matches!(&obj, Obj::Candidates { candidates, .. }
                if candidates.iter().any(|c| c.interval_overlap));
            let codes = [
                if n > 1 { Some("multi_candidate_residual") } else { None },
                if overlap { Some("interval_overlap") } else { None },
            ].into_iter().flatten().map(|s| s.to_string()).collect::<Vec<_>>();
            let status = if n > 1 { JudgmentStatus::Multi } else { JudgmentStatus::Ok };
            let id = obj.object_id_ref().to_string();
            commit(store, &chain, &actor, req.base_revision, req.allow_fork,
                "fit_models",
                json!({"sub_id": sub_id, "candidates_id": id, "count": n}),
                judgment(status, codes, vec![id.clone()],
                    "Guinier and Porod intervals remain separate candidates; all legal windows retained"),
                Some(obj), ts_ms)
        }
        "decision" => {
            // Human adjudication: adopt/reject candidates, select a subtraction
            // version to continue, or parameter adjustments that rebuild.
            let mut decisions: Vec<(String, CandidateDecision)> = Vec::new();
            if let Some(map) = req.payload.get("decisions").and_then(|v| v.as_object()) {
                for (cid, d) in map {
                    let dec = match d.as_str() {
                        Some("adopted") => CandidateDecision::Adopted,
                        Some("rejected") => CandidateDecision::Rejected,
                        _ => CandidateDecision::Proposed,
                    };
                    decisions.push((cid.clone(), dec));
                }
            }
            let cands_id = req.payload.get("candidates_id").and_then(|v| v.as_str());
            let new_obj = if let Some(cid) = cands_id {
                let base_obj = find_obj(store, cid).ok_or_else(|| bad_request("unknown candidates_id"))?;
                let next = engine::with_decisions(cid, &base_obj, &decisions)
                    .ok_or_else(|| bad_request("decision target is not a candidates object"))?;
                Some(next)
            } else {
                None
            };
            let select_sub = req.payload.get("select_sub").and_then(|v| v.as_str()).map(String::from);
            let mut affected = Vec::new();
            if let Some(o) = &new_obj {
                affected.push(o.object_id_ref().to_string());
            }
            if let Some(s) = &select_sub {
                affected.push(s.clone());
            }
            let detail = json!({
                "candidates_id": new_obj.as_ref().map(|o| o.object_id_ref().to_string()),
                "source_sub": req.payload.get("source_sub").and_then(|v| v.as_str()),
                "select_sub": select_sub,
                "rejected_kept": true,
            });
            commit(store, &chain, &actor, req.base_revision, req.allow_fork,
                "decision", detail,
                judgment(JudgmentStatus::Ok, vec!["candidates_retained".to_string()], affected,
                    "unselected candidates and rejected branches are retained; no irreversible overwrite"),
                new_obj, ts_ms)
        }
        "revise_raw" => {
            // Re-import a corrected/expanded raw record; the old raw object
            // stays in store, normalization/subtraction downstream are new
            // versions reachable from the new raw id (dependency propagation).
            let mut ir: ImportRequest = serde_json::from_value(req.payload.clone())
                .map_err(|e| bad_request(e.to_string()))?;
            if let Some(note) = req.payload.get("revision_note").and_then(|v| v.as_str()) {
                ir.format_tag = format!("{} | revise:{}", ir.format_tag, note);
            }
            let obj = engine::import_raw(&ir);
            let id = obj.object_id_ref().to_string();
            commit(store, &chain, &actor, req.base_revision, req.allow_fork,
                "import",
                json!({"label": ir.label, "path": ir.path, "revision": true}),
                judgment(JudgmentStatus::Ok, vec!["source_revision".to_string()],
                    vec![id], "revised source imported; prior version retained"),
                Some(obj), ts_ms)
        }
        _ => Err(bad_request(format!("unknown action {}", req.action))),
    }
}

fn pstr(p: &Value, k: &str) -> Result<String, StoreError> {
    require_str(p, k).map_err(bad_request)
}

fn bad_request(msg: impl Into<String>) -> StoreError {
    StoreError::Sql(rusqlite::Error::ToSqlConversionFailure(Box::new(
        std::io::Error::new(std::io::ErrorKind::InvalidInput, msg.into()),
    )))
}

#[allow(clippy::too_many_arguments)]
fn commit(
    store: &mut Store,
    chain: &str,
    actor: &str,
    base: Option<i64>,
    allow_fork: bool,
    kind: &str,
    detail: Value,
    judgment_value: Judgment,
    object: Option<Obj>,
    ts_ms: i64,
) -> Result<Event, StoreError> {
    let parent = store.chain_head(chain)?;
    let seq = store.next_seq(chain)?;
    let id_body = hash_canonical(&json!({
        "chain": chain, "parent": parent, "seq": seq,
        "kind": kind, "detail": detail,
        "judgment": &judgment_value,
        "object": object.as_ref().map(|o| o.object_id_ref()),
    }));
    let id = format!("evt:{}", &id_body[..16]);
    store.append(
        crate::store::Append {
            chain,
            kind,
            detail,
            judgment: serde_json::to_value(&judgment_value).unwrap(),
            actor,
            object,
            base_revision: base,
            allow_fork,
            chain_label: None,
        },
        &id,
        ts_ms,
    )
}

fn judgment(status: JudgmentStatus, codes: Vec<String>, affected: Vec<String>, reason: &str) -> Judgment {
    Judgment {
        status,
        codes,
        affected,
        reason: reason.to_string(),
    }
}
