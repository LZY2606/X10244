//! SQLite-backed event store.
//!
//! Guarantees:
//! * every derivation switch only happens after a fully successful commit;
//!   a crash mid-write leaves either the old version or the complete new one.
//! * two editors starting from the same head both save -> one gets a
//!   resolvable 409-style conflict (`base_seq` mismatch).
//! * export/delete/import preserves stable event ids, order and provenance.

use crate::events::{Event, EventKind};
use crate::id::stable_id;
use rusqlite::{params, Connection};
use std::sync::Mutex;

#[derive(Debug)]
pub struct Store {
    conn: Mutex<Connection>,
}

/// Result of a successful append.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CommitOutcome {
    pub seq: i64,
    pub event_id: String,
    pub head_seq: i64,
}

/// Conflict raised when two sessions edit from the same base head.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Conflict {
    pub expected_base: i64,
    pub actual_head: i64,
    pub message: String,
}

const SCHEMA_VERSION: i64 = 1;

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let store = Self { conn: Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }

    /// In-memory store for tests.
    pub fn open_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self { conn: Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let v: i64 =
            conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0)).unwrap_or(0);
        if v == 0 {
            Self::migrate_v0_to_v1(&mut conn)?;
        }
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS events (
                seq        INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id   TEXT NOT NULL UNIQUE,
                parent_seq INTEGER NOT NULL,
                at_ms      INTEGER NOT NULL,
                kind_json  TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_seq ON events(seq);
            "#,
        )?;
        Ok(())
    }

    /// v0 (legacy_events with full event envelopes) -> v1. Runs inside a
    /// single transaction so an interrupted migration leaves a coherent DB.
    fn migrate_v0_to_v1(conn: &mut Connection) -> rusqlite::Result<()> {
        let legacy_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='legacy_events'",
            [],
            |r| r.get(0),
        )?;
        if legacy_exists == 0 {
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            return Ok(());
        }
        let tx = conn.transaction()?;
        tx.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS events (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL UNIQUE,
                parent_seq INTEGER NOT NULL,
                at_ms INTEGER NOT NULL,
                kind_json TEXT NOT NULL
            );
            "#,
        )?;
        {
            let mut stmt = tx.prepare("SELECT payload FROM legacy_events ORDER BY id")?;
            let rows: Vec<String> =
                stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<_, _>>()?;
            drop(stmt);
            for payload in rows {
                let ev: Event = serde_json::from_str(&payload).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                tx.execute(
                    "INSERT INTO events (event_id, parent_seq, at_ms, kind_json) VALUES (?1,?2,?3,?4)",
                    params![ev.id, ev.parent_seq, ev.at_ms, serde_json::to_string(&ev.kind).unwrap()],
                )?;
            }
        }
        tx.execute_batch("DROP TABLE legacy_events;")?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
        Ok(())
    }

    /// Install an *older* schema to exercise the migration path in tests.
    pub fn install_legacy_schema(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("DROP TABLE IF EXISTS events; DROP TABLE IF EXISTS meta;")?;
        conn.execute_batch(
            r#"
            CREATE TABLE legacy_events (
                id INTEGER PRIMARY KEY,
                payload TEXT NOT NULL
            );
            "#,
        )?;
        conn.pragma_update(None, "user_version", 0)?;
        Ok(())
    }

    /// Migration entry point used when an older schema is detected.
    pub fn migrate_from_legacy(&self) -> rusqlite::Result<i64> {
        let v: i64 = self
            .conn
            .lock()
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?;
        if v >= SCHEMA_VERSION {
            return Ok(v);
        }
        // v0 -> v1: legacy rows are complete event JSON envelopes; promote them.
        if v == 0 {
            let mut conn = self.conn.lock().unwrap();
            let tx = conn.transaction()?;
            tx.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS events (
                    seq INTEGER PRIMARY KEY AUTOINCREMENT,
                    event_id TEXT NOT NULL UNIQUE,
                    parent_seq INTEGER NOT NULL,
                    at_ms INTEGER NOT NULL,
                    kind_json TEXT NOT NULL
                );
                "#,
            )?;
            let has_legacy: i64 =
                tx.query_row("SELECT COUNT(*) FROM legacy_events", [], |r| r.get(0))?;
            if has_legacy > 0 {
                let mut stmt = tx.prepare("SELECT payload FROM legacy_events ORDER BY id")?;
                let rows: Vec<String> = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<_, _>>()?;
                drop(stmt);
                for payload in rows {
                    let ev: Event = serde_json::from_str(&payload)
                        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?;
                    tx.execute(
                        "INSERT INTO events (event_id, parent_seq, at_ms, kind_json) VALUES (?1,?2,?3,?4)",
                        params![ev.id, ev.parent_seq, ev.at_ms, serde_json::to_string(&ev.kind).unwrap()],
                    )?;
                }
                tx.execute_batch("DROP TABLE legacy_events;")?;
            } else {
                tx.execute_batch("DROP TABLE IF EXISTS legacy_events;")?;
            }
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        }
        Ok(v)
    }

    pub fn head_seq(&self) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0))
    }

    /// Append an event. `base_seq` is the head the editor started from; a
    /// mismatch with the current head produces a resolvable conflict.
    pub fn append(
        &self,
        kind: EventKind,
        at_ms: i64,
        base_seq: i64,
    ) -> Result<CommitOutcome, Conflict> {
        let mut conn = self.conn.lock().unwrap();
        let head: i64 = conn
            .query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0))
            .map_err(|_| Conflict {
                expected_base: base_seq,
                actual_head: -1,
                message: "无法读取当前版本".into(),
            })?;
        if head != base_seq {
            return Err(Conflict {
                expected_base: base_seq,
                actual_head: head,
                message: format!("当前版本 {head} 与你编辑时的基础版本 {base_seq} 不一致，请比较两个分支后选择如何合并"),
            });
        }
        let tx = conn
            .transaction()
            .map_err(|_| conflict_internal(base_seq, head))?;
        let kind_json = serde_json::to_string(&kind).unwrap();
        let event_id = stable_id(
            "ev",
            &[&head.to_string(), &kind_json, &at_ms.to_string()],
        );
        tx.execute(
            "INSERT INTO events (event_id, parent_seq, at_ms, kind_json) VALUES (?1,?2,?3,?4)",
            params![event_id, head, at_ms, kind_json],
        )
        .map_err(|_| conflict_internal(base_seq, head))?;
        let seq = tx.last_insert_rowid();
        tx.commit().map_err(|_| conflict_internal(base_seq, head))?;
        Ok(CommitOutcome { seq, event_id, head_seq: seq })
    }

    /// Crash-simulation append: writes the row inside an explicit transaction
    /// but, when `crash` is set, rolls back and reports a synthetic kill so the
    /// store must later expose either old or complete-new state — never partial.
    pub fn append_with_crash(
        &self,
        kind: EventKind,
        at_ms: i64,
        base_seq: i64,
        crash: bool,
    ) -> Result<CommitOutcome, Conflict> {
        if !crash {
            return self.append(kind, at_ms, base_seq);
        }
        let mut conn = self.conn.lock().unwrap();
        let head: i64 = conn
            .query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0))
            .map_err(|_| conflict_internal(base_seq, -1))?;
        if head != base_seq {
            return Err(conflict_internal(base_seq, head));
        }
        let tx = conn.transaction().map_err(|_| conflict_internal(base_seq, head))?;
        let kind_json = serde_json::to_string(&kind).unwrap();
        let event_id = stable_id("ev", &[&head.to_string(), &kind_json, &at_ms.to_string()]);
        tx.execute(
            "INSERT INTO events (event_id, parent_seq, at_ms, kind_json) VALUES (?1,?2,?3,?4)",
            params![event_id, head, at_ms, kind_json],
        )
        .map_err(|_| conflict_internal(base_seq, head))?;
        // Simulated power loss: discard the uncommitted write.
        tx.rollback().map_err(|_| conflict_internal(base_seq, head))?;
        Err(Conflict {
            expected_base: base_seq,
            actual_head: head,
            message: "SIMULATED_CRASH_BEFORE_COMMIT".into(),
        })
    }

    /// Load all events in seq order.
    pub fn load_events(&self) -> rusqlite::Result<Vec<Event>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT seq, event_id, parent_seq, at_ms, kind_json FROM events ORDER BY seq")?;
        let rows = stmt.query_map([], |r| {
            let seq: i64 = r.get(0)?;
            let id: String = r.get(1)?;
            let parent_seq: i64 = r.get(2)?;
            let at_ms: i64 = r.get(3)?;
            let kind_json: String = r.get(4)?;
            let kind: EventKind = serde_json::from_str(&kind_json).unwrap();
            Ok(Event { seq, id, parent_seq, at_ms, kind })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Export the entire case pack as a single JSON envelope with stable ids.
    pub fn export_pack(&self) -> rusqlite::Result<String> {
        let events = self.load_events()?;
        let pack = serde_json::json!({
            "format": "saxs-bench-case",
            "format_version": 1,
            "schema_version": SCHEMA_VERSION,
            "rules_version_hint": crate::saxs::RULES_VERSION,
            "events": events,
        });
        Ok(serde_json::to_string_pretty(&pack).unwrap())
    }

    /// Import a case pack, preserving event ids and order. Refuses if it would
    /// create divergent history (different existing events).
    pub fn import_pack(&self, json: &str) -> rusqlite::Result<usize> {
        let pack: serde_json::Value =
            serde_json::from_str(json).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let events_val = pack
            .get("events")
            .and_then(|v| v.as_array())
            .ok_or_else(|| rusqlite::Error::InvalidQuery)?;
        let mut imported: Vec<Event> = Vec::new();
        for v in events_val {
            let ev: Event = serde_json::from_value(v.clone())
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?;
            imported.push(ev);
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let existing: i64 = tx.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
        if existing > 0 {
            // Verify identity/order consistency with existing history.
            let mut stmt = tx.prepare("SELECT event_id FROM events ORDER BY seq")?;
            let have: Vec<String> =
                stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<_, _>>()?;
            drop(stmt);
            if have.len() > imported.len() {
                return Err(rusqlite::Error::InvalidQuery);
            }
            for (i, id) in have.iter().enumerate() {
                if id != &imported[i].id {
                    return Err(rusqlite::Error::InvalidQuery);
                }
            }
        }
        for ev in imported.iter().skip(existing as usize) {
            tx.execute(
                "INSERT INTO events (event_id, parent_seq, at_ms, kind_json) VALUES (?1,?2,?3,?4)",
                params![ev.id, ev.parent_seq, ev.at_ms, serde_json::to_string(&ev.kind).unwrap()],
            )?;
        }
        tx.commit()?;
        Ok(imported.len())
    }

    /// Hard reset used by the replay control and import tests.
    pub fn wipe(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("DELETE FROM events; DELETE FROM sqlite_sequence WHERE name='events';")?;
        Ok(())
    }
}

fn conflict_internal(base: i64, head: i64) -> Conflict {
    Conflict {
        expected_base: base,
        actual_head: head,
        message: "存储层内部错误".into(),
    }
}
