//! Dependency-graph propagation. Each derivation node declares:
//!   * direct source inputs (raw record@version, instrument/mask hash),
//!   * upstream nodes (@current published version + their artifact hash),
//!   * a config hash.
//! The stored inputs_hash of the published version is compared to the live
//! fingerprint: unchanged nodes are reported valid WITH the reason, and only
//! affected nodes are recomputed.

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::db::model::checksum;
use crate::db::repo;

#[derive(Debug, Clone)]
pub struct Fingerprint {
    pub hash: String,
    pub parts: Vec<(String, String)>,
}

pub fn fingerprint(parts: Vec<(String, String)>) -> Fingerprint {
    let material: Vec<String> = parts.iter().map(|(k, v)| format!("{k}={v}")).collect();
    Fingerprint { hash: checksum(&material.iter().map(|s| s.as_str()).collect::<Vec<_>>()), parts }
}

/// Raw material fingerprint at a *specific* version.
pub fn raw_fingerprint(c: &Connection, record_id: &str, version: i64) -> rusqlite::Result<String> {
    let meta: String = c.query_row(
        "SELECT exposure_json||transmission_json||COALESCE(thickness_json,'')||source_file
         FROM raw_records WHERE record_id=?1 AND version=?2",
        params![record_id, version],
        |r| r.get(0),
    )?;
    let mut s = c.prepare(
        "SELECT ordinal,q_raw,i_raw,sig_raw FROM raw_points
         WHERE record_id=?1 AND version=?2 ORDER BY ordinal",
    )?;
    let pts: Vec<String> = s
        .query_map(params![record_id, version], |r| {
            Ok(format!("{}:{}:{}:{}", r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(checksum(&[&meta, &pts.join("|")]))
}

pub fn config_hash(config: &Value) -> String {
    // Stable serialization: sort keys by rendering canonical JSON via a
    // hand-rolled key sort.
    let stable = stable_string(config);
    checksum(&[&stable])
}

fn stable_string(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<_> = m.keys().collect();
            keys.sort();
            let inner = keys
                .into_iter()
                .map(|k| format!("{}:{}", k, stable_string(&m[k])))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
        Value::Array(a) => format!("[{}]", a.iter().map(stable_string).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Validity {
    Valid,
    Stale,
    NeverRun,
}

#[derive(Debug, Clone)]
pub struct NodeStatus {
    pub node: String,
    pub kind: String,
    pub title: String,
    pub current_version: i64,
    pub validity: Validity,
    pub reasons: Vec<String>,
    pub live_fingerprint: String,
}

/// Validate one node by diffing stored deps of its published version against
/// live fingerprints. Exact reasons are returned so the UI can explain why a
/// result is still valid or what forced recomputation.
pub fn node_status(c: &Connection, node: &str, live: &Fingerprint) -> rusqlite::Result<NodeStatus> {
    let (kind, title, current): (String, String, i64) =
        c.query_row("SELECT kind,title,current_version FROM derivations WHERE node=?1", params![node], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    let mut reasons = Vec::new();
    let validity = if current == 0 {
        Validity::NeverRun
    } else {
        let stored: String = c.query_row(
            "SELECT inputs_hash FROM derivation_versions WHERE node=?1 AND version=?2",
            params![node, current],
            |r| r.get(0),
        )?;
        if stored == live.hash {
            reasons.push("all direct sources, upstream versions and config match the published fingerprint".into());
            Validity::Valid
        } else {
            // Diff per dep kind for a human-readable cause list.
            let mut stmt = c.prepare(
                "SELECT dep_kind,ref_id,ref_version,ref_hash FROM version_deps
                 WHERE node=?1 AND version=?2",
            )?;
            let deps = stmt.query_map(params![node, current], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))
            })?;
            for d in deps {
                let (kind_, ref_id, ref_version, ref_hash) = d?;
                let live_hash = match kind_.as_str() {
                    "raw" => raw_fingerprint(c, &ref_id, ref_version).unwrap_or_default(),
                    "instrument" | "mask" => ref_id_hash(c, &kind_, &ref_id).unwrap_or_default(),
                    "node" => node_artifact_hash(c, &ref_id, ref_version).unwrap_or_default(),
                    "config" => ref_hash.clone(),
                    _ => ref_hash.clone(),
                };
                if live_hash != ref_hash {
                    reasons.push(format!("{kind_} {ref_id} changed ({})", kind_));
                }
            }
            if reasons.is_empty() {
                reasons.push("fingerprint differs (config or candidate decision changed)".into());
            }
            Validity::Stale
        }
    };
    Ok(NodeStatus { node: node.into(), kind, title, current_version: current, validity, reasons, live_fingerprint: live.hash.clone() })
}

fn ref_id_hash(c: &Connection, kind: &str, id: &str) -> rusqlite::Result<String> {
    let v = if kind == "instrument" {
        c.query_row("SELECT spec_json FROM instruments WHERE instrument_id=?1", params![id], |r| r.get::<_, String>(0))?
    } else {
        c.query_row("SELECT checksum FROM masks WHERE mask_id=?1", params![id], |r| r.get::<_, String>(0))?
    };
    Ok(checksum(&[&v]))
}

pub fn node_artifact_hash(c: &Connection, node: &str, version: i64) -> rusqlite::Result<String> {
    let mut stmt = c.prepare(
        "SELECT artifact_id,candidate_key,payload_json,checksum FROM artifacts
         WHERE node=?1 AND version=?2 ORDER BY artifact_id",
    )?;
    let mut mat = Vec::new();
    let rows = stmt.query_map(params![node, version], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
    })?;
    for r in rows {
        let (id, key, _payload, cs) = r?;
        mat.push(format!("{id}|{key}|{cs}"));
    }
    Ok(checksum(&[&mat.join("||")]))
}

/// DFS over graph edges (declared in derivation configs as `depends_on`)
/// returning nodes in recomputation order; stale nodes only.
pub fn affected_order(c: &Connection, roots: &[String], live_of: &dyn Fn(&str) -> Fingerprint) -> rusqlite::Result<Vec<NodeStatus>> {
    let mut order = Vec::new();
    let mut stack: Vec<String> = roots.to_vec();
    let mut seen = std::collections::HashSet::new();
    while let Some(node) = stack.pop() {
        if !seen.insert(node.clone()) {
            continue;
        }
        if repo::node_exists(c, &node)? {
            let st = node_status(c, &node, &live_of(&node))?;
            for child in children_of(c, &node)? {
                stack.push(child);
            }
            order.push(st);
        }
    }
    Ok(order.into_iter().filter(|s| s.validity != Validity::Valid).collect())
}

/// Children are nodes whose current version depends on `parent`.
pub fn children_of(c: &Connection, parent: &str) -> rusqlite::Result<Vec<String>> {
    let cur: i64 = c.query_row("SELECT current_version FROM derivations WHERE node=?1", params![parent], |r| r.get(0))?;
    let mut s = c.prepare(
        "SELECT DISTINCT d.node FROM version_deps v JOIN derivations d
         ON d.node=v.node AND d.current_version=v.version
         WHERE v.dep_kind='node' AND v.ref_id=?1 AND v.ref_version=?2",
    )?;
    let rows = s.query_map(params![parent, cur], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn status_event_payload(st: &NodeStatus) -> Value {
    json!({
        "node": st.node,
        "kind": st.kind,
        "title": st.title,
        "current_version": st.current_version,
        "validity": format!("{:?}", st.validity).to_lowercase(),
        "reasons": st.reasons,
        "fingerprint": st.live_fingerprint,
    })
}
