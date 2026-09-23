//! SQLite 存储层。
//!
//! 写入语义（RULE_STORE_V1）：
//! * 每次推导产生独立版本；只有完整成功后才切换 current_pointer。
//! * 写入经“暂存 -> 单事务 finalize”：进程在写入中断后重启，只能看到
//!   旧版本或完整新版本（启动时回收所有未 finalize 的暂存事务）。
//! * 两位使用者从同一 edit_group 版本编辑时，expected_head 校验失败并
//!   返回可解决冲突（记录 ConflictResolution 事件后才能提交）。
//! * 旧版本库通过 migrations 链迁移，不丢弃任何历史。

use rusqlite::{params, Connection};
use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::events::{DecisionBody, Event, EventKind, Outcome};
use crate::model::*;

pub const RULE_STORE: &str = "rules/store@1";
pub const SCHEMA_VERSION: u32 = 3;

pub type StoreResult<T> = Result<T, String>;

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictError {
    pub edit_group: String,
    pub expected_head: Option<u64>,
    pub actual_head: u64,
    pub message: String,
}

impl std::fmt::Display for ConflictError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "编辑冲突: {}", self.message)
    }
}

#[derive(Debug, Clone)]
pub struct PendingTx {
    pub tx_id: String,
}

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // 固定可读格式（UTC 秒足够用于演示；不引时间库）。
    format!("unix:{}", secs)
}

impl Store {
    pub fn open(path: &str) -> StoreResult<Self> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?);
        Self::init_conn(&conn)?;
        let store = Store { conn: Mutex::new(conn) };
        store.recover_staged()?;
        Ok(store)
    }

    pub fn open_in_memory() -> StoreResult<Self> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        Self::init_conn(&conn)?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    fn init_conn(conn: &Connection) -> StoreResult<()> {
        conn.pragma_update(None, "foreign_keys", "ON").map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "synchronous", "FULL").ok();
        conn.pragma_update(None, "busy_timeout", 5000).map_err(|e| e.to_string())?;
        let v: Option<i64> = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).ok();
        let current = v.unwrap_or(0) as u32;
        if current == 0 {
            Self::migrate_v1(conn)?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION).map_err(|e| e.to_string())?;
        } else if current < SCHEMA_VERSION {
            Self::migrate_from(conn, current)?;
        }
        Ok(())
    }

    fn migrate_v1(conn: &Connection) -> StoreResult<()> {
        conn.execute_batch(include_str!("migrations/001_init.sql")).map_err(|e| e.to_string())?;
        conn.execute_batch(include_str!("migrations/002_staged.sql")).map_err(|e| e.to_string())?;
        conn.execute_batch(include_str!("migrations/003_edit_groups.sql")).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 从旧版本迁移：逐版本应用，历史数据保留。
    fn migrate_from(conn: &Connection, from: u32) -> StoreResult<()> {
        if from < 2 {
            conn.execute_batch(include_str!("migrations/002_staged.sql")).map_err(|e| e.to_string())?;
        }
        if from < 3 {
            conn.execute_batch(include_str!("migrations/003_edit_groups.sql")).map_err(|e| e.to_string())?;
            // 迁移时为已有事件补默认 edit_group。
            conn.execute("UPDATE meta SET edit_group='main' WHERE edit_group IS NULL OR edit_group=''", []).ok();
        }
        conn.pragma_update(None, "user_version", SCHEMA_VERSION).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn schema_version(&self) -> StoreResult<u32> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())?;
        Ok(v as u32)
    }

    /// 测试支持：把库降级到旧版本（模拟收到旧库），仅用于迁移测试。
    pub fn downgrade_for_migration_test(&self, to: u32) -> StoreResult<()> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        if to == 1 {
            conn.execute_batch("DROP TABLE IF EXISTS edit_heads;").map_err(|e| e.to_string())?;
            conn.execute_batch("DROP TABLE IF EXISTS staged_write;").map_err(|e| e.to_string())?;
        }
        if to == 2 {
            conn.execute_batch("DROP TABLE IF EXISTS edit_heads;").map_err(|e| e.to_string())?;
        }
        conn.pragma_update(None, "user_version", to).map_err(|e| e.to_string())?;
        Ok(())
    }
}

// ───────────────────────────────────────────────────────────────────────────
// 写事务：暂存 -> finalize，支持崩溃注入与恢复
// ───────────────────────────────────────────────────────────────────────────

impl Store {
    /// 开启一次暂存写入。在 finalize 前，新内容只存在于未提交事务中。
    pub fn begin_write(&self, note: &str) -> StoreResult<String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx_id = format!("tx-{}-{:x}", now_iso(), rand_nonce());
        conn.execute("INSERT INTO staged_write(tx_id, created_at, note) VALUES (?1,?2,?3)",
            params![tx_id, now_iso(), note]).map_err(|e| e.to_string())?;
        // staged_write 行立刻落盘（独立自动提交），作为恢复标记；实际内容在事务里。
        Ok(tx_id)
    }

    /// 原子提交：内容写入 + 指针切换 + 事件在单个事务中完成。
    /// tx_id 先作为恢复标记落盘；提交完成后删除标记。
    /// fail_after_stage=true 时在内容写完、提交前模拟进程崩溃（ROLLBACK + 保留标记）。
    pub fn commit<F>(&self, tx_id: &str, fail_after_stage: bool, f: F) -> StoreResult<u64>
    where F: FnOnce(&Connection) -> StoreResult<u64> {
        {
            let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
            conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
            let result = f(&conn);
            match result {
                Ok(seq) => {
                    if fail_after_stage {
                        // 模拟进程在 finalize 前被杀死：回滚事务、保留 staged_write 标记、泄漏锁。
                        conn.execute_batch("ROLLBACK").ok();
                        return Err("SIMULATED_CRASH_AFTER_STAGE".to_string());
                    }
                    conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
                    let _ = seq;
                }
                Err(e) => {
                    conn.execute_batch("ROLLBACK").ok();
                    return Err(e);
                }
            }
        }
        {
            let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
            conn.execute("DELETE FROM staged_write WHERE tx_id=?1", params![tx_id]).map_err(|e| e.to_string())?;
        }
        let seq = self.last_seq()?;
        Ok(seq)
    }

    pub fn abort_staged(&self, tx_id: &str) -> StoreResult<()> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute_batch("ROLLBACK").ok();
        conn.execute("DELETE FROM staged_write WHERE tx_id=?1", params![tx_id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 重启恢复：所有未 finalize 的暂存标记对应的内容都已随旧事务消失，
    /// 因此删除标记即可——库里只剩旧版本（新版本要么从未提交，要么完整提交）。
    pub fn recover_staged(&self) -> StoreResult<Vec<String>> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let stale: Vec<String> = {
            let mut stmt = conn.prepare("SELECT tx_id FROM staged_write").map_err(|e| e.to_string())?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
            let mut v = Vec::new();
            for r in rows { v.push(r.map_err(|e| e.to_string())?); }
            v
        };
        for tx in &stale {
            conn.execute("DELETE FROM staged_write WHERE tx_id=?1", params![tx]).map_err(|e| e.to_string())?;
        }
        Ok(stale)
    }

    pub fn list_staged(&self) -> StoreResult<Vec<String>> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT tx_id FROM staged_write").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
        let mut v = Vec::new();
        for r in rows { v.push(r.map_err(|e| e.to_string())?); }
        Ok(v)
    }
}

fn rand_nonce() -> u64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as u64).unwrap_or(1);
    nanos.wrapping_mul(0x9e3779b97f4a7c15)
}

// ───────────────────────────────────────────────────────────────────────────
// 低层读写辅助
// ───────────────────────────────────────────────────────────────────────────

fn json<T: Serialize>(v: &T) -> StoreResult<String> {
    serde_json::to_string(v).map_err(|e| e.to_string())
}
fn de<'a, T: Deserialize<'a>>(s: &'a str) -> StoreResult<T> {
    serde_json::from_str(s).map_err(|e| e.to_string())
}
use serde::{Deserialize, Serialize};

fn insert_event(
    conn: &Connection,
    kind: EventKind,
    action: &str,
    summary: &str,
    author: &str,
    edit_group: &str,
    expected_head: Option<u64>,
    supersedes: &[u64],
    outcome: Outcome,
    affected: &[String],
    versions: &[String],
    rules: &[&str],
    codes: &[String],
    payload_json: &str,
) -> StoreResult<u64> {
    conn.execute(
        "INSERT INTO events(kind,action,summary,author,edit_group,expected_head,supersedes,outcome,affected_json,versions_json,rules_json,codes_json,payload_json,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            format!("{:?}", kind).to_lowercase(),
            action, summary, author, edit_group,
            expected_head.map(|v| v as i64),
            serde_json::to_string(supersedes).unwrap_or_default(),
            format!("{:?}", outcome).to_lowercase(),
            serde_json::to_string(affected).unwrap_or_default(),
            serde_json::to_string(versions).unwrap_or_default(),
            serde_json::to_string(rules).unwrap_or_default(),
            serde_json::to_string(codes).unwrap_or_default(),
            payload_json,
            now_iso(),
        ],
    ).map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid() as u64)
}

fn check_head(conn: &Connection, edit_group: &str, expected: Option<u64>) -> StoreResult<u64> {
    let actual: i64 = conn.query_row(
        "SELECT head_seq FROM edit_heads WHERE edit_group=?1", params![edit_group], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let actual = actual as u64;
    if let Some(exp) = expected {
        if exp != actual {
            return Err(serde_json::to_string(&ConflictError {
                edit_group: edit_group.to_string(),
                expected_head: Some(exp),
                actual_head: actual,
                message: format!("分支 '{}' 已从 #{} 前进到 #{}；请先比较两个分支，解决冲突后携带 resolved_head 重试", edit_group, exp, actual),
            }).unwrap_or_default());
        }
    }
    Ok(actual)
}

fn advance_head(conn: &Connection, edit_group: &str, seq: u64) -> StoreResult<()> {
    conn.execute(
        "INSERT INTO edit_heads(edit_group, head_seq) VALUES (?1,?2)
         ON CONFLICT(edit_group) DO UPDATE SET head_seq=excluded.head_seq WHERE excluded.head_seq > edit_heads.head_seq",
        params![edit_group, seq as i64],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

impl Store {
    pub fn last_seq(&self) -> StoreResult<u64> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let v: i64 = conn.query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        Ok(v as u64)
    }

    pub fn head_seq(&self, edit_group: &str) -> StoreResult<u64> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let v: i64 = conn.query_row("SELECT head_seq FROM edit_heads WHERE edit_group=?1", params![edit_group], |r| r.get(0)).unwrap_or(0);
        Ok(v as u64)
    }

    pub fn all_events(&self) -> StoreResult<Vec<Event>> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        read_events(&conn, "SELECT seq,kind,action,summary,author,edit_group,expected_head,supersedes,outcome,affected_json,versions_json,rules_json,codes_json,payload_json,created_at FROM events ORDER BY seq")
    }

    pub fn events_until(&self, seq: u64) -> StoreResult<Vec<Event>> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        read_events(&conn, &format!(
            "SELECT seq,kind,action,summary,author,edit_group,expected_head,supersedes,outcome,affected_json,versions_json,rules_json,codes_json,payload_json,created_at FROM events WHERE seq <= {} ORDER BY seq", seq))
    }

    pub fn event(&self, seq: u64) -> StoreResult<Option<Event>> {
        Ok(self.all_events()?.into_iter().find(|e| e.seq == seq))
    }
}

fn read_events(conn: &Connection, sql: &str) -> StoreResult<Vec<Event>> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| {
        let parse_arr: fn(&str) -> Vec<String> = |s| serde_json::from_str(s).unwrap_or_default();
        let parse_u: fn(&str) -> Vec<u64> = |s| serde_json::from_str(s).unwrap_or_default();
        Ok(Event {
            seq: row.get(0)?,
            kind: parse_kind(&row.get::<_, String>(1)?),
            action: row.get(2)?,
            summary: row.get(3)?,
            author: row.get(4)?,
            edit_group: row.get(5)?,
            expected_head: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
            supersedes: parse_u(&row.get::<_, String>(7)?),
            outcome: parse_outcome(&row.get::<_, String>(8)?),
            affected: parse_arr(&row.get::<_, String>(9)?),
            versions: parse_arr(&row.get::<_, String>(10)?),
            rules: parse_arr(&row.get::<_, String>(11)?),
            codes: parse_arr(&row.get::<_, String>(12)?),
            payload_json: row.get(13)?,
            created_at: row.get(14)?,
        })
    }).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows { out.push(r.map_err(|e| e.to_string())?); }
    Ok(out)
}

fn parse_kind(s: &str) -> EventKind {
    match s {
        "materialimport" => EventKind::MaterialImport,
        "derived" => EventKind::Derived,
        "decision" => EventKind::Decision,
        "conflictresolution" => EventKind::ConflictResolution,
        "imported" => EventKind::Imported,
        "replaymark" => EventKind::ReplayMark,
        other => {
            if other.contains("material") { EventKind::MaterialImport }
            else if other.contains("conflict") { EventKind::ConflictResolution }
            else if other.contains("decision") { EventKind::Decision }
            else { EventKind::Derived }
        }
    }
}
fn parse_outcome(s: &str) -> Outcome {
    match s { "blocked" => Outcome::Blocked, "warning" => Outcome::Warning, _ => Outcome::Success }
}
