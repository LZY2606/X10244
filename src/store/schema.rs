use rusqlite::{Connection, Transaction};

pub fn create_legacy_v1(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        PRAGMA user_version = 1;
        CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE slots (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            label TEXT NOT NULL
        );
        CREATE TABLE nodes (
            id TEXT PRIMARY KEY,
            slot_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            version INTEGER NOT NULL,
            rev INTEGER NOT NULL,
            fingerprint TEXT NOT NULL,
            parents TEXT NOT NULL DEFAULT '[]',
            rule_version TEXT,
            params TEXT NOT NULL DEFAULT '{}',
            payload TEXT NOT NULL
        );
        CREATE TABLE pointer_log (
            rev INTEGER NOT NULL,
            slot_id TEXT NOT NULL,
            node_id TEXT NOT NULL,
            PRIMARY KEY (rev, slot_id)
        );
        CREATE TABLE events (
            id TEXT PRIMARY KEY,
            rev INTEGER NOT NULL,
            seq INTEGER NOT NULL UNIQUE,
            kind TEXT NOT NULL,
            payload TEXT NOT NULL
        );
        CREATE TABLE revisions (
            rev INTEGER PRIMARY KEY,
            event_id TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| e.to_string())
}

pub fn init_or_migrate(conn: &mut Connection) -> Result<(), String> {
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .map_err(|e| e.to_string())?;
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);
    if version == 0 {
        create_current(conn)?;
    } else if version == 1 {
        migrate_1_to_2(conn)?;
    } else if version != super::SCHEMA_VERSION {
        return Err(format!("未知 schema 版本: {version}"));
    }
    Ok(())
}

pub fn create_current(conn: &mut Connection) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute_batch(
        r#"
        PRAGMA user_version = 2;
        CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE slots (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            label TEXT NOT NULL
        );
        CREATE TABLE nodes (
            id TEXT PRIMARY KEY,
            slot_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            version INTEGER NOT NULL,
            rev INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'valid',
            fingerprint TEXT NOT NULL,
            parents TEXT NOT NULL DEFAULT '[]',
            rule_version TEXT,
            params TEXT NOT NULL DEFAULT '{}',
            payload TEXT NOT NULL,
            diagnostics TEXT NOT NULL DEFAULT '[]'
        );
        CREATE TABLE pointer_log (
            rev INTEGER NOT NULL,
            slot_id TEXT NOT NULL,
            node_id TEXT NOT NULL,
            PRIMARY KEY (rev, slot_id)
        );
        CREATE TABLE events (
            id TEXT PRIMARY KEY,
            rev INTEGER NOT NULL,
            seq INTEGER NOT NULL UNIQUE,
            kind TEXT NOT NULL,
            actor TEXT NOT NULL DEFAULT 'reviewer',
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE revisions (
            rev INTEGER PRIMARY KEY,
            event_id TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX idx_nodes_slot ON nodes(slot_id, version);
        CREATE INDEX idx_pointer_rev ON pointer_log(rev);
        CREATE INDEX idx_events_rev ON events(rev);
        "#,
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

fn migrate_1_to_2(conn: &mut Connection) -> Result<(), String> {
    let tx: Transaction = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute_batch(
        r#"
        ALTER TABLE nodes ADD COLUMN status TEXT NOT NULL DEFAULT 'valid';
        ALTER TABLE nodes ADD COLUMN diagnostics TEXT NOT NULL DEFAULT '[]';
        ALTER TABLE events ADD COLUMN actor TEXT NOT NULL DEFAULT 'legacy';
        ALTER TABLE events ADD COLUMN created_at TEXT NOT NULL DEFAULT 'legacy-import';
        CREATE INDEX IF NOT EXISTS idx_nodes_slot ON nodes(slot_id, version);
        CREATE INDEX IF NOT EXISTS idx_pointer_rev ON pointer_log(rev);
        CREATE INDEX IF NOT EXISTS idx_events_rev ON events(rev);
        PRAGMA user_version = 2;
        "#,
    )
    .map_err(|e| e.to_string())?;

    let last_rev: i64 = tx
        .query_row("SELECT COALESCE(MAX(rev),0) FROM revisions", [], |r| r.get(0))
        .unwrap_or(0);
    let last_seq: i64 = tx
        .query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0))
        .unwrap_or(0);
    let rev = last_rev + 1;
    let seq = last_seq + 1;
    let event_id = "evt_migration_v1_v2";
    let created_at = "migration";
    tx.execute(
        "INSERT INTO events (id, rev, seq, kind, actor, payload, created_at)
         VALUES (?1, ?2, ?3, 'migration', 'system',
         '{\"from\":1,\"to\":2,\"note\":\"旧版本迁移：节点补充 status/diagnostics，事件补充 actor/created_at\"}', ?4)",
        rusqlite::params![event_id, rev, seq, created_at],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO revisions (rev, event_id, created_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![rev, event_id, created_at],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}
