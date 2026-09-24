use crate::graph::Workspace;
use crate::model::*;
use crate::store::{SaveOutcome, Store};
use serde_json::json;
use std::path::PathBuf;

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/casepack")
}

/// Load every source fixture, preserving the raw representation and attaching
/// diagnostics for anything the pipeline cannot normalize (e.g. unknown units).
pub fn load_sources() -> Vec<SourceRecord> {
    let dir = fixtures_dir().join("sources");
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let text = std::fs::read_to_string(&path).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        let kind = match doc["kind"].as_str().unwrap() {
            "sample" => SourceKind::Sample,
            "buffer" => SourceKind::Buffer,
            "instrument_response" => SourceKind::InstrumentResponse,
            "mask" => SourceKind::Mask,
            other => panic!("unknown kind {}", other),
        };
        let meta = doc["meta"].as_object().map(|m| Meta {
            exposure_s: m["exposure_s"].as_f64().unwrap(),
            thickness_mm: m["thickness_mm"].as_f64().unwrap(),
            transmission: m["transmission"].as_f64().unwrap(),
            transmission_sigma: m["transmission_sigma"].as_f64().unwrap(),
        });
        let q_unit = doc["q_unit"].as_str().unwrap().to_string();
        let i_unit = doc["i_unit"].as_str().unwrap().to_string();
        let points: Vec<Point> = doc["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| Point {
                q: p[0].as_f64().unwrap(),
                i: p[1].as_f64().unwrap(),
                sigma: p[2].as_f64().unwrap(),
            })
            .collect();
        let mut diagnostics = Vec::new();
        // unit gate: values we cannot convert stay in place with a diagnostic
        if !matches!(q_unit.as_str(), "1/angstrom" | "1/nm") {
            diagnostics.push(Diagnostic {
                code: "unit-unconvertible".into(),
                message: format!("q unit '{}' not convertible; raw values kept in place", q_unit),
                subject: Some(doc["id"].as_str().unwrap().into()),
            });
        }
        if !matches!(i_unit.as_str(), "arbitrary" | "1/cm" | "flag")
            && kind != SourceKind::InstrumentResponse
        {
            diagnostics.push(Diagnostic {
                code: "unit-unconvertible".into(),
                message: format!("I unit '{}' not convertible; raw values kept in place", i_unit),
                subject: Some(doc["id"].as_str().unwrap().into()),
            });
        }
        if kind == SourceKind::InstrumentResponse {
            diagnostics.push(Diagnostic {
                code: "unit-unconvertible".into(),
                message: format!(
                    "instrument response unit '{}' kept as raw reference, not folded into normalization",
                    i_unit
                ),
                subject: Some(doc["id"].as_str().unwrap().into()),
            });
        }
        out.push(SourceRecord {
            id: doc["id"].as_str().unwrap().into(),
            name: doc["name"].as_str().unwrap().into(),
            kind,
            version: 1,
            meta,
            curve: Some(Curve { q_unit, i_unit, points }),
            raw_repr: doc["raw_repr"].as_str().unwrap().into(),
            diagnostics,
        });
    }
    out
}

pub fn replay_script() -> serde_json::Value {
    let text = std::fs::read_to_string(fixtures_dir().join("replay.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Execute the scripted replay against a store. Every step appends a
/// machine-readable event; every state transition commits a full version.
pub fn run_replay(store: &Store) -> serde_json::Value {
    let script = replay_script();
    let sample = script["sample"].as_str().unwrap().to_string();
    let mut ws = Workspace::default();
    let mut log = Vec::new();

    for step in script["steps"].as_array().unwrap() {
        let op = step["op"].as_str().unwrap();
        let note = step["note"].as_str().unwrap_or("");
        let base = store.current_version();
        match op {
            "import_all" => {
                for src in load_sources() {
                    ws.add_source(src);
                }
                let v = store.commit_version(&ws, base, note).unwrap();
                store.append_event("system", "import", base, v, json!({
                    "note": note,
                    "sources": ws.sources.keys().collect::<Vec<_>>(),
                    "preserved": ["raw q/I/sigma", "exposure", "thickness", "transmission",
                                   "buffer candidates", "instrument response", "mask provenance"],
                }));
                log.push(json!({"op": op, "version": v}));
            }
            "build" | "revise_params" | "source_change" | "attempt_buffer" | "multi_candidate" => {
                let buffers: Vec<String> = step["buffers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| b.as_str().unwrap().to_string())
                    .collect();
                let scale = step["scale"].as_f64().unwrap();
                let scale_sigma = step["scale_sigma"].as_f64().unwrap();
                if op == "source_change" {
                    // a new buffer version arrives: only its dependents recompute
                    for b in &buffers {
                        ws.bump_source(b);
                    }
                }
                let report = ws.recompute(&sample, &buffers, scale, scale_sigma);
                let v = store.commit_version(&ws, base, note).unwrap();
                let kind = match op {
                    "build" => "build",
                    "revise_params" => "revise_params",
                    "source_change" => "source_change",
                    "attempt_buffer" => "attempt_buffer",
                    _ => "multi_candidate_residual",
                };
                store.append_event("system", kind, base, v, json!({
                    "note": note,
                    "buffers": buffers,
                    "scale": scale,
                    "scale_sigma": scale_sigma,
                    "recomputed": report.recomputed,
                    "still_valid": report.still_valid,
                    "failed": report.failed,
                }));
                log.push(json!({"op": op, "version": v, "recomputed": report.recomputed.len(),
                                "still_valid": report.still_valid.len(), "failed": report.failed}));
            }
            "decision" => {
                let actor = step["actor"].as_str().unwrap();
                let idx = step["candidate_index"].as_u64().unwrap() as usize;
                let accept = step["accept"].as_bool().unwrap();
                // decide on the guinier node of the first subtraction branch
                let gid = ws
                    .nodes
                    .keys()
                    .find(|k| k.ends_with("#guinier") && k.contains("buffer-v1"))
                    .cloned()
                    .expect("guinier node");
                {
                    let node = ws.nodes.get_mut(&gid).unwrap();
                    if let Some(c) = node.candidates.get_mut(idx) {
                        c.accepted = Some(accept);
                    }
                }
                let cand_id = ws.nodes[&gid].candidates[idx].id.clone();
                match store.save_branch(&ws, base, note).unwrap() {
                    SaveOutcome::Committed(v) => {
                        store.append_event(actor, "decision", base, v, json!({
                            "note": note, "node": gid, "candidate": cand_id, "accept": accept,
                        }));
                        log.push(json!({"op": op, "actor": actor, "version": v}));
                    }
                    SaveOutcome::Conflict(c) => {
                        store.append_event(actor, "conflict", base, c.branch_b, json!({
                            "note": note, "node": gid, "candidate": cand_id, "accept": accept,
                            "conflict_id": c.id, "branch_a": c.branch_a, "branch_b": c.branch_b,
                        }));
                        // resolve per script: named reviewer's branch wins, loser retained
                        let winner = if step["resolve"].as_str() == Some("reviewer-a") {
                            c.branch_a
                        } else {
                            c.branch_b
                        };
                        store.resolve_conflict(&c.id, winner).unwrap();
                        store.append_event("system", "resolve", base, winner, json!({
                            "conflict_id": c.id, "winner": winner,
                            "loser_retained": if winner == c.branch_a { c.branch_b } else { c.branch_a },
                        }));
                        log.push(json!({"op": op, "actor": actor, "conflict": c.id, "winner": winner}));
                    }
                }
            }
            other => panic!("unknown replay op {}", other),
        }
    }
    json!({"steps": log, "final_version": store.current_version()})
}
