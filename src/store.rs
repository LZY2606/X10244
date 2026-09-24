use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct Event {
    pub seq: i64,
    pub id: String,
    pub ts: String,
    pub actor: String,
    pub kind: String,
    pub payload: Value,
    pub base_seq: Option<i64>,
}

#[derive(Debug)]
pub struct Conflict {
    pub expected: Option<i64>,
    pub head_seq: i64,
    pub head_event: String,
}

pub struct Store {
    pub conn: Mutex<Connection>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS events (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT NOT NULL,
  ts TEXT NOT NULL,
  actor TEXT NOT NULL,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL,
  base_seq INTEGER
);
CREATE TABLE IF NOT EXISTS sources (
  id TEXT PRIMARY KEY,
  slot TEXT NOT NULL,
  kind TEXT NOT NULL,
  version INTEGER NOT NULL,
  supersedes TEXT,
  payload TEXT NOT NULL,
  created_seq INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS derived (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  inputs TEXT NOT NULL,
  params TEXT NOT NULL,
  payload TEXT NOT NULL,
  diagnostics TEXT NOT NULL,
  candidates TEXT NOT NULL,
  status TEXT NOT NULL,
  created_seq INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS decisions (
  id TEXT PRIMARY KEY,
  target TEXT NOT NULL,
  action TEXT NOT NULL,
  branch_base_seq INTEGER NOT NULL,
  note TEXT NOT NULL,
  payload TEXT NOT NULL,
  conflict INTEGER NOT NULL DEFAULT 0,
  resolved_by TEXT,
  created_seq INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS pointers (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_seq INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sources_slot ON sources(slot, version);
CREATE INDEX IF NOT EXISTS idx_derived_status ON derived(status);
CREATE TABLE IF NOT EXISTS recomputes (
  seq INTEGER NOT NULL,
  old_id TEXT NOT NULL,
  new_id TEXT NOT NULL,
  reason TEXT NOT NULL
);
"#;

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let store = Store { conn: Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }

    pub(crate) fn migrate(&self) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let v: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0);
        if v == 0 {
            tx.execute_batch(SCHEMA)?;
            tx.execute_batch("PRAGMA user_version=2;")?;
        } else if v == 1 {
            // 旧版本迁移：v1 没有 candidates/status 列与 pointers/recomputes 表。
            tx.execute_batch(
                "ALTER TABLE derived ADD COLUMN status TEXT NOT NULL DEFAULT 'valid';
                 ALTER TABLE derived ADD COLUMN candidates TEXT NOT NULL DEFAULT '[]';
                 CREATE TABLE IF NOT EXISTS pointers (
                   key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_seq INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS recomputes (
                   seq INTEGER NOT NULL, old_id TEXT NOT NULL,
                   new_id TEXT NOT NULL, reason TEXT NOT NULL);
                 PRAGMA user_version=2;",
            )?;
        }
        // 已是最新：确保缺失对象补齐（幂等）。
        tx.execute_batch(SCHEMA)?;
        tx.commit()
    }

    pub fn head(&self) -> rusqlite::Result<Option<(i64, String)>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT seq, id FROM events ORDER BY seq DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
    }

    pub fn events(&self) -> rusqlite::Result<Vec<Event>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT seq,id,ts,actor,kind,payload,base_seq FROM events ORDER BY seq",
        )?;
        let rows = stmt.query_map([], row_to_event)?;
        rows.collect()
    }

    pub fn wipe(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "DELETE FROM events; DELETE FROM sources; DELETE FROM derived;
             DELETE FROM decisions; DELETE FROM pointers;
             DELETE FROM sqlite_sequence WHERE name='events';",
        )
    }
}

fn row_to_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    let payload: String = r.get(5)?;
    Ok(Event {
        seq: r.get(0)?,
        id: r.get(1)?,
        ts: r.get(2)?,
        actor: r.get(3)?,
        kind: r.get(4)?,
        payload: serde_json::from_str(&payload).unwrap_or(json!({})),
        base_seq: r.get(6)?,
    })
}

/// 在单个事务里完成“事件追加 + 物化”，保证要么看到旧版本，要么看到完整新版本。
pub fn commit_event<T>(
    store: &Store,
    actor: &str,
    kind: &str,
    payload: &Value,
    base_seq: Option<i64>,
    materialize: T,
) -> Result<Event, String>
where
    T: FnOnce(&Connection, &Event) -> Result<(), String>,
{
    let mut conn = store.conn.lock().unwrap();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let head: Option<i64> = tx
        .query_row("SELECT seq FROM events ORDER BY seq DESC LIMIT 1", [], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(expected) = base_seq {
        if head != Some(expected) {
            let head_event: String = tx
                .query_row("SELECT id FROM events ORDER BY seq DESC LIMIT 1", [], |r| r.get(0))
                .unwrap_or_default();
            return Err(serde_json::to_string(&json!({
                "error": "conflict",
                "expected": expected,
                "head_seq": head.unwrap_or(-1),
                "head_event": head_event,
            }))
            .unwrap());
        }
    }
    let seq = head.unwrap_or(0) + 1;
    let id = crate::ids::event_id(seq, kind, payload);
    let ts = format!("2026-09-24T{:06}+09:00", (seq * 37) % 1_000_000);
    tx.execute(
        "INSERT INTO events(seq,id,ts,actor,kind,payload,base_seq) VALUES(?,?,?,?,?,?,?)",
        params![seq, id, ts, actor, kind, payload.to_string(), base_seq],
    )
    .map_err(|e| e.to_string())?;
    let event = Event {
        seq,
        id: id.clone(),
        ts,
        actor: actor.to_string(),
        kind: kind.to_string(),
        payload: payload.clone(),
        base_seq,
    };
    materialize(&tx, &event).map_err(|e| {
        // 物化失败：事务回滚，调用方重启后只能看到上一完整版本。
        format!("materialize_failed:{e}")
    })?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(event)
}
