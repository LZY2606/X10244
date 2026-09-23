//! Storage guarantees: atomic version switch, write-interruption recovery,
//! concurrent-save conflict, export/delete/import identity, legacy migration.

use saxs_bench::case::*;
use saxs_bench::events::EventKind;
use saxs_bench::store::Store;

fn first_event() -> EventKind {
    base_event_kinds(0).into_iter().next().unwrap()
}

#[test]
fn append_is_sequential_and_head_advances() {
    let store = Store::open_memory().unwrap();
    let kinds = base_event_kinds(0);
    let mut base = 0;
    for (i, k) in kinds.into_iter().enumerate() {
        let out = store.append(k, 1000 + i as i64, base).unwrap();
        assert_eq!(out.seq, i as i64 + 1);
        base = out.seq;
    }
    assert_eq!(store.head_seq().unwrap(), 11);
}

#[test]
fn crash_before_commit_leaves_old_version_never_partial() {
    let store = Store::open_memory().unwrap();
    let kinds = base_event_kinds(0);
    // commit 4 events
    let mut base = 0;
    for (i, k) in kinds.iter().take(4).enumerate() {
        base = store.append(k.clone(), 1000 + i as i64, base).unwrap().seq;
    }
    assert_eq!(store.head_seq().unwrap(), 4);

    // crash on the 5th
    let err = store
        .append_with_crash(kinds[4].clone(), 2000, 4, true)
        .unwrap_err();
    assert_eq!(err.message, "SIMULATED_CRASH_BEFORE_COMMIT");

    // store still shows exactly the old complete prefix
    assert_eq!(store.head_seq().unwrap(), 4);
    let evs = store.load_events().unwrap();
    assert_eq!(evs.len(), 4);
    // and a fresh successful commit yields a complete version 5
    let out = store.append(kinds[4].clone(), 2001, 4).unwrap();
    assert_eq!(out.seq, 5);
    assert_eq!(store.load_events().unwrap().len(), 5);
}

#[test]
fn two_editors_from_same_base_one_gets_resolvable_conflict() {
    let store = Store::open_memory().unwrap();
    let kinds = base_event_kinds(0);
    store.append(kinds[0].clone(), 1, 0).unwrap();
    // both editors saw head=1
    let a = store.append(kinds[1].clone(), 2, 1);
    assert!(a.is_ok());
    // second editor still assumes base=1
    let b = store.append(kinds[2].clone(), 3, 1);
    let conflict = b.unwrap_err();
    assert_eq!(conflict.expected_base, 1);
    assert_eq!(conflict.actual_head, 2);
    // after refreshing base to 2, the second change can be applied
    assert!(store.append(kinds[2].clone(), 4, 2).is_ok());
}

#[test]
fn export_wipe_import_preserves_identity_order_and_provenance() {
    let store = Store::open_memory().unwrap();
    let kinds = base_event_kinds(0);
    let mut base = 0;
    for (i, k) in kinds.into_iter().enumerate() {
        base = store.append(k, 1000 + i as i64, base).unwrap().seq;
    }
    let before = store.load_events().unwrap();
    let pack = store.export_pack().unwrap();
    store.wipe().unwrap();
    assert_eq!(store.head_seq().unwrap(), 0);
    let n = store.import_pack(&pack).unwrap();
    assert_eq!(n, before.len());
    let after = store.load_events().unwrap();
    assert_eq!(after.len(), before.len());
    for (a, b) in before.iter().zip(after.iter()) {
        assert_eq!(a.id, b.id, "stable identity preserved");
        assert_eq!(a.seq, b.seq, "order preserved");
        assert_eq!(a.kind, b.kind, "provenance content preserved");
        assert_eq!(a.parent_seq, b.parent_seq);
    }
}

#[test]
fn import_is_deterministic_across_fresh_databases() {
    let kinds = base_event_kinds(0);
    let s1 = Store::open_memory().unwrap();
    let s2 = Store::open_memory().unwrap();
    let events = build_events(kinds.clone(), 5000);
    let pack = serde_json::json!({"format":"saxs-bench-case","format_version":1,"events": events}).to_string();
    s1.import_pack(&pack).unwrap();
    s2.import_pack(&pack).unwrap();
    let a = s1.load_events().unwrap();
    let b = s2.load_events().unwrap();
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.id, y.id);
    }
}

#[test]
fn importing_divergent_history_is_rejected() {
    let s1 = Store::open_memory().unwrap();
    let mut kinds = base_event_kinds(0);
    s1.append(kinds[0].clone(), 1, 0).unwrap();
    // build a pack whose first event differs
    let mut diverging = build_events(kinds.clone(), 5000);
    diverging[0].id = "ev_tampered".into();
    let pack = serde_json::json!({"format":"saxs-bench-case","events": diverging}).to_string();
    assert!(s1.import_pack(&pack).is_err());
    let _ = kinds.drain(..);
}

#[test]
fn legacy_v0_schema_migrates_and_keeps_events() {
    // Build a v0 database on disk with a legacy table containing full event
    // envelopes, exactly as an older release would have written it.
    let kinds = base_event_kinds(0);
    let events = build_events(kinds, 7000);
    let dir = std::env::temp_dir().join(format!("saxs_legacy_{}.sqlite3", std::process::id()));
    let _ = std::fs::remove_file(&dir);
    {
        let conn = rusqlite::Connection::open(&dir).unwrap();
        conn.execute_batch(
            "CREATE TABLE legacy_events (id INTEGER PRIMARY KEY, payload TEXT NOT NULL);",
        )
        .unwrap();
        for e in &events {
            conn.execute(
                "INSERT INTO legacy_events (payload) VALUES (?1)",
                rusqlite::params![serde_json::to_string(e).unwrap()],
            )
            .unwrap();
        }
        conn.pragma_update(None, "user_version", 0).unwrap();
    }
    // Opening must transparently migrate old -> new inside one transaction.
    let store = Store::open(dir.to_str().unwrap()).unwrap();
    let loaded = store.load_events().unwrap();
    assert_eq!(loaded.len(), events.len());
    for (a, b) in loaded.iter().zip(events.iter()) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.seq, b.seq);
        assert_eq!(a.kind, b.kind);
    }
    let _ = std::fs::remove_file(&dir);
}
