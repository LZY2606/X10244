//! Dependency-graph propagation: given a changed node, report which derived
//! nodes are affected (and must be recomputed) and which remain valid, with a
//! machine-readable reason for each verdict.

use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::store::Store;

/// Returns { node_id: { verdict: "affected"|"valid", reason: ... } }.
pub fn impact(store: &Store, changed: &str) -> Value {
    let nodes = store.nodes();
    let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for n in &nodes {
        for d in &n.deps {
            dependents.entry(d.clone()).or_default().push(n.id.clone());
        }
    }
    let mut affected: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(changed.to_string());
    while let Some(cur) = queue.pop_front() {
        if let Some(deps) = dependents.get(&cur) {
            for d in deps {
                if affected.insert(d.clone()) {
                    queue.push_back(d.clone());
                }
            }
        }
    }
    let mut report = serde_json::Map::new();
    for n in &nodes {
        if n.id == changed {
            report.insert(
                n.id.clone(),
                json!({ "verdict": "changed", "reason": "source of the change" }),
            );
        } else if affected.contains(&n.id) {
            let path_via = n
                .deps
                .iter()
                .find(|d| *d == changed || affected.contains(*d))
                .cloned()
                .unwrap_or_default();
            report.insert(
                n.id.clone(),
                json!({
                    "verdict": "affected",
                    "reason": format!("depends on '{}' which changed; recomputation required", path_via),
                }),
            );
        } else {
            report.insert(
                n.id.clone(),
                json!({
                    "verdict": "valid",
                    "reason": format!("no dependency path from '{}'; current version still valid", changed),
                }),
            );
        }
    }
    Value::Object(report)
}
