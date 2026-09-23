//! SQLite storage: append-only events, versioned nodes, atomic pointer
//! switching, conflict detection, export/import and schema migration.
//!
//! Invariants:
//!  * raw sources are never rewritten; every derivation is a new version row.
//!  * the `current` pointer flips only after the whole transaction succeeds,
//!    so a crash mid-write leaves either the old version or a complete new one.
//!  * saving against a stale base version yields a resolvable conflict; the
//!    losing branch is kept, never overwritten.

use crate::model::{Event, Node, NodeId, NodeState};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

pub const SCHEMA_VERSION: i64 = 2;

pub struct Store {
    pub conn: Connection,
}

#[derive(Debug)]
pub enum SaveError {
    Conflict { current_head: i64, your_base: i64 },
    Other(String),
}

impl Store {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn schema_version(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    /// Create or migrate the schema. v1 -> v2 adds `rule_version` to nodes.
    pub fn migrate(&self) -> Result<(), String> {
        let v = self.schema_version();
        if v == 0 {
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     CREATE TABLE IF NOT EXISTS events(
                        seq INTEGER PRIMARY KEY AUTOINCREMENT,
                        kind TEXT NOT NULL,
                        payload TEXT NOT NULL,
                        created_at TEXT NOT NULL);
                     CREATE TABLE IF NOT EXISTS nodes(
                        id TEXT NOT NULL,
                        version INTEGER NOT NULL,
                        kind TEXT NOT NULL,
                        event_seq INTEGER NOT NULL,
                        state TEXT NOT NULL,
                        deps TEXT NOT NULL,
                        rule_version INTEGER NOT NULL DEFAULT 1,
                        payload TEXT NOT NULL,
                        PRIMARY KEY(id, version));
                     CREATE TABLE IF NOT EXISTS heads(
                        name TEXT PRIMARY KEY,
                        event_seq INTEGER NOT NULL);
                     INSERT OR REPLACE INTO meta(key, value) VALUES('schema_version', '2');",
                )
                .map_err(|e| e.to_string())?;
        } else if v == 1 {
            self.conn
                .execute_batch(
                    "ALTER TABLE nodes ADD COLUMN rule_version INTEGER NOT NULL DEFAULT 1;
                     UPDATE meta SET value='2' WHERE key='schema_version';",
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn head(&self) -> i64 {
        self.conn
            .query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0))
            .unwrap_or(0)
    }

    /// Append an event plus any node versions it produced, then move the head
    /// pointer — all in one transaction. `fail_hook` (tests only) aborts the
    /// transaction after node rows are written, simulating a crash mid-write.
    pub fn commit(
        &self,
        kind: &str,
        payload: serde_json::Value,
        nodes: &[Node],
        expected_base: Option<i64>,
        fail_hook: bool,
    ) -> Result<i64, SaveError> {
        if let Some(base) = expected_base {
            let head = self.head();
            if head != base {
                return Err(SaveError::Conflict { current_head: head, your_base: base });
            }
        }
        let tx = self.conn.unchecked_transaction().map_err(|e| SaveError::Other(e.to_string()))?;
        let created_at = "2026-09-24T00:00:00Z"; // deterministic clock for replay
        tx.execute(
            "INSERT INTO events(kind, payload, created_at) VALUES(?1, ?2, ?3)",
            params![kind, payload.to_string(), created_at],
        )
        .map_err(|e| SaveError::Other(e.to_string()))?;
        let seq = tx.last_insert_rowid();
        for n in nodes {
            let mut node = n.clone();
            node.event_seq = seq;
            tx.execute(
                "INSERT INTO nodes(id, version, kind, event_seq, state, deps, rule_version, payload)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    node.id,
                    node.version as i64,
                    node.kind,
                    node.event_seq,
                    serde_json::to_string(&node.state).unwrap(),
                    serde_json::to_string(&node.deps).unwrap(),
                    node.rule_version as i64,
                    node.payload.to_string(),
                ],
            )
            .map_err(|e| SaveError::Other(e.to_string()))?;
        }
        if fail_hook {
            drop(tx); // rollback: crash mid-write
            return Err(SaveError::Other("simulated crash mid-write".into()));
        }
        tx.execute(
            "INSERT OR REPLACE INTO heads(name, event_seq) VALUES('main', ?1)",
            params![seq],
        )
        .map_err(|e| SaveError::Other(e.to_string()))?;
        tx.commit().map_err(|e| SaveError::Other(e.to_string()))?;
        Ok(seq)
    }

    pub fn events(&self) -> Vec<Event> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, kind, payload, created_at FROM events ORDER BY seq")
            .unwrap();
        stmt.query_map([], |r| {
            Ok(Event {
                seq: r.get(0)?,
                kind: r.get(1)?,
                payload: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(json!(null)),
                created_at: r.get(3)?,
            })
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
    }

    /// Latest version of every node as of `as_of_seq` (None = latest).
    pub fn nodes_at(&self, as_of_seq: Option<i64>) -> Vec<Node> {
        let seq = as_of_seq.unwrap_or(i64::MAX);
        let mut all = self.all_nodes();
        all.retain(|n| n.event_seq <= seq);
        let mut latest: std::collections::BTreeMap<NodeId, Node> = Default::default();
        for n in all {
            latest
                .entry(n.id.clone())
                .and_modify(|e| {
                    if n.version > e.version {
                        *e = n.clone();
                    }
                })
                .or_insert(n);
        }
        latest.into_values().collect()
    }

    pub fn all_nodes(&self) -> Vec<Node> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, version, kind, event_seq, state, deps, rule_version, payload FROM nodes ORDER BY event_seq, id, version")
            .unwrap();
        stmt.query_map([], |r| {
            Ok(Node {
                id: r.get(0)?,
                version: r.get::<_, i64>(1)? as u64,
                kind: r.get(2)?,
                event_seq: r.get(3)?,
                state: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(NodeState::Valid),
                deps: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                rule_version: r.get::<_, i64>(6)? as u32,
                payload: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(json!(null)),
            })
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
    }

    pub fn next_version(&self, id: &str) -> u64 {
        self.conn
            .query_row("SELECT COALESCE(MAX(version),0) FROM nodes WHERE id=?1", params![id], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0) as u64
            + 1
    }

    /// Full-fidelity snapshot: identity, order and provenance preserved.
    pub fn export_json(&self) -> serde_json::Value {
        json!({
            "format": "saxs-workbench-export",
            "schema_version": SCHEMA_VERSION,
            "events": self.events(),
            "nodes": self.all_nodes(),
        })
    }

    pub fn import_json(&self, snap: &serde_json::Value) -> Result<(), String> {
        let events = snap.get("events").and_then(|v| v.as_array()).ok_or("bad export: events")?;
        let nodes = snap.get("nodes").and_then(|v| v.as_array()).ok_or("bad export: nodes")?;
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for e in events {
            let ev: Event = serde_json::from_value(e.clone()).map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO events(seq, kind, payload, created_at) VALUES(?1,?2,?3,?4)",
                params![ev.seq, ev.kind, ev.payload.to_string(), ev.created_at],
            )
            .map_err(|e| e.to_string())?;
        }
        for n in nodes {
            let node: Node = serde_json::from_value(n.clone()).map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO nodes(id, version, kind, event_seq, state, deps, rule_version, payload)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    node.id,
                    node.version as i64,
                    node.kind,
                    node.event_seq,
                    serde_json::to_string(&node.state).unwrap(),
                    serde_json::to_string(&node.deps).unwrap(),
                    node.rule_version as i64,
                    node.payload.to_string(),
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        let head: i64 = events
            .iter()
            .filter_map(|e| e.get("seq").and_then(|s| s.as_i64()))
            .max()
            .unwrap_or(0);
        tx.execute("INSERT OR REPLACE INTO heads(name, event_seq) VALUES('main', ?1)", params![head])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
}
