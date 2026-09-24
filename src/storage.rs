use crate::events::{Event, EventKind};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;
use std::sync::Mutex;

#[derive(Debug)]
pub enum StorageError {
    Sqlite(rusqlite::Error),
    Conflict {
        expected: Vec<String>,
        actual: Vec<String>,
    },
    Invalid(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Sqlite(error) => write!(formatter, "{error}"),
            StorageError::Conflict { expected, actual } => {
                write!(
                    formatter,
                    "expected heads {expected:?}, found heads {actual:?}"
                )
            }
            StorageError::Invalid(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        StorageError::Sqlite(error)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventEnvelope {
    pub events: Vec<Event>,
}

#[derive(Debug)]
pub struct Storage {
    connection: Mutex<Connection>,
}

impl Storage {
    pub fn open(path: &str) -> Result<Self, StorageError> {
        let mut connection = Connection::open(path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn in_memory() -> Result<Self, StorageError> {
        let mut connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn load_events(&self) -> Result<Vec<Event>, StorageError> {
        let connection = self.connection.lock().expect("storage lock");
        let mut statement = connection.prepare(
            "SELECT id, kind, timestamp, actor, revision, payload FROM events ORDER BY revision ASC, id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            let id: String = row.get(0)?;
            let kind: EventKind = serde_json::from_str(&row.get::<_, String>(1)?).unwrap();
            let timestamp: String = row.get(2)?;
            let actor: String = row.get(3)?;
            let revision: i64 = row.get(4)?;
            let payload = serde_json::from_str(&row.get::<_, String>(5)?).unwrap();
            Ok((id, kind, timestamp, actor, revision, payload))
        })?;
        let mut by_id = std::collections::HashMap::new();
        for row in rows {
            let (id, kind, timestamp, actor, revision, payload) = row?;
            by_id.insert(
                id.clone(),
                Event {
                    id,
                    parent_ids: Vec::new(),
                    kind,
                    timestamp,
                    actor,
                    revision,
                    payload,
                },
            );
        }
        let mut parent_statement = connection
            .prepare("SELECT event_id, parent_id FROM event_parents ORDER BY position ASC")?;
        let parent_rows = parent_statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in parent_rows {
            let (event_id, parent_id) = row?;
            if let Some(event) = by_id.get_mut(&event_id) {
                event.parent_ids.push(parent_id);
            }
        }
        Ok(by_id.into_values().collect())
    }

    pub fn append_event(
        &self,
        mut event: Event,
        expected_heads: Option<Vec<String>>,
    ) -> Result<Event, StorageError> {
        let mut connection = self.connection.lock().expect("storage lock");
        let transaction = connection.transaction()?;
        let next_revision = transaction.query_row(
            "SELECT COALESCE(MAX(revision), 0) + 1 FROM events",
            [],
            |row| row.get(0),
        )?;
        event.revision = next_revision;
        validate_parents(&transaction, &event)?;
        if let Some(expected) = expected_heads {
            let actual = current_heads(&transaction)?;
            if expected != actual {
                return Err(StorageError::Conflict { expected, actual });
            }
        }
        insert_event(&transaction, &event)?;
        transaction.commit()?;
        Ok(event)
    }

    pub fn append_conflicting_event(
        &self,
        mut event: Event,
        expected_heads: Vec<String>,
        conflict_actor: &str,
    ) -> Result<(Event, Event), StorageError> {
        let mut connection = self.connection.lock().expect("storage lock");
        let transaction = connection.transaction()?;
        let mut next_revision = transaction.query_row(
            "SELECT COALESCE(MAX(revision), 0) + 1 FROM events",
            [],
            |row| row.get(0),
        )?;
        event.revision = next_revision;
        validate_parents(&transaction, &event)?;
        insert_event(&transaction, &event)?;
        let actual = current_heads(&transaction)?;
        let conflict_id = format!("conflict:{}", event.id);
        next_revision += 1;
        let mut parents = actual.clone();
        if !parents.contains(&event.id) {
            parents.push(event.id.clone());
        }
        let conflict = Event {
            id: conflict_id,
            parent_ids: parents,
            kind: EventKind::ConflictDetected,
            timestamp: event.timestamp.clone(),
            actor: conflict_actor.to_string(),
            revision: next_revision,
            payload: crate::events::EventPayload::ConflictDetected {
                fork_id: format!("fork:{}", event.id),
                expected_parent: expected_heads.first().cloned().unwrap_or_default(),
                actual_parent: actual.first().cloned().unwrap_or_default(),
                actor: event.actor.clone(),
                summary: "Two reviewers edited the same event parent; choose a branch to continue."
                    .to_string(),
            },
        };
        insert_event(&transaction, &conflict)?;
        transaction.commit()?;
        Ok((event, conflict))
    }

    pub fn replace_with_events(&self, events: Vec<Event>) -> Result<(), StorageError> {
        let mut connection = self.connection.lock().expect("storage lock");
        let transaction = connection.transaction()?;
        transaction.execute_batch("DELETE FROM event_parents; DELETE FROM events; DELETE FROM sqlite_sequence WHERE name IN ('events', 'event_parents');")?;
        for (revision, mut event) in events.into_iter().enumerate() {
            event.revision = revision as i64 + 1;
            validate_parents(&transaction, &event)?;
            insert_event(&transaction, &event)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn export_json(&self) -> Result<String, StorageError> {
        Ok(serde_json::to_string_pretty(&EventEnvelope {
            events: self.load_events()?,
        })
        .unwrap())
    }

    pub fn import_json(&self, json: &str) -> Result<usize, StorageError> {
        let envelope: EventEnvelope =
            serde_json::from_str(json).map_err(|error| StorageError::Invalid(error.to_string()))?;
        self.replace_with_events(envelope.events.clone())?;
        Ok(envelope.events.len())
    }

    pub fn event_count(&self) -> Result<i64, StorageError> {
        let connection = self.connection.lock().expect("storage lock");
        Ok(connection.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?)
    }
}

fn validate_parents(
    transaction: &rusqlite::Transaction,
    event: &Event,
) -> Result<(), StorageError> {
    if event.parent_ids.iter().collect::<HashSet<_>>().len() != event.parent_ids.len() {
        return Err(StorageError::Invalid("duplicate parent ids".to_string()));
    }
    for parent in &event.parent_ids {
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE id = ?1)",
            [parent],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(StorageError::Invalid(format!(
                "missing parent event {parent}"
            )));
        }
    }
    let duplicate_id: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM events WHERE id = ?1)",
        [&event.id],
        |row| row.get(0),
    )?;
    if duplicate_id {
        return Err(StorageError::Invalid(format!(
            "duplicate event id {}",
            event.id
        )));
    }
    Ok(())
}

fn insert_event(transaction: &rusqlite::Transaction, event: &Event) -> Result<(), StorageError> {
    transaction.execute(
        "INSERT INTO events (id, kind, timestamp, actor, revision, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            event.id,
            serde_json::to_string(&event.kind).unwrap(),
            event.timestamp,
            event.actor,
            event.revision,
            serde_json::to_string(&event.payload).unwrap(),
        ],
    )?;
    for (position, parent) in event.parent_ids.iter().enumerate() {
        transaction.execute(
            "INSERT INTO event_parents (event_id, parent_id, position) VALUES (?1, ?2, ?3)",
            rusqlite::params![event.id, parent, position as i64],
        )?;
    }
    Ok(())
}

fn current_heads(transaction: &rusqlite::Transaction) -> Result<Vec<String>, StorageError> {
    let mut statement = transaction.prepare(
        "SELECT id FROM events e WHERE NOT EXISTS (SELECT 1 FROM event_parents p WHERE p.parent_id = e.id) ORDER BY id",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn migrate(connection: &mut Connection) -> Result<(), StorageError> {
    let user_version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if user_version > 1 {
        return Err(StorageError::Invalid(format!(
            "unsupported database version {user_version}"
        )));
    }
    if user_version == 0 {
        let has_legacy: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='saxs_events_v0'",
            [],
            |row| row.get(0),
        )?;
        connection.execute_batch(include_str!("../migrations/001_init.sql"))?;
        if has_legacy == 1 {
            migrate_legacy_events(connection)?;
        }
        connection.pragma_update(None, "user_version", 1)?;
    }
    Ok(())
}

fn migrate_legacy_events(connection: &mut Connection) -> Result<(), StorageError> {
    let mut legacy: Vec<(String, String, String, String, String)> = {
        let mut statement = connection.prepare("SELECT id, parent_id, actor, created_at, event_json FROM saxs_events_v0 ORDER BY seq ASC")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (revision, (id, parent, actor, created_at, event_json)) in legacy.drain(..).enumerate() {
        let mut event: Event = serde_json::from_str(&event_json)
            .map_err(|error| StorageError::Invalid(error.to_string()))?;
        event.id = id;
        event.parent_ids = if parent.is_empty() {
            Vec::new()
        } else {
            vec![parent]
        };
        event.actor = actor;
        event.timestamp = created_at;
        event.revision = revision as i64 + 1;
        insert_event(&connection.transaction()?, &event)?;
    }
    Ok(())
}

#[cfg(test)]
pub fn simulate_interrupted_write(connection: &mut Connection) -> Result<(), StorageError> {
    let transaction = connection.transaction()?;
    transaction.execute_batch("CREATE TABLE IF NOT EXISTS write_intent (note TEXT); INSERT INTO write_intent VALUES ('interrupted')")?;
    drop(transaction);
    Ok(())
}

pub fn latest_event_id(connection: &Connection) -> Result<Option<String>, StorageError> {
    Ok(connection
        .query_row(
            "SELECT id FROM events ORDER BY revision DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?)
}
