//! SQLite event store with materialized read tables.
//!
//! Durability contract: a new derivation is only observable after its event
//! commits inside one transaction; an interrupted write leaves the previous
//! version intact (the partial transaction is simply rolled back / discarded).

use crate::model::{Derivation, Event, InstrumentResponse, Params, RawCurve};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Mutex;

pub const SCHEMA_VERSION: i64 = 3;

pub struct Store {
    pub conn: Mutex<Connection>,
}

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Conflict { expected_seq: i64, actual_seq: i64 },
    Other(String),
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for StoreError {}
impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}
impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Other(e.to_string())
    }
}

/// A transaction that can be left deliberately uncommitted so tests can prove
/// interrupted writes never expose a partial new version.
pub struct PendingTx<'a> {
    guard: Option<std::sync::MutexGuard<'a, Connection>>,
    tx: Option<rusqlite::Transaction<'a>>,
    pub seq: i64,
}

impl Store {
    pub fn open(path: &str) -> Result<Self, StoreError> {
        let conn = if path == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(path)?
        };
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        let store = Store { conn: Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let current: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);
        // Legacy v1/v2 tables are preserved; each migration only adds what is
        // needed, proving old databases still open and replay correctly.
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS events (
                seq INTEGER PRIMARY KEY,
                event_id TEXT NOT NULL UNIQUE,
                event_type TEXT NOT NULL,
                at TEXT NOT NULL,
                actor TEXT NOT NULL,
                client TEXT NOT NULL,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL,
                body TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS curves (
                id TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                supersedes TEXT,
                json TEXT NOT NULL,
                created_seq INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS raw_points (
                id TEXT PRIMARY KEY,
                curve_id TEXT NOT NULL,
                idx INTEGER NOT NULL,
                q_raw TEXT NOT NULL, i_raw TEXT NOT NULL, sigma_raw TEXT NOT NULL,
                q_value REAL, i_value REAL, sigma_value REAL,
                q_unit TEXT, i_unit TEXT,
                masked INTEGER NOT NULL, mask_reasons TEXT NOT NULL,
                FOREIGN KEY(curve_id) REFERENCES curves(id)
            );
            CREATE TABLE IF NOT EXISTS responses (id TEXT PRIMARY KEY, json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS params_versions (
                version TEXT PRIMARY KEY, json TEXT NOT NULL, created_seq INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS derivations (
                id TEXT NOT NULL,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                json TEXT NOT NULL,
                created_seq INTEGER NOT NULL,
                PRIMARY KEY (id, created_seq)
            );
            CREATE TABLE IF NOT EXISTS decisions (
                seq INTEGER PRIMARY KEY,
                branch TEXT NOT NULL,
                action TEXT NOT NULL,
                target TEXT,
                json TEXT NOT NULL
            );
            "#,
        )?;
        if current < 2 {
            conn.execute_batch(
                "ALTER TABLE curves ADD COLUMN IF NOT EXISTS payload_key TEXT;
                 CREATE TABLE IF NOT EXISTS exports_log (seq INTEGER PRIMARY KEY, at TEXT, kind TEXT);",
            )?;
        }
        if current < 3 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS casepack_runs (
                    run_id TEXT PRIMARY KEY, finished_seq INTEGER NOT NULL, at TEXT NOT NULL
                 );",
            )?;
        }
        conn.execute(
            "INSERT INTO meta(key,value) VALUES('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![SCHEMA_VERSION.to_string()],
        )?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    pub fn last_seq(&self) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0))?)
    }

    /// Begin an append. Callers finish with `commit_pending` or drop without
    /// committing to simulate an interrupted write.
    pub fn begin_append(
        &self,
        expected_seq: Option<i64>,
    ) -> Result<(PendingTx<'_>, i64), StoreError> {
        let mut guard = self.conn.lock().unwrap();
        let actual: i64 = guard.query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0))?;
        if let Some(exp) = expected_seq {
            if exp != actual {
                return Err(StoreError::Conflict { expected_seq: exp, actual_seq: actual });
            }
        }
        let tx = guard.transaction()?;
        let seq = actual + 1;
        Ok((PendingTx { guard: Some(guard), tx: Some(tx), seq }, seq))
    }

    pub fn commit_append(
        &self,
        pending: &mut PendingTx,
        event: &Event,
        materialize: Materialization,
    ) -> Result<(), StoreError> {
        let tx = pending.tx.as_ref().unwrap();
        tx.execute(
            "INSERT INTO events(seq,event_id,event_type,at,actor,client,prev_hash,hash,body)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                event.seq,
                event.event_id,
                event.event_type,
                event.at,
                event.actor,
                event.client,
                event.prev_hash,
                event.hash,
                serde_json::to_string(&event.body)?
            ],
        )?;
        materialize_apply(tx, materialize)?;
        pending.tx.take().unwrap().commit()?;
        pending.guard.take();
        Ok(())
    }

    /// Drop a pending transaction without committing (interrupted write).
    pub fn abort_append(&self, pending: PendingTx) {
        drop(pending);
    }

    pub fn events(&self, up_to_seq: Option<i64>) -> Result<Vec<Event>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from("SELECT seq,event_id,event_type,at,actor,client,prev_hash,hash,body FROM events");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![];
        if let Some(s) = up_to_seq {
            sql.push_str(" WHERE seq <= ?");
            args.push(Box::new(s));
        }
        sql.push_str(" ORDER BY seq");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |r| {
            let body: String = r.get(8)?;
            Ok(Event {
                seq: r.get(0)?,
                event_id: r.get(1)?,
                event_type: r.get(2)?,
                at: r.get(3)?,
                actor: r.get(4)?,
                client: r.get(5)?,
                prev_hash: r.get(6)?,
                hash: r.get(7)?,
                body: serde_json::from_str(&body).unwrap_or(serde_json::Value::Null),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
};

pub enum Materialization {
    None,
    Curve(RawCurve),
    Response(InstrumentResponse),
    Params(Params, i64),
    Derivation(Derivation),
    Decision { branch: String, action: String, target: Option<String>, seq: i64 },
    CasepackRun { run_id: String, finished_seq: i64, at: String },
}

fn materialize_apply(tx: &rusqlite::Transaction, m: Materialization) -> Result<(), StoreError> {
    match m {
        Materialization::None => {}
        Materialization::Curve(c) => {
            tx.execute(
                "INSERT OR IGNORE INTO curves(id,role,supersedes,json,created_seq,payload_key)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    c.id,
                    c.role.as_str(),
                    c.supersedes,
                    serde_json::to_string(&c)?,
                    0,
                    c.payload_key
                ],
            )?;
            for p in &c.points {
                tx.execute(
                    "INSERT OR IGNORE INTO raw_points(id,curve_id,idx,q_raw,i_raw,sigma_raw,
                       q_value,i_value,sigma_value,q_unit,i_unit,masked,mask_reasons)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                    params![
                        crate::ids::raw_point_id(&c.id, p.idx, &p.q_raw, &p.i_raw),
                        c.id,
                        p.idx as i64,
                        p.q_raw,
                        p.i_raw,
                        p.sigma_raw,
                        p.q.value,
                        p.i.value,
                        p.sigma.value,
                        c.q_unit,
                        c.i_unit,
                        p.masked as i64,
                        p.mask_reasons.join("; ")
                    ],
                )?;
            }
        }
        Materialization::Response(r) => {
            tx.execute(
                "INSERT OR IGNORE INTO responses(id,json) VALUES(?1,?2)",
                params![r.id, serde_json::to_string(&r)?],
            )?;
        }
        Materialization::Params(p, seq) => {
            tx.execute(
                "INSERT OR REPLACE INTO params_versions(version,json,created_seq) VALUES(?1,?2,?3)",
                params![p.version, serde_json::to_string(&p)?, seq],
            )?;
        }
        Materialization::Derivation(d) => {
            tx.execute(
                "INSERT OR IGNORE INTO derivations(id,kind,status,json,created_seq)
                 VALUES(?1,?2,?3,?4,?5)",
                params![d.id, d.kind, d.status, serde_json::to_string(&d)?, d.created_event_seq],
            )?;
        }
        Materialization::Decision { branch, action, target, seq } => {
            tx.execute(
                "INSERT INTO decisions(seq,branch,action,target,json) VALUES(?1,?2,?3,?4,'{}')",
                params![seq, branch, action, target],
            )?;
        }
        Materialization::CasepackRun { run_id, finished_seq, at } => {
            tx.execute(
                "INSERT OR REPLACE INTO casepack_runs(run_id,finished_seq,at) VALUES(?1,?2,?3)",
                params![run_id, finished_seq, at],
            )?;
        }
    }
    Ok(())
}

impl Store {
    pub fn curve(&self, id: &str) -> Result<Option<RawCurve>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let s: Option<String> = conn.query_row(
            "SELECT json FROM curves WHERE id=?1",
            params![id],
            |r| r.get(0),
        ).optional()?;
        Ok(s.map(|j| serde_json::from_str(&j).unwrap()))
    }

    pub fn curves(&self) -> Result<Vec<RawCurve>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT json FROM curves ORDER BY created_seq, id")?;
        let rows = stmt.query_map([], |r| {
            let j: String = r.get(0)?;
            Ok(serde_json::from_str::<RawCurve>(&j).unwrap())
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn response(&self, id: &str) -> Result<Option<InstrumentResponse>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let s: Option<String> = conn.query_row(
            "SELECT json FROM responses WHERE id=?1",
            params![id],
            |r| r.get(0),
        ).optional()?;
        Ok(s.map(|j| serde_json::from_str(&j).unwrap()))
    }

    pub fn responses(&self) -> Result<Vec<InstrumentResponse>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT json FROM responses ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            let j: String = r.get(0)?;
            Ok(serde_json::from_str::<InstrumentResponse>(&j).unwrap())
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Full export bundle: identity is fully reconstructible from `events`.
    pub fn export(&self) -> Result<serde_json::Value, StoreError> {
        let events = self.events(None)?;
        Ok(serde_json::json!({
            "bundle": "saxs-station-archive",
            "schema_version": SCHEMA_VERSION,
            "rule_version": crate::RULE_VERSION,
            "events": events,
        }))
    }

    /// Import an archive. When `replace` is true the local database is emptied
    /// first; ids/order/provenance are reproduced deterministically.
    pub fn import(&self, bundle: serde_json::Value, replace: bool) -> Result<usize, StoreError> {
        let events: Vec<Event> = serde_json::from_value(
            bundle.get("events").cloned().ok_or_else(|| StoreError::Other("missing events".into()))?,
        )?;
        {
            let conn = self.conn.lock().unwrap();
            let mut tx = conn.transaction()?;
            if replace {
                for t in &["decisions", "derivations", "params_versions", "responses",
                           "raw_points", "curves", "casepack_runs", "exports_log", "events", "meta"] {
                    tx.execute(&format!("DELETE FROM {t}"), [])?;
                }
            }
            for e in &events {
                let exists: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM events WHERE seq=?1",
                    params![e.seq],
                    |r| r.get(0),
                )?;
                if exists > 0 {
                    continue;
                }
                tx.execute(
                    "INSERT INTO events(seq,event_id,event_type,at,actor,client,prev_hash,hash,body)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        e.seq, e.event_id, e.event_type, e.at, e.actor, e.client,
                        e.prev_hash, e.hash, serde_json::to_string(&e.body)?
                    ],
                )?;
                rematerialize_event(&tx, e)?;
            }
            tx.commit()?;
        }
        Ok(events.len())
    }
}

fn rematerialize_event(tx: &rusqlite::Transaction, e: &Event) -> Result<(), StoreError> {
    match e.event_type.as_str() {
        "curve_registered" => {
            if let Ok(c) = serde_json::from_value::<RawCurve>(e.body["curve"].clone()) {
                materialize_apply(tx, Materialization::Curve(c))?;
            }
        }
        "response_registered" => {
            if let Ok(r) = serde_json::from_value::<InstrumentResponse>(e.body["response"].clone()) {
                materialize_apply(tx, Materialization::Response(r))?;
            }
        }
        "params_revised" => {
            if let Ok(p) = serde_json::from_value::<Params>(e.body["params"].clone()) {
                materialize_apply(tx, Materialization::Params(p, e.seq))?;
            }
        }
        "derivation_recorded" => {
            if let Ok(d) = serde_json::from_value::<Derivation>(e.body["derivation"].clone()) {
                materialize_apply(tx, Materialization::Derivation(d))?;
            }
        }
        "decision_recorded" => {
            materialize_apply(
                tx,
                Materialization::Decision {
                    branch: e.body["branch"].as_str().unwrap_or("").to_string(),
                    action: e.body["action"].as_str().unwrap_or("").to_string(),
                    target: e.body["target"].as_str().map(String::from),
                    seq: e.seq,
                },
            )?;
        }
        "casepack_replayed" => {
            materialize_apply(
                tx,
                Materialization::CasepackRun {
                    run_id: e.body["run_id"].as_str().unwrap_or("").to_string(),
                    finished_seq: e.seq,
                    at: e.at.clone(),
                },
            )?;
        }
        _ => {}
    }
    Ok(())
}
