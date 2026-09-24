//! SQLite storage for the SAXS workbench.
//!
//! Durability model:
//! * Every append (one atomic unit of work) inserts its event(s) and derived
//!   objects in a single transaction. The current branch pointer moves inside
//!   the same transaction, so a crash leaves either the old version or the
//!   complete new one — never a half-written derivation.
//! * `append` is optimistic on `base_seq`. Two editors starting from the same
//!   seq cannot both commit: the loser gets [`StoreError::Conflict`] and must
//!   rebase/resolve.

use crate::domain::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Mutex;

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Conflict { expected: i64, actual: i64 },
    NotFound(String),
    Corrupt(String),
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Sql(e) => write!(f, "sqlite: {e}"),
            StoreError::Conflict { expected, actual } => write!(
                f,
                "optimistic conflict: base seq {expected} is stale; current seq is {actual}"
            ),
            StoreError::NotFound(s) => write!(f, "not found: {s}"),
            StoreError::Corrupt(s) => write!(f, "corrupt state: {s}"),
        }
    }
}
impl std::error::Error for StoreError {}
impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}

/// One atomic unit of work to append.
pub struct Commit {
    pub events: Vec<Event>,
    pub raws: Vec<RawRecord>,
    pub norms: Vec<Normalized>,
    pub subs: Vec<Subtracted>,
    pub fits: Vec<Fit>,
}

/// Result of an append.
pub struct Committed {
    pub seq: i64,
    pub branch_tip: i64,
}

pub struct Store {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS events (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT NOT NULL UNIQUE,
  dataset_id TEXT NOT NULL,
  branch TEXT NOT NULL DEFAULT 'main',
  event_type TEXT NOT NULL,
  base_seq INTEGER NOT NULL DEFAULT 0,
  author TEXT NOT NULL,
  timestamp TEXT NOT NULL,
  object_id TEXT NOT NULL DEFAULT '',
  payload TEXT NOT NULL,
  judgments TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX IF NOT EXISTS events_branch ON events(branch, seq);

CREATE TABLE IF NOT EXISTS raw_records (
  id TEXT PRIMARY KEY,
  logical_key TEXT NOT NULL,
  version INTEGER NOT NULL,
  json TEXT NOT NULL,
  created_seq INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS normalized (
  id TEXT PRIMARY KEY,
  record_id TEXT NOT NULL,
  json TEXT NOT NULL,
  created_seq INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS subtracted (
  id TEXT PRIMARY KEY,
  sample_record_id TEXT NOT NULL,
  buffer_record_id TEXT NOT NULL,
  json TEXT NOT NULL,
  created_seq INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS fits (
  id TEXT PRIMARY KEY,
  subtraction_id TEXT NOT NULL,
  json TEXT NOT NULL,
  created_seq INTEGER NOT NULL
);
-- Current pointer per branch. Moves inside the same tx as the new content.
CREATE TABLE IF NOT EXISTS branch_tip (
  dataset_id TEXT NOT NULL,
  branch TEXT NOT NULL,
  tip_seq INTEGER NOT NULL,
  PRIMARY KEY(dataset_id, branch)
);
-- Latest version per logical raw key (used by revisions and buffer switching).
CREATE TABLE IF NOT EXISTS raw_head (
  dataset_id TEXT NOT NULL,
  logical_key TEXT NOT NULL,
  record_id TEXT NOT NULL,
  version INTEGER NOT NULL,
  PRIMARY KEY(dataset_id, logical_key)
);
"#;

impl Store {
    pub fn open(path: &str) -> Result<Store, StoreError> {
        let conn = Connection::open(path)?;
        init(&conn)?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    /// In-memory store (tests / ephemeral sessions).
    pub fn memory() -> Result<Store, StoreError> {
        let conn = Connection::open_in_memory()?;
        init(&conn)?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    pub fn current_seq(&self, dataset_id: &str) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(tip_seq(&conn, dataset_id, "main")?)
    }

    /// Append a commit atomically. Optimistic on the *first* event's base_seq:
    /// it must equal the current main tip, otherwise Conflict.
    pub fn append(&self, commit: Commit) -> Result<Committed, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        append_tx(&tx, commit, false)?;
        let committed = finish(&tx)?;
        tx.commit()?;
        Ok(committed)
    }

    /// Test-only: write all content but deliberately leave the transaction
    /// (and thus the branch pointer move) uncommitted, then roll back. Used to
    /// prove recovery never observes a half-written version.
    pub fn append_simulated_crash(&self, commit: Commit) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        append_tx(&tx, commit, true)?;
        // Explicit rollback simulates a process killed mid-write.
        tx.rollback()?;
        Ok(())
    }

    pub fn all_events(&self, dataset_id: &str) -> Result<Vec<Event>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT json FROM events WHERE dataset_id=?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![dataset_id], |r| {
            let s: String = r.get(0)?;
            Ok(s)
        })?;
        let mut out = Vec::new();
        for s in rows {
            out.push(serde_json::from_str::<Event>(&s?)?);
        }
        Ok(out)
    }

    pub fn raw_records(&self) -> Result<Vec<RawRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_all::<RawRecord>(&conn, "raw_records")
    }
    pub fn normalized(&self) -> Result<Vec<Normalized>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_all::<Normalized>(&conn, "normalized")
    }
    pub fn subtracted(&self) -> Result<Vec<Subtracted>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_all::<Subtracted>(&conn, "subtracted")
    }
    pub fn fits(&self) -> Result<Vec<Fit>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_all::<Fit>(&conn, "fits")
    }

    pub fn raw_head(&self, dataset_id: &str, logical_key: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT record_id FROM raw_head WHERE dataset_id=?1 AND logical_key=?2",
                params![dataset_id, logical_key],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    pub fn replace_all(&self, bundle: &DatasetBundle) -> Result<i64, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute_batch(
            "DELETE FROM events; DELETE FROM raw_records; DELETE FROM normalized;
             DELETE FROM subtracted; DELETE FROM fits; DELETE FROM branch_tip; DELETE FROM raw_head;",
        )?;
        // Directly insert without optimistic checks (import trusts the bundle).
        for e in &bundle.events {
            insert_event(&tx, e)?;
        }
        for r in &bundle.raw_records {
            upsert_raw(&tx, r, 0, &bundle.dataset_id)?;
        }
        for n in &bundle.normalized {
            put(&tx, "normalized", &n.id, &n.record_id, "", n, 0)?;
        }
        for s in &bundle.subtracted {
            put(
                &tx,
                "subtracted",
                &s.id,
                &s.sample_record_id,
                &s.buffer_record_id,
                s,
                0,
            )?;
        }
        for f in &bundle.fits {
            put(&tx, "fits", &f.id, &f.subtraction_id, "", f, 0)?;
        }
        if let Some(last) = bundle.events.last() {
            tx.execute(
                "INSERT OR REPLACE INTO branch_tip(dataset_id,branch,tip_seq) VALUES(?1,'main',?2)",
                params![bundle.dataset_id, last.seq],
            )?;
        }
        tx.commit()?;
        Ok(bundle.events.last().map(|e| e.seq).unwrap_or(0))
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row("SELECT value FROM meta WHERE key=?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .optional()?)
    }
    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)",
            params![key, value],
        )?;
        Ok(())
    }
}
