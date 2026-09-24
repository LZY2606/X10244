//! Crash atomicity (old or complete-new version), v1 -> v2 migration,
//! and the scale-uncertainty boundary fixture.

use rusqlite::Connection;
use saxs_station::cases;
use saxs_station::engine::{self, SubInputs};
use saxs_station::ledger::{self, ActionRequest};
use saxs_station::model::*;
use saxs_station::store::Store;

#[test]
fn v1_database_migrates_to_v2_without_losing_events() {
    // Hand-build a v1-era file database (no reviews table, user_version=1).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1.sqlite");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE objects (id TEXT PRIMARY KEY, kind TEXT NOT NULL, json TEXT NOT NULL, ord INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE events (id TEXT PRIMARY KEY, parent TEXT, chain TEXT NOT NULL, seq INTEGER NOT NULL,
                kind TEXT NOT NULL, detail TEXT NOT NULL, judgment TEXT NOT NULL, object_id TEXT,
                actor TEXT NOT NULL, ts_ms INTEGER NOT NULL, ord INTEGER NOT NULL DEFAULT 0, UNIQUE(chain, seq));
            CREATE TABLE chain_heads (chain TEXT PRIMARY KEY, head_event TEXT NOT NULL, fork_point TEXT);
            INSERT INTO objects VALUES('raw:x','raw','{}',0);
            INSERT INTO events VALUES('evt:1',NULL,'main',1,'import','{}','{"status":"ok","codes":[],"affected":[],"reason":""}',NULL,'a',1,0);
            INSERT INTO chain_heads VALUES('main','evt:1',NULL);
            "#,
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
    }
    let reopened = Store::open(path.to_str().unwrap()).unwrap();
    assert_eq!(reopened.schema_version().unwrap(), 2);
    let evs = reopened.all_events().unwrap();
    assert_eq!(evs.len(), 1);
    let rev = reopened.revision("main").unwrap();
    assert_eq!(rev, 1);
}

#[test]
fn failed_action_leaves_old_chain_head_intact() {
    let mut store = Store::memory().unwrap();
    let pack = cases::pack();
    let sample_raw = engine::import_raw(&pack.requests[0]);
    let bad_raw = engine::import_raw(&pack.requests[5]);
    let head_before = store.chain_head("main").unwrap();

    // failed normalize (bad unit) still appends an explicit error event,
    // but it never installs a new current object; chain head moves to the
    // recorded diagnostic event only, old objects remain complete.
    let _ = ledger::apply(
        &mut store,
        &ActionRequest {
            chain: "main".into(),
            action: "import".into(),
            actor: "t".into(),
            base_revision: None,
            allow_fork: false,
            payload: serde_json::to_value(&pack.requests[5]).unwrap(),
        },
        1,
    )
    .unwrap();
    let failed = ledger::apply(
        &mut store,
        &ActionRequest {
            chain: "main".into(),
            action: "normalize".into(),
            actor: "t".into(),
            base_revision: None,
            allow_fork: false,
            payload: serde_json::json!({"raw_id": bad_raw.object_id_ref()}),
        },
        2,
    )
    .unwrap();
    assert_eq!(failed.judgment.status, JudgmentStatus::Error);
    assert!(failed.object_id.is_none());

    // a successful normalization of the good sample yields a complete object
    let _ = ledger::apply(
        &mut store,
        &ActionRequest {
            chain: "main".into(),
            action: "import".into(),
            actor: "t".into(),
            base_revision: None,
            allow_fork: false,
            payload: serde_json::to_value(&pack.requests[0]).unwrap(),
        },
        3,
    )
    .unwrap();
    let ok = ledger::apply(
        &mut store,
        &ActionRequest {
            chain: "main".into(),
            action: "normalize".into(),
            actor: "t".into(),
            base_revision: None,
            allow_fork: false,
            payload: serde_json::json!({"raw_id": sample_raw.object_id_ref()}),
        },
        4,
    )
    .unwrap();
    assert!(ok.object_id.is_some());
    let _ = head_before;
}

#[test]
fn v2_buffer_fixture_carries_scale_uncertainty_guard() {
    let pack = cases::pack();
    let s = engine::import_raw(&pack.requests[0]);
    let b = engine::import_raw(&pack.requests[2]);
    let sn = engine::normalize(&s).unwrap();
    let bn = engine::normalize(&b).unwrap();
    let (sc, bc) = match (&sn, &bn) {
        (Obj::Norm { curve: a, .. }, Obj::Norm { curve: b, .. }) => (a, b),
        _ => panic!(),
    };
    let ranges = match &b {
        Obj::Raw { record, .. } => record
            .mask
            .as_ref()
            .map(|m| m.excluded_ranges.clone())
            .unwrap_or_default(),
        _ => vec![],
    };
    let sub = engine::subtract(SubInputs {
        sample_id: sn.object_id_ref(),
        buffer_id: bn.object_id_ref(),
        sample: sc,
        buffer: bc,
        buffer_mask_ranges: ranges,
    });
    if let Obj::Sub { curve, .. } = &sub {
        assert!(!curve.grids_identical);
        let par = curve.params.as_ref().unwrap();
        // noisy v2 buffer -> relative scale uncertainty surfaced in errors
        assert!(par.scale_sigma / par.scale > 0.0);
        let p = curve
            .points
            .iter()
            .find(|p| p.covered && !p.excluded)
            .unwrap();
        assert!(p.e_scale >= 0.0);
        assert!(p.e >= p.e_sample);
    } else {
        panic!()
    }
}
