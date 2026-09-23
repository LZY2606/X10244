//! Pipeline: builds versioned nodes from imports and propagates changes
//! through the dependency graph, recomputing only affected nodes and
//! explaining why each untouched node is still valid.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::*;
use crate::store::{payload_hash, Dep, Node, Store};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecomputeReport {
    pub node: String,
    pub action: String, // recomputed | unchanged | failed
    pub reason: String,
}

fn curve_json(c: &Curve) -> serde_json::Value {
    json!({"q": c.q, "i": c.i, "sigma": c.sigma, "q_unit": c.q_unit.label(), "i_unit": c.i_unit.label()})
}

pub fn curve_from_payload(p: &serde_json::Value, key: &str) -> Option<Curve> {
    let c = p.get(key)?;
    Some(Curve {
        q: serde_json::from_value(c["q"].clone()).ok()?,
        i: serde_json::from_value(c["i"].clone()).ok()?,
        sigma: serde_json::from_value(c["sigma"].clone()).ok()?,
        q_unit: Unit::parse(c["q_unit"].as_str().unwrap_or("")),
        i_unit: Unit::parse(c["i_unit"].as_str().unwrap_or("")),
    })
}

fn mk_node(store: &Store, id: &str, kind: &str, payload: serde_json::Value, deps: Vec<Dep>, event: &str) -> Result<Node> {
    let version = store.next_version(id)?;
    let input_hash = payload_hash(&payload, &deps);
    Ok(Node {
        id: id.into(),
        version,
        kind: kind.into(),
        status: "ok".into(),
        payload,
        deps,
        input_hash,
        created_event: event.into(),
    })
}

/// Import: raw representation is preserved verbatim; the derived
/// `canonical` layer is the only place where unit normalization happens.
pub fn import_record(store: &Store, id: &str, raw: serde_json::Value, actor: &str) -> Result<Node> {
    let curve = Curve {
        q: serde_json::from_value(raw["q"].clone()).unwrap_or_default(),
        i: serde_json::from_value(raw["i"].clone()).unwrap_or_default(),
        sigma: serde_json::from_value(raw["sigma"].clone()).unwrap_or_default(),
        q_unit: Unit::parse(raw["q_unit"].as_str().unwrap_or("")),
        i_unit: Unit::parse(raw["i_unit"].as_str().unwrap_or("")),
    };
    let (canonical, diags) = to_canonical(&curve);
    let payload = json!({
        "raw": raw,
        "canonical": canonical.as_ref().map(curve_json),
        "diagnostics": diags,
    });
    let node = mk_node(store, id, "import", payload, vec![], "pending")?;
    let ev = store.commit(
        std::slice::from_ref(&node),
        &[(id.into(), node.version)],
        "import",
        actor,
        json!({"node": id, "version": node.version, "diagnostics": diags.len(),
               "rule": "raw_preserved_derived_normalized"}),
    )?;
    let mut node = node;
    node.created_event = ev.id.clone();
    store.conn.execute(
        "UPDATE nodes SET created_event=?1 WHERE id=?2 AND version=?3",
        rusqlite::params![ev.id, node.id, node.version],
    )?;
    Ok(node)
}

pub fn run_normalization(store: &Store, out_id: &str, import_id: &str, norm: &NormFactors, actor: &str) -> Result<Node> {
    let imp = store.current_node(import_id)?;
    let curve = curve_from_payload(&imp.payload, "canonical")
        .ok_or_else(|| anyhow::anyhow!("import {import_id} has no canonical curve"))?;
    let (nc, diags) = normalize(&curve, norm);
    let deps = vec![Dep { id: import_id.into(), version: imp.version }];
    let payload = json!({"curve": curve_json(&nc), "norm": norm, "diagnostics": diags});
    let node = mk_node(store, out_id, "normalized", payload, deps, "pending")?;
    let ev = store.commit(
        std::slice::from_ref(&node),
        &[(out_id.into(), node.version)],
        "normalize",
        actor,
        json!({"node": out_id, "version": node.version, "source": format!("{}@{}", import_id, imp.version),
               "rule": "sigma_includes_normalization_uncertainty"}),
    )?;
    store.conn.execute("UPDATE nodes SET created_event=?1 WHERE id=?2 AND version=?3",
        rusqlite::params![ev.id, node.id, node.version])?;
    Ok(node)
}

/// Background subtraction. A no-overlap attempt records a machine-readable
/// rejection event instead of producing a node.
pub fn run_subtraction(
    store: &Store,
    out_id: &str,
    sample_id: &str,
    buffer_id: &str,
    scale: f64,
    scale_sigma: f64,
    actor: &str,
) -> Result<Option<Node>> {
    let s = store.current_node(sample_id)?;
    let b = store.current_node(buffer_id)?;
    let sc = curve_from_payload(&s.payload, "curve")
        .or_else(|| curve_from_payload(&s.payload, "canonical"))
        .ok_or_else(|| anyhow::anyhow!("sample {sample_id} has no curve"))?;
    let bc = curve_from_payload(&b.payload, "curve")
        .or_else(|| curve_from_payload(&b.payload, "canonical"))
        .ok_or_else(|| anyhow::anyhow!("buffer {buffer_id} has no curve"))?;
    match subtract(&sc, &bc, scale, scale_sigma) {
        Ok(res) => {
            let deps = vec![
                Dep { id: sample_id.into(), version: s.version },
                Dep { id: buffer_id.into(), version: b.version },
            ];
            let payload = json!({
                "result": res,
                "sources": {"sample": format!("{}@{}", sample_id, s.version),
                            "buffer": format!("{}@{}", buffer_id, b.version)},
            });
            let node = mk_node(store, out_id, "subtracted", payload, deps, "pending")?;
            let ev = store.commit(
                std::slice::from_ref(&node),
                &[(out_id.into(), node.version)],
                "subtract",
                actor,
                json!({
                    "node": out_id, "version": node.version,
                    "sample": format!("{}@{}", sample_id, s.version),
                    "buffer": format!("{}@{}", buffer_id, b.version),
                    "overlap": res.overlap,
                    "interpolated": res.interpolated,
                    "rule": if res.interpolated {
                        "grid_mismatch_no_direct_subtract_interpolate_within_overlap"
                    } else {
                        "grids_equal_direct_subtract"
                    },
                    "scale": scale, "scale_sigma": scale_sigma,
                    "diagnostics": res.diagnostics,
                }),
            )?;
            store.conn.execute("UPDATE nodes SET created_event=?1 WHERE id=?2 AND version=?3",
                rusqlite::params![ev.id, node.id, node.version])?;
            Ok(Some(node))
        }
        Err(SubError::NoOverlap { sample_range, buffer_range }) => {
            store.commit(
                &[], &[],
                "subtract_rejected",
                actor,
                json!({
                    "node": out_id,
                    "sample": format!("{}@{}", sample_id, s.version),
                    "buffer": format!("{}@{}", buffer_id, b.version),
                    "rule": "no_subtraction_without_overlap",
                    "sample_range": sample_range,
                    "buffer_range": buffer_range,
                    "result": "rejected_no_node_created",
                }),
            )?;
            Ok(None)
        }
    }
}

/// Evaluate Guinier/Porod interval candidates on a subtracted curve.
/// All legal candidates are kept; overlapping intervals are flagged, never
/// merged into a single model.
pub fn run_fits(store: &Store, out_id: &str, sub_id: &str, candidates: &[(String, [f64; 2])], actor: &str) -> Result<Node> {
    let sub = store.current_node(sub_id)?;
    let res: SubResult = serde_json::from_value(sub.payload["result"].clone())?;
    let q: Vec<f64> = res.points.iter().map(|p| p.q).collect();
    let i: Vec<f64> = res.points.iter().map(|p| p.i).collect();
    let (fits, diags) = fit_candidates(&q, &i, candidates);
    let deps = vec![Dep { id: sub_id.into(), version: sub.version }];
    let payload = json!({
        "fits": fits.iter().map(|(n, f)| json!({"name": n, "fit": f})).collect::<Vec<_>>(),
        "diagnostics": diags,
        "rule": "guinier_porod_intervals_not_auto_merged",
    });
    let node = mk_node(store, out_id, "fit_candidates", payload, deps, "pending")?;
    let ev = store.commit(
        std::slice::from_ref(&node),
        &[(out_id.into(), node.version)],
        "fit_intervals",
        actor,
        json!({
            "node": out_id, "version": node.version,
            "source": format!("{}@{}", sub_id, sub.version),
            "candidates": fits.iter().map(|(n, f)| json!({
                "name": n, "interval": f.interval, "rms": f.rms, "n_points": f.n_points
            })).collect::<Vec<_>>(),
            "rule": "multi_candidate_residuals_kept_no_auto_merge",
            "diagnostics": diags,
        }),
    )?;
    store.conn.execute("UPDATE nodes SET created_event=?1 WHERE id=?2 AND version=?3",
        rusqlite::params![ev.id, node.id, node.version])?;
    Ok(node)
}

/// Dependency-graph propagation: recompute only nodes downstream of
/// `changed_id`. Nodes whose recomputed input hash matches the stored hash
/// are reported as still valid (no new version created).
pub fn recompute_affected(store: &Store, changed_id: &str, actor: &str) -> Result<Vec<RecomputeReport>> {
    // build downstream adjacency from current nodes
    let current = store.current_map()?;
    let mut downstream: Vec<String> = Vec::new();
    let mut frontier = vec![changed_id.to_string()];
    while let Some(x) = frontier.pop() {
        for (id, _) in &current {
            if id == &x || downstream.contains(id) {
                continue;
            }
            let n = store.current_node(id)?;
            if n.deps.iter().any(|d| d.id == x) {
                downstream.push(id.clone());
                frontier.push(id.clone());
            }
        }
    }
    // topo order: parents before children (deps first)
    let mut order: Vec<String> = Vec::new();
    let mut remaining = downstream.clone();
    while !remaining.is_empty() {
        let mut progressed = false;
        let mut next_remaining = Vec::new();
        for id in remaining {
            let n = store.current_node(&id)?;
            let deps_done = n.deps.iter().all(|d| !downstream.contains(&d.id) || order.contains(&d.id));
            if deps_done {
                order.push(id);
                progressed = true;
            } else {
                next_remaining.push(id);
            }
        }
        if !progressed {
            order.extend(next_remaining);
            break;
        }
        remaining = next_remaining;
    }
    let mut reports = Vec::new();
    for id in order {
        let old = store.current_node(&id)?;
        let report = match old.kind.as_str() {
            "normalized" => {
                let norm: NormFactors = serde_json::from_value(old.payload["norm"].clone())?;
                let dep_id = old.deps[0].id.clone();
                let imp = store.current_node(&dep_id)?;
                let curve = curve_from_payload(&imp.payload, "canonical").unwrap();
                let (nc, diags) = normalize(&curve, &norm);
                let deps = vec![Dep { id: dep_id.clone(), version: imp.version }];
                let payload = json!({"curve": curve_json(&nc), "norm": norm, "diagnostics": diags});
                finish_recompute(store, &old, payload, deps, actor)?
            }
            "subtracted" => {
                let res: SubResult = serde_json::from_value(old.payload["result"].clone())?;
                let (sid, bid) = (old.deps[0].id.clone(), old.deps[1].id.clone());
                let s = store.current_node(&sid)?;
                let b = store.current_node(&bid)?;
                let sc = curve_from_payload(&s.payload, "curve").or_else(|| curve_from_payload(&s.payload, "canonical")).unwrap();
                let bc = curve_from_payload(&b.payload, "curve").or_else(|| curve_from_payload(&b.payload, "canonical")).unwrap();
                match subtract(&sc, &bc, res.scale, res.scale_sigma) {
                    Ok(r) => {
                        let deps = vec![
                            Dep { id: sid.clone(), version: s.version },
                            Dep { id: bid.clone(), version: b.version },
                        ];
                        let payload = json!({
                            "result": r,
                            "sources": {"sample": format!("{}@{}", sid, s.version),
                                        "buffer": format!("{}@{}", bid, b.version)},
                        });
                        finish_recompute(store, &old, payload, deps, actor)?
                    }
                    Err(SubError::NoOverlap { .. }) => RecomputeReport {
                        node: id.clone(),
                        action: "failed".into(),
                        reason: "recompute: overlap disappeared, subtraction no longer defined; old version retained".into(),
                    },
                }
            }
            "fit_candidates" => {
                let candidates: Vec<(String, [f64; 2])> =
                    serde_json::from_value(old.payload["fits"].clone())
                        .map(|v: Vec<serde_json::Value>| {
                            v.iter().map(|f| (
                                f["name"].as_str().unwrap_or("").to_string(),
                                serde_json::from_value::<[f64; 2]>(f["fit"]["interval"].clone()).unwrap_or([0.0, 0.0]),
                            )).collect()
                        })
                        .unwrap_or_default();
                let dep_id = old.deps[0].id.clone();
                let sub = store.current_node(&dep_id)?;
                let res: SubResult = serde_json::from_value(sub.payload["result"].clone())?;
                let q: Vec<f64> = res.points.iter().map(|p| p.q).collect();
                let i: Vec<f64> = res.points.iter().map(|p| p.i).collect();
                let (fits, diags) = fit_candidates(&q, &i, &candidates);
                let deps = vec![Dep { id: dep_id.clone(), version: sub.version }];
                let payload = json!({
                    "fits": fits.iter().map(|(n, f)| json!({"name": n, "fit": f})).collect::<Vec<_>>(),
                    "diagnostics": diags,
                    "rule": "guinier_porod_intervals_not_auto_merged",
                });
                finish_recompute(store, &old, payload, deps, actor)?
            }
            other => RecomputeReport {
                node: id.clone(),
                action: "unchanged".into(),
                reason: format!("kind '{other}' has no recompute rule; not downstream-affected content"),
            },
        };
        reports.push(report);
    }
    if reports.is_empty() {
        reports.push(RecomputeReport {
            node: changed_id.into(),
            action: "unchanged".into(),
            reason: "no downstream nodes depend on this change".into(),
        });
    }
    Ok(reports)
}

fn finish_recompute(
    store: &Store,
    old: &Node,
    payload: serde_json::Value,
    deps: Vec<Dep>,
    actor: &str,
) -> Result<RecomputeReport> {
    let new_hash = payload_hash(&payload, &deps);
    if new_hash == old.input_hash {
        return Ok(RecomputeReport {
            node: old.id.clone(),
            action: "unchanged".into(),
            reason: format!("recomputed inputs identical to {}@{} (hash {}); existing version still valid",
                old.id, old.version, &new_hash[..8]),
        });
    }
    let version = store.next_version(&old.id)?;
    let node = Node {
        id: old.id.clone(),
        version,
        kind: old.kind.clone(),
        status: "ok".into(),
        payload,
        deps,
        input_hash: new_hash.clone(),
        created_event: "pending".into(),
    };
    let ev = store.commit(
        std::slice::from_ref(&node),
        &[(old.id.clone(), version)],
        "recompute",
        actor,
        json!({"node": old.id, "from_version": old.version, "to_version": version,
               "rule": "dependency_graph_propagation_partial_update"}),
    )?;
    store.conn.execute("UPDATE nodes SET created_event=?1 WHERE id=?2 AND version=?3",
        rusqlite::params![ev.id, node.id, node.version])?;
    Ok(RecomputeReport {
        node: old.id.clone(),
        action: "recomputed".into(),
        reason: format!("inputs changed; {}@{} -> {}@{}", old.id, old.version, old.id, version),
    })
}
