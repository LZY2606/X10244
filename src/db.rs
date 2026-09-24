//! SQLite storage: schema, migrations, crash recovery.
//!
//! Atomicity model: every derivation write happens inside one transaction
//! together with the current-pointer switch and its event-log entry, so a
//! process killed mid-write leaves either the old version or the complete
//! new version. A write journal plus `status='building'` rows are the
//! belt-and-suspenders cleaned up by `recover` on open.

use rusqlite::Connection;

pub const SCHEMA_VERSION: u32 = 2;

pub type R<T> = Result<T, String>;

pub trait Sql<T> {
    fn s(self) -> R<T>;
}
impl<T, E: std::fmt::Display> Sql<T> for Result<T, E> {
    fn s(self) -> R<T> {
        self.map_err(|e| e.to_string())
    }
}

pub const V1_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS cases(
  id INTEGER PRIMARY KEY,
  uid TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  meta_json TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS curves(
  id INTEGER PRIMARY KEY,
  uid TEXT NOT NULL UNIQUE,
  case_id INTEGER NOT NULL,
  label TEXT NOT NULL,
  kind TEXT NOT NULL,
  unit_q TEXT NOT NULL,
  unit_i TEXT NOT NULL,
  q_json TEXT NOT NULL,
  i_json TEXT NOT NULL,
  err_json TEXT NOT NULL,
  meta_json TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS derivations(
  id INTEGER PRIMARY KEY,
  uid TEXT NOT NULL UNIQUE,
  case_id INTEGER NOT NULL,
  kind TEXT NOT NULL,
  params_json TEXT NOT NULL,
  inputs_json TEXT NOT NULL,
  result_json TEXT,
  diagnostics_json TEXT NOT NULL DEFAULT '[]',
  status TEXT NOT NULL,
  base_pointer INTEGER,
  seq INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS pointers(
  case_id INTEGER NOT NULL,
  name TEXT NOT NULL,
  value INTEGER NOT NULL,
  PRIMARY KEY(case_id, name)
);
CREATE TABLE IF NOT EXISTS events(
  id INTEGER PRIMARY KEY,
  case_id INTEGER NOT NULL,
  seq INTEGER NOT NULL,
  ts INTEGER NOT NULL,
  actor TEXT NOT NULL,
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  pointers_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS interval_candidates(
  id INTEGER PRIMARY KEY,
  uid TEXT NOT NULL UNIQUE,
  derivation_id INTEGER NOT NULL,
  case_id INTEGER NOT NULL,
  kind TEXT NOT NULL,
  label TEXT NOT NULL,
  q_min REAL NOT NULL,
  q_max REAL NOT NULL,
  fit_json TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  created_seq INTEGER NOT NULL
);
"#;

pub const V2_MIGRATION: &str = r#"
ALTER TABLE derivations ADD COLUMN branch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE interval_candidates ADD COLUMN decided_by TEXT;
ALTER TABLE interval_candidates ADD COLUMN decided_seq INTEGER;
CREATE TABLE IF NOT EXISTS write_journal(
  id INTEGER PRIMARY KEY,
  tag TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  finished INTEGER NOT NULL DEFAULT 0
);
"#;

pub fn migrate(conn: &Connection) -> R<()> {
    let v: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .s()?;
    if v < 1 {
        conn.execute_batch(V1_SCHEMA).s()?;
        conn.execute_batch("PRAGMA user_version = 1").s()?;
    }
    if v < 2 {
        conn.execute_batch(V2_MIGRATION).s()?;
        conn.execute_batch("PRAGMA user_version = 2").s()?;
    }
    Ok(())
}

/// Remove half-written state left by an interrupted writer.
pub fn recover(conn: &Connection) -> R<usize> {
    let d = conn
        .execute("DELETE FROM derivations WHERE status='building'", [])
        .s()?;
    let j = conn
        .execute("DELETE FROM write_journal WHERE finished=0", [])
        .s()?;
    Ok(d + j)
}

pub fn open(path: &str) -> R<Connection> {
    let conn = Connection::open(path).s()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")
        .s()?;
    migrate(&conn)?;
    recover(&conn)?;
    Ok(conn)
}

pub fn open_memory() -> R<Connection> {
    let conn = Connection::open_in_memory().s()?;
    migrate(&conn)?;
    recover(&conn)?;
    Ok(conn)
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

static UID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Stable identity that survives export / delete / re-import.
pub fn new_uid(prefix: &str) -> String {
    let n = UID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{}-{:016x}{:016x}", prefix, nanos, n.wrapping_mul(0x9e3779b97f4a7c15))
}
