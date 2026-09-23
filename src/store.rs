//! SQLite storage: immutable node versions, append-only event log, atomic
//! current-pointer switching, optimistic concurrency, schema migration,
//! export/import. Every derivation produces an independent version; the
//! current pointer flips only after the full write succeeds (one
//! transaction), so an interrupted process sees either the old version or a
//! complete new one.

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::domain::SCHEMA_VERSION;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dep {
    pub id: String,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub version: i64,
    pub kind: String,
    pub status: String, // current | ok | unchosen | superseded (advisory)
    pub payload: serde_json::Value,
    pub deps: Vec<Dep>,
    pub input_hash: String,
    pub created_event: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub id: String,
    pub ts: String,
    pub kind: String,
    pub actor: String,
    pub payload: serde_json::Value,
    pub current_snapshot: BTreeMap<String, i64>,
}

#[derive(Debug)]
pub enum SaveError {
    Conflict { expected_seq: i64, actual_seq: i64 },
    Other(anyhow::Error),
}

pub struct Store {
    pub conn: Connection,
}

fn hash_payload(s: &str) -> String {
    // FNV-1a 64-bit, stable across processes (no RandomState).
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

pub fn payload_hash(payload: &serde_json::Value, deps: &[Dep]) -> String {
    let mut s = serde_json::to_string(payload).unwrap_or_default();
    for d in deps {
        s.push_str(&format!("|{}@{}", d.id, d.version));
    }
    hash_payload(&s)
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let st = Store { conn };
        st.migrate()?;
        Ok(st)
    }

    pub fn open_memory() -> Result<Store> {
        let conn = Connection::open_in_memory()?;
        let st = Store { conn };
        st.migrate()?;
        Ok(st)
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |r| r.get::<_, String>(),
            )
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if version < 1 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS nodes(
                   id TEXT NOT NULL, version INTEGER NOT NULL, kind TEXT NOT NULL,
                   status TEXT NOT NULL, payload TEXT NOT NULL, deps TEXT NOT NULL,
                   created_event TEXT NOT NULL,
                   PRIMARY KEY(id, version));
                 CREATE TABLE IF NOT EXISTS current(node_id TEXT PRIMARY KEY, version INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS events(
                   seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL, ts TEXT NOT NULL,
                   kind TEXT NOT NULL, payload TEXT NOT NULL, current_snapshot TEXT NOT NULL);",
            )?;
        }
        if version < 2 {
            // v2: machine-readable actor on events, input_hash on nodes for
            // dependency-graph revalidation.
            self.conn.execute_batch(
                "ALTER TABLE events ADD COLUMN actor TEXT NOT NULL DEFAULT 'system';
                 ALTER TABLE nodes ADD COLUMN input_hash TEXT NOT NULL DEFAULT '';",
            )?;
        }
        self.conn.execute(
            "INSERT INTO meta(key,value) VALUES('schema_version',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    pub fn max_seq(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0))?)
    }

    pub fn current_map(&self) -> Result<BTreeMap<String, i64>> {
        let mut m = BTreeMap::new();
        let mut stmt = self.conn.prepare("SELECT node_id, version FROM current ORDER BY node_id")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for r in rows {
            let (k, v) = r?;
            m.insert(k, v);
        }
        Ok(m)
    }

    pub fn get_node(&self, id: &str, version: i64) -> Result<Node> {
        let row = self.conn.query_row(
            "SELECT id,version,kind,status,payload,deps,input_hash,created_event FROM nodes WHERE id=?1 AND version=?2",
            params![id, version],
            |r| {
                Ok((
                    r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?, r.get::<_, String>(7)?,
                ))
            },
        );
        match row {
            Ok((id, version, kind, status, payload, deps, input_hash, created_event)) => Ok(Node {
                id, version, kind, status,
                payload: serde_json::from_str(&payload)?,
                deps: serde_json::from_str(&deps)?,
                input_hash, created_event,
            }),
            Err(e) => Err(anyhow!("node {id}@{version} not found: {e}")),
        }
    }

    pub fn current_node(&self, id: &str) -> Result<Node> {
        let v: i64 = self
            .conn
            .query_row("SELECT version FROM current WHERE node_id=?1", params![id], |r| r.get(0))
            .with_context(|| format!("no current version for {id}"))?;
        self.get_node(id, v)
    }

    pub fn node_versions(&self, id: &str) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare("SELECT version FROM nodes WHERE id=?1 ORDER BY version")?;
        let rows = stmt.query_map(params![id], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn next_version(&self, id: &str) -> Result<i64> {
        Ok(self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version),0)+1 FROM nodes WHERE id=?1",
                params![id],
                |r| r.get(0),
            )?)
    }

    pub fn events(&self) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq,id,ts,kind,actor,payload,current_snapshot FROM events ORDER BY seq")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
                r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (seq, id, ts, kind, actor, payload, snap) = r?;
            out.push(Event {
                seq, id, ts, kind, actor,
                payload: serde_json::from_str(&payload)?,
                current_snapshot: serde_json::from_str(&snap)?,
            });
        }
        Ok(out)
    }

    /// Atomically commit a set of new node versions, current-pointer moves
    /// and one event. Either everything lands or nothing does.
    pub fn commit(
        &self,
        new_nodes: &[Node],
        current_updates: &[(String, i64)],
        event_kind: &str,
        actor: &str,
        event_payload: serde_json::Value,
    ) -> Result<Event> {
        let tx = self.conn.unchecked_transaction()?;
        for n in new_nodes {
            tx.execute(
                "INSERT INTO nodes(id,version,kind,status,payload,deps,input_hash,created_event)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    n.id, n.version, n.kind, n.status,
                    serde_json::to_string(&n.payload)?,
                    serde_json::to_string(&n.deps)?,
                    n.input_hash, n.created_event,
                ],
            )?;
        }
        for (id, v) in current_updates {
            tx.execute(
                "INSERT INTO current(node_id,version) VALUES(?1,?2)
                 ON CONFLICT(node_id) DO UPDATE SET version=excluded.version",
                params![id, v],
            )?;
        }
        let seq: i64 = tx.query_row("SELECT COALESCE(MAX(seq),0)+1 FROM events", [], |r| r.get(0))?;
        let event_id = format!("evt-{seq:04}");
        // snapshot of current AFTER this commit
        let mut snap = BTreeMap::new();
        {
            let mut stmt = tx.prepare("SELECT node_id, version FROM current")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            for r in rows {
                let (k, v) = r?;
                snap.insert(k, v);
            }
        }
        let ts = format!("2026-09-23T00:{:02}:{:02}Z", seq / 60 % 60, seq % 60);
        tx.execute(
            "INSERT INTO events(seq,id,ts,kind,actor,payload,current_snapshot) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                seq, event_id, ts, event_kind, actor,
                serde_json::to_string(&event_payload)?,
                serde_json::to_string(&snap)?,
            ],
        )?;
        tx.commit()?;
        Ok(Event { seq, id: event_id, ts, kind: event_kind.into(), actor: actor.into(), payload: event_payload, current_snapshot: snap })
    }

    /// Optimistic-concurrency guard: the caller must base its edit on the
    /// latest event sequence, otherwise a resolvable conflict is reported.
    pub fn check_base(&self, base_seq: i64) -> std::result::Result<(), SaveError> {
        let actual = self.max_seq().map_err(SaveError::Other)?;
        if actual != base_seq {
            return Err(SaveError::Conflict { expected_seq: base_seq, actual_seq: actual });
        }
        Ok(())
    }

    /// Replay: restore the current-pointer map captured at event `seq`,
    /// and record the replay itself as a new event. Historical events and
    /// node versions are never overwritten.
    pub fn replay_to(&self, seq: i64, actor: &str) -> Result<Event> {
        let events = self.events()?;
        let target = events.iter().find(|e| e.seq == seq).ok_or_else(|| anyhow!("no event seq {seq}"))?;
        let snap = target.current_snapshot.clone();
        let updates: Vec<(String, i64)> = snap.iter().map(|(k, v)| (k.clone(), *v)).collect();
        // remove current entries that did not exist at that point
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM current", [])?;
        for (id, v) in &updates {
            tx.execute("INSERT INTO current(node_id,version) VALUES(?1,?2)", params![id, v])?;
        }
        tx.commit()?;
        self.commit(
            &[], &[],
            "replay",
            actor,
            serde_json::json!({
                "restored_event_seq": seq,
                "restored_event_id": target.id,
                "note": "current pointers restored to the dependency versions and visible conclusions of that event; history not overwritten"
            }),
        )
    }

    /// Remove node versions that are neither current nor referenced by any
    /// event snapshot — i.e. orphans from a torn write. Returns pruned ids.
    pub fn recover(&self) -> Result<Vec<String>> {
        let mut referenced: std::collections::BTreeSet<(String, i64)> = self
            .current_map()?
            .into_iter()
            .collect();
        for e in self.events()? {
            for (k, v) in e.current_snapshot {
                referenced.insert((k, v));
            }
        }
        let mut all = Vec::new();
        {
            let mut stmt = self.conn.prepare("SELECT id, version FROM nodes")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            for r in rows {
                all.push(r?);
            }
        }
        let mut pruned = Vec::new();
        for (id, v) in all {
            if !referenced.contains(&(id.clone(), v)) {
                self.conn.execute("DELETE FROM nodes WHERE id=?1 AND version=?2", params![id, v])?;
                pruned.push(format!("{id}@{v}"));
            }
        }
        Ok(pruned)
    }

    pub fn export_json(&self) -> Result<serde_json::Value> {
        let mut nodes = Vec::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT id,version,kind,status,payload,deps,input_hash,created_event FROM nodes ORDER BY id,version")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?, r.get::<_, String>(7)?,
                ))
            })?;
            for r in rows {
                let (id, version, kind, status, payload, deps, input_hash, ce) = r?;
                nodes.push(serde_json::json!({
                    "id": id, "version": version, "kind": kind, "status": status,
                    "payload": serde_json::from_str::<serde_json::Value>(&payload)?,
                    "deps": serde_json::from_str::<serde_json::Value>(&deps)?,
                    "input_hash": input_hash, "created_event": ce,
                }));
            }
        }
        Ok(serde_json::json!({
            "format": "saxs-bench-export",
            "schema_version": SCHEMA_VERSION,
            "nodes": nodes,
            "current": self.current_map()?,
            "events": self.events()?,
        }))
    }

    pub fn import_json(&self, dump: &serde_json::Value) -> Result<()> {
        if dump.get("format").and_then(|f| f.as_str()) != Some("saxs-bench-export") {
            bail!("not a saxs-bench export");
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch("DELETE FROM events; DELETE FROM current; DELETE FROM nodes;")?;
        for n in dump["nodes"].as_array().cloned().unwrap_or_default() {
            tx.execute(
                "INSERT INTO nodes(id,version,kind,status,payload,deps,input_hash,created_event)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    n["id"].as_str().unwrap_or(""), n["version"].as_i64().unwrap_or(0),
                    n["kind"].as_str().unwrap_or(""), n["status"].as_str().unwrap_or(""),
                    serde_json::to_string(&n["payload"])?, serde_json::to_string(&n["deps"])?,
                    n["input_hash"].as_str().unwrap_or(""), n["created_event"].as_str().unwrap_or(""),
                ],
            )?;
        }
        if let Some(obj) = dump["current"].as_object() {
            for (k, v) in obj {
                tx.execute("INSERT INTO current(node_id,version) VALUES(?1,?2)",
                    params![k, v.as_i64().unwrap_or(0)])?;
            }
        }
        for e in dump["events"].as_array().cloned().unwrap_or_default() {
            tx.execute(
                "INSERT INTO events(seq,id,ts,kind,actor,payload,current_snapshot) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    e["seq"].as_i64().unwrap_or(0), e["id"].as_str().unwrap_or(""),
                    e["ts"].as_str().unwrap_or(""), e["kind"].as_str().unwrap_or(""),
                    e["actor"].as_str().unwrap_or("system"),
                    serde_json::to_string(&e["payload"])?,
                    serde_json::to_string(&e["current_snapshot"])?,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Stable fingerprint of the whole visible state (used by tests to prove
    /// export/delete/import preserves identity, order and provenance).
    pub fn state_fingerprint(&self) -> Result<String> {
        let dump = self.export_json()?;
        Ok(hash_payload(&serde_json::to_string(&dump)?))
    }
}
