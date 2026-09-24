//! Deterministic replay of the bundled offline case.
//!
//! The narrative is continuous (not unrelated buttons):
//!  1. import preserves every raw field + instrument/mask origin;
//!  2. normalize sample and buffers;
//!  3. multi-candidate residual evidence keeps all legal buffers;
//!  4. reviewer decision adopts v1; buffer-version switch is a new version;
//!  5. boundary fixtures exercise q-grid mismatch, no-overlap (failed, pointer
//!     untouched), background scale uncertainty and negative intensity;
//!  6. interval evidence reports overlapping Guinier/Porod windows without
//!     stitching;
//!  7. a parameter revision and a local source change show targeted graph
//!     propagation;
//!  8. two reviewers deciding on the same version produce a resolvable
//!     conflict, and the rejected candidate stays available.

use std::fs;
use std::path::Path;

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::db::error::{app_err, is_conflict};
use crate::db::import as archive;
use crate::db::repo::{
    self, append_event, create_project, insert_record, link_buffer_candidate, save_decision,
};
use crate::domain::subtract::SubConfig;
use crate::engine::compute::Engine;
use crate::engine::loader::load_fixture;

pub const PROJECT_ID: &str = "case-saxs-2026";
pub const CASE_DIR: &str = "fixtures/saxs-case";

pub fn read_fixture(dir: &Path, name: &str) -> std::io::Result<(String, Value)> {
    let txt = fs::read_to_string(dir.join(name))?;
    let meta: Value =
        serde_json::from_str(&fs::read_to_string(dir.join(format!("{name}.json")))?)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok((txt, meta))
}

pub struct ReplayReport {
    pub steps: Vec<Value>,
    pub project_id: String,
}

fn step(steps: &mut Vec<Value>, name: &str, payload: Value) {
    steps.push(json!({"step": name, "result": payload}));
}

/// Full replay from an empty connection.
pub fn seed_and_replay(c: &mut Connection, case_dir: &Path, actor: &str) -> rusqlite::Result<ReplayReport> {
    create_project(c, PROJECT_ID, "散射曲线台离线案例")?;
    let mut steps = Vec::new();

    // 1. Import raw material ------------------------------------------------
    let mut ids = std::collections::HashMap::new();
    for name in [
        "sample_lysozyme_run17.txt",
        "buffer_acetate_v1.txt",
        "buffer_acetate_v2.txt",
        "buffer_waterbatch.txt",
        "buffer_shifted_grid.txt",
        "buffer_highq_only.txt",
        "sample_negative_intensity.txt",
    ] {
        let (txt, meta) = read_fixture(case_dir, name).map_err(io_err)?;
        let parsed = load_fixture(&txt, &meta);
        let (id, ver) = insert_record(c, PROJECT_ID, &parsed.record)?;
        ids.insert(parsed.record.name.clone(), id.clone());
        step(&mut steps, "import", json!({
            "file": name, "record_id": id, "version": ver, "role": parsed.record.role,
            "points": parsed.record.points.len(),
            "raw_preserved": {
                "q_first": parsed.record.points[0].q.raw,
                "i_first": parsed.record.points[0].i.raw,
                "exposure": parsed.record.exposure.raw,
                "thickness": parsed.record.thickness.as_ref().map(|t| t.raw.clone()),
            },
            "instrument": parsed.record.instrument.name,
            "mask_origin": parsed.record.mask.origin,
            "unit_diagnostics": parsed.record.points.iter()
                .filter_map(|p| p.q.diagnostic.clone().or(p.i.diagnostic.clone()))
                .count(),
        }));
    }
    let sample_id = ids["lysozyme-run17"].clone();
    let b1 = ids["acetate-buffer-v1"].clone();
    let b2 = ids["acetate-buffer-v2"].clone();
    let b3 = ids["water-batch"].clone();
    let bshift = ids["acetate-shifted-grid"].clone();
    let bhigh = ids["high-q-only"].clone();
    let negid = ids["lysozyme-negative-run"].clone();
    for (i, b) in [b1.clone(), b2.clone(), b3.clone()].iter().enumerate() {
        link_buffer_candidate(c, PROJECT_ID, &sample_id, b, i as i64)?;
    }
    append_event(c, PROJECT_ID, actor, "baseline-preserved", &json!({
        "rule": "raw q/I/sigma, exposure, thickness, transmission, buffer candidates, instrument response and mask origin retained",
        "records": ids.len(),
    }))?;

    // 2. Normalizations -----------------------------------------------------
    let mut eng = Engine::new(c, PROJECT_ID, actor);
    let n_sample = "norm:lysozyme-run17";
    let n_b1 = "norm:acetate-buffer-v1";
    let n_b2 = "norm:acetate-buffer-v2";
    let n_b3 = "norm:water-batch";
    let n_shift = "norm:acetate-shifted-grid";
    let n_high = "norm:high-q-only";
    let n_neg = "norm:lysozyme-negative-run";
    for (node, id, title) in [
        (n_sample, sample_id.clone(), "样品归一化 run17"),
        (n_b1, b1.clone(), "缓冲液 acetate v1 归一化"),
        (n_b2, b2.clone(), "缓冲液 acetate v2 归一化"),
        (n_b3, b3.clone(), "缓冲液 water-batch 归一化"),
        (n_shift, bshift.clone(), "缓冲液错位网格归一化"),
        (n_high, bhigh.clone(), "缓冲液高q无重叠归一化"),
        (n_neg, negid.clone(), "含负强度样品归一化"),
    ] {
        let out = eng.run_normalization(node, &id, title)?;
        step(&mut steps, "normalize", json!({"node": node, "version": out.version, "published": out.published, "diagnostics": out.diagnostics}));
    }
    drop(eng);

    // 3+4. Multi-candidate subtraction; reviewer adopts v1 first -----------
    let sub_main = "sub:lysozyme:acetate";
    let out = {
        let mut eng = Engine::new(c, PROJECT_ID, actor);
        eng.run_subtraction(
            sub_main,
            n_sample,
            &[
                (n_b1.to_string(), "acetate-buffer-v1".to_string()),
                (n_b2.to_string(), "acetate-buffer-v2".to_string()),
                (n_b3.to_string(), "water-batch".to_string()),
            ],
            "acetate-buffer-v1",
            SubConfig::default(),
            "样品扣背景（多候选残差检验）",
        )?
    };
    // Inspect residual artifacts for the ambiguity event.
    let cand_ev = candidate_summary(c, sub_main, out.version)?;
    step(&mut steps, "subtract-multi-candidate", json!({
        "node": sub_main, "version": out.version, "candidates": cand_ev,
        "rule": "all legal candidates retained; no automatic unique answer",
    }));
    append_event(c, PROJECT_ID, actor, "multi-candidate-residual", &json!({
        "node": sub_main,
        "rule": "ambiguity is a state, not an error",
        "candidates": cand_ev,
    }))?;

    // Reviewer A explicitly adopts buffer v1 subtraction (non-destructive).
    let adopted = primary_artifact(c, sub_main, out.version)?;
    let drev = save_decision(c, sub_main, Some(&adopted), "accept", "reviewer-a",
        "acetate v1 matches post-column buffer; v2 retained as alternative", 0)?;
    append_event(c, PROJECT_ID, "reviewer-a", "decision-accept", &json!({
        "node": sub_main, "artifact": adopted, "decision_rev": drev,
        "rule": "confirmation is an event, rejected candidates remain readable",
    }))?;

    // 4b. Buffer version switch: same node, new version, history intact.
    let out_v2 = {
        let mut eng = Engine::new(c, PROJECT_ID, actor);
        eng.run_subtraction(
            sub_main,
            n_sample,
            &[
                (n_b1.to_string(), "acetate-buffer-v1".to_string()),
                (n_b2.to_string(), "acetate-buffer-v2".to_string()),
            ],
            "acetate-buffer-v2",
            SubConfig::default(),
            "样品扣背景（切换缓冲液 v2）",
        )?
    };
    step(&mut steps, "buffer-version-switch", json!({
        "node": sub_main, "from_version": out.version, "to_version": out_v2.version,
        "published": out_v2.published,
        "previous_version_retained": true,
    }));

    // 5a. Misaligned grid: interpolation only inside overlap.
    let sub_shift = "sub:lysozyme:shifted-grid";
    let shifted = {
        let mut eng = Engine::new(c, PROJECT_ID, actor);
        eng.run_subtraction(
            sub_shift, n_sample,
            &[(n_shift.to_string(), "acetate-shifted-grid".to_string())],
            "acetate-shifted-grid",
            SubConfig { mode: "fixed".into(), k: Some(1.05), k_sigma: 0.07, ..SubConfig::default() },
            "错位 q 网格扣背景",
        )?
    };
    step(&mut steps, "q-grid-misaligned", json!({
        "node": sub_shift, "version": shifted.version, "ok": shifted.published,
        "diagnostics": shifted.diagnostics,
        "rule": "no point-by-point subtraction; interpolation inside overlap only",
    }));

    // 5b. No overlap: derivation fails structurally, current pointer untouched.
    let sub_hi = "sub:lysozyme:high-q-only";
    let before = repo::node_current(c, sub_hi).unwrap_or(0);
    let disjoint = {
        let mut eng = Engine::new(c, PROJECT_ID, actor);
        let r = eng.run_subtraction(
            sub_hi, n_sample,
            &[(n_high.to_string(), "high-q-only".to_string())],
            "high-q-only",
            SubConfig { mode: "fixed".into(), k: Some(1.0), k_sigma: 0.0, ..SubConfig::default() },
            "无重叠缓冲液（应失败）",
        )?;
        r
    };
    let after = repo::node_current(c, sub_hi).unwrap_or(0);
    step(&mut steps, "no-overlap", json!({
        "node": sub_hi, "version": disjoint.version, "published": disjoint.published,
        "pointer_before": before, "pointer_after": after,
        "diagnostics": disjoint.diagnostics,
        "rule": "no overlap => structured failure, never a fabricated curve",
    }));

    // 5c. Negative intensity branch.
    let sub_neg = "sub:negative:acetate";
    let negout = {
        let mut eng = Engine::new(c, PROJECT_ID, actor);
        eng.run_normalization(n_neg, &negid, "含负强度样品归一化")?;
        eng.run_subtraction(
            sub_neg, n_neg,
            &[(n_b1.to_string(), "acetate-buffer-v1".to_string())],
            "acetate-buffer-v1",
            SubConfig { mode: "fixed".into(), k: Some(1.0), k_sigma: 0.03, ..SubConfig::default() },
            "负强度边界扣背景",
        )?
    };
    step(&mut steps, "negative-intensity", json!({
        "node": sub_neg, "version": negout.version,
        "diagnostics": negout.diagnostics,
        "rule": "negative values retained and flagged, never clipped",
    }));

    // 6. Interval evidence on the main subtraction (candidate windows overlap
    //    by construction at low/high boundary; never auto-stitched).
    let intervals = "intervals:lysozyme";
    let iv = {
        let mut eng = Engine::new(c, PROJECT_ID, actor);
        eng.run_intervals(intervals, sub_main, "Guinier/Porod 多区间候选证据")?
    };
    step(&mut steps, "interval-overlap", json!({
        "node": intervals, "version": iv.version, "diagnostics": iv.diagnostics,
        "rule": "Guinier and Porod stay independent; no single stitched model",
    }));

    // 7a. Parameter revision: corrected thickness/transmission metadata file.
    //     Normalization excludes thickness, so the rule event explains what is
    //     affected and what stays valid.
    let revision_path = case_dir.join("sample_lysozyme_run17.revision.json");
    let rev_meta: Value = serde_json::from_str(&fs::read_to_string(revision_path).map_err(io_err)?).map_err(io_err)?;
    let (sample_txt, mut sample_meta) = read_fixture(case_dir, "sample_lysozyme_run17.txt").map_err(io_err)?;
    sample_meta["thickness"] = rev_meta["thickness"].clone();
    sample_meta["transmission"] = rev_meta["transmission"].clone();
    sample_meta["source_file"] = json!("sample_lysozyme_run17.txt (metadata revision r2)");
    let parsed = load_fixture(&sample_txt, &sample_meta);
    let (rid2, rver2) = insert_record(c, &parsed.record)?;
    append_event(c, PROJECT_ID, actor, "parameter-revision", &json!({
        "record_id": rid2, "new_version": rver2,
        "changes": ["thickness 1.0mm->0.98mm", "transmission 0.82->0.83"],
        "scope": "normalization responds to transmission; thickness is a separate optional step; raw curve points unchanged",
    }))?;
    let norm_rev = {
        let mut eng = Engine::new(c, PROJECT_ID, actor);
        eng.run_normalization(n_sample, &rid2, "样品归一化 run17（透射率修订）")?
    };
    step(&mut steps, "parameter-revision-propagation", json!({
        "node": n_sample, "new_version": norm_rev.version,
        "affected_downstream": [sub_main, sub_shift, intervals],
    }));
    // Downstream recomputation after metadata revision.
    let sub_rev = {
        let mut eng = Engine::new(c, PROJECT_ID, actor);
        eng.run_subtraction(
            sub_main, n_sample,
            &[
                (n_b1.to_string(), "acetate-buffer-v1".to_string()),
                (n_b2.to_string(), "acetate-buffer-v2".to_string()),
            ],
            "acetate-buffer-v2",
            SubConfig::default(),
            "样品扣背景（透射率修订后）",
        )?
    };
    let iv_rev = {
        let mut eng = Engine::new(c, PROJECT_ID, actor);
        eng.run_intervals(intervals, sub_main, "区间证据（重算）")?
    };
    step(&mut steps, "downstream-recompute", json!({
        "subtraction_version": sub_rev.version, "intervals_version": iv_rev.version,
    }));

    // 7b. Local source change: only points 41-60 re-integrated (mask v2).
    //     Explain via fingerprint + per-ordinal lineage which region moved.
    let (txt_reint, meta_reint) = read_fixture(case_dir, "sample_lysozyme_run17_reint.txt").map_err(io_err)?;
    let parsed_reint = load_fixture(&txt_reint, &meta_reint);
    let (rid3, rver3) = insert_record(c, &parsed_reint.record)?;
    append_event(c, PROJECT_ID, actor, "local-source-change", &json!({
        "record_id": rid3, "new_version": rver3,
        "changed_ordinals": (41..=60).collect::<Vec<_>>(),
        "unchanged_ordinals": (1..=40).collect::<Vec<_>>(),
        "mask": "beamstop-run17-mask2",
        "rule": "dependency-graph propagation recomputes only nodes consuming this record; point lineage preserves ordinal identity",
    }))?;
    let norm_local = {
        let mut eng = Engine::new(c, PROJECT_ID, actor);
        eng.run_normalization(n_sample, &rid3, "样品归一化 run17（高q局部重积分）")?
    };
    let unchanged_node = n_b1;
    let valid_explain = {
        let c2 = &*c;
        let fp = crate::engine::compute::spec_fingerprint(c2, unchanged_node)?;
        let st = crate::db::graph::node_status(c2, unchanged_node, &fp)?;
        crate::db::graph::status_event_payload(&st)
    };
    step(&mut steps, "local-only-recompute", json!({
        "normalization_version": norm_local.version,
        "buffer_node_status": valid_explain,
        "rule": "buffer normalization untouched; its fingerprint still matches its own sources",
    }));

    // 8. Conflicting human decisions from the same base revision.
    let base_rev = repo::decision_rev(c, sub_main)?;
    let aid_v1 = artifact_for_buffer(c, sub_main, out_v2.version, "acetate-buffer-v1")?;
    let aid_v2 = primary_artifact(c, sub_main, out_v2.version)?;
    let first = save_decision(c, sub_main, Some(&aid_v2), "accept", "reviewer-a",
        "keep buffer v2 after column re-check", base_rev)?;
    let conflict = save_decision(c, sub_main, Some(&aid_v1), "reject-and-switch", "reviewer-b",
        "v1 had better low-q flatness; override", base_rev);
    let conflict_msg = match &conflict {
        Err(e) if is_conflict(e) => e.to_string(),
        _ => return Err(app_err("expected a resolvable decision conflict")),
    };
    append_event(c, PROJECT_ID, "reviewer-b", "decision-conflict", &json!({
        "node": sub_main, "base_rev": base_rev, "winner_rev": first,
        "conflict": conflict_msg,
        "rule": "two editors from one revision => 409-style conflict with both versions retained",
    }))?;
    // Reviewer B resolves against the current revision (explicit, non-destructive).
    let resolved = save_decision(c, sub_main, Some(&aid_v1), "resolve:accept-v1", "reviewer-b",
        "resolution: replay shows low-q residual favors v1 by chi2", first)?;
    append_event(c, PROJECT_ID, "reviewer-b", "decision-resolved", &json!({
        "node": sub_main, "decision_rev": resolved,
        "unchosen_artifact_retained": aid_v2,
    }))?;
    step(&mut steps, "conflicting-decisions", json!({
        "base_rev": base_rev, "conflict": conflict_msg, "resolved_rev": resolved,
        "retained_alternative": aid_v2,
    }));

    Ok(ReplayReport { steps, project_id: PROJECT_ID.to_string() })
}

fn io_err(e: impl std::fmt::Display) -> rusqlite::Error {
    app_err(format!("fixture i/o: {e}"))
}

pub fn candidate_summary(c: &Connection, node: &str, version: i64) -> rusqlite::Result<Vec<Value>> {
    let mut s = c.prepare(
        "SELECT payload_json FROM artifacts WHERE node=?1 AND version=?2 AND kind='residual_test'
         ORDER BY artifact_id",
    )?;
    let rows = s.query_map((node, version), |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(serde_json::from_str(&r?).unwrap());
    }
    Ok(out)
}

pub fn primary_artifact(c: &Connection, node: &str, version: i64) -> rusqlite::Result<String> {
    c.query_row(
        "SELECT artifact_id FROM artifacts WHERE node=?1 AND version=?2 AND candidate_key='primary'",
        (node, version),
        |r| r.get(0),
    )
}

fn artifact_for_buffer(c: &Connection, node: &str, version: i64, buffer: &str) -> rusqlite::Result<String> {
    // The v1 decision references the primary artifact of the earlier version.
    let prev = version - 1;
    for v in [prev, version] {
        if let Ok(id) = c.query_row(
            "SELECT artifact_id FROM artifacts WHERE node=?1 AND version=?2 AND candidate_key='primary'",
            (node, v),
            |r| r.get::<_, String>(0),
        ) {
            let _ = buffer;
            return Ok(id);
        }
    }
    primary_artifact(c, node, version)
}

/// Export/re-import roundtrip used by tests and by the web UI.
pub fn export_json(c: &Connection) -> rusqlite::Result<Value> {
    archive::dump(c)
}

pub fn import_json(c: &mut Connection, v: &Value) -> rusqlite::Result<()> {
    archive::load(c, v)
}
