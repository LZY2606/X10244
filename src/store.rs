use crate::domain::{Event, EventKind, ProjectState};
use crate::engine::replay;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("optimistic lock failed for branch {0}; expected event {1}, current event {2}")]
    Conflict(String, String, String),
    #[error("branch {0} is not known")]
    UnknownBranch(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PublishInput {
    pub branch: String,
    pub parent_event_id: String,
    pub actor: String,
    pub kind: EventKind,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ForkInput {
    pub source_branch: String,
    pub new_branch: String,
    pub at_event_id: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ExportBundle {
    pub format: String,
    pub schema_version: u32,
    pub events: Vec<Event>,
    pub branches: Vec<(String, Option<String>)>,
}

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &str) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        migrate(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn seed_case(&self) -> Result<()> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let count: i64 =
            connection.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        if count == 0 {
            let bundle: ExportBundle =
                serde_json::from_str(include_str!("../cases/saxs-station/case.json"))?;
            import_bundle_locked(&connection, &bundle)?;
        }
        Ok(())
    }

    pub fn publish(&self, input: &PublishInput) -> Result<Event> {
        let mut connection = self.connection.lock().expect("store mutex poisoned");
        let transaction = connection.transaction()?;
        let current_head = head_event_locked(&transaction, &input.branch)?;
        if current_head != Some(input.parent_event_id.clone()) {
            return Err(StoreError::Conflict(
                input.branch.clone(),
                input.parent_event_id.clone(),
                current_head.unwrap_or_else(|| "<empty>".to_string()),
            ));
        }
        let sequence = event_count_locked(&transaction) + 1;
        let event_id = format!("evt-{:04}", sequence);
        let event = Event {
            event_id: event_id.clone(),
            parent_event_id: current_head.clone(),
            branch: input.branch.clone(),
            actor: input.actor.clone(),
            created_at: "2026-09-24T00:00:00Z".to_string(),
            kind: input.kind.clone(),
        };
        let payload = serde_json::to_string(&event)?;
        transaction.execute(
            "INSERT INTO events(event_id, parent_event_id, branch, actor, created_at, kind, sequence_number, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event.event_id,
                event.parent_event_id,
                event.branch,
                event.actor,
                event.created_at,
                serde_json::to_string(&event.kind)?,
                sequence,
                payload
            ],
        )?;
        transaction.execute(
            "INSERT INTO refs(branch, head_event_id) VALUES (?1, ?2)
             ON CONFLICT(branch) DO UPDATE SET head_event_id = excluded.head_event_id",
            params![event.branch, event.event_id],
        )?;
        transaction.commit()?;
        Ok(event)
    }

    pub fn fork(&self, input: &ForkInput) -> Result<()> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let source_head = head_event_locked(&connection, &input.source_branch)?
            .ok_or_else(|| StoreError::UnknownBranch(input.source_branch.clone()))?;
        let head = input.at_event_id.clone().unwrap_or(source_head);
        ensure_event_locked(&connection, &head)?;
        connection.execute(
            "INSERT INTO refs(branch, head_event_id) VALUES (?1, ?2)
             ON CONFLICT(branch) DO NOTHING",
            params![input.new_branch, head],
        )?;
        Ok(())
    }

    pub fn branches(&self) -> Result<Vec<(String, Option<String>)>> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let mut statement =
            connection.prepare("SELECT branch, head_event_id FROM refs ORDER BY branch")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn events_for_branch(&self, branch: &str) -> Result<Vec<Event>> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let mut current = head_event_locked(&connection, branch)?;
        let mut reversed = Vec::new();
        while let Some(event_id) = current {
            let payload: String = connection.query_row(
                "SELECT payload FROM events WHERE event_id = ?1",
                params![event_id],
                |row| row.get(0),
            )?;
            let event: Event = serde_json::from_str(&payload)?;
            current = event.parent_event_id.clone();
            reversed.push(event);
        }
        reversed.reverse();
        Ok(reversed)
    }

    pub fn state(&self, branch: &str) -> Result<ProjectState> {
        Ok(replay(&self.events_for_branch(branch)?))
    }

    pub fn export(&self) -> Result<ExportBundle> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let mut statement =
            connection.prepare("SELECT payload FROM events ORDER BY sequence_number, event_id")?;
        let rows = statement.query_map([], |row| {
            let payload: String = row.get(0)?;
            Ok(payload)
        })?;
        let mut events = Vec::new();
        for row in rows {
            events.push(serde_json::from_str(&row?)?);
        }
        let mut refs_statement =
            connection.prepare("SELECT branch, head_event_id FROM refs ORDER BY branch")?;
        let refs = refs_statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut branches = Vec::new();
        for branch in refs {
            branches.push(branch?);
        }
        Ok(ExportBundle {
            format: "saxs-station-export/1".to_string(),
            schema_version: 2,
            events,
            branches,
        })
    }

    pub fn import_bundle(&self, bundle: &ExportBundle) -> Result<()> {
        let mut connection = self.connection.lock().expect("store mutex poisoned");
        let transaction = connection.transaction()?;
        import_bundle_locked(&transaction, bundle)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn reset_for_demo(&self) -> Result<()> {
        let mut connection = self.connection.lock().expect("store mutex poisoned");
        let transaction = connection.transaction()?;
        transaction
            .execute_batch("DELETE FROM refs; DELETE FROM events; DELETE FROM schema_meta;")?;
        transaction.commit()?;
        drop(connection);
        self.seed_case()
    }

    pub fn crash_midwrite(&self, branch: &str) -> Result<()> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let count = event_count_locked(&connection);
        let next = count + 1;
        let event_id = format!("evt-crash-{:04}", next);
        let payload = format!("{{\"event_id\":\"{}\",\"broken\":", event_id);
        let tx = connection.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO events(event_id, parent_event_id, branch, actor, created_at, kind, sequence_number, payload)
             VALUES (?1, NULL, ?2, 'crash-test', '2026-09-24T00:00:00Z', 'project_created', ?3, ?4)",
            params![event_id, branch, next, payload],
        )?;
        drop(tx);
        Ok(())
    }
}

fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
         INSERT OR IGNORE INTO schema_meta(key, value) VALUES ('version', 1);",
    )?;
    let version: i64 = connection.query_row(
        "SELECT value FROM schema_meta WHERE key = 'version'",
        [],
        |row| row.get(0),
    )?;
    if version >= 2 {
        return Ok(());
    }
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS events(
            event_id TEXT PRIMARY KEY,
            parent_event_id TEXT REFERENCES events(event_id) DEFERRABLE INITIALLY DEFERRED,
            branch TEXT NOT NULL,
            actor TEXT NOT NULL,
            created_at TEXT NOT NULL,
            kind TEXT NOT NULL,
            sequence_number INTEGER UNIQUE,
            payload TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_events_branch ON events(branch);
        CREATE TABLE IF NOT EXISTS refs(
            branch TEXT PRIMARY KEY,
            head_event_id TEXT REFERENCES events(event_id) DEFERRABLE INITIALLY DEFERRED
        );",
    )?;
    let has_legacy = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='legacy_events'")?
        .exists([])?;
    if has_legacy {
        connection.execute_batch(
            "INSERT OR IGNORE INTO events(event_id, parent_event_id, branch, actor, created_at, kind, sequence_number, payload)
             SELECT event_id, parent_event_id, branch, actor, created_at, kind, sequence_number, payload FROM legacy_events;
             INSERT OR IGNORE INTO refs(branch, head_event_id) SELECT branch, head_event_id FROM legacy_refs;",
        )?;
    }
    connection.execute("UPDATE schema_meta SET value = 2 WHERE key = 'version'", [])?;
    Ok(())
}

fn head_event_locked(connection: &Connection, branch: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT head_event_id FROM refs WHERE branch = ?1",
            params![branch],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::Sqlite)
}

fn ensure_event_locked(connection: &Connection, event_id: &str) -> Result<()> {
    let exists: i64 = connection.query_row(
        "SELECT COUNT(*) FROM events WHERE event_id = ?1",
        params![event_id],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Err(StoreError::UnknownBranch(event_id.to_string()));
    }
    Ok(())
}

fn event_count_locked(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap_or(0)
}

fn import_bundle_locked(connection: &Connection, bundle: &ExportBundle) -> Result<()> {
    for (sequence, event) in bundle.events.iter().enumerate() {
        let inferred_sequence: i64 = event
            .event_id
            .strip_prefix("evt-")
            .and_then(|suffix| suffix.parse().ok())
            .unwrap_or(sequence as i64 + 1);
        connection.execute(
            "INSERT OR IGNORE INTO events(event_id, parent_event_id, branch, actor, created_at, kind, sequence_number, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event.event_id,
                event.parent_event_id,
                event.branch,
                event.actor,
                event.created_at,
                serde_json::to_string(&event.kind)?,
                inferred_sequence,
                serde_json::to_string(event)?
            ],
        )?;
    }
    for (branch, head) in &bundle.branches {
        connection.execute(
            "INSERT INTO refs(branch, head_event_id) VALUES (?1, ?2)
             ON CONFLICT(branch) DO UPDATE SET head_event_id = excluded.head_event_id",
            params![branch, head],
        )?;
    }
    let _ = Value::Null;
    Ok(())
}
