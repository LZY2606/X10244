use crate::graph::Workspace;
use crate::model::*;
use rusqlite::{params, Connection};
use serde_json::json;

pub const SCHEMA_VERSION: u32 = 2;

pub struct Store {
    pub conn: Connection,
}

#[derive(Debug)]
pub enum SaveOutcome {
    Committed(u64),
    Conflict(Conflict),
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Store> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let store = Store { conn };
        store.migrate()?;
        store.recover()?;
        Ok(store)
    }

    pub fn open_memory() -> rusqlite::Result<Store> {
        let conn = Connection::open_in_memory()?;
        let store = Store { conn };
        store.migrate()?;
        store.recover()?;
        Ok(store)
    }

    fn user_version(&self) -> u32 {
        self.conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        let v = self.user_version();
        if v < 1 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS versions(
                    id INTEGER PRIMARY KEY, parent INTEGER, state TEXT NOT NULL,
                    created TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS sources(
                    id TEXT NOT NULL, version INTEGER NOT NULL, json TEXT NOT NULL,
                    PRIMARY KEY(id, version));
                 CREATE TABLE IF NOT EXISTS source_snapshots(
                    version_id INTEGER NOT NULL, id TEXT NOT NULL, json TEXT NOT NULL,
                    PRIMARY KEY(version_id, id));
                 CREATE TABLE IF NOT EXISTS nodes(
                    version_id INTEGER NOT NULL, id TEXT NOT NULL, json TEXT NOT NULL,
                    PRIMARY KEY(version_id, id));
                 CREATE TABLE IF NOT EXISTS events(
                    seq INTEGER PRIMARY KEY, ts TEXT NOT NULL, actor TEXT NOT NULL,
                    kind TEXT NOT NULL, base_version INTEGER NOT NULL,
                    resulting_version INTEGER NOT NULL, payload TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS current(version_id INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS fingerprints(id TEXT PRIMARY KEY, fp INTEGER NOT NULL);
                 INSERT INTO current(version_id) SELECT 0 WHERE NOT EXISTS(SELECT 1 FROM current);",
            )?;
            self.conn.execute_batch("PRAGMA user_version = 1")?;
        }
        if v < 2 {
            // v2: conflicts table + versions.note column
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS conflicts(
                    id TEXT PRIMARY KEY, base_version INTEGER NOT NULL,
                    branch_a INTEGER NOT NULL, branch_b INTEGER NOT NULL,
                    resolved INTEGER);
                 ALTER TABLE versions ADD COLUMN note TEXT;",
            )?;
            self.conn.execute_batch("PRAGMA user_version = 2")?;
        }
        Ok(())
    }

    /// Crash recovery: any version left in 'building' state was interrupted
    /// mid-write; abandon it. The current pointer only ever references
    /// 'complete' versions, so readers see either the old or the new version.
    fn recover(&self) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE versions SET state='abandoned' WHERE state='building'",
            [],
        )?;
        Ok(())
    }

    pub fn current_version(&self) -> u64 {
        self.conn
            .query_row("SELECT version_id FROM current", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as u64
    }

    fn set_current(&self, v: u64) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE current SET version_id = ?1", params![v as i64])?;
        Ok(())
    }

    /// Simulate an interrupted write: a 'building' version row plus partial
    /// node payload is persisted without ever switching the current pointer.
    pub fn simulate_interrupted_write(&self, note: &str) -> rusqlite::Result<u64> {
        self.conn.execute(
            "INSERT INTO versions(parent, state, created, note) VALUES(?1,'building',?2,?3)",
            params![self.current_version() as i64, now(), note],
        )?;
        let id = self.conn.last_insert_rowid() as u64;
        self.conn.execute(
            "INSERT INTO nodes(version_id, id, json) VALUES(?1,'node:partial','{}')",
            params![id as i64],
        )?;
        Ok(id)
    }

    /// Atomically commit a workspace snapshot as a new version. The current
    /// pointer switches only after every row is durably written.
    pub fn commit_version(
        &self,
        ws: &Workspace,
        parent: u64,
        note: &str,
    ) -> rusqlite::Result<u64> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO versions(parent, state, created, note) VALUES(?1,'building',?2,?3)",
            params![parent as i64, now(), note],
        )?;
        let vid = tx.last_insert_rowid() as u64;
        for src in ws.sources.values() {
            tx.execute(
                "INSERT OR REPLACE INTO sources(id, version, json) VALUES(?1,?2,?3)",
                params![src.id, src.version as i64, serde_json::to_string(src).unwrap()],
            )?;
            tx.execute(
                "INSERT INTO source_snapshots(version_id, id, json) VALUES(?1,?2,?3)",
                params![vid as i64, src.id, serde_json::to_string(src).unwrap()],
            )?;
        }
        for node in ws.nodes.values() {
            tx.execute(
                "INSERT INTO nodes(version_id, id, json) VALUES(?1,?2,?3)",
                params![vid as i64, node.id, serde_json::to_string(node).unwrap()],
            )?;
        }
        for (id, fp) in &ws.fingerprints {
            tx.execute(
                "INSERT OR REPLACE INTO fingerprints(id, fp) VALUES(?1,?2)",
                params![id, *fp as i64],
            )?;
        }
        tx.execute("UPDATE versions SET state='complete' WHERE id=?1", params![vid as i64])?;
        tx.execute("UPDATE current SET version_id=?1", params![vid as i64])?;
        tx.commit()?;
        Ok(vid)
    }

    pub fn load_workspace(&self, version: u64) -> Workspace {
        let mut ws = Workspace::default();
        {
            let mut stmt = self.conn.prepare("SELECT id, fp FROM fingerprints").unwrap();
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).unwrap();
            for r in rows.flatten() {
                ws.fingerprints.insert(r.0, r.1 as u64);
            }
        }
        let source_rows: Vec<String> = if version > 0 {
            let mut stmt = self
                .conn
                .prepare("SELECT json FROM source_snapshots WHERE version_id=?1 ORDER BY id")
                .unwrap();
            stmt.query_map(params![version as i64], |r| r.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        } else {
            Vec::new()
        };
        let mut stmt = self
            .conn
            .prepare("SELECT json FROM sources ORDER BY id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok());
        let latest: Vec<String> = rows.collect();
        for j in if source_rows.is_empty() { latest } else { source_rows } {
            let src: SourceRecord = serde_json::from_str(&j).unwrap();
            ws.fingerprints.entry(src.id.clone()).or_insert(src.version);
            ws.sources.insert(src.id.clone(), src);
        }
        if version > 0 {
            let mut stmt = self
                .conn
                .prepare("SELECT id, json FROM nodes WHERE version_id=?1 ORDER BY id")
                .unwrap();
            let rows = stmt
                .query_map(params![version as i64], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .unwrap()
                .filter_map(|r| r.ok());
            for (id, j) in rows {
                let node: DerivedNode = serde_json::from_str(&j).unwrap();
                ws.nodes.insert(id.clone(), node);
            }
        }
        ws
    }

    pub fn append_event(&self, actor: &str, kind: &str, base: u64, resulting: u64, payload: serde_json::Value) -> u64 {
        self.conn
            .execute(
                "INSERT INTO events(ts, actor, kind, base_version, resulting_version, payload)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![now(), actor, kind, base as i64, resulting as i64, payload.to_string()],
            )
            .unwrap();
        self.conn.last_insert_rowid() as u64
    }

    pub fn events(&self) -> Vec<Event> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, ts, actor, kind, base_version, resulting_version, payload FROM events ORDER BY seq")
            .unwrap();
        stmt.query_map([], |r| {
            Ok(Event {
                seq: r.get::<_, i64>(0)? as u64,
                ts: r.get(1)?,
                actor: r.get(2)?,
                kind: r.get(3)?,
                base_version: r.get::<_, i64>(4)? as u64,
                resulting_version: r.get::<_, i64>(5)? as u64,
                payload: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or(json!(null)),
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Optimistic-concurrency save: both users start from `base`. The first
    /// commit wins the current pointer; a commit whose base is no longer
    /// current produces a resolvable conflict — both branches are kept.
    pub fn save_branch(
        &self,
        ws: &Workspace,
        base: u64,
        note: &str,
    ) -> rusqlite::Result<SaveOutcome> {
        let current = self.current_version();
        let vid = self.commit_version(ws, base, note)?;
        if base != current {
            let conflict = Conflict {
                id: stable_id("conflict", &format!("{}:{}:{}", base, current, vid)),
                base_version: base,
                branch_a: current,
                branch_b: vid,
                resolved: None,
            };
            self.conn.execute(
                "INSERT INTO conflicts(id, base_version, branch_a, branch_b, resolved)
                 VALUES(?1,?2,?3,?4,NULL)",
                params![conflict.id, base as i64, current as i64, vid as i64],
            )?;
            return Ok(SaveOutcome::Conflict(conflict));
        }
        Ok(SaveOutcome::Committed(vid))
    }

    pub fn resolve_conflict(&self, id: &str, winner: u64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE conflicts SET resolved=?1 WHERE id=?2",
            params![winner as i64, id],
        )?;
        self.set_current(winner)?;
        Ok(())
    }

    pub fn conflicts(&self) -> Vec<Conflict> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, base_version, branch_a, branch_b, resolved FROM conflicts ORDER BY rowid")
            .unwrap();
        stmt.query_map([], |r| {
            Ok(Conflict {
                id: r.get(0)?,
                base_version: r.get::<_, i64>(1)? as u64,
                branch_a: r.get::<_, i64>(2)? as u64,
                branch_b: r.get::<_, i64>(3)? as u64,
                resolved: r.get::<_, Option<i64>>(4)?.map(|v| v as u64),
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn versions(&self) -> Vec<(u64, u64, String, String)> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, COALESCE(parent,0), state, COALESCE(note,'') FROM versions ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)? as u64,
                r.get::<_, i64>(1)? as u64,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Full-fidelity archive: identities, order and provenance preserved.
    pub fn export_archive(&self) -> serde_json::Value {
        json!({
            "format": "saxs-bench-archive/1",
            "schema_version": SCHEMA_VERSION,
            "current": self.current_version(),
            "versions": self.versions().iter().map(|(id, p, s, n)| json!({"id": id, "parent": p, "state": s, "note": n})).collect::<Vec<_>>(),
            "sources": self.all_sources_json(),
            "source_snapshots": self.all_source_snapshots_json(),
            "nodes": self.all_nodes_json(),
            "events": self.events(),
            "conflicts": self.conflicts(),
            "fingerprints": self.fingerprints_json(),
        })
    }

    fn fingerprints_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        if let Ok(mut stmt) = self.conn.prepare("SELECT id, fp FROM fingerprints ORDER BY id") {
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).unwrap();
            for r in rows.flatten() {
                map.insert(r.0, json!(r.1));
            }
        }
        serde_json::Value::Object(map)
    }

    fn all_sources_json(&self) -> Vec<serde_json::Value> {
        let mut stmt = self.conn.prepare("SELECT json FROM sources ORDER BY id, version").unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .map(|j| serde_json::from_str(&j).unwrap())
            .collect()
    }

    fn all_nodes_json(&self) -> Vec<serde_json::Value> {
        let mut stmt = self.conn.prepare("SELECT version_id, json FROM nodes ORDER BY version_id, id").unwrap();
        stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .map(|(v, j)| json!({"version_id": v, "node": serde_json::from_str::<serde_json::Value>(&j).unwrap()}))
            .collect()
    }

    fn all_source_snapshots_json(&self) -> Vec<serde_json::Value> {
        let mut stmt = self
            .conn
            .prepare("SELECT version_id, json FROM source_snapshots ORDER BY version_id, id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .map(|(v, j)| json!({"version_id": v, "source": serde_json::from_str::<serde_json::Value>(&j).unwrap()}))
            .collect()
    }

    /// Import an archive into this (expected-empty) store, preserving every
    /// stable identity, ordering and provenance link.
    pub fn import_archive(&self, archive: &serde_json::Value) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for v in archive["versions"].as_array().unwrap() {
            tx.execute(
                "INSERT INTO versions(id, parent, state, created, note) VALUES(?1,?2,?3,'imported',?4)",
                params![
                    v["id"].as_i64().unwrap(),
                    v["parent"].as_i64().unwrap(),
                    v["state"].as_str().unwrap(),
                    v["note"].as_str().unwrap_or("")
                ],
            )?;
        }
        for s in archive["sources"].as_array().unwrap() {
            let src: SourceRecord = serde_json::from_value(s.clone()).unwrap();
            tx.execute(
                "INSERT OR REPLACE INTO sources(id, version, json) VALUES(?1,?2,?3)",
                params![src.id, src.version as i64, serde_json::to_string(&src).unwrap()],
            )?;
        }
        for n in archive["nodes"].as_array().unwrap() {
            let node: DerivedNode = serde_json::from_value(n["node"].clone()).unwrap();
            tx.execute(
                "INSERT INTO nodes(version_id, id, json) VALUES(?1,?2,?3)",
                params![n["version_id"].as_i64().unwrap(), node.id, serde_json::to_string(&node).unwrap()],
            )?;
        }
        if let Some(snaps) = archive["source_snapshots"].as_array() {
            for s in snaps {
                let src: SourceRecord = serde_json::from_value(s["source"].clone()).unwrap();
                tx.execute(
                    "INSERT INTO source_snapshots(version_id, id, json) VALUES(?1,?2,?3)",
                    params![s["version_id"].as_i64().unwrap(), src.id, serde_json::to_string(&src).unwrap()],
                )?;
            }
        }
        for e in archive["events"].as_array().unwrap() {
            let ev: Event = serde_json::from_value(e.clone()).unwrap();
            tx.execute(
                "INSERT INTO events(seq, ts, actor, kind, base_version, resulting_version, payload)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![ev.seq as i64, ev.ts, ev.actor, ev.kind, ev.base_version as i64, ev.resulting_version as i64, ev.payload.to_string()],
            )?;
        }
        for c in archive["conflicts"].as_array().unwrap() {
            let cf: Conflict = serde_json::from_value(c.clone()).unwrap();
            tx.execute(
                "INSERT INTO conflicts(id, base_version, branch_a, branch_b, resolved) VALUES(?1,?2,?3,?4,?5)",
                params![cf.id, cf.base_version as i64, cf.branch_a as i64, cf.branch_b as i64, cf.resolved.map(|r| r as i64)],
            )?;
        }
        if let Some(fp) = archive["fingerprints"].as_object() {
            for (id, v) in fp {
                tx.execute(
                    "INSERT OR REPLACE INTO fingerprints(id, fp) VALUES(?1,?2)",
                    params![id, v.as_i64().unwrap()],
                )?;
            }
        }
        tx.execute(
            "UPDATE current SET version_id=?1",
            params![archive["current"].as_i64().unwrap()],
        )?;
        tx.commit()?;
        Ok(())
    }
}

pub fn now() -> String {
    // deterministic-ish timestamp without extra deps
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    format!("{}Z", secs)
}
