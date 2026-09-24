use crate::domain::ArtifactEnvelope;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventRecord {
    pub id: String,
    pub seq: i64,
    pub at: i64,
    pub branch: String,
    pub parent_event_id: Option<String>,
    pub expected_parent_event_id: Option<String>,
    pub event_type: String,
    pub summary: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactRef {
    pub artifact_id: String,
    pub version: u32,
    pub event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RefLog {
    pub branch: String,
    pub logical_id: String,
    pub version: u32,
    pub event_id: String,
    pub seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Package {
    pub package_kind: String,
    pub schema_version: u32,
    pub events: Vec<EventRecord>,
    pub artifacts: Vec<ArtifactEnvelope>,
    pub refs: Vec<ArtifactRef>,
    pub ref_log: Vec<RefLog>,
}

#[derive(Debug, Clone)]
pub struct AppendEvent {
    pub id: String,
    pub branch: String,
    pub expected_parent_event_id: Option<String>,
    pub event_type: String,
    pub summary: String,
    pub payload: Value,
    pub artifacts: Vec<ArtifactEnvelope>,
    pub refs: Vec<(String, u32)>,
}

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Conflict(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Sqlite(e) => write!(f, "{e}"),
            StoreError::Json(e) => write!(f, "{e}"),
            StoreError::Conflict(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StoreError {}
impl From<rusqlite::Error> for StoreError { fn from(value: rusqlite::Error) -> Self { Self::Sqlite(value) } }
impl From<serde_json::Error> for StoreError { fn from(value: serde_json::Error) -> Self { Self::Json(value) } }

impl Store {
    pub fn open(path: &str) -> Result<Self, StoreError> {
        let mut conn = if path == ":memory:" { Connection::open_in_memory()? } else { Connection::open(path)? };
        conn.pragma_update(None, "busy_timeout", 10_000)?;
        conn.pragma_update(None, "foreign_keys", "on")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&mut conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn is_empty(&self) -> Result<bool, StoreError> {
        let conn = self.conn.lock().expect("store mutex");
        let count: i64 = conn.query_row("select count(*) from events", [], |r| r.get(0))?;
        Ok(count == 0)
    }

    pub fn append_event(&self, input: AppendEvent) -> Result<EventRecord, StoreError> {
        let mut conn = self.conn.lock().expect("store mutex");
        let tx = conn.transaction()?;
        let parent: Option<String> = tx
            .query_row(
                "select id from events where branch=?1 order by seq desc limit 1",
                params![input.branch],
                |r| r.get(0),
            )
            .optional()?;
        if input.expected_parent_event_id.is_some() && input.expected_parent_event_id != parent {
            return Err(StoreError::Conflict(format!(
                "branch {} moved from {:?} to {:?}; resolve the conflict before saving",
                input.branch, input.expected_parent_event_id, parent
            )));
        }
        let seq = tx.query_row("select coalesce(max(seq), 0) + 1 from events", [], |r| r.get(0))?;
        let at = input.id.trim_start_matches(|c: char| !c.is_ascii_digit()).chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<i64>().unwrap_or(seq);
        let event = EventRecord {
            id: input.id.clone(),
            seq,
            at,
            branch: input.branch.clone(),
            parent_event_id: parent,
            expected_parent_event_id: input.expected_parent_event_id,
            event_type: input.event_type,
            summary: input.summary,
            payload: input.payload,
        };
        tx.execute(
            "insert into events(id, seq, at, branch, parent_event_id, expected_parent_event_id, event_type, summary, payload)
             values (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![event.id, event.seq, event.at, event.branch, event.parent_event_id, event.expected_parent_event_id, event.event_type, event.summary, event.payload],
        )?;
        for artifact in &input.artifacts {
            if artifact.event_id != event.id {
                return Err(StoreError::Conflict(format!("artifact {} belongs to {} but event is {}", artifact.id, artifact.event_id, event.id)));
            }
            let json = serde_json::to_string(&artifact.artifact)?;
            tx.execute(
                "insert into artifacts(id, version, event_id, created_at, deprecated_by_event, inputs, artifact_json)
                 values (?1,?2,?3,?4,?5,?6,?7)",
                params![artifact.id, artifact.version, artifact.event_id, artifact.created_at, artifact.deprecated_by_event, serde_json::to_string(&artifact.inputs)?, json],
            )?;
        }
        for (logical_id, version) in &input.refs {
            tx.execute("insert into refs(artifact_id, version, event_id) values (?1,?2,?3)", params![logical_id, version, event.id])?;
            tx.execute("insert into ref_log(branch, logical_id, version, event_id, seq) values (?1,?2,?3,?4,?5)", params![input.branch, logical_id, version, event.id, seq])?;
        }
        tx.commit()?;
        Ok(event)
    }
