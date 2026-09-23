//! 存储层：SQLite 版本化实体 + 事件日志。
//! - 每次推导产生独立版本，只有完整成功后才切换当前指针；
//! - 两阶段写入：stage（committed=0 落盘）→ activate（同事务置 committed=1 并切指针）；
//!   进程中断后重启，要么看到旧版本，要么看到完整新版本；
//! - 乐观并发：保存携带 base_head，不一致则报可解决冲突。
use crate::model::{Entity, Event};
use rusqlite::{params, Connection, OptionalExtension};

pub const SCHEMA_VERSION: i64 = 2;

pub struct Store {
    pub conn: Connection,
}

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Conflict { stream: String, expected: i64, actual: i64 },
    NotFound(String),
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone)]
pub struct StagedWrite {
    pub entities: Vec<Entity>,
    /// (stream, new_current_entity_id)
    pub pointer_moves: Vec<(String, String)>,
    pub events: Vec<Event>,
    /// 乐观并发：调用方看到的 head seq；None 表示不检查。
    pub base_head: Option<i64>,
}

impl Store {
    pub fn open(path: &str) -> Result<Store> {
        let conn = Connection::open(path)?;
        let mut store = Store { conn };
        store.init()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Store> {
        let conn = Connection::open_in_memory()?;
        let mut store = Store { conn };
        store.init()?;
        Ok(store)
    }

    fn init(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);",
        )?;
        let v: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0))
            .optional()?;
        match v {
            None => {
                self.create_schema_v2()?;
                self.conn.execute(
                    "INSERT INTO meta(key,value) VALUES('schema_version',?1)",
                    params![SCHEMA_VERSION.to_string()],
                )?;
            }
            Some(v) => {
                let v: i64 = v.parse().unwrap_or(1);
                if v < SCHEMA_VERSION {
                    self.migrate(v, SCHEMA_VERSION)?;
                }
            }
        }
        // 中断恢复：清理任何未激活的暂存行（stage 后未 activate）。
        self.conn.execute("DELETE FROM entities WHERE committed=0", [])?;
        self.conn.execute("DELETE FROM events WHERE committed=0", [])?;
        self.conn.execute("DELETE FROM pointer_log WHERE committed=0", [])?;
        Ok(())
    }

    fn create_schema_v2(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entities(
                id TEXT PRIMARY KEY, stream TEXT NOT NULL, kind TEXT NOT NULL,
                seq INTEGER NOT NULL, payload TEXT NOT NULL, committed INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS streams(
                stream TEXT PRIMARY KEY, current TEXT);
             CREATE TABLE IF NOT EXISTS events(
                id TEXT PRIMARY KEY, seq INTEGER NOT NULL, kind TEXT NOT NULL,
                rule TEXT, summary TEXT, payload TEXT, committed INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS pointer_log(
                stream TEXT NOT NULL, entity_id TEXT NOT NULL, seq INTEGER NOT NULL,
                committed INTEGER NOT NULL DEFAULT 0);
             CREATE INDEX IF NOT EXISTS idx_entities_stream ON entities(stream);
             CREATE INDEX IF NOT EXISTS idx_pointer_log ON pointer_log(stream, seq);",
        )?;
        Ok(())
    }

    /// 旧版本迁移：v1 -> v2 增加 pointer_log 表与 committed 列。
    fn migrate(&mut self, from: i64, to: i64) -> Result<()> {
        if from < 2 && to >= 2 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS pointer_log(
                    stream TEXT NOT NULL, entity_id TEXT NOT NULL, seq INTEGER NOT NULL,
                    committed INTEGER NOT NULL DEFAULT 1);
                 CREATE INDEX IF NOT EXISTS idx_pointer_log ON pointer_log(stream, seq);",
            )?;
            // v1 的 entities/events 若无 committed 列则补上
            let has_col: bool = self
                .conn
                .prepare("SELECT committed FROM entities LIMIT 1")
                .is_ok();
            if !has_col {
                self.conn.execute_batch(
                    "ALTER TABLE entities ADD COLUMN committed INTEGER NOT NULL DEFAULT 1;
                     ALTER TABLE events ADD COLUMN committed INTEGER NOT NULL DEFAULT 1;",
                )?;
            }
            // 为现有 streams 回填 pointer_log
            self.conn.execute_batch(
                "INSERT INTO pointer_log(stream, entity_id, seq, committed)
                 SELECT s.stream, s.current,
                        COALESCE((SELECT MAX(e.seq) FROM entities e WHERE e.id=s.current),0), 1
                 FROM streams s
                 WHERE NOT EXISTS (SELECT 1 FROM pointer_log p WHERE p.stream=s.stream);",
            )?;
            self.conn.execute(
                "INSERT OR REPLACE INTO meta(key,value) VALUES('schema_version',?1)",
                params![to.to_string()],
            )?;
        }
        Ok(())
    }

    pub fn head_seq(&self) -> Result<i64> {
        let seq: Option<i64> = self
            .conn
            .query_row("SELECT MAX(seq) FROM events WHERE committed=1", [], |r| r.get(0))
            .optional()?
            .flatten();
        Ok(seq.unwrap_or(0))
    }

    fn next_entity_seq(&self) -> Result<i64> {
        let seq: Option<i64> = self
            .conn
            .query_row("SELECT MAX(seq) FROM entities", [], |r| r.get(0))
            .optional()?
            .flatten();
        Ok(seq.unwrap_or(0) + 1)
    }

    /// 阶段一：暂存写入（committed=0，落盘但不可见）。
    pub fn stage(&mut self, write: &StagedWrite) -> Result<()> {
        let tx = self.conn.transaction()?;
        let mut seq: i64 =
            tx.query_row("SELECT COALESCE(MAX(seq),0) FROM entities", [], |r| r.get(0))?;
        for e in &write.entities {
            seq += 1;
            tx.execute(
                "INSERT OR REPLACE INTO entities(id,stream,kind,seq,payload,committed)
                 VALUES(?1,?2,?3,?4,?5,0)",
                params![e.id, e.stream, e.kind, seq, e.payload.to_string()],
            )?;
        }
        for ev in &write.events {
            tx.execute(
                "INSERT OR REPLACE INTO events(id,seq,kind,rule,summary,payload,committed)
                 VALUES(?1,?2,?3,?4,?5,?6,0)",
                params![ev.id, ev.seq, ev.kind, ev.rule, ev.summary, ev.payload.to_string()],
            )?;
        }
        let head = self.head_seq()?;
        for (stream, entity_id) in &write.pointer_moves {
            tx.execute(
                "INSERT INTO pointer_log(stream,entity_id,seq,committed) VALUES(?1,?2,?3,0)",
                params![stream, entity_id, head + 1],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 阶段二：原子激活。乐观并发检查在此进行。
    pub fn activate(&mut self, write: &StagedWrite) -> Result<()> {
        if let Some(base) = write.base_head {
            let actual = self.head_seq()?;
            if actual != base {
                return Err(StoreError::Conflict {
                    stream: "head".into(),
                    expected: base,
                    actual,
                });
            }
        }
        let tx = self.conn.transaction()?;
        for e in &write.entities {
            tx.execute("UPDATE entities SET committed=1 WHERE id=?1", params![e.id])?;
        }
        for ev in &write.events {
            tx.execute("UPDATE events SET committed=1 WHERE id=?1", params![ev.id])?;
        }
        for (stream, entity_id) in &write.pointer_moves {
            tx.execute(
                "INSERT OR REPLACE INTO streams(stream,current) VALUES(?1,?2)",
                params![stream, entity_id],
            )?;
            tx.execute(
                "UPDATE pointer_log SET committed=1 WHERE stream=?1 AND entity_id=?2",
                params![stream, entity_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 完整写入（stage+activate），供常规路径使用。
    pub fn commit(&mut self, write: &StagedWrite) -> Result<()> {
        self.stage(write)?;
        self.activate(write)
    }

    pub fn current(&self, stream: &str) -> Result<Option<Entity>> {
        let id: Option<String> = self
            .conn
            .query_row("SELECT current FROM streams WHERE stream=?1", params![stream], |r| r.get(0))
            .optional()?;
        match id {
            Some(id) => self.get(&id).map(Some),
            None => Ok(None),
        }
    }

    pub fn get(&self, id: &str) -> Result<Entity> {
        let row = self
            .conn
            .query_row(
                "SELECT id,stream,kind,seq,payload FROM entities WHERE id=?1 AND committed=1",
                params![id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, String>(4)?)),
            )
            .optional()?;
        match row {
            Some((id, stream, kind, seq, payload)) => Ok(Entity {
                id,
                stream,
                kind,
                seq,
                payload: serde_json::from_str(&payload).unwrap_or(serde_json::json!({})),
            }),
            None => Err(StoreError::NotFound(id.to_string())),
        }
    }

    pub fn streams(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT stream,current FROM streams ORDER BY stream")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn events(&self) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,seq,kind,rule,summary,payload FROM events WHERE committed=1 ORDER BY seq",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|(id, seq, kind, rule, summary, payload)| Event {
                id,
                seq,
                kind,
                rule,
                summary,
                payload: serde_json::from_str(&payload).unwrap_or(serde_json::json!({})),
            })
            .collect())
    }

    /// 重放：返回 target_seq 时刻各流的指针（读视图，不破坏当前状态）。
    pub fn pointers_at(&self, target_seq: i64) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.stream, p.entity_id FROM pointer_log p
             WHERE p.committed=1 AND p.seq<=?1
             GROUP BY p.stream HAVING p.seq=MAX(p.seq) ORDER BY p.stream",
        )?;
        let rows = stmt
            .query_map(params![target_seq], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 依赖解释：某项结果为何仍有效 / 为何需要重算。
    pub fn explain(&self, entity_id: &str) -> Result<serde_json::Value> {
        let e = self.get(entity_id)?;
        let deps: Vec<String> = serde_json::from_value(
            e.payload.get("deps").cloned().unwrap_or(serde_json::json!([])),
        )
        .unwrap_or_default();
        let mut reasons = Vec::new();
        let mut valid = true;
        for dep in &deps {
            match self.get(dep) {
                Ok(d) => {
                    let cur = self.current(&d.stream)?;
                    match cur {
                        Some(c) if c.id == d.id => reasons.push(serde_json::json!({
                            "dep": dep, "stream": d.stream, "status": "unchanged"
                        })),
                        Some(c) => {
                            valid = false;
                            reasons.push(serde_json::json!({
                                "dep": dep, "stream": d.stream, "status": "superseded",
                                "current": c.id
                            }));
                        }
                        None => reasons.push(serde_json::json!({"dep": dep, "status": "stream_missing"})),
                    }
                }
                Err(_) => {
                    valid = false;
                    reasons.push(serde_json::json!({"dep": dep, "status": "missing"}));
                }
            }
        }
        Ok(serde_json::json!({
            "entity": entity_id,
            "status": if valid { "valid" } else { "stale" },
            "reasons": reasons,
        }))
    }

    /// 导出完整档案（身份、顺序、来源关系全部保留）。
    pub fn export_archive(&self) -> Result<serde_json::Value> {
        let mut stmt = self.conn.prepare(
            "SELECT id,stream,kind,seq,payload FROM entities WHERE committed=1 ORDER BY seq",
        )?;
        let entities: Vec<serde_json::Value> = stmt
            .query_map([], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, String>(0)?, "stream": r.get::<_, String>(1)?,
                    "kind": r.get::<_, String>(2)?, "seq": r.get::<_, i64>(3)?,
                    "payload": serde_json::from_str::<serde_json::Value>(&r.get::<_, String>(4)?).unwrap_or(serde_json::json!({})),
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(serde_json::json!({
            "format": "saxs-bench-archive/1",
            "entities": entities,
            "streams": self.streams()?,
            "events": self.events()?,
        }))
    }

    /// 导入档案：保持 id、顺序与来源关系。
    pub fn import_archive(&mut self, archive: &serde_json::Value) -> Result<()> {
        let entities = archive.get("entities").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let streams = archive.get("streams").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let events = archive.get("events").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let tx = self.conn.transaction()?;
        tx.execute_batch("DELETE FROM entities; DELETE FROM events; DELETE FROM streams; DELETE FROM pointer_log;")?;
        for e in &entities {
            tx.execute(
                "INSERT INTO entities(id,stream,kind,seq,payload,committed) VALUES(?1,?2,?3,?4,?5,1)",
                params![
                    e["id"].as_str().unwrap_or(""),
                    e["stream"].as_str().unwrap_or(""),
                    e["kind"].as_str().unwrap_or(""),
                    e["seq"].as_i64().unwrap_or(0),
                    e["payload"].to_string()
                ],
            )?;
        }
        for s in &streams {
            let pair = s.as_array().cloned().unwrap_or_default();
            if pair.len() == 2 {
                tx.execute(
                    "INSERT INTO streams(stream,current) VALUES(?1,?2)",
                    params![pair[0].as_str().unwrap_or(""), pair[1].as_str().unwrap_or("")],
                )?;
                let seq: i64 = tx
                    .query_row(
                        "SELECT COALESCE(MAX(seq),0) FROM events",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                tx.execute(
                    "INSERT INTO pointer_log(stream,entity_id,seq,committed) VALUES(?1,?2,?3,1)",
                    params![pair[0].as_str().unwrap_or(""), pair[1].as_str().unwrap_or(""), seq.max(1)],
                )?;
            }
        }
        for ev in &events {
            tx.execute(
                "INSERT INTO events(id,seq,kind,rule,summary,payload,committed) VALUES(?1,?2,?3,?4,?5,?6,1)",
                params![
                    ev["id"].as_str().unwrap_or(""),
                    ev["seq"].as_i64().unwrap_or(0),
                    ev["kind"].as_str().unwrap_or(""),
                    ev["rule"].as_str().map(|s| s.to_string()),
                    ev["summary"].as_str().unwrap_or(""),
                    ev["payload"].to_string()
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    #[allow(dead_code)]
    fn next_seq_helper(&self) -> Result<i64> {
        self.next_entity_seq()
    }
}
