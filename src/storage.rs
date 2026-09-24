use crate::domain::EventRecord;
use crate::domain::CommandRequest;
use crate::engine::{fixture, initial_event, replay, AppliedEvent, Projection};
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "busy_timeout", 5000).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
        conn.pragma_update(None, "foreign_keys", "ON").map_err(|e| e.to_string())?;
        let store = Self { conn: Mutex::new(conn) };
        store.migrate()?;
        store.bootstrap_if_empty()?;
        Ok(store)
    }

    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        let store = Self { conn: Mutex::new(conn) };
        store.migrate()?;
        store.bootstrap_if_empty()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if version < 1 {
            conn.execute_batch(
                "CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE events (seq INTEGER PRIMARY KEY CHECK (seq > 0), event_id TEXT NOT NULL UNIQUE, json TEXT NOT NULL, hash TEXT NOT NULL);
                 CREATE TABLE event_parents (event_id TEXT PRIMARY KEY, parent_event_id TEXT, hash_chain TEXT NOT NULL);",
            ).map_err(|e| e.to_string())?;
            conn.execute("PRAGMA user_version=1", []).map_err(|e| e.to_string())?;
        }
        if version < 2 {
            conn.execute_batch(
                "CREATE TABLE node_versions (node_id TEXT NOT NULL, version INTEGER NOT NULL, fingerprint TEXT NOT NULL, json TEXT NOT NULL, created_event TEXT NOT NULL, PRIMARY KEY(node_id, version));
                 CREATE TABLE current_nodes (node_id TEXT PRIMARY KEY, version INTEGER NOT NULL, fingerprint TEXT NOT NULL);
                 CREATE TABLE audit_tail (event_id TEXT PRIMARY KEY, completed_at TEXT NOT NULL);",
            ).map_err(|e| e.to_string())?;
            conn.execute("PRAGMA user_version=2", []).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn bootstrap_if_empty(&self) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if count != 0 { return Ok(()); }
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let applied = initial_event(&fixture(), 1);
        insert_applied(&*tx, &applied)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn projection_at(&self, target: Option<&str>) -> Result<Projection, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let events = load_events(&conn)?;
        drop(conn);
        replay(&events, target)
    }

    pub fn latest(&self) -> Result<Projection, String> { self.projection_at(None) }

    pub fn append(&self, request: CommandRequest) -> Result<AppliedEvent, (u16, String)> {
        let mut conn = self.conn.lock().map_err(|e| (500, e.to_string()))?;
        let tx = conn.transaction().map_err(|e| (500, e.to_string()))?;
        let events = load_events(&tx).map_err(|e| (500, e))?;
        let previous = replay(&events, None).map_err(|e| (500, e))?;
        let seq = events.len() + 1;
        match crate::engine::apply_command(&previous, request, seq) {
            Ok(applied) => {
                insert_applied(&*tx, &applied).map_err(|e| (500, e))?;
                tx.commit().map_err(|e| (500, e.to_string()))?;
                Ok(applied)
            }
            Err((status, message, Some(applied))) => {
                insert_applied(&*tx, &applied).map_err(|e| (500, e))?;
                tx.commit().map_err(|e| (500, e.to_string()))?;
                Err((status, message))
            }
            Err((status, message, None)) => Err((status, message)),
        }
    }

    pub fn events(&self) -> Result<Vec<EventRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        load_events(&conn)
    }

    pub fn export(&self) -> Result<serde_json::Value, String> {
        let events = self.events()?;
        Ok(serde_json::json!({"format":"saxs-event-bundle/v1","events":events}))
    }

    pub fn import_bundle(&self, bundle: serde_json::Value) -> Result<usize, String> {
        let events_json = bundle.get("events").and_then(|v| v.as_array()).ok_or("missing events array")?;
        let mut events = Vec::new();
        for value in events_json {
            let event: EventRecord = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            events.push(event);
        }
        verify_chain(&events)?;
        let _ = replay(&events, None)?;
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute_batch("DELETE FROM current_nodes; DELETE FROM node_versions; DELETE FROM audit_tail; DELETE FROM event_parents; DELETE FROM events; DELETE FROM metadata;").map_err(|e| e.to_string())?;
        let projection = replay(&events, None).map_err(|e| e.to_string())?;
        for event in &events {
            insert_event(&tx, event)?;
        }
        for node in projection.nodes.values() {
            insert_node(&tx, node, &event_id_for_version(&events, node))?;
        }
        tx.execute("INSERT INTO audit_tail(event_id, completed_at) VALUES (?1, ?2)", params![events.last().unwrap().event_id, events.last().unwrap().timestamp]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(events.len())
    }
}

fn event_id_for_version(events: &[EventRecord], node: &crate::domain::GraphNode) -> String {
    let _ = events;
    node.created_by_event.clone()
}

pub fn load_events(conn: &Connection) -> Result<Vec<EventRecord>, String> {
    let mut stmt = conn.prepare("SELECT json FROM events ORDER BY seq").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
    let mut events = Vec::new();
    for row in rows { events.push(serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?); }
    Ok(events)
}

fn insert_event<C: Borrow<Connection>>(conn: &C, event: &EventRecord) -> Result<(), String> {
    let seq: u32 = event.event_id.trim_start_matches('E').parse::<u32>().map_err(|e| e.to_string())?;
    let conn = conn.borrow();
    let json = serde_json::to_string(event).map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO events(seq,event_id,json,hash) VALUES (?1,?2,?3,?4)", params![seq, event.event_id, json, event.hash]).map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO event_parents(event_id,parent_event_id,hash_chain) VALUES (?1,?2,?3)", params![event.event_id, event.parent_event_id, event.hash]).map_err(|e| e.to_string())?;
    Ok(())
}

fn insert_node(conn: &Connection, node: &crate::domain::GraphNode, created_event: &str) -> Result<(), String> {
    let json = serde_json::to_string(node).map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO node_versions(node_id,version,fingerprint,json,created_event) VALUES (?1,?2,?3,?4,?5)", params![node.id,node.version,node.fingerprint,json,created_event]).map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO current_nodes(node_id,version,fingerprint) VALUES (?1,?2,?3) ON CONFLICT(node_id) DO UPDATE SET version=excluded.version, fingerprint=excluded.fingerprint", params![node.id,node.version,node.fingerprint]).map_err(|e| e.to_string())?;
    Ok(())
}

fn insert_applied(conn: &Connection, applied: &AppliedEvent) -> Result<(), String> {
    insert_event(conn, &applied.event)?;
    let mut graph = crate::pipeline::rebuild_graph(
        applied.state.case.as_ref().ok_or("missing case")?,
        &applied.event.event_id,
        &applied.state.decisions,
        &applied.state.conflicts,
    );
    let previous_events: Vec<EventRecord> = applied.state.events.iter().take(applied.state.events.len().saturating_sub(1)).cloned().collect();
    if !previous_events.is_empty() {
        if let Ok(previous) = replay(&previous_events, None) {
            for (id, node) in &mut graph {
                if let Some(old) = previous.nodes.get(id) {
                    if old.fingerprint == node.fingerprint { node.version = old.version; } else { node.version = old.version + 1; }
                }
            }
        }
    }
    for node in graph.values() { insert_node(conn, node, &applied.event.event_id)?; }
    conn.execute("INSERT INTO audit_tail(event_id, completed_at) VALUES (?1,?2)", params![applied.event.event_id, applied.event.timestamp]).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn verify_chain(events: &[EventRecord]) -> Result<(), String> {
    let mut parent = None;
    for (idx, event) in events.iter().enumerate() {
        if event.event_id != format!("E{:04}", idx + 1) { return Err("non-contiguous event identity".into()); }
        if event.parent_event_id != parent { return Err("broken parent chain".into()); }
        parent = Some(event.event_id.clone());
    }
    Ok(())
}

pub fn create_legacy_v1(path: &str) -> Result<(), String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "DELETE").map_err(|e| e.to_string())?;
    conn.execute_batch("CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE events (seq INTEGER PRIMARY KEY CHECK (seq > 0), event_id TEXT NOT NULL UNIQUE, json TEXT NOT NULL, hash TEXT NOT NULL); CREATE TABLE event_parents (event_id TEXT PRIMARY KEY, parent_event_id TEXT, hash_chain TEXT NOT NULL); PRAGMA user_version=1;").map_err(|e| e.to_string())?;
    let applied = initial_event(&fixture(), 1);
    let tx_conn = conn;
    insert_event(&tx_conn, &applied.event)?;
    Ok(())
}
