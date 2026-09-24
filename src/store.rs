//! SQLite storage: an append-only, content-addressed ledger.
//!
//! Every user action commits exactly one transaction (objects + event +
//! chain head + revision bump). A crash leaves either the old complete
//! version or the new complete version, never a half-written one.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::model::{Event, Obj};

pub const SCHEMA_VERSION: i64 = 2;

pub struct Store {
    pub conn: Connection,
}

#[derive(Debug)]
pub enum StoreError {
    Conflict {
        current_head: String,
        supplied_base: String,
        message: String,
    },
    Sql(rusqlite::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Conflict { message, .. } => write!(f, "{message}"),
            StoreError::Sql(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for StoreError {}
impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 10_000)?;
        let mut s = Store { conn };
        s.migrate()?;
        Ok(s)
    }

    /// In-memory store (tests, import staging).
    pub fn memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut s = Store { conn };
        s.migrate()?;
        Ok(s)
    }

    fn migrate(&mut self) -> rusqlite::Result<()> {
        let v: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS objects (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                json TEXT NOT NULL,
                ord INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                parent TEXT,
                chain TEXT NOT NULL,
                seq INTEGER NOT NULL,
                kind TEXT NOT NULL,
                detail TEXT NOT NULL,
                judgment TEXT NOT NULL,
                object_id TEXT,
                actor TEXT NOT NULL,
                ts_ms INTEGER NOT NULL,
                ord INTEGER NOT NULL DEFAULT 0,
                UNIQUE(chain, seq)
            );
            CREATE INDEX IF NOT EXISTS idx_events_chain ON events(chain, seq);
            CREATE TABLE IF NOT EXISTS chain_heads (
                chain TEXT PRIMARY KEY,
                head_event TEXT NOT NULL,
                fork_point TEXT
            );
            CREATE TABLE IF NOT EXISTS reviews (
                chain TEXT NOT NULL,
                revision INTEGER NOT NULL,
                PRIMARY KEY (chain)
            );
            "#,
        )?;
        if v < 2 {
            // v1 -> v2 migration: per-chain optimistic revision counters.
            self.conn.execute_batch(
                r#"
                INSERT OR IGNORE INTO reviews(chain, revision)
                SELECT chain, COALESCE(MAX(seq), 0) FROM events GROUP BY chain;
                "#,
            )?;
            self.conn.pragma_update(None, "user_version", 2)?;
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    pub fn schema_version(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
    }

    /// Current head of a chain (None for a brand new chain).
    pub fn chain_head(&self, chain: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT head_event FROM chain_heads WHERE chain = ?1",
                params![chain],
                |r| r.get::<_, String>(0),
            )
            .optional()
    }

    pub fn revision(&self, chain: &str) -> rusqlite::Result<i64> {
        Ok(self
            .conn
            .query_row(
                "SELECT revision FROM reviews WHERE chain = ?1",
                params![chain],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    pub fn next_seq(&self, chain: &str) -> rusqlite::Result<i64> {
        Ok(self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE chain = ?1",
                params![chain],
                |r| r.get(0),
            )?)
    }
}

/// Inputs of one atomic ledger append.
pub struct Append<'a> {
    pub chain: &'a str,
    pub kind: &'a str,
    pub detail: Value,
    pub judgment: Value,
    pub actor: &'a str,
    pub object: Option<Obj>,
    /// review base revision the editor started from (optimistic lock)
    pub base_revision: Option<i64>,
    /// when true, a stale base creates a resolvable fork instead of erroring
    pub allow_fork: bool,
    pub chain_label: Option<&'a str>,
}

impl Store {
    /// Append one action atomically. Returns the committed event.
    /// On stale base: either a Conflict error or a new forked chain carrying
    /// a `conflict` event for human resolution.
    pub fn append(&mut self, a: Append<'_>, event_id: &str, ts_ms: i64) -> Result<Event, StoreError> {
        let tx = self.conn.transaction()?;
        let cur_rev: i64 = tx
            .query_row(
                "SELECT revision FROM reviews WHERE chain = ?1",
                params![a.chain],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);

        let (chain, parent, fork_point) = if let Some(base) = a.base_revision {
            if base != cur_rev {
                if !a.allow_fork {
                    let head: String = tx
                        .query_row(
                            "SELECT head_event FROM chain_heads WHERE chain = ?1",
                            params![a.chain],
                            |r| r.get(0),
                        )
                        .optional()?
                        .unwrap_or_default();
                    return Err(StoreError::Conflict {
                        current_head: head,
                        supplied_base: base.to_string(),
                        message: format!(
                            "review base revision {} is stale (current {}) on chain {}",
                            base, cur_rev, a.chain
                        ),
                    });
                }
                // Fork a new chain at the current head; the conflict event
                // records both bases so the reviewer can resolve explicitly.
                let head: Option<String> = tx
                    .query_row(
                        "SELECT head_event FROM chain_heads WHERE chain = ?1",
                        params![a.chain],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?;
                let fork_chain = format!("{}-fork-{}", a.chain, &event_id[..8]);
                if let Some(h) = &head {
                    tx.execute(
                        "INSERT INTO chain_heads(chain, head_event, fork_point) VALUES(?1, ?2, ?2)",
                        params![fork_chain, h],
                    )?;
                    let maxseq: i64 = tx
                        .query_row(
                            "SELECT COALESCE(MAX(seq),0) FROM events WHERE chain=?1",
                            params![a.chain],
                            |r| r.get(0),
                        )?;
                    tx.execute(
                        "INSERT OR REPLACE INTO reviews(chain, revision) VALUES(?1, ?2)",
                        params![fork_chain, maxseq],
                    )?;
                }
                (fork_chain, head.clone(), head.clone())
            } else {
                (a.chain.to_string(), head_of_tx(&tx, a.chain)?, None)
            }
        } else {
            (a.chain.to_string(), head_of_tx(&tx, a.chain)?, None)
        };

        let seq: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE chain = ?1",
                params![chain],
                |r| r.get(0),
            )?;

        let kind_code = a.kind;

        let object_id = if let Some(obj) = &a.object {
            let id = obj.object_id_ref().to_string();
            let kind = kind_of(obj);
            let ord = max_ord(&tx)? + 1;
            tx.execute(
                "INSERT OR IGNORE INTO objects(id, kind, json, ord) VALUES(?1, ?2, ?3, ?4)",
                params![id, kind, serde_json::to_string(obj).unwrap(), ord],
            )?;
            Some(id)
        } else {
            None
        };

        let event = Event {
            id: event_id.to_string(),
            parent: parent.clone(),
            chain: chain.clone(),
            seq,
            kind: serde_json::from_value(Value::String(kind_code.to_string())).unwrap(),
            detail: a.detail.clone(),
            judgment: serde_json::from_value(a.judgment.clone()).unwrap(),
            object_id,
            actor: a.actor.to_string(),
            ts_ms,
        };

        let eord = max_event_ord(&tx)? + 1;
        tx.execute(
            "INSERT INTO events(id, parent, chain, seq, kind, detail, judgment,
                object_id, actor, ts_ms, ord)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                event.id,
                event.parent,
                event.chain,
                event.seq,
                kind_code,
                serde_json::to_string(&event.detail).unwrap(),
                serde_json::to_string(&event.judgment).unwrap(),
                event.object_id,
                event.actor,
                event.ts_ms,
                eord
            ],
        )?;
        tx.execute(
            "INSERT INTO chain_heads(chain, head_event, fork_point)
             VALUES(?1, ?2, ?3)
             ON CONFLICT(chain) DO UPDATE SET head_event = excluded.head_event",
            params![chain, event.id, fork_point],
        )?;
        tx.execute(
            "INSERT INTO reviews(chain, revision) VALUES(?1, ?2)
             ON CONFLICT(chain) DO UPDATE SET revision = revision + 1",
            params![chain, seq],
        )?;
        tx.commit()?;
        Ok(event)
    }

    pub fn all_objects(&self) -> rusqlite::Result<Vec<Obj>> {
        let mut stmt = self
            .conn
            .prepare("SELECT json FROM objects ORDER BY ord ASC, id ASC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for j in rows {
            out.push(serde_json::from_str(&j?).unwrap());
        }
        Ok(out)
    }

    pub fn object(&self, id: &str) -> rusqlite::Result<Option<Obj>> {
        self.conn
            .query_row("SELECT json FROM objects WHERE id=?1", params![id], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(|j| serde_json::from_str(&j).unwrap())
            .map_or(Ok(None), |o| Ok(Some(o)))
    }

    pub fn all_events(&self) -> rusqlite::Result<Vec<Event>> {
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM events ORDER BY ord ASC")?;
        let rows = stmt.query_map([], row_to_event)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn event(&self, id: &str) -> rusqlite::Result<Option<Event>> {
        self.conn
            .query_row("SELECT * FROM events WHERE id=?1", params![id], row_to_event)
            .optional()
    }

    pub fn chains(&self) -> rusqlite::Result<Vec<(String, String, Option<String>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT chain, head_event, fork_point FROM chain_heads ORDER BY chain")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Export archive preserving global order and all stable identities.
    pub fn export_archive(&self) -> rusqlite::Result<Value> {
        let objs = self.all_objects()?;
        let events = self.all_events()?;
        Ok(serde_json::json!({
            "archive": "saxs-station",
            "schema_version": SCHEMA_VERSION,
            "objects": objs,
            "events": events,
        }))
    }

    /// Import an archive into this (empty or staging) store, preserving
    /// ids, order and parent/object relations.
    pub fn import_archive(&mut self, archive: &Value) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        let objs = archive["objects"].as_array().cloned().unwrap_or_default();
        for (i, o) in objs.iter().enumerate() {
            let obj: Obj = serde_json::from_value(o.clone()).unwrap();
            tx.execute(
                "INSERT OR REPLACE INTO objects(id, kind, json, ord) VALUES(?1,?2,?3,?4)",
                params![obj.object_id_ref(), kind_of(&obj), o.to_string(), i as i64],
            )?;
        }
        let events = archive["events"].as_array().cloned().unwrap_or_default();
        let mut per_chain: BTreeMap<String, i64> = BTreeMap::new();
        for (i, ev) in events.iter().enumerate() {
            let e: Event = serde_json::from_value(ev.clone()).unwrap();
            tx.execute(
                "INSERT OR REPLACE INTO events(id, parent, chain, seq, kind, detail,
                    judgment, object_id, actor, ts_ms, ord)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    e.id, e.parent, e.chain, e.seq,
                    serde_json::to_value(e.kind).unwrap().as_str().unwrap().to_string(),
                    serde_json::to_string(&e.detail).unwrap(),
                    serde_json::to_string(&e.judgment).unwrap(),
                    e.object_id, e.actor, e.ts_ms, i as i64
                ],
            )?;
            *per_chain.entry(e.chain.clone()).or_default() = e.seq;
            tx.execute(
                "INSERT INTO chain_heads(chain, head_event) VALUES(?1, ?2)
                 ON CONFLICT(chain) DO UPDATE SET head_event=excluded.head_event",
                params![e.chain, e.id],
            )?;
        }
        for (chain, seq) in per_chain {
            tx.execute(
                "INSERT OR REPLACE INTO reviews(chain, revision) VALUES(?1, ?2)",
                params![chain, seq],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn head_of_tx(
    tx: &rusqlite::Transaction<'_>,
    chain: &str,
) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT head_event FROM chain_heads WHERE chain = ?1",
        params![chain],
        |r| r.get::<_, String>(0),
    )
    .optional()
}

fn kind_of(o: &Obj) -> &'static str {
    match o {
        Obj::Raw { .. } => "raw",
        Obj::Norm { .. } => "norm",
        Obj::Sub { .. } => "sub",
        Obj::Candidates { .. } => "candidates",
    }
}

fn max_ord(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<i64> {
    tx.query_row("SELECT COALESCE(MAX(ord),0) FROM objects", [], |r| r.get(0))
}
fn max_event_ord(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<i64> {
    tx.query_row("SELECT COALESCE(MAX(ord),0) FROM events", [], |r| r.get(0))
}

fn row_to_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    Ok(Event {
        id: r.get("id")?,
        parent: r.get("parent")?,
        chain: r.get("chain")?,
        seq: r.get("seq")?,
        kind: serde_json::from_value(Value::String(r.get::<_, String>("kind")?)).unwrap(),
        detail: serde_json::from_str(&r.get::<_, String>("detail")?).unwrap(),
        judgment: serde_json::from_str(&r.get::<_, String>("judgment")?).unwrap(),
        object_id: r.get("object_id")?,
        actor: r.get("actor")?,
        ts_ms: r.get("ts_ms")?,
    })
}
