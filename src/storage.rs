use crate::state::{Action, AppState};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;

const SCHEMA_VERSION: i64 = 3;

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportBundle {
    pub format: String,
    pub storage_schema_version: i64,
    pub action_count: usize,
    pub initial_case_id: String,
    pub state: AppState,
    pub actions: Vec<Action>,
}

pub fn open(path: &Path) -> Result<Connection, String> {
    let mut connection = Connection::open(path).map_err(|error| error.to_string())?;
    migrate(&mut connection)?;
    Ok(connection)
}

pub fn open_in_memory() -> Result<Connection, String> {
    let mut connection = Connection::open_in_memory().map_err(|error| error.to_string())?;
    migrate(&mut connection)?;
    Ok(connection)
}

pub fn migrate(connection: &mut Connection) -> Result<(), String> {
    let tx = connection.transaction().map_err(|error| error.to_string())?;
    tx.execute_batch(
        r#"
        PRAGMA journal_mode=WAL;
        PRAGMA foreign_keys=ON;
        "#,
    )
    .map_err(|error| error.to_string())?;
    let legacy_user_version: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap_or(0);
    if legacy_user_version == 1 {
        tx.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS versions(version_id INTEGER PRIMARY KEY, event_count INTEGER NOT NULL, state_json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS events(event_id INTEGER PRIMARY KEY, version_id INTEGER NOT NULL, event_type TEXT NOT NULL, basis_code TEXT NOT NULL, message TEXT NOT NULL, payload_json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS actions(seq INTEGER PRIMARY KEY AUTOINCREMENT, action_json TEXT NOT NULL, resulting_version INTEGER NOT NULL);
            "#,
        )
        .map_err(|error| error.to_string())?;
    } else if legacy_user_version == 2 {
        tx.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS versions(version_id INTEGER PRIMARY KEY, event_count INTEGER NOT NULL, state_json TEXT NOT NULL, replay_actions_json TEXT NOT NULL DEFAULT '[]');
            CREATE TABLE IF NOT EXISTS events(event_id INTEGER PRIMARY KEY, version_id INTEGER NOT NULL, event_type TEXT NOT NULL, basis_code TEXT NOT NULL, message TEXT NOT NULL, payload_json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS actions(seq INTEGER PRIMARY KEY AUTOINCREMENT, action_json TEXT NOT NULL, resulting_version INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS stable_identity_audit(stable_id TEXT PRIMARY KEY, kind TEXT NOT NULL, first_version INTEGER NOT NULL, last_version INTEGER NOT NULL, source_refs_json TEXT NOT NULL);
            "#,
        )
        .map_err(|error| error.to_string())?;
    } else {
        tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS versions (
            version_id INTEGER PRIMARY KEY,
            event_count INTEGER NOT NULL,
            state_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        );
        CREATE TABLE IF NOT EXISTS events (
            event_id INTEGER PRIMARY KEY,
            version_id INTEGER NOT NULL,
            event_type TEXT NOT NULL,
            basis_code TEXT NOT NULL,
            message TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            FOREIGN KEY(version_id) REFERENCES versions(version_id)
        );
        CREATE TABLE IF NOT EXISTS actions (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            action_json TEXT NOT NULL,
            resulting_version INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS stable_identity_audit (
            stable_id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            first_version INTEGER NOT NULL,
            last_version INTEGER NOT NULL,
            source_refs_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS locks (
            lock_name TEXT PRIMARY KEY,
            holder TEXT NOT NULL,
            version_id INTEGER NOT NULL
        );
        "#,
    )
        .map_err(|error| error.to_string())?;
    }
    let current: Option<i64> = tx
        .query_row("SELECT value FROM schema_meta WHERE key='schema_version'", [], |row| row.get(0))
        .ok()
        .and_then(|value: String| value.parse().ok());
    match current {
        Some(version) if version == SCHEMA_VERSION => {}
        Some(1) => {
            tx.execute_batch("ALTER TABLE versions ADD COLUMN replay_actions_json TEXT NOT NULL DEFAULT '[]';")
                .map_err(|error| error.to_string())?;
            tx.execute_batch("CREATE TABLE IF NOT EXISTS stable_identity_audit (stable_id TEXT PRIMARY KEY, kind TEXT NOT NULL, first_version INTEGER NOT NULL, last_version INTEGER NOT NULL, source_refs_json TEXT NOT NULL);")
                .map_err(|error| error.to_string())?;
            tx.execute_batch("CREATE TABLE IF NOT EXISTS locks (lock_name TEXT PRIMARY KEY, holder TEXT NOT NULL, version_id INTEGER NOT NULL);")
                .map_err(|error| error.to_string())?;
            tx.execute("INSERT OR REPLACE INTO schema_meta(key,value) VALUES('schema_version', ?1)", params![SCHEMA_VERSION.to_string()])
                .map_err(|error| error.to_string())?;
        }
        Some(2) => {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS locks (lock_name TEXT PRIMARY KEY, holder TEXT NOT NULL, version_id INTEGER NOT NULL);")
                .map_err(|error| error.to_string())?;
            tx.execute("INSERT OR REPLACE INTO schema_meta(key,value) VALUES('schema_version', ?1)", params![SCHEMA_VERSION.to_string()])
                .map_err(|error| error.to_string())?;
        }
        Some(other) if other > SCHEMA_VERSION => return Err(format!("database schema {other} is newer than application")),
        Some(_) => unreachable!(),
        None => {
            tx.execute(
                "INSERT INTO schema_meta(key,value) VALUES('schema_version', ?1)",
                params![SCHEMA_VERSION.to_string()],
            )
            .map_err(|error| error.to_string())?;
        }
    }
    tx.commit().map_err(|error| error.to_string())
}

pub fn save_state(connection: &mut Connection, state: &AppState, actions: &[Action]) -> Result<i64, String> {
    let tx = connection.transaction().map_err(|error| error.to_string())?;
    let state_json = serde_json::to_string(state).map_err(|error| error.to_string())?;
    let actions_json = serde_json::to_string(actions).map_err(|error| error.to_string())?;
    let previous_version = current_version_id(&tx)?;
    tx.execute(
        "INSERT INTO versions(version_id, event_count, state_json, replay_actions_json) VALUES(?1, ?2, ?3, ?4)",
        params![state.current_version(), state.events.len(), state_json, actions_json],
    )
    .map_err(|error| error.to_string())?;
    for event in &state.events {
        tx.execute(
            "INSERT OR REPLACE INTO events(event_id, version_id, event_type, basis_code, message, payload_json) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                event.event_id,
                state.current_version(),
                event.event_type,
                event.basis_code,
                event.message,
                serde_json::to_string(event).unwrap_or_default()
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    for (seq, action) in actions.iter().enumerate() {
        tx.execute(
            "INSERT OR REPLACE INTO actions(seq, action_json, resulting_version) VALUES(?1,?2,?3)",
            params![(seq + 1) as i64, serde_json::to_string(action).unwrap_or_default(), state.current_version()],
        )
        .map_err(|error| error.to_string())?;
    }
    let _ = previous_version;
    write_identity_audit(&tx, state)?;
    tx.commit().map_err(|error| error.to_string())?;
    current_version_id(connection).map_err(|error| error.to_string())
}

pub fn save_state_atomic_barrier(connection: &mut Connection, state: &AppState, actions: &[Action], crash_after_insert: bool) -> Result<(), String> {
    let tx = connection.transaction().map_err(|error| error.to_string())?;
    let state_json = serde_json::to_string(state).map_err(|error| error.to_string())?;
    let actions_json = serde_json::to_string(actions).map_err(|error| error.to_string())?;
    tx.execute(
        "INSERT INTO versions(version_id, event_count, state_json, replay_actions_json) VALUES(?1,?2,?3,?4)",
        params![state.current_version(), state.events.len(), state_json, actions_json],
    )
    .map_err(|error| error.to_string())?;
    if crash_after_insert {
        tx.rollback().map_err(|error| error.to_string())?;
        return Err("simulated write interruption before commit".to_string());
    }
    tx.commit().map_err(|error| error.to_string())
}

fn current_version_id(connection: &Connection) -> rusqlite::Result<i64> {
    connection.query_row("SELECT COALESCE(MAX(version_id),0) FROM versions", [], |row| row.get(0))
}

pub fn load_latest(connection: &Connection) -> Result<Option<AppState>, String> {
    let json: Option<String> = connection
        .query_row(
            "SELECT state_json FROM versions ORDER BY version_id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .map(|value| Some(value))
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|error| error.to_string())?;
    json.map(|value| serde_json::from_str(&value).map_err(|error| error.to_string())).transpose()
}

pub fn load_version(connection: &Connection, version: i64) -> Result<AppState, String> {
    let json: String = connection
        .query_row("SELECT state_json FROM versions WHERE version_id=?1", params![version], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&json).map_err(|error| error.to_string())
}

pub fn load_actions(connection: &Connection) -> Result<Vec<Action>, String> {
    let json: String = connection
        .query_row("SELECT replay_actions_json FROM versions ORDER BY version_id DESC LIMIT 1", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&json).map_err(|error| error.to_string())
}

pub fn export_bundle(state: &AppState, actions: &[Action]) -> Result<Vec<u8>, String> {
    let bundle = ExportBundle {
        format: "saxs-provenance-bench/v3".to_string(),
        storage_schema_version: SCHEMA_VERSION,
        action_count: actions.len(),
        initial_case_id: state.case_definition.case_id.clone(),
        state: state.clone(),
        actions: actions.to_vec(),
    };
    serde_json::to_vec_pretty(&bundle).map_err(|error| error.to_string())
}

pub fn import_bundle(bytes: &[u8]) -> Result<(AppState, Vec<Action>), String> {
    let bundle: ExportBundle = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if bundle.storage_schema_version > SCHEMA_VERSION {
        return Err("export bundle is from a newer schema".to_string());
    }
    Ok((bundle.state, bundle.actions))
}

fn write_identity_audit(connection: &rusqlite::Transaction<'_>, state: &AppState) -> Result<(), String> {
    let version = state.current_version();
    for curve in state.source_versions.values() {
        for point in &curve.points {
            let refs = serde_json::json!([curve.current_version, curve.source_file, point.point_id]).to_string();
            connection
                .execute(
                    "INSERT INTO stable_identity_audit(stable_id,kind,first_version,last_version,source_refs_json)
                     VALUES(?1,'raw_point',?2,?3,?4)
                     ON CONFLICT(stable_id) DO UPDATE SET last_version=excluded.last_version, source_refs_json=excluded.source_refs_json",
                    params![format!("raw:{}:{}", curve.current_version, point.point_id), version, version, refs],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    for curve in state.derived.normalized.values() {
        for point in &curve.points {
            let refs = serde_json::json!([curve.raw_curve_version, point.source_point_id, point.stable_id]).to_string();
            connection
                .execute(
                    "INSERT INTO stable_identity_audit(stable_id,kind,first_version,last_version,source_refs_json)
                     VALUES(?1,'normalized_point',?2,?3,?4)
                     ON CONFLICT(stable_id) DO UPDATE SET last_version=excluded.last_version, source_refs_json=excluded.source_refs_json",
                    params![point.stable_id, version, version, refs],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    for subtraction in &state.derived.subtraction_candidates {
        for point in &subtraction.points {
            let refs = serde_json::json!([
                subtraction.sample_version,
                subtraction.buffer_version,
                point.sample_point_id,
                point.buffer_low_id,
                point.buffer_high_id
            ])
            .to_string();
            connection
                .execute(
                    "INSERT INTO stable_identity_audit(stable_id,kind,first_version,last_version,source_refs_json)
                     VALUES(?1,'subtracted_point',?2,?3,?4)
                     ON CONFLICT(stable_id) DO UPDATE SET last_version=excluded.last_version, source_refs_json=excluded.source_refs_json",
                    params![point.stable_id, version, version, refs],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
