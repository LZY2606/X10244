//! SQLite storage: append-only events, per-node versions, atomic head switch.
//!
//! - Every derivation produces an independent version; the head pointer only
//!   switches after the full write succeeds (single transaction).
//! - A crash mid-write leaves either the old version or a complete new one.
//! - Two users saving from the same base version produce a resolvable
//!   conflict; the losing branch is kept, never overwritten.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const SCHEMA_VERSION: i64 = 2;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub deps: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub id: String, // stable identity: "<node>@v<seq>"
    pub node_id: String,
    pub seq: i64,
    pub state: Value, // domain payload: curve / candidates / diagnostics / validity
    pub event_id: i64,
    pub chosen: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub kind: String,
    pub actor: String,
    pub summary: String,
    pub payload: Value,
    pub heads_snapshot: Value,
    pub seq_time: i64,
}

#[derive(Debug)]
pub enum SaveError {
    Conflict { head: String },
    NodeMissing(String),
    Sql(rusqlite::Error),
}

impl From<rusqlite::Error> for SaveError {
    fn from(e: rusqlite::Error) -> Self {
        SaveError::Sql(e)
    }
}

pub struct Store {
    pub conn: Connection,
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        let mut s = Store { conn };
        s.migrate()?;
        Ok(s)
    }

    pub fn open_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let mut s = Store { conn };
        s.migrate()?;
        Ok(s)
    }

    fn schema_version(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key='schema_version'",
                [],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    fn migrate(&mut self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        let v = self.schema_version();
        if v < 1 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS nodes(
                    id TEXT PRIMARY KEY, kind TEXT NOT NULL,
                    name TEXT NOT NULL, deps TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS versions(
                    id TEXT PRIMARY KEY, node_id TEXT NOT NULL, seq INTEGER NOT NULL,
                    state TEXT NOT NULL, event_id INTEGER NOT NULL,
                    UNIQUE(node_id, seq));
                 CREATE TABLE IF NOT EXISTS heads(
                    node_id TEXT PRIMARY KEY, version_id TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS events(
                    id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL,
                    summary TEXT NOT NULL, payload TEXT NOT NULL,
                    heads_snapshot TEXT NOT NULL, seq_time INTEGER NOT NULL);",
            )?;
        }
        if v < 2 {
            // v1 -> v2: actor on events, chosen flag on versions.
            self.conn
                .execute_batch("ALTER TABLE events ADD COLUMN actor TEXT NOT NULL DEFAULT 'system';")?;
            self.conn.execute_batch(
                "ALTER TABLE versions ADD COLUMN chosen INTEGER NOT NULL DEFAULT 1;",
            )?;
        }
        self.conn.execute(
            "INSERT INTO schema_meta(key,value) VALUES('schema_version',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0)
            == 0
    }

    pub fn add_node(&mut self, node: &Node) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO nodes(id,kind,name,deps) VALUES(?1,?2,?3,?4)
             ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, name=excluded.name, deps=excluded.deps",
            params![node.id, node.kind, node.name, serde_json::to_string(&node.deps).unwrap()],
        )?;
        Ok(())
    }

    pub fn nodes(&self) -> Vec<Node> {
        let mut st = self.conn.prepare("SELECT id,kind,name,deps FROM nodes ORDER BY id").unwrap();
        st.query_map([], |r| {
            Ok(Node {
                id: r.get(0)?,
                kind: r.get(1)?,
                name: r.get(2)?,
                deps: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
            })
        })
        .unwrap()
        .filter_map(|n| n.ok())
        .collect()
    }

    fn heads_map_conn(conn: &Connection) -> Value {
        let mut st = conn.prepare("SELECT node_id, version_id FROM heads").unwrap();
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap();
        let mut m = serde_json::Map::new();
        for r in rows.flatten() {
            m.insert(r.0, json!(r.1));
        }
        Value::Object(m)
    }

    pub fn head(&self, node_id: &str) -> Option<String> {
        self.conn
            .query_row("SELECT version_id FROM heads WHERE node_id=?1", params![node_id], |r| {
                r.get(0)
            })
            .ok()
    }

    fn next_seq(conn: &Connection, node_id: &str) -> i64 {
        conn.query_row(
            "SELECT COALESCE(MAX(seq),0)+1 FROM versions WHERE node_id=?1",
            params![node_id],
            |r| r.get(0),
        )
        .unwrap_or(1)
    }

    fn insert_event_conn(
        conn: &Connection,
        kind: &str,
        actor: &str,
        summary: &str,
        payload: &Value,
    ) -> rusqlite::Result<i64> {
        let heads = Self::heads_map_conn(conn);
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO events(kind,actor,summary,payload,heads_snapshot,seq_time)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![kind, actor, summary, payload.to_string(), heads.to_string(), t],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Save a new version for a node and switch its head atomically.
    /// `base` is the version the editor started from; a moved head yields a
    /// resolvable conflict instead of an overwrite.
    pub fn save_version(
        &mut self,
        node_id: &str,
        state: &Value,
        base: Option<&str>,
        chosen: bool,
        event_kind: &str,
        actor: &str,
        summary: &str,
        payload: &Value,
    ) -> Result<(String, i64), SaveError> {
        if self.head(node_id).is_none() && !self.node_exists(node_id) {
            return Err(SaveError::NodeMissing(node_id.to_string()));
        }
        if let Some(b) = base {
            let head = self.head(node_id);
            if head.as_deref() != Some(b) {
                return Err(SaveError::Conflict {
                    head: head.unwrap_or_default(),
                });
            }
        }
        let tx = self.conn.transaction()?;
        let seq = Self::next_seq(&tx, node_id);
        let vid = format!("{}@v{}", node_id, seq);
        // Version row written first; head switches only after everything succeeds.
        tx.execute(
            "INSERT INTO versions(id,node_id,seq,state,event_id,chosen) VALUES(?1,?2,?3,?4,0,?5)",
            params![vid, node_id, seq, state.to_string(), chosen as i64],
        )?;
        if chosen {
            tx.execute(
                "INSERT INTO heads(node_id,version_id) VALUES(?1,?2)
                 ON CONFLICT(node_id) DO UPDATE SET version_id=excluded.version_id",
                params![node_id, vid],
            )?;
        }
        let event_id = Self::insert_event_conn(&tx, event_kind, actor, summary, payload)?;
        tx.execute(
            "UPDATE versions SET event_id=?1 WHERE id=?2",
            params![event_id, vid],
        )?;
        tx.commit()?;
        Ok((vid, event_id))
    }

    /// Record an event without producing a version (decisions, diagnostics).
    pub fn record_event(
        &mut self,
        kind: &str,
        actor: &str,
        summary: &str,
        payload: &Value,
    ) -> rusqlite::Result<i64> {
        Self::insert_event_conn(&self.conn, kind, actor, summary, payload)
    }

    /// Choose an existing version as the head (decision); the previously
    /// chosen version stays in history as an unchosen candidate.
    pub fn choose_version(
        &mut self,
        node_id: &str,
        version_id: &str,
        actor: &str,
        rationale: &str,
    ) -> Result<i64, SaveError> {
        let tx = self.conn.transaction()?;
        let exists: bool = tx
            .query_row(
                "SELECT COUNT(*) FROM versions WHERE id=?1 AND node_id=?2",
                params![version_id, node_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)?;
        if !exists {
            return Err(SaveError::NodeMissing(format!("{}/{}", node_id, version_id)));
        }
        tx.execute("UPDATE versions SET chosen=0 WHERE node_id=?1", params![node_id])?;
        tx.execute("UPDATE versions SET chosen=1 WHERE id=?1", params![version_id])?;
        tx.execute(
            "INSERT INTO heads(node_id,version_id) VALUES(?1,?2)
             ON CONFLICT(node_id) DO UPDATE SET version_id=excluded.version_id",
            params![node_id, version_id],
        )?;
        let eid = Self::insert_event_conn(
            &tx,
            "decision",
            actor,
            &format!("chose {} for {}", version_id, node_id),
            &json!({ "node": node_id, "chosen": version_id, "rationale": rationale }),
        )?;
        tx.commit()?;
        Ok(eid)
    }

    fn node_exists(&self, node_id: &str) -> bool {
        self.conn
            .query_row("SELECT COUNT(*) FROM nodes WHERE id=?1", params![node_id], |r| {
                r.get::<_, i64>(0)
            })
            .map(|c| c > 0)
            .unwrap_or(false)
    }

    pub fn version(&self, version_id: &str) -> Option<Version> {
        self.conn
            .query_row(
                "SELECT id,node_id,seq,state,event_id,chosen FROM versions WHERE id=?1",
                params![version_id],
                |r| {
                    Ok(Version {
                        id: r.get(0)?,
                        node_id: r.get(1)?,
                        seq: r.get(2)?,
                        state: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or(json!(null)),
                        event_id: r.get(4)?,
                        chosen: r.get::<_, i64>(5)? != 0,
                    })
                },
            )
            .ok()
    }

    pub fn versions(&self, node_id: &str) -> Vec<Version> {
        let mut st = self
            .conn
            .prepare("SELECT id,node_id,seq,state,event_id,chosen FROM versions WHERE node_id=?1 ORDER BY seq")
            .unwrap();
        st.query_map(params![node_id], |r| {
            Ok(Version {
                id: r.get(0)?,
                node_id: r.get(1)?,
                seq: r.get(2)?,
                state: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or(json!(null)),
                event_id: r.get(4)?,
                chosen: r.get::<_, i64>(5)? != 0,
            })
        })
        .unwrap()
        .filter_map(|v| v.ok())
        .collect()
    }

    pub fn events(&self) -> Vec<Event> {
        let mut st = self
            .conn
            .prepare("SELECT id,kind,actor,summary,payload,heads_snapshot,seq_time FROM events ORDER BY id")
            .unwrap();
        st.query_map([], |r| {
            Ok(Event {
                id: r.get(0)?,
                kind: r.get(1)?,
                actor: r.get(2)?,
                summary: r.get(3)?,
                payload: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(json!(null)),
                heads_snapshot: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(json!(null)),
                seq_time: r.get(6)?,
            })
        })
        .unwrap()
        .filter_map(|e| e.ok())
        .collect()
    }

    pub fn event(&self, id: i64) -> Option<Event> {
        self.events().into_iter().find(|e| e.id == id)
    }

    /// Heads as of a given event: restores the dependency versions and visible
    /// conclusions of that moment instead of overwriting history with the
    /// latest computation.
    pub fn heads_at(&self, event_id: i64) -> Value {
        self.event(event_id).map(|e| e.heads_snapshot).unwrap_or(json!({}))
    }

    /// Full dump for export; identities, order and provenance survive a
    /// delete + reimport round trip.
    pub fn export_json(&self) -> Value {
        json!({
            "schema_version": SCHEMA_VERSION,
            "nodes": self.nodes(),
            "versions": self.nodes().iter().flat_map(|n| self.versions(&n.id)).collect::<Vec<_>>(),
            "heads": self.heads(),
            "events": self.events(),
        })
    }

    pub fn heads(&self) -> Value {
        Self::heads_map_conn(&self.conn)
    }

    pub fn import_json(&mut self, dump: &Value) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch("DELETE FROM heads; DELETE FROM versions; DELETE FROM events; DELETE FROM nodes;")?;
        for n in dump["nodes"].as_array().cloned().unwrap_or_default() {
            tx.execute(
                "INSERT INTO nodes(id,kind,name,deps) VALUES(?1,?2,?3,?4)",
                params![n["id"].as_str().unwrap(), n["kind"].as_str().unwrap(),
                        n["name"].as_str().unwrap(), serde_json::to_string(&n["deps"]).unwrap()],
            )?;
        }
        for v in dump["versions"].as_array().cloned().unwrap_or_default() {
            tx.execute(
                "INSERT INTO versions(id,node_id,seq,state,event_id,chosen) VALUES(?1,?2,?3,?4,?5,?6)",
                params![v["id"].as_str().unwrap(), v["node_id"].as_str().unwrap(),
                        v["seq"].as_i64().unwrap(), v["state"].to_string(),
                        v["event_id"].as_i64().unwrap(), v["chosen"].as_bool().unwrap_or(true) as i64],
            )?;
        }
        if let Some(obj) = dump["heads"].as_object() {
            for (k, v) in obj {
                tx.execute(
                    "INSERT INTO heads(node_id,version_id) VALUES(?1,?2)",
                    params![k, v.as_str().unwrap()],
                )?;
            }
        }
        for e in dump["events"].as_array().cloned().unwrap_or_default() {
            tx.execute(
                "INSERT INTO events(id,kind,actor,summary,payload,heads_snapshot,seq_time)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![e["id"].as_i64().unwrap(), e["kind"].as_str().unwrap(),
                        e["actor"].as_str().unwrap_or("system"), e["summary"].as_str().unwrap(),
                        e["payload"].to_string(), e["heads_snapshot"].to_string(),
                        e["seq_time"].as_i64().unwrap_or(0)],
            )?;
        }
        tx.commit()
    }

    /// Simulate an interrupted write: version row inserted but transaction
    /// rolled back before the head switch. Head must remain at the old version.
    pub fn simulate_interrupted_write(&mut self, node_id: &str, state: &Value) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        let seq = Self::next_seq(&tx, node_id);
        let vid = format!("{}@v{}", node_id, seq);
        tx.execute(
            "INSERT INTO versions(id,node_id,seq,state,event_id,chosen) VALUES(?1,?2,?3,?4,0,1)",
            params![vid, node_id, seq, state.to_string()],
        )?;
        // crash before head switch / event insert
        tx.rollback()
    }
}
