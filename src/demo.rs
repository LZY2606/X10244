//! Offline case package replay: "散射曲线台 — SAXS 归一化、背景扣除与多区间模型证据".
//! Loads fixtures/casepack and replays the full narrative: baseline import
//! preserving raw q/I/sigma, exposure, thickness, transmission, buffer
//! candidates, instrument response and mask source; then a parameter
//! revision, a localized source change (buffer version switch) and a
//! conflicting human decision, passing through grid mismatch, no-overlap,
//! background scale uncertainty, negative intensity, interval overlap and
//! multi-candidate residuals.

use anyhow::Result;
use serde_json::json;
use std::path::Path;

use crate::domain::NormFactors;
use crate::pipeline::*;
use crate::store::{Dep, Node, SaveError, Store};

pub const NORM: NormFactors = NormFactors {
    exposure_s: 10.0,
    thickness_cm: 0.10,
    transmission: 0.32,
    rel_unc_exposure: 0.01,
    rel_unc_thickness: 0.02,
    rel_unc_transmission: 0.03,
};

pub fn load_fixture(dir: &Path, name: &str) -> Result<serde_json::Value> {
    let text = std::fs::read_to_string(dir.join(name))?;
    Ok(serde_json::from_str(&text)?)
}

pub fn reset_store(store: &Store) -> Result<()> {
    store.conn.execute_batch("DELETE FROM events; DELETE FROM current; DELETE FROM nodes;")?;
    Ok(())
}

pub fn run_casepack(store: &Store, casepack: &Path) -> Result<Vec<String>> {
    let mut log = Vec::new();
    let actor = "casepack";

    // ---- 1. Baseline imports: everything raw is preserved ----------------
    let sample = load_fixture(casepack, "sample.json")?;
    import_record(store, "imp.sample", sample, actor)?;
    log.push("import imp.sample: raw q/I/sigma + exposure/thickness/transmission/mask preserved".into());

    for (id, file) in [
        ("imp.buffer.v1", "buffer_v1.json"),
        ("imp.buffer.v2", "buffer_v2.json"),
        ("imp.buffer.misaligned", "buffer_misaligned.json"),
        ("imp.buffer.nooverlap", "buffer_nooverlap.json"),
        ("imp.instrument", "instrument_response.json"),
    ] {
        let raw = load_fixture(casepack, file)?;
        import_record(store, id, raw, actor)?;
        log.push(format!("import {id}"));
    }

    // ---- 2. Normalization -------------------------------------------------
    run_normalization(store, "nrm.sample", "imp.sample", &NORM, actor)?;
    log.push("normalize nrm.sample (sigma includes normalization uncertainty)".into());

    // ---- 3. Background subtraction, buffer v1, scale 1.02 ----------------
    run_subtraction(store, "sub.main", "nrm.sample", "imp.buffer.v1", 1.02, 0.02, actor)?;
    log.push("subtract sub.main = nrm.sample - 1.02*buffer.v1 (negative points kept)".into());

    // ---- 4. Multi-interval candidates (overlapping, never merged) --------
    let candidates = vec![
        ("guinier".to_string(), [0.012, 0.045]),
        ("guinier".to_string(), [0.015, 0.060]),
        ("porod".to_string(), [0.150, 0.280]),
    ];
    run_fits(store, "fit.main", "sub.main", &candidates, actor)?;
    log.push("fit.main: 2 Guinier + 1 Porod candidates, overlaps flagged, not merged".into());

    // ---- 5. Parameter revision: scale 1.02 -> 1.00 ------------------------
    run_subtraction(store, "sub.main", "nrm.sample", "imp.buffer.v1", 1.00, 0.02, actor)?;
    let rep = recompute_affected(store, "sub.main", actor)?;
    log.push(format!("param revision scale=1.00; propagation: {}", fmt_reports(&rep)));

    // ---- 6. Localized source change: buffer version switch v1 -> v2 ------
    run_subtraction(store, "sub.main", "nrm.sample", "imp.buffer.v2", 1.00, 0.02, actor)?;
    let rep = recompute_affected(store, "sub.main", actor)?;
    store.commit(&[], &[], "buffer_version_switch", actor, json!({
        "from": "imp.buffer.v1", "to": "imp.buffer.v2",
        "affected": rep.iter().map(|r| r.node.clone()).collect::<Vec<_>>(),
        "rule": "dependency_graph_propagation_partial_update",
        "untouched": ["imp.sample", "nrm.sample"],
    }))?;
    log.push(format!("buffer switch v1->v2; propagation: {}", fmt_reports(&rep)));

    // ---- 7. Grid mismatch branch: interpolation within overlap only ------
    run_subtraction(store, "sub.misaligned", "nrm.sample", "imp.buffer.misaligned", 1.00, 0.02, actor)?;
    log.push("sub.misaligned: grids differ, interpolated within overlap only".into());

    // ---- 8. No-overlap attempt: rejected, no node created ----------------
    let rejected = run_subtraction(store, "sub.nooverlap", "nrm.sample", "imp.buffer.nooverlap", 1.00, 0.02, actor)?;
    debug_assert!(rejected.is_none());
    log.push("sub.nooverlap: rejected (no overlapping q range), event recorded".into());

    // ---- 9. Conflicting human decisions on the fit candidates ------------
    let base = store.max_seq()?;
    // user A accepts the narrow Guinier candidate
    let choice_a = json!({"candidate": "guinier[0.012,0.045]", "verdict": "accept",
                          "rationale": "lowest RMS in the Guinier region"});
    commit_decision(store, "decision.fit", &choice_a, "user-a")?;
    log.push("user-a accepted guinier[0.012,0.045]".into());
    // user B, still based on the same version, accepts the wide candidate
    let choice_b = json!({"candidate": "guinier[0.015,0.060]", "verdict": "accept",
                          "rationale": "wider interval, more points"});
    match store.check_base(base) {
        Err(SaveError::Conflict { expected_seq, actual_seq }) => {
            store.commit(&[], &[], "decision_conflict", "user-b", json!({
                "rule": "optimistic_concurrency_resolvable_conflict",
                "expected_seq": expected_seq, "actual_seq": actual_seq,
                "branch_a": choice_a, "branch_b": choice_b,
            }))?;
            log.push(format!("conflict detected: user-b based on evt {expected_seq}, current {actual_seq}"));
            // resolution: keep B, retain A as an unchosen candidate
            let cur = store.current_node("decision.fit")?;
            let unchosen = cur.payload["choice"].clone();
            let version = store.next_version("decision.fit")?;
            let node = Node {
                id: "decision.fit".into(),
                version,
                kind: "decision".into(),
                status: "ok".into(),
                payload: json!({"choice": choice_b, "unchosen": [unchosen],
                                "rule": "confirmation_not_irreversible_unchosen_kept"}),
                deps: vec![Dep { id: "fit.main".into(), version: store.current_node("fit.main")?.version }],
                input_hash: String::new(),
                created_event: "pending".into(),
            };
            let ev = store.commit(std::slice::from_ref(&node),
                &[("decision.fit".into(), version)],
                "decision_resolved", "user-b",
                json!({"kept": choice_b, "unchosen": [choice_a],
                       "rule": "both_branches_preserved"}))?;
            store.conn.execute("UPDATE nodes SET created_event=?1 WHERE id=?2 AND version=?3",
                rusqlite::params![ev.id, node.id, node.version])?;
            log.push("conflict resolved: kept user-b branch, user-a branch retained as unchosen".into());
        }
        _ => anyhow::bail!("expected a concurrency conflict in the casepack"),
    }
    Ok(log)
}

pub fn commit_decision(store: &Store, id: &str, choice: &serde_json::Value, actor: &str) -> Result<Node> {
    let version = store.next_version(id)?;
    let deps = vec![Dep { id: "fit.main".into(), version: store.current_node("fit.main")?.version }];
    let node = Node {
        id: id.into(),
        version,
        kind: "decision".into(),
        status: "ok".into(),
        payload: json!({"choice": choice}),
        deps,
        input_hash: String::new(),
        created_event: "pending".into(),
    };
    let ev = store.commit(std::slice::from_ref(&node),
        &[(id.into(), version)],
        "decision", actor, json!({"node": id, "choice": choice}))?;
    store.conn.execute("UPDATE nodes SET created_event=?1 WHERE id=?2 AND version=?3",
        rusqlite::params![ev.id, node.id, node.version])?;
    Ok(node)
}

fn fmt_reports(reps: &[RecomputeReport]) -> String {
    reps.iter().map(|r| format!("{}:{}", r.node, r.action)).collect::<Vec<_>>().join(", ")
}
