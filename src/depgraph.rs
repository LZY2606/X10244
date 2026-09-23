//! Dependency-graph propagation: when a source changes, only affected
//! derivations are recomputed; everything else is reported still-valid with
//! a human-readable reason. Recompute runs as a single atomic event.

use crate::model::{Curve, Diagnostic, IntervalCandidate, Node, NodeId, NodeState};
use crate::rules;
use crate::store::Store;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub struct RecomputeReport {
    pub event_seq: i64,
    pub recomputed: Vec<NodeId>,
    pub still_valid: Vec<(NodeId, String)>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Transitive dependents of `changed` among `nodes`.
pub fn affected(nodes: &[Node], changed: &[NodeId]) -> BTreeSet<NodeId> {
    let mut children: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for n in nodes {
        for d in &n.deps {
            children.entry(d.clone()).or_default().push(n.id.clone());
        }
    }
    let mut out = BTreeSet::new();
    let mut queue: VecDeque<NodeId> = changed.iter().cloned().collect();
    let mut seen: BTreeSet<NodeId> = changed.iter().cloned().collect();
    while let Some(id) = queue.pop_front() {
        if let Some(kids) = children.get(&id) {
            for k in kids {
                if seen.insert(k.clone()) {
                    out.insert(k.clone());
                    queue.push_back(k.clone());
                }
            }
        }
    }
    out
}

fn curve_of(node: &Node) -> Option<Curve> {
    serde_json::from_value(node.payload.get("curve")?.clone()).ok()
}

/// Recompute every node that depends on `changed`. Returns a report
/// explaining, for every current node, why it was recomputed or why it is
/// still valid.
pub fn recompute(store: &Store, changed: &[NodeId], reason: &str) -> Result<RecomputeReport, String> {
    let current = store.nodes_at(None);
    let dirty = affected(&current, changed);
    let mut by_id: BTreeMap<NodeId, Node> = current.iter().map(|n| (n.id.clone(), n.clone())).collect();
    let mut new_versions: Vec<Node> = vec![];
    let mut diagnostics: Vec<Diagnostic> = vec![];
    let mut recomputed: Vec<NodeId> = vec![];

    // deterministic order: by event_seq then id
    let mut order: Vec<Node> = current
        .iter()
        .filter(|n| dirty.contains(&n.id))
        .cloned()
        .collect();
    order.sort_by_key(|n| (n.event_seq, n.id.clone()));

    for node in order {
        let latest = |id: &str| -> Option<Node> {
            new_versions
                .iter()
                .rev()
                .find(|n| &n.id == id)
                .cloned()
                .or_else(|| by_id.get(id).cloned())
        };
        let rebuilt: Option<(serde_json::Value, Vec<NodeId>, Vec<Diagnostic>)> = match node.kind.as_str() {
            "derived:normalized" => {
                let src_id = node.payload.get("source")?.as_str()?.to_string();
                let src = latest(&src_id)?;
                let curve = curve_of(&src)?;
                let m = &node.payload["meta"];
                let exposure = crate::model::Quantity { value: m["exposure_s"].as_f64()?, unit: "s".into(), sigma: None, raw: None };
                let thickness = crate::model::Quantity { value: m["thickness_mm"].as_f64()?, unit: "mm".into(), sigma: None, raw: None };
                let transmission = crate::model::Quantity {
                    value: m["transmission"].as_f64()?,
                    unit: "dimensionless".into(),
                    sigma: m["transmission_sigma"].as_f64(),
                    raw: None,
                };
                let (c, d) = rules::normalize(&curve, &exposure, &thickness, &transmission);
                Some((serde_json::json!({ "curve": c, "source": src_id, "meta": m }), vec![src.id], d))
            }
            "derived:subtracted" => {
                let m = &node.payload["meta"];
                let sample_id = m["sample"].as_str()?.to_string();
                let buffer_id = m["buffer"].as_str()?.to_string();
                let sample = curve_of(&latest(&sample_id)?)?;
                let buffer = curve_of(&latest(&buffer_id)?)?;
                let scale = m["scale"].as_f64()?;
                let scale_sigma = m["scale_sigma"].as_f64()?;
                let sub = rules::subtract(&sample, &buffer, scale, scale_sigma);
                let mut diags = sub.diagnostics.clone();
                let payload = match sub.curve {
                    Some(c) => serde_json::json!({
                        "curve": c, "meta": m,
                        "grid_judgment": sub.judgment,
                    }),
                    None => {
                        diags.push(Diagnostic {
                            code: "recompute_no_result".into(),
                            message: "subtraction still impossible after recompute".into(),
                            affected: vec![node.id.clone()],
                        });
                        serde_json::json!({ "meta": m, "grid_judgment": sub.judgment })
                    }
                };
                Some((payload, vec![sample_id, buffer_id], diags))
            }
            "derived:fit" => {
                let src_id = node.payload.get("source")?.as_str()?.to_string();
                let src = latest(&src_id)?;
                let curve = curve_of(&src)?;
                let model = node.payload["model"].as_str()?.to_string();
                let q_min = node.payload["q_min"].as_f64()?;
                let q_max = node.payload["q_max"].as_f64()?;
                let (cand, d) = if model == "guinier" {
                    rules::guinier(&curve, q_min, q_max)
                } else {
                    rules::porod(&curve, q_min, q_max)
                };
                Some((
                    serde_json::json!({
                        "candidate": cand, "source": src_id,
                        "model": model, "q_min": q_min, "q_max": q_max,
                        "interval_judgment": rules::interval_judgment(std::slice::from_ref(&cand)),
                    }),
                    vec![src.id],
                    d,
                ))
            }
            _ => None,
        };
        if let Some((payload, deps, diags)) = rebuilt {
            diagnostics.extend(diags);
            let version = store.next_version(&node.id);
            new_versions.push(Node {
                id: node.id.clone(),
                version,
                kind: node.kind.clone(),
                event_seq: 0,
                state: NodeState::Valid,
                deps,
                rule_version: rules::RULE_VERSION,
                payload,
            });
            recomputed.push(node.id.clone());
        }
    }

    let event_seq = store
        .commit(
            "recompute",
            serde_json::json!({
                "reason": reason,
                "changed_inputs": changed,
                "recomputed": recomputed,
            }),
            &new_versions,
            None,
            false,
        )
        .map_err(|e| format!("{:?}", e))?;

    let still_valid = current
        .iter()
        .filter(|n| !dirty.contains(&n.id) && !changed.contains(&n.id))
        .map(|n| {
            (
                n.id.clone(),
                format!("no dependency on changed inputs; version {} from event {} unchanged", n.version, n.event_seq),
            )
        })
        .collect();

    Ok(RecomputeReport { event_seq, recomputed, still_valid, diagnostics })
}
