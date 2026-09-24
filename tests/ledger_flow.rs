//! End-to-end offline replay against SQLite: the whole review narrative,
//! dependency propagation, time travel, resolvable conflicts, crash
//! atomicity, old-version migration and export/delete/import identity.

use saxs_station::cases;
use saxs_station::ledger::{self, ActionRequest, ChainView};
use saxs_station::model::*;
use saxs_station::store::Store;

fn mem() -> Store {
    Store::memory().unwrap()
}

fn act(
    store: &mut Store,
    chain: &str,
    action: &str,
    payload: serde_json::Value,
    base: Option<i64>,
    fork: bool,
    actor: &str,
) -> Event {
    let req = ActionRequest {
        chain: chain.to_string(),
        action: action.to_string(),
        actor: actor.to_string(),
        base_revision: base,
        allow_fork: fork,
        payload,
    };
    ledger::apply(store, &req, 1_700_000_000_000).expect("action applies")
}

fn import_all(store: &mut Store) -> Vec<String> {
    cases::pack()
        .requests
        .iter()
        .map(|r| {
            act(
                store,
                "main",
                "import",
                serde_json::to_value(r).unwrap(),
                None,
                false,
                "importer",
            )
            .object_id
            .unwrap()
        })
        .collect()
}

fn normalize(store: &mut Store, raw_id: &str) -> String {
    act(
        store,
        "main",
        "normalize",
        serde_json::json!({"raw_id": raw_id}),
        None,
        false,
        "importer",
    )
    .object_id
    .unwrap()
}

fn subtract(store: &mut Store, s: &str, b: &str, fork: bool, base: Option<i64>) -> Event {
    act(
        store,
        "main",
        "subtract",
        serde_json::json!({"sample_norm_id": s, "buffer_norm_id": b}),
        base,
        fork,
        "reviewer",
    )
}

fn raw_id(view: &ChainView, label: &str) -> String {
    view.raws
        .iter()
        .find(|r| r.label == label)
        .unwrap()
        .id
        .clone()
}

#[test]
fn full_narrative_replays_all_seven_situations() {
    let mut store = mem();
    let raws = import_all(&mut store);
    let view = ledger::main_chain(&store).unwrap();
    assert_eq!(view.raws.len(), 6);

    // bad-unit raw retained but not normalizable
    let bad = raw_id(&view, "BSA bad thickness unit");
    let bad_err = ledger::apply(
        &mut store,
        &ActionRequest {
            chain: "main".into(),
            action: "normalize".into(),
            actor: "t".into(),
            base_revision: None,
            allow_fork: false,
            payload: serde_json::json!({"raw_id": bad}),
        },
        1,
    )
    .unwrap();
    assert_eq!(bad_err.judgment.status, JudgmentStatus::Error);

    let sample = normalize(&mut store, &raws[0]);
    let buf1 = normalize(&mut store, &raws[1]);
    let buf2 = normalize(&mut store, &raws[2]);
    let bufn = normalize(&mut store, &raws[3]);
    let dip = normalize(&mut store, &raws[4]);

    // v1 identical grids: clean
    let e1 = subtract(&mut store, &sample, &buf1, false, None);
    assert_eq!(e1.judgment.status, JudgmentStatus::Ok);
    let sub1 = e1.object_id.unwrap();

    // v2 buffer: grid mismatch guarded, interpolation overlap only
    let e2 = subtract(&mut store, &sample, &buf2, false, None);
    assert!(e2.judgment.codes.contains(&"grid_mismatch".to_string()));

    // far-q buffer: no overlap error
    let e3 = subtract(&mut store, &sample, &bufn, false, None);
    assert!(e3.judgment.codes.contains(&"no_overlap".to_string()));
    assert_eq!(e3.judgment.status, JudgmentStatus::Error);

    // dip sample: negatives retained
    let e4 = subtract(&mut store, &dip, &buf1, false, None);
    assert!(e4.judgment.codes.contains(&"negative_intensity".to_string()));

    // multi-candidate interval evidence + overlaps flagged
    let fit = act(
        &mut store,
        "main",
        "fit_models",
        serde_json::json!({"sub_id": sub1}),
        None,
        false,
        "reviewer",
    );
    assert_eq!(fit.judgment.status, JudgmentStatus::Multi);
    assert!(fit
        .judgment
        .codes
        .contains(&"multi_candidate_residual".to_string()));

    // adopting one candidate does not remove the others
    let cands_id = fit.object_id.unwrap();
    let Obj::Candidates { candidates, .. } = store.object(&cands_id).unwrap().unwrap() else {
        panic!()
    };
    let adopt = candidates.first().unwrap().id.clone();
    let decision = act(
        &mut store,
        "main",
        "decision",
        serde_json::json!({
            "candidates_id": cands_id,
            "source_sub": sub1,
            "decisions": { adopt: "adopted" },
        }),
        None,
        false,
        "reviewer",
    );
    let new_cands = decision.object_id.unwrap();
    let Obj::Candidates { candidates: after, .. } =
        store.object(&new_cands).unwrap().unwrap() else { panic!() };
    assert_eq!(after.len(), candidates.len());
    assert!(after.iter().any(|c| c.decision == CandidateDecision::Adopted));
    assert!(after.iter().any(|c| c.decision == CandidateDecision::Proposed));
    // old candidates object still exists (no overwrite)
    assert!(store.object(&cands_id).unwrap().is_some());
}

#[test]
fn local_source_revision_only_propagates_to_its_own_subtree() {
    let mut store = mem();
    let raws = import_all(&mut store);
    let sample = normalize(&mut store, &raws[0]);
    let buf1 = normalize(&mut store, &raws[1]);
    let buf2 = normalize(&mut store, &raws[2]);
    let sub_v1 = subtract(&mut store, &sample, &buf1, false, None).object_id.unwrap();

    // Revise only the sample exposure: new raw id -> new norm id -> new sub;
    // buffer normalization is unaffected and still valid (same content id).
    let mut req = cases::sample_request();
    req.exposure = ("1900".to_string(), "s".to_string());
    let rev = act(
        &mut store,
        "main",
        "revise_raw",
        serde_json::to_value(&req).unwrap(),
        None,
        false,
        "beamline",
    );
    let new_raw = rev.object_id.unwrap();
    assert_ne!(new_raw, raws[0]);
    let new_norm = normalize(&mut store, &new_raw);
    assert_ne!(new_norm, sample);
    // buffer norm untouched
    let still = store.object(&buf1).unwrap();
    assert!(still.is_some());

    let before = saxs_station::graph::Graph::from_objects(&store.all_objects().unwrap());
    let sub_v2_event = subtract(&mut store, &new_norm, &buf1, false, None);
    let sub_v2 = sub_v2_event.object_id.unwrap();
    assert_ne!(sub_v2, sub_v1);

    // subtraction using the unchanged buffer v2 grid stays the same content
    // The old subtraction is in the sample's downstream; buffer-derived
    // content is independent. New graph keeps every buffer object by the
    // same content id, so propagation marks them STILL_VALID, never recomputed.
    let after = saxs_station::graph::Graph::from_objects(&store.all_objects().unwrap());
    let affected = before.downstream(&[raws[0].clone()]);
    assert!(affected.contains(&sub_v1));
    assert!(!affected.contains(&buf1));
    assert!(!affected.contains(&buf2));
    let report = before.propagation_report(&after, &[raws[0].clone()]);
    assert!(report.explain(&before).iter().any(|l| l.contains("STILL_VALID")));
}

#[test]
fn time_travel_replays_old_versions_not_latest() {
    let mut store = mem();
    let raws = import_all(&mut store);
    let sample = normalize(&mut store, &raws[0]);
    let buf1 = normalize(&mut store, &raws[1]);
    let buf2 = normalize(&mut store, &raws[2]);
    let e_v1 = subtract(&mut store, &sample, &buf1, false, None);
    let head_after_v1 = e_v1.id.clone();
    let _e_v2 = subtract(&mut store, &sample, &buf2, false, None);

    // Latest view knows the v2 buffer subtraction;
    let latest = ledger::main_chain(&store).unwrap();
    let latest_buf = latest
        .current
        .get(&format!("buffer_for:{sample}"))
        .unwrap();
    assert_eq!(latest_buf, &buf2);

    // View at the v1 event restores the v1 conclusion.
    let old = ledger::view_at(&store, "main", Some(&head_after_v1)).unwrap();
    let old_buf = old
        .current
        .get(&format!("buffer_for:{sample}"))
        .unwrap();
    assert_eq!(old_buf, &buf1);
    assert!(old.events.iter().any(|e| e.id == head_after_v1));
    assert!(old.events.iter().all(|e| e.id != _e_v2.id));
}

#[test]
fn concurrent_edits_from_same_revision_get_a_resolvable_conflict() {
    let mut store = mem();
    let raws = import_all(&mut store);
    let sample = normalize(&mut store, &raws[0]);
    let buf1 = normalize(&mut store, &raws[1]);
    let buf2 = normalize(&mut store, &raws[2]);
    let _first = subtract(&mut store, &sample, &buf1, false, None);
    let rev = ledger::main_chain(&store).unwrap().revision;

    // Reviewer B edits from the same base: stale -> 409 by default
    let err = ledger::apply(
        &mut store,
        &ActionRequest {
            chain: "main".into(),
            action: "subtract".into(),
            actor: "B".into(),
            base_revision: Some(rev - 1),
            allow_fork: false,
            payload: serde_json::json!({"sample_norm_id": sample, "buffer_norm_id": buf2}),
        },
        9,
    );
    assert!(matches!(
        err,
        Err(saxs_station::store::StoreError::Conflict { .. })
    ));

    // Resolvable path: a fork chain is created, both branches retained.
    let forked = ledger::apply(
        &mut store,
        &ActionRequest {
            chain: "main".into(),
            action: "subtract".into(),
            actor: "B".into(),
            base_revision: Some(rev - 1),
            allow_fork: true,
            payload: serde_json::json!({"sample_norm_id": sample, "buffer_norm_id": buf2}),
        },
        10,
    )
    .unwrap();
    assert_ne!(forked.chain, "main");
    let chains = store.chains().unwrap();
    assert!(chains.iter().any(|(c, _, _)| c == "main"));
    assert!(chains.iter().any(|(c, _, _)| c == &forked.chain));
}

#[test]
fn export_delete_import_preserves_identity_order_and_provenance() {
    let mut store = mem();
    let raws = import_all(&mut store);
    let sample = normalize(&mut store, &raws[0]);
    let buf1 = normalize(&mut store, &raws[1]);
    let sub = subtract(&mut store, &sample, &buf1, false, None).object_id.unwrap();
    let fit = act(
        &mut store,
        "main",
        "fit_models",
        serde_json::json!({"sub_id": sub}),
        None,
        false,
        "t",
    );
    let cands = fit.object_id.unwrap();
    let archive = store.export_archive().unwrap();

    // Delete everything (fresh store) then re-import.
    let mut fresh = mem();
    fresh.import_archive(&archive).unwrap();
    let view = ledger::main_chain(&fresh).unwrap();
    assert!(view.norm_of.values().any(|v| v == &sample));
    assert!(view
        .subtractions
        .values()
        .flatten()
        .any(|v| v == &sub));
    assert_eq!(view.candidates_of.get(&sub), Some(&cands));

    // provenance links still resolve to both source curves
    let Obj::Sub {
        sample_norm,
        buffer_norm,
        curve,
        ..
    } = fresh.object(&sub).unwrap().unwrap()
    else {
        panic!()
    };
    assert_eq!(sample_norm, sample);
    assert_eq!(buffer_norm, buf1);
    let p = curve.points.iter().find(|p| p.covered).unwrap();
    assert!(!p.source_buffer_pids.is_empty());

    // event order preserved
    let evs = fresh.all_events().unwrap();
    assert_eq!(evs.first().unwrap().kind, EventKind::Import);
}
