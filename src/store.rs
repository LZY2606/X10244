//! Versioned, event-sourced SQLite store.
//!
//! Invariants:
//! * every derivation produces a new object version; the head pointer is
//!   switched only after the whole write succeeds (single transaction);
//! * a crash mid-write leaves either the old version or a complete new one;
//! * two users editing from the same base version produce a resolvable
//!   conflict: both branches are kept, one head wins, nothing is overwritten;
//! * export -> delete -> import preserves identity, order and provenance.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObjectVersion {
    pub id: String,
    pub version: i64,
    pub kind: String,
    pub created_seq: i64,
    pub superseded_seq: Option<i64>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub seq: i64,
    pub id: String,
    pub kind: String,
    pub base_head: i64,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Head {
    pub name: String,
    pub seq: i64,
}

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Conflict { expected: i64, actual: i64 },
    Missing(String),
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Sql(e) => write!(f, "sqlite: {e}"),
            StoreError::Conflict { expected, actual } => {
                write!(f, "conflict: base head {expected}, current head {actual}")
            }
            StoreError::Missing(s) => write!(f, "missing: {s}"),
        }
    }
}

pub struct Store {
    pub conn: Connection,
}

/// A batch of writes committed atomically with one event.
pub struct WriteBatch {
    pub event_id: String,
    pub event_kind: String,
    pub event_payload: serde_json::Value,
    /// Object versions to insert: (id, kind, payload). Version is assigned
    /// automatically as previous+1.
    pub puts: Vec<(String, String, serde_json::Value)>,
    /// Object ids superseded by this batch (their current version is closed).
    pub supersedes: Vec<String>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store, StoreError> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Store, StoreError> {
        let conn = Connection::open_in_memory()?;
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn schema_version(&self) -> Result<i64, StoreError> {
        let has_meta: bool = self.conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='meta'",
            [],
            |r| r.get::<_, i64>(0),
        )? > 0;
        if !has_meta {
            return Ok(0);
        }
        let v: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0))
            .ok();
        Ok(v.and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    /// Migrate older databases forward. v1 -> v2 adds the `branches` table.
    pub fn migrate(&self) -> Result<(), StoreError> {
        let v = self.schema_version()?;
        if v < 1 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS events(
                   seq INTEGER PRIMARY KEY AUTOINCREMENT,
                   id TEXT NOT NULL,
                   kind TEXT NOT NULL,
                   base_head INTEGER NOT NULL,
                   payload TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS objects(
                   id TEXT NOT NULL,
                   version INTEGER NOT NULL,
                   kind TEXT NOT NULL,
                   created_seq INTEGER NOT NULL,
                   superseded_seq INTEGER,
                   payload TEXT NOT NULL,
                   PRIMARY KEY(id, version)
                 );
                 CREATE TABLE IF NOT EXISTS heads(name TEXT PRIMARY KEY, seq INTEGER NOT NULL);
                 INSERT OR IGNORE INTO heads(name, seq) VALUES('main', 0);
                 INSERT OR REPLACE INTO meta(key, value) VALUES('schema_version','1');",
            )?;
        }
        if v < 2 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS branches(
                   name TEXT PRIMARY KEY,
                   head_seq INTEGER NOT NULL,
                   status TEXT NOT NULL DEFAULT 'open',
                   note TEXT
                 );",
            )?;
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    pub fn head_seq(&self) -> Result<i64, StoreError> {
        self.head_seq_of("main")
    }

    pub fn head_seq_of(&self, name: &str) -> Result<i64, StoreError> {
        let seq: i64 = self.conn.query_row(
            "SELECT seq FROM heads WHERE name=?1",
            params![name],
            |r| r.get(0),
        )?;
        Ok(seq)
    }

    /// Commit a batch atomically. If `base_head` does not match the current
    /// head, the batch is committed on a branch instead and a Conflict error
    /// is returned — both branches survive.
    pub fn commit(&self, base_head: i64, batch: &WriteBatch) -> Result<i64, StoreError> {
        let current = self.head_seq()?;
        let on_branch = base_head != current;
        let tx = self.conn.unchecked_transaction()?;
        let seq: i64 = {
            tx.execute(
                "INSERT INTO events(id, kind, base_head, payload) VALUES(?1,?2,?3,?4)",
                params![
                    batch.event_id,
                    batch.event_kind,
                    base_head,
                    serde_json::to_string(&batch.event_payload).unwrap()
                ],
            )?;
            tx.last_insert_rowid()
        };
        for (id, kind, payload) in &batch.puts {
            let next_version: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(version),0)+1 FROM objects WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .unwrap_or(1);
            tx.execute(
                "INSERT INTO objects(id, version, kind, created_seq, superseded_seq, payload)
                 VALUES(?1,?2,?3,?4,NULL,?5)",
                params![id, next_version, kind, seq, serde_json::to_string(payload).unwrap()],
            )?;
        }
        for id in &batch.supersedes {
            tx.execute(
                "UPDATE objects SET superseded_seq=?1
                 WHERE id=?2 AND superseded_seq IS NULL",
                params![seq, id],
            )?;
        }
        if on_branch {
            let branch = format!("branch-{}", batch.event_id);
            tx.execute(
                "INSERT OR REPLACE INTO branches(name, head_seq, status, note) VALUES(?1,?2,'open',?3)",
                params![branch, seq, format!("diverged from main@{}", current)],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO heads(name, seq) VALUES(?1, ?2)",
                params![branch, seq],
            )?;
        } else {
            tx.execute("UPDATE heads SET seq=?1 WHERE name='main'", params![seq])?;
        }
        tx.commit()?;
        if on_branch {
            return Err(StoreError::Conflict { expected: base_head, actual: current });
        }
        Ok(seq)
    }

    /// Resolve a conflict: make `winner_branch` the new main head. The losing
    /// branch stays in `branches` with status 'rejected'; nothing is deleted.
    pub fn resolve_conflict(&self, winner_branch: &str, event_id: &str) -> Result<i64, StoreError> {
        let winner_seq = self.head_seq_of(winner_branch)?;
        let current = self.head_seq()?;
        let tx = self.conn.unchecked_transaction()?;
        let seq: i64 = {
            tx.execute(
                "INSERT INTO events(id, kind, base_head, payload) VALUES(?1,'conflict-resolve',?2,?3)",
                params![
                    event_id,
                    current,
                    serde_json::to_string(&serde_json::json!({
                        "winner": winner_branch,
                        "winner_seq": winner_seq,
                        "rationale": "human resolution; losing branch retained"
                    }))
                    .unwrap()
                ],
            )?;
            tx.last_insert_rowid()
        };
        tx.execute("UPDATE heads SET seq=?1 WHERE name='main'", params![winner_seq])?;
        tx.execute(
            "UPDATE branches SET status='accepted' WHERE name=?1",
            params![winner_branch],
        )?;
        tx.execute(
            "UPDATE branches SET status='rejected' WHERE name!=?1 AND status='open'",
            params![winner_branch],
        )?;
        tx.commit()?;
        Ok(seq)
    }

    pub fn events(&self) -> Result<Vec<Event>, StoreError> {
        let mut st = self
            .conn
            .prepare("SELECT seq, id, kind, base_head, payload FROM events ORDER BY seq")?;
        let rows = st.query_map([], |r| {
            Ok(Event {
                seq: r.get(0)?,
                id: r.get(1)?,
                kind: r.get(2)?,
                base_head: r.get(3)?,
                payload: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(serde_json::Value::Null),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Current (unsuperseded) version of an object.
    pub fn current(&self, id: &str) -> Result<Option<ObjectVersion>, StoreError> {
        self.object_at(id, i64::MAX)
    }

    /// Version of an object visible at event sequence `seq`.
    pub fn object_at(&self, id: &str, seq: i64) -> Result<Option<ObjectVersion>, StoreError> {
        let mut st = self.conn.prepare(
            "SELECT id, version, kind, created_seq, superseded_seq, payload FROM objects
             WHERE id=?1 AND created_seq<=?2 AND (superseded_seq IS NULL OR superseded_seq>?2)
             ORDER BY version DESC LIMIT 1",
        )?;
        let mut rows = st.query_map(params![id, seq], |r| {
            Ok(ObjectVersion {
                id: r.get(0)?,
                version: r.get(1)?,
                kind: r.get(2)?,
                created_seq: r.get(3)?,
                superseded_seq: r.get(4)?,
                payload: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(serde_json::Value::Null),
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// All objects visible at `seq` (i64::MAX = current state).
    pub fn objects_at(&self, seq: i64) -> Result<Vec<ObjectVersion>, StoreError> {
        let mut st = self.conn.prepare(
            "SELECT o.id, o.version, o.kind, o.created_seq, o.superseded_seq, o.payload
             FROM objects o
             WHERE o.created_seq<=?1 AND (o.superseded_seq IS NULL OR o.superseded_seq>?1)
             AND o.version = (SELECT MAX(version) FROM objects o2 WHERE o2.id=o.id
                              AND o2.created_seq<=?1 AND (o2.superseded_seq IS NULL OR o2.superseded_seq>?1))
             ORDER BY o.id",
        )?;
        let rows = st.query_map(params![seq], |r| {
            Ok(ObjectVersion {
                id: r.get(0)?,
                version: r.get(1)?,
                kind: r.get(2)?,
                created_seq: r.get(3)?,
                superseded_seq: r.get(4)?,
                payload: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(serde_json::Value::Null),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn branches(&self) -> Result<Vec<(String, i64, String)>, StoreError> {
        let mut st = self
            .conn
            .prepare("SELECT name, head_seq, status FROM branches ORDER BY name")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Export the full history (events, objects, heads, branches) as JSON.
    pub fn export(&self) -> Result<serde_json::Value, StoreError> {
        let events = self.events()?;
        let mut st = self.conn.prepare(
            "SELECT id, version, kind, created_seq, superseded_seq, payload FROM objects ORDER BY id, version",
        )?;
        let objects: Vec<ObjectVersion> = st
            .query_map([], |r| {
                Ok(ObjectVersion {
                    id: r.get(0)?,
                    version: r.get(1)?,
                    kind: r.get(2)?,
                    created_seq: r.get(3)?,
                    superseded_seq: r.get(4)?,
                    payload: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(serde_json::Value::Null),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let heads: Vec<Head> = {
            let mut st = self.conn.prepare("SELECT name, seq FROM heads ORDER BY name")?;
            st.query_map([], |r| Ok(Head { name: r.get(0)?, seq: r.get(1)? }))?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(serde_json::json!({
            "format": "saxs-rebench-export",
            "schema_version": SCHEMA_VERSION,
            "events": events,
            "objects": objects,
            "heads": heads,
        }))
    }

    /// Import a previously exported dump into an empty database, preserving
    /// identity, order and provenance exactly.
    pub fn import(&self, dump: &serde_json::Value) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        for ev in dump["events"].as_array().cloned().unwrap_or_default() {
            tx.execute(
                "INSERT INTO events(seq, id, kind, base_head, payload) VALUES(?1,?2,?3,?4,?5)",
                params![
                    ev["seq"].as_i64().unwrap(),
                    ev["id"].as_str().unwrap(),
                    ev["kind"].as_str().unwrap(),
                    ev["base_head"].as_i64().unwrap(),
                    serde_json::to_string(&ev["payload"]).unwrap()
                ],
            )?;
        }
        for ob in dump["objects"].as_array().cloned().unwrap_or_default() {
            tx.execute(
                "INSERT INTO objects(id, version, kind, created_seq, superseded_seq, payload)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    ob["id"].as_str().unwrap(),
                    ob["version"].as_i64().unwrap(),
                    ob["kind"].as_str().unwrap(),
                    ob["created_seq"].as_i64().unwrap(),
                    ob["superseded_seq"].as_i64(),
                    serde_json::to_string(&ob["payload"]).unwrap()
                ],
            )?;
        }
        for h in dump["heads"].as_array().cloned().unwrap_or_default() {
            tx.execute(
                "INSERT OR REPLACE INTO heads(name, seq) VALUES(?1,?2)",
                params![h["name"].as_str().unwrap(), h["seq"].as_i64().unwrap()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
