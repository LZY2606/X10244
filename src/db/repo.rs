//! Data access: immutable raw material, versioned derivations, append-only
//! events and pointer journal.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::db::error::app_err;
use crate::db::model::{deterministic_id, InstrumentIn, MaskIn, PointIn, RecordIn, Scalar};

pub fn now_ts() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("2026-01-01T00:00:{}Z", d.as_millis() % 60000)
}

pub struct EventOut {
    pub seq: i64,
    pub event_id: String,
}

pub fn create_project(c: &Connection, project_id: &str, name: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO projects(project_id,name,created_at) VALUES (?1,?2,?3)",
        params![project_id, name, now_ts()],
    )?;
    Ok(())
}

pub fn next_seq(c: &Connection, project_id: &str) -> rusqlite::Result<i64> {
    let m: Option<i64> = c
        .query_row("SELECT MAX(seq) FROM events WHERE project_id=?1", params![project_id], |r| r.get(0))
        .optional()?;
    Ok(m.unwrap_or(0) + 1)
}

pub fn append_event(
    c: &Connection,
    project_id: &str,
    actor: &str,
    event_type: &str,
    payload: &Value,
) -> rusqlite::Result<EventOut> {
    let seq = next_seq(c, project_id)?;
    let event_id = deterministic_id("event", &[project_id, &seq.to_string(), event_type]);
    c.execute(
        "INSERT INTO events(event_id,project_id,seq,ts,actor,event_type,payload_json,actor_label)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            event_id,
            project_id,
            seq,
            now_ts(),
            actor,
            event_type,
            payload.to_string(),
            actor
        ],
    )?;
    Ok(EventOut { seq, event_id })
}

fn ensure_instrument(c: &Connection, project_id: &str, ins: &InstrumentIn) -> rusqlite::Result<String> {
    let id = deterministic_id("instrument", &[project_id, &ins.name]);
    c.execute(
        "INSERT OR IGNORE INTO instruments(instrument_id,project_id,name,spec_json,created_at)
         VALUES (?1,?2,?3,?4,?5)",
        params![id, project_id, ins.name, ins.spec.to_string(), now_ts()],
    )?;
    Ok(id)
}

fn ensure_mask(c: &Connection, project_id: &str, mask: &MaskIn) -> rusqlite::Result<String> {
    let id = deterministic_id("mask", &[project_id, &mask.name, &mask.checksum]);
    c.execute(
        "INSERT OR IGNORE INTO masks(mask_id,project_id,name,origin,locator,checksum,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![id, project_id, mask.name, mask.origin, mask.locator, mask.checksum, now_ts()],
    )?;
    Ok(id)
}

/// Insert a raw record revision. Record id is stable across revisions; the
/// prior revision's superseded_by link is recorded without deleting anything.
pub fn insert_record(
    c: &Connection,
    project_id: &str,
    rec: &RecordIn,
) -> rusqlite::Result<(String, i64)> {
    let instrument_id = ensure_instrument(c, project_id, &rec.instrument)?;
    let mask_id = ensure_mask(c, project_id, &rec.mask)?;
    let record_id = deterministic_id("record", &[project_id, &rec.role, &rec.name]);
    let prev: Option<i64> = c
        .query_row(
            "SELECT MAX(version) FROM raw_records WHERE record_id=?1",
            params![record_id],
            |r| r.get(0),
        )
        .optional()?;
    let version = prev.unwrap_or(0) + 1;
    if let Some(pv) = prev {
        c.execute(
            "UPDATE raw_records SET superseded_by=?1 WHERE record_id=?2 AND version=?3",
            params![&record_id, &record_id, pv],
        )?;
    }
    c.execute(
        "INSERT INTO raw_records(record_id,project_id,role,name,version,thickness_json,
           exposure_json,transmission_json,instrument_id,mask_id,source_file,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            record_id,
            project_id,
            rec.role,
            rec.name,
            version,
            rec.thickness.as_ref().map(|s| serde_json::to_string(s).unwrap()),
            serde_json::to_string(&rec.exposure).unwrap(),
            serde_json::to_string(&rec.transmission).unwrap(),
            instrument_id,
            mask_id,
            rec.source_file,
            now_ts()
        ],
    )?;
    for p in &rec.points {
        insert_point(c, &record_id, version, p)?;
    }
    Ok((record_id, version))
}

fn insert_point(c: &Connection, record_id: &str, version: i64, p: &PointIn) -> rusqlite::Result<()> {
    let canon = |s: &Scalar| s.canonical.unwrap_or(f64::NAN);
    let mut flags = Vec::new();
    for s in [&p.q, &p.i, &p.sig] {
        if let Some(d) = &s.diagnostic {
            flags.push(d.clone());
        }
    }
    c.execute(
        "INSERT INTO raw_points(record_id,version,ordinal,q_raw,i_raw,sig_raw,q,i,sig,flags_json,note)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            record_id,
            version,
            p.ordinal,
            p.q.raw,
            p.i.raw,
            p.sig.raw,
            canon(&p.q),
            canon(&p.i),
            canon(&p.sig),
            serde_json::to_string(&flags).unwrap(),
            p.note
        ],
    )?;
    Ok(())
}

pub fn link_buffer_candidate(
    c: &Connection,
    project_id: &str,
    sample_record_id: &str,
    buffer_record_id: &str,
    ordinal: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "INSERT OR REPLACE INTO buffer_candidates(project_id,sample_record_id,buffer_record_id,ordinal)
         VALUES (?1,?2,?3,?4)",
        params![project_id, sample_record_id, buffer_record_id, ordinal],
    )?;
    Ok(())
}

pub fn ensure_node(c: &Connection, project_id: &str, node: &str, kind: &str, title: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO derivations(node,project_id,kind,title,current_version)
         VALUES (?1,?2,?3,?4,0)",
        params![node, project_id, kind, title],
    )?;
    Ok(())
}

pub fn node_current(c: &Connection, node: &str) -> rusqlite::Result<i64> {
    c.query_row("SELECT current_version FROM derivations WHERE node=?1", params![node], |r| r.get(0))
}

pub fn node_exists(c: &Connection, node: &str) -> rusqlite::Result<bool> {
    Ok(c
        .query_row("SELECT 1 FROM derivations WHERE node=?1", params![node], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Begin a new derivation version in 'pending'. Returns the new version
/// number. The caller must finish with publish_version or abort_version.
pub fn begin_version(
    c: &Connection,
    node: &str,
    inputs_hash: &str,
    config: &Value,
) -> rusqlite::Result<i64> {
    let next = node_current(c, node)? + 1;
    c.execute(
        "INSERT INTO derivation_versions(node,version,status,inputs_hash,config_json,created_at)
         VALUES (?1,?2,'pending',?3,?4,?5)",
        params![node, next, inputs_hash, config.to_string(), now_ts()],
    )?;
    Ok(next)
}

pub fn add_dep(
    c: &Connection,
    node: &str,
    version: i64,
    dep_kind: &str,
    ref_id: &str,
    ref_version: i64,
    ref_hash: &str,
) -> rusqlite::Result<()> {
    c.execute(
        "INSERT OR REPLACE INTO version_deps(node,version,dep_kind,ref_id,ref_version,ref_hash)
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![node, version, dep_kind, ref_id, ref_version, ref_hash],
    )?;
    Ok(())
}

pub fn insert_artifact(
    c: &Connection,
    node: &str,
    version: i64,
    candidate_key: &str,
    kind: &str,
    adopted: bool,
    payload: &Value,
) -> rusqlite::Result<String> {
    let body = payload.to_string();
    let checksum = crate::db::model::checksum(&[node, &version.to_string(), candidate_key, &body]);
    let id = deterministic_id("artifact", &[node, &version.to_string(), candidate_key, &checksum]);
    c.execute(
        "INSERT INTO artifacts(artifact_id,node,version,candidate_key,kind,adopted,payload_json,checksum,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![id, node, version, candidate_key, kind, adopted as i64, body, checksum, now_ts()],
    )?;
    Ok(id)
}

pub fn insert_lineage(c: &Connection, artifact_id: &str, parent_points: &Value) -> rusqlite::Result<String> {
    let pid = deterministic_id("point", &[artifact_id, &parent_points.to_string()]);
    c.execute(
        "INSERT OR IGNORE INTO point_lineage(point_id,artifact_id,parent_points_json) VALUES (?1,?2,?3)",
        params![pid, artifact_id, parent_points.to_string()],
    )?;
    Ok(pid)
}

pub fn set_diagnostics(c: &Connection, node: &str, version: i64, diagnostics: &[String]) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE derivation_versions SET diagnostics_json=?1 WHERE node=?2 AND version=?3",
        params![serde_json::to_string(diagnostics).unwrap(), node, version],
    )?;
    Ok(())
}

/// Atomically validate every artifact checksum, mark the version complete and
/// move the current pointer. All in one transaction: a crash mid-write leaves
/// the pending version behind and the pointer on the previous complete one.
pub fn publish_version(c: &mut Connection, project_id: &str, node: &str, version: i64) -> rusqlite::Result<()> {
    let tx = c.transaction()?;
    let n_bad: i64 = tx.query_row(
        "SELECT COUNT(*) FROM artifacts WHERE node=?1 AND version=?2",
        params![node, version],
        |r| r.get(0),
    )?;
    if n_bad == 0 {
        return Err(app_err("refusing to publish version with no artifacts"));
    }
    // Re-verify every payload byte; a torn write fails here and the pointer
    // never moves.
    let mut bad = 0usize;
    {
        let mut stmt = tx.prepare(
            "SELECT candidate_key, payload_json, checksum FROM artifacts
             WHERE node=?1 AND version=?2",
        )?;
        let rows = stmt.query_map(params![node, version], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })?;
        for row in rows {
            let (key, body, want) = row?;
            let got = crate::db::model::checksum(&[node, &version.to_string(), &key, &body]);
            if got != want {
                bad += 1;
            }
        }
    }
    if bad > 0 {
        tx.execute(
            "UPDATE derivation_versions SET status='aborted' WHERE node=?1 AND version=?2",
            params![node, version],
        )?;
        tx.commit()?;
        return Err(app_err(format!("{bad} artifacts failed checksum validation")));
    }
    tx.execute(
        "UPDATE derivation_versions SET status='complete', completed_at=?1,
                 checksum=(SELECT ?2)
          WHERE node=?3 AND version=?4 AND status='pending'",
        params![now_ts(), crate::db::model::checksum(&[node, &version.to_string(), "complete"]), node, version],
    )?;
    let prev: i64 = tx.query_row("SELECT current_version FROM derivations WHERE node=?1", params![node], |r| r.get(0))?;
    tx.execute("UPDATE derivations SET current_version=?1 WHERE node=?2", params![version, node])?;
    let seq = {
        let m: Option<i64> = tx
            .query_row("SELECT MAX(seq) FROM pointers_log WHERE project_id=?1", params![project_id], |r| r.get(0))
            .optional()?;
        m.unwrap_or(0) + 1
    };
    tx.execute(
        "INSERT INTO pointers_log(project_id,seq,node,from_version,to_version) VALUES (?1,?2,?3,?4,?5)",
        params![project_id, seq, node, prev, version],
    )?;
    tx.commit()
}

pub fn abort_version(c: &Connection, node: &str, version: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE derivation_versions SET status='aborted' WHERE node=?1 AND version=?2 AND status='pending'",
        params![node, version],
    )?;
    Ok(())
}

/// Recovery after a crash/interrupted write: pending versions never publish.
/// Artifacts are checksummed; torn versions are aborted. Fully verified
/// versions are only promoted when the caller explicitly asks — publication
/// normally requires the atomic commit — so after restart the user sees
/// either the old pointer or a complete new version, never a torn one.
pub fn recover_pending(c: &mut Connection, promote_verified: bool) -> rusqlite::Result<Vec<(String, i64, String)>> {
    let pending: Vec<(String, i64)> = {
        let mut s = c.prepare("SELECT node, version FROM derivation_versions WHERE status='pending'")?;
        let rows = s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut out = Vec::new();
    for (node, version) in pending {
        let total: i64 = c.query_row(
            "SELECT COUNT(*) FROM artifacts WHERE node=?1 AND version=?2",
            params![node, version],
            |r| r.get(0),
        )?;
        let mut bad = 0i64;
        if total > 0 {
            let mut s = c.prepare("SELECT candidate_key,payload_json,checksum FROM artifacts WHERE node=?1 AND version=?2")?;
            let rows = s.query_map(params![node, version], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            })?;
            for r in rows {
                let (key, body, want) = r?;
                if crate::db::model::checksum(&[&node, &version.to_string(), &key, &body]) != want {
                    bad += 1;
                }
            }
        }
        let state = if total == 0 || bad > 0 {
            c.execute("UPDATE derivation_versions SET status='aborted' WHERE node=?1 AND version=?2", params![node, version])?;
            "aborted"
        } else if promote_verified {
            let project_id: String =
                c.query_row("SELECT project_id FROM derivations WHERE node=?1", params![node], |r| r.get(0))?;
            let tx = c.transaction()?;
            let prev: i64 = tx.query_row("SELECT current_version FROM derivations WHERE node=?1", params![node], |r| r.get(0))?;
            tx.execute(
                "UPDATE derivation_versions SET status='complete', completed_at=?1 WHERE node=?2 AND version=?3",
                params![now_ts(), node, version],
            )?;
            tx.execute("UPDATE derivations SET current_version=?1 WHERE node=?2", params![version, node])?;
            let seq: i64 = tx.query_row(
                "SELECT COALESCE(MAX(seq),0)+1 FROM pointers_log WHERE project_id=?1",
                params![project_id],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO pointers_log(project_id,seq,node,from_version,to_version)
                 VALUES (?1,?2,?3,?4,?5)",
                params![project_id, seq, node, prev, version],
            )?;
            tx.commit()?;
            "promoted"
        } else {
            "pending-verified"
        };
        out.push((node, version, state.to_string()));
    }
    Ok(out)
}

pub fn set_node_config(c: &mut Connection, node: &str, config: &Value, actor: &str, expected_rev: i64) -> rusqlite::Result<i64> {
    let rev: i64 = c.query_row("SELECT rev FROM node_config WHERE node=?1", params![node], |r| r.get(0)).unwrap_or(0);
    if rev != 0 && rev != expected_rev {
        return Err(app_err(
            format!("config conflict on {node}: expected rev {expected_rev}, current {rev}"),
        ));
    }
    let new_rev = rev + 1;
    c.execute(
        "INSERT INTO node_config(node,config_json,rev,updated_by,updated_at)
         VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(node) DO UPDATE SET config_json=excluded.config_json, rev=excluded.rev,
           updated_by=excluded.updated_by, updated_at=excluded.updated_at",
        params![node, config.to_string(), new_rev, actor, now_ts()],
    )?;
    Ok(new_rev)
}

pub fn get_node_config(c: &Connection, node: &str) -> rusqlite::Result<Option<(Value, i64)>> {
    c.query_row("SELECT config_json,rev FROM node_config WHERE node=?1", params![node], |r| {
        Ok((serde_json::from_str(&r.get::<_, String>(0)?).unwrap_or(Value::Null), r.get(1)?))
    }).optional()
}

/// Record a human decision. decisions are rev-checked so two reviewers saving
/// over each other produce a resolvable 409 instead of a silent overwrite.
pub fn save_decision(
    c: &mut Connection,
    node: &str,
    artifact_id: Option<&str>,
    action: &str,
    actor: &str,
    rationale: &str,
    expected_rev: i64,
) -> rusqlite::Result<i64> {
    let rev: i64 = c
        .query_row("SELECT decision_rev FROM node_decisions WHERE node=?1", params![node], |r| r.get(0))
        .unwrap_or(0);
    if rev != expected_rev {
        return Err(app_err(
            format!("decision conflict on {node}: expected rev {expected_rev}, current {rev}"),
        ));
    }
    let new_rev = rev + 1;
    c.execute(
        "INSERT INTO node_decisions(node,adopted_artifact_id,decision_rev,action,decided_by,rationale,updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(node) DO UPDATE SET adopted_artifact_id=excluded.adopted_artifact_id,
           decision_rev=excluded.decision_rev, action=excluded.action, decided_by=excluded.decided_by,
           rationale=excluded.rationale, updated_at=excluded.updated_at",
        params![node, artifact_id, new_rev, action, actor, rationale, now_ts()],
    )?;
    if let Some(aid) = artifact_id {
        c.execute("UPDATE artifacts SET adopted=(artifact_id=?2) WHERE node=?1", params![node, aid])?;
    }
    Ok(new_rev)
}

pub fn decision_rev(c: &Connection, node: &str) -> rusqlite::Result<i64> {
    Ok(c
        .query_row("SELECT decision_rev FROM node_decisions WHERE node=?1", params![node], |r| r.get(0))
        .optional()?
        .unwrap_or(0))
}
