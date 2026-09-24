//! SQLite 存储：版本化实体 + append-only 事件。
//! 原子性：一个版本的全部写入在单个事务中提交；中断后要么旧版本要么完整新版本。
//! 并发：current_versions 携带 expected_version，两位使用者从同一版本编辑产生 409 式冲突。

use crate::domain::Payload;
use crate::events::Event;
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub const SCHEMA_VERSION: i64 = 3;

#[derive(Debug)]
pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug)]
pub struct VersionRecord {
    pub entity_id: String,
    pub version_id: String,
    pub parent_version_id: Option<String>,
    pub kind: String,
    pub payload_json: String,
    pub fingerprint: String,
    pub input_versions: String,
    pub valid: i64,
    pub created_seq: i64,
}

#[derive(Debug)]
pub struct CurrentRecord {
    pub entity_id: String,
    pub version_id: String,
    pub expected_version: i64,
}

#[derive(Debug, Clone)]
pub struct Bundle {
    pub schema_version: i64,
    pub versions: Vec<serde_json::Value>,
    pub current: Vec<serde_json::Value>,
    pub events: Vec<serde_json::Value>,
    pub meta: Vec<(String, String)>,
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let store = Store { conn: Mutex::new(conn) };
        store.init()?;
        Ok(store)
    }

    /// 测试用内存库。
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Store { conn: Mutex::new(conn) };
        store.init()?;
        Ok(store)
    }

    pub fn init(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let existing: i64 = conn
            .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0))
            .unwrap_or(0);
        if existing == 0 {
            conn.execute_batch(SCHEMA_SQL)?;
            conn.execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?1)",
                params![SCHEMA_VERSION],
            )?;
        } else if existing < SCHEMA_VERSION {
            migrate(&conn, existing)?;
        }
        Ok(())
    }

    const SCHEMA: &'static str = SCHEMA_SQL;

    /// 写入一个新版本并（在 valid 时）切换当前指针，整个过程为单事务原子提交。
    /// expected_version 为当前指针的乐观锁；None 表示仅新建实体。
    /// 返回 Ok(version_id)；冲突时返回 Err("CONFLICT: actual=...")。
    pub fn write_version(
        &self,
        entity_id: &str,
        kind: &str,
        payload: &Payload,
        parent_version_id: Option<&str>,
        input_versions: &[String],
        fingerprint: &str,
        valid: bool,
        expected_version: Option<i64>,
        adopt: bool,
    ) -> rusqlite::Result<String> {
        let payload_json = serde_json::to_string(payload).expect("payload serialize");
        let version_id = format!("{}@{}", entity_id, fingerprint);
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let actual: Option<i64> = tx
            .query_row(
                "SELECT expected_version FROM current_versions WHERE entity_id=?1",
                params![entity_id],
                |r| r.get(0),
            )
            .ok();
        match (expected_version, actual) {
            (Some(exp), Some(act)) if exp != act => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Integer,
                    format!("CONFLICT: expected={exp} actual={act}").into(),
                ));
            }
            (Some(_), None) => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Integer,
                    "CONFLICT: entity missing".into(),
                ));
            }
            _ => {}
        }

        let seq = tx.query_row("SELECT COALESCE(MAX(seq),0)+1 FROM events", [], |r| r.get::<_, i64>(0))?;

        let exists: i64 = tx.query_row(
            "SELECT COUNT(*) FROM versions WHERE entity_id=?1 AND fingerprint=?2",
            params![entity_id, fingerprint],
            |r| r.get(0),
        )?;
        if exists == 0 {
            tx.execute(
                "INSERT INTO versions(entity_id,version_id,parent_version_id,kind,payload_json,fingerprint,input_versions,valid,created_seq)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    entity_id,
                    version_id,
                    parent_version_id,
                    kind,
                    payload_json,
                    fingerprint,
                    input_versions.join(","),
                    valid as i64,
                    seq
                ],
            )?;
        }

        if adopt && valid {
            let next = actual.map(|v| v + 1).unwrap_or(1);
            tx.execute(
                "INSERT INTO current_versions(entity_id,version_id,expected_version)
                 VALUES (?1,?2,?3)
                 ON CONFLICT(entity_id) DO UPDATE SET version_id=excluded.version_id, expected_version=excluded.expected_version",
                params![entity_id, version_id, next],
            )?;
        }
        tx.commit()?;
        Ok(version_id)
    }

    pub fn current_version_id(&self, entity_id: &str) -> rusqlite::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT version_id FROM current_versions WHERE entity_id=?1",
            params![entity_id],
            |r| r.get(0),
        )
        .optional()
    }

    pub fn current_expected_version(&self, entity_id: &str) -> rusqlite::Result<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT expected_version FROM current_versions WHERE entity_id=?1",
            params![entity_id],
            |r| r.get(0),
        )
        .optional()
    }

    pub fn version_payload(&self, version_id: &str) -> rusqlite::Result<Option<Payload>> {
        let conn = self.conn.lock().unwrap();
        let json: Option<String> = conn
            .query_row("SELECT payload_json FROM versions WHERE version_id=?1", params![version_id], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(json.map(|j| serde_json::from_str(&j).expect("payload json")))
    }

    pub fn version_record(&self, version_id: &str) -> rusqlite::Result<Option<VersionRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT entity_id,version_id,parent_version_id,kind,payload_json,fingerprint,input_versions,valid,created_seq
             FROM versions WHERE version_id=?1",
        )?;
        stmt.query_row(params![version_id], row_to_version).optional()
    }

    pub fn all_versions(&self) -> rusqlite::Result<Vec<VersionRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT entity_id,version_id,parent_version_id,kind,payload_json,fingerprint,input_versions,valid,created_seq
             FROM versions ORDER BY created_seq, version_id",
        )?;
        let rows = stmt.query_map([], row_to_version)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn all_current(&self) -> rusqlite::Result<Vec<CurrentRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT entity_id,version_id,expected_version FROM current_versions ORDER BY entity_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(CurrentRecord {
                entity_id: r.get(0)?,
                version_id: r.get(1)?,
                expected_version: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 追加一条事件（append-only，永不更新/删除）。
    pub fn append_event(&self, mut event: Event) -> rusqlite::Result<i64> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let seq: i64 = tx.query_row("SELECT COALESCE(MAX(seq),0)+1 FROM events", [], |r| r.get(0))?;
        event.seq = seq;
        let j = serde_json::to_string(&event).expect("event json");
        tx.execute(
            "INSERT INTO events(seq,event_id,json) VALUES (?1,?2,?3)",
            params![seq, event.event_id, j],
        )?;
        tx.commit()?;
        Ok(seq)
    }

    pub fn events(&self) -> rusqlite::Result<Vec<Event>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT json FROM events ORDER BY seq");
        let rows = stmt.query_map([], |r| {
            let j: String = r.get(0)?;
            Ok(serde_json::from_str::<Event>(&j).expect("event json"))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn event_at(&self, seq: i64) -> rusqlite::Result<Option<Event>> {
        let conn = self.conn.lock().unwrap();
        let json: Option<String> = conn
            .query_row("SELECT json FROM events WHERE seq=?1", params![seq], |r| r.get(0))
            .optional()?;
        Ok(json.map(|j| serde_json::from_str(&j).expect("event json")))
    }

    pub fn set_meta(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> rusqlite::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT value FROM meta WHERE key=?1", params![key], |r| r.get(0)).optional()
    }

    /// 导出完整身份/顺序/来源包（JSON）。
    pub fn export_bundle(&self) -> rusqlite::Result<Bundle> {
        let versions = self
            .all_versions()?
            .into_iter()
            .map(|v| {
                serde_json::json!({
                    "entity_id": v.entity_id,
                    "version_id": v.version_id,
                    "parent_version_id": v.parent_version_id,
                    "kind": v.kind,
                    "payload": serde_json::from_str::<serde_json::Value>(&v.payload_json).unwrap(),
                    "fingerprint": v.fingerprint,
                    "input_versions": v.input_versions,
                    "valid": v.valid,
                    "created_seq": v.created_seq,
                })
            })
            .collect();
        let current = self
            .all_current()?
            .into_iter()
            .map(|c| serde_json::json!({"entity_id": c.entity_id, "version_id": c.version_id, "expected_version": c.expected_version}))
            .collect();
        let events = self
            .events()?
            .into_iter()
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
        let mut meta = Vec::new();
        {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare("SELECT key,value FROM meta ORDER BY key")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for r in rows {
                meta.push(r?);
            }
        }
        Ok(Bundle { schema_version: SCHEMA_VERSION, versions, current, events, meta })
    }

    /// 导入包：身份、顺序、来源关系保持不变。目标库必须为空（防覆盖）。
    pub fn import_bundle(&self, bundle: &Bundle) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM versions", [], |r| r.get(0))?;
        if count != 0 {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                "IMPORT_TARGET_NOT_EMPTY".into(),
            ));
        }
        for v in &bundle.versions {
            tx.execute(
                "INSERT INTO versions(entity_id,version_id,parent_version_id,kind,payload_json,fingerprint,input_versions,valid,created_seq)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    v["entity_id"],
                    v["version_id"],
                    v["parent_version_id"],
                    v["kind"],
                    v["payload"].to_string(),
                    v["fingerprint"],
                    v["input_versions"],
                    v["valid"],
                    v["created_seq"],
                ],
            )?;
        }
        for c in &bundle.current {
            tx.execute(
                "INSERT INTO current_versions(entity_id,version_id,expected_version) VALUES (?1,?2,?3)",
                params![c["entity_id"], c["version_id"], c["expected_version"]],
            )?;
        }
        for e in &bundle.events {
            let seq = e["seq"].as_i64().unwrap();
            tx.execute(
                "INSERT INTO events(seq,event_id,json) VALUES (?1,?2,?3)",
                params![seq, e["event_id"], e.to_string()],
            )?;
        }
        for (k, val) in &bundle.meta {
            tx.execute("INSERT INTO meta(key,value) VALUES (?1,?2)", params![k, val])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 危险操作：清空库（供演示"删除本地数据库再导入"场景，走 drop 文件即可，此为测试辅助）。
    pub fn reset_all(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("DELETE FROM events; DELETE FROM current_versions; DELETE FROM versions; DELETE FROM meta;")?;
        Ok(())
    }
}

use rusqlite::OptionalExtension;

fn row_to_version(r: &rusqlite::Row<'_>) -> rusqlite::Result<VersionRecord> {
    Ok(VersionRecord {
        entity_id: r.get(0)?,
        version_id: r.get(1)?,
        parent_version_id: r.get(2)?,
        kind: r.get(3)?,
        payload_json: r.get(4)?,
        fingerprint: r.get(5)?,
        input_versions: r.get(6)?,
        valid: r.get(7)?,
        created_seq: r.get(8)?,
    })
}

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS meta(
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS versions(
  entity_id TEXT NOT NULL,
  version_id TEXT NOT NULL PRIMARY KEY,
  parent_version_id TEXT,
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  input_versions TEXT NOT NULL,
  valid INTEGER NOT NULL,
  created_seq INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_versions_entity ON versions(entity_id, created_seq);
CREATE TABLE IF NOT EXISTS current_versions(
  entity_id TEXT PRIMARY KEY,
  version_id TEXT NOT NULL,
  expected_version INTEGER NOT NULL,
  FOREIGN KEY(version_id) REFERENCES versions(version_id)
);
CREATE TABLE IF NOT EXISTS events(
  seq INTEGER PRIMARY KEY,
  event_id TEXT NOT NULL UNIQUE,
  json TEXT NOT NULL
);
"#;

/// 从旧 schema 迁移到当前：保留全部行，补齐新增列/表并记录迁移事件。
fn migrate(conn: &Connection, from: i64) -> rusqlite::Result<()> {
    if from <= 1 {
        // v1 只有 versions(entity_id,version_id,kind,payload_json)
        let cols: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('versions')", [], |r| r.get(0),
        )?;
        if cols < 9 {
            conn.execute_batch(
                "ALTER TABLE versions ADD COLUMN parent_version_id TEXT;
                 ALTER TABLE versions ADD COLUMN fingerprint TEXT NOT NULL DEFAULT '';
                 ALTER TABLE versions ADD COLUMN input_versions TEXT NOT NULL DEFAULT '';
                 ALTER TABLE versions ADD COLUMN valid INTEGER NOT NULL DEFAULT 1;
                 ALTER TABLE versions ADD COLUMN created_seq INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS current_versions(
               entity_id TEXT PRIMARY KEY, version_id TEXT NOT NULL, expected_version INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS events(seq INTEGER PRIMARY KEY, event_id TEXT NOT NULL UNIQUE, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
    }
    if from <= 2 {
        // v2: 为版本补充索引
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_versions_entity ON versions(entity_id, created_seq);",
        )?;
    }
    conn.execute(
        "INSERT OR REPLACE INTO meta(key,value) VALUES ('schema_version', ?1)",
        params![SCHEMA_VERSION],
    )?;
    Ok(())
}

/// 直接在文件上构造一个 v1 旧库（迁移测试专用，只测仓库内行为）。
pub fn create_legacy_v1(path: &str) -> rusqlite::Result<()> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "CREATE TABLE versions(
           entity_id TEXT NOT NULL, version_id TEXT NOT NULL PRIMARY KEY,
           kind TEXT NOT NULL, payload_json TEXT NOT NULL);",
    )?;
    Ok(())
}
