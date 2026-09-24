//! SQLite 版本化存储。
//!
//! 语义：
//! - 原始记录与派生记录都是只增的不可变修订版本；
//! - 每次推导产生独立版本，只有完整成功后才切换 current 指针；
//! - 两阶段提交：阶段一写入 pending 修订（可持久化），阶段二原子翻转指针+事件；
//!   阶段一后崩溃，重启恢复时看到旧版本，pending 作为废弃版本清除；
//! - 人工裁定带 base_revision，乐观并发：基线已移动则产生可解决冲突；
//! - 导出/导入保持稳定身份、顺序与来源关系。

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;

use crate::model::{make_id, now_iso, Event, NodeKind};

const SCHEMA_VERSION: i64 = 1;

pub fn canonicalize(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let sorted: BTreeMap<_, _> = m.iter().map(|(k, v)| (k.clone(), canonicalize(v))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn hash_value(v: &Value) -> String {
    let c = canonicalize(v);
    let s = serde_json::to_vec(&c).unwrap_or_default();
    format!("{:016x}", fnv1a64(&s))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DepRef {
    pub revision_id: i64,
    pub input_hash: String,
}

#[derive(Debug, Clone)]
pub struct Revision {
    pub id: i64,
    pub node_id: String,
    pub version: i64,
    pub kind: String,
    pub parent_revision: Option<i64>,
    pub status: String,
    pub deps: Vec<String>,
    pub dep_hashes: BTreeMap<String, DepRef>,
    pub input_hash: String,
    pub data: Value,
    pub author: String,
    pub created_at: String,
    pub base_revision: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConflictRecord {
    pub id: i64,
    pub node_id: String,
    pub attempted_base: i64,
    pub current_revision: i64,
    pub proposal: Value,
    pub author: String,
    pub status: String,
    pub created_at: String,
}

pub struct Store {
    conn: Connection,
}

pub struct Draft {
    pub node_id: Option<String>,
    pub kind: NodeKind,
    pub data: Value,
    pub deps: Vec<(String, i64, String)>,
    pub input_hash: String,
    pub parent: Option<i64>,
    pub base_revision: Option<i64>,
    pub author: String,
}

fn row_to_rev(r: &rusqlite::Row) -> rusqlite::Result<Revision> {
    let deps_json: String = r.get("deps_json")?;
    let dep_hashes_json: String = r.get("dep_hashes_json")?;
    let data_json: String = r.get("data_json")?;
    let dep_hashes: BTreeMap<String, DepRef> =
        serde_json::from_str(&dep_hashes_json).unwrap_or_default();
    Ok(Revision {
        id: r.get("id")?,
        node_id: r.get("node_id")?,
        version: r.get("version")?,
        kind: r.get("kind")?,
        parent_revision: r.get("parent_revision")?,
        status: r.get("status")?,
        deps: serde_json::from_str(&deps_json).unwrap_or_default(),
        dep_hashes,
        input_hash: r.get("input_hash")?,
        data: serde_json::from_str(&data_json).unwrap_or(Value::Null),
        author: r.get("author")?,
        created_at: r.get("created_at")?,
        base_revision: r.get("base_revision")?,
    })
}

fn insert_revision(
    tx: &rusqlite::Transaction,
    node: &str,
    version: i64,
    kind: &str,
    parent: Option<i64>,
    status: &str,
    deps: &[(String, i64, String)],
    input_hash: &str,
    data: &Value,
    author: &str,
    base_revision: Option<i64>,
) -> Result<i64> {
    let dep_ids: Vec<String> = deps.iter().map(|(n, _, _)| n.clone()).collect();
    let dep_hashes: BTreeMap<&str, Value> = deps
        .iter()
        .map(|(n, rid, h)| {
            (n.as_str(), serde_json::json!({"revision_id": rid, "input_hash": h}))
        })
        .collect();
    tx.execute(
        "INSERT INTO revisions(node_id,version,kind,parent_revision,status,deps_json,input_hash,data_json,author,base_revision,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            node,
            version,
            kind,
            parent,
            status,
            serde_json::to_string(&dep_ids)?,
            serde_json::to_value(dep_hashes)?.to_string(),
            input_hash,
            data.to_string(),
            author,
            base_revision,
            now_iso()
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

fn node_exists_tx(tx: &rusqlite::Transaction, node: &str) -> bool {
    tx.query_row("SELECT 1 FROM nodes WHERE id=?1", params![node], |_| Ok(())).is_ok()
}

fn ensure_node_tx(tx: &rusqlite::Transaction, node: &str, kind: NodeKind) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO nodes(id,kind,seq,created_at) VALUES(?1,?2,0,?3)",
        params![node, kind.as_str(), now_iso()],
    )?;
    let n: i64 = tx.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
    tx.execute("UPDATE nodes SET seq=?1 WHERE id=?2 AND seq=0", params![n, node])?;
    Ok(())
}

fn next_node_seq_tx(tx: &rusqlite::Transaction) -> Result<u64> {
    let n: i64 = tx.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
    Ok(n as u64 + 1)
}

fn next_version_tx(tx: &rusqlite::Transaction, node: &str) -> Result<i64> {
    let v: Option<i64> = tx
        .query_row("SELECT MAX(version) FROM revisions WHERE node_id=?1", params![node], |r| r.get(0))
        .ok()
        .flatten();
    Ok(v.unwrap_or(0) + 1)
}

impl Store {
    pub fn open(path: &std::path::Path) -> Result<Store> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let mut s = Store { conn };
        s.init_schema()?;
        s.run_old_version_migration()?;
        s.recover_pending()?;
        Ok(s)
    }

    /// 内存库（测试用）。
    pub fn open_memory() -> Result<Store> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut s = Store { conn };
        s.init_schema()?;
        s.run_old_version_migration()?;
        Ok(s)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS nodes(
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                seq INTEGER NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS revisions(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                node_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                kind TEXT NOT NULL,
                parent_revision INTEGER,
                status TEXT NOT NULL,           -- pending | committed
                deps_json TEXT NOT NULL,
                dep_hashes_json TEXT NOT NULL DEFAULT '{}',
                input_hash TEXT NOT NULL,
                data_json TEXT NOT NULL,
                author TEXT NOT NULL,
                base_revision INTEGER,
                created_at TEXT NOT NULL,
                UNIQUE(node_id, version)
            );
            CREATE TABLE IF NOT EXISTS current(
                node_id TEXT PRIMARY KEY,
                revision_id INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS events(
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS conflicts(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                node_id TEXT NOT NULL,
                attempted_base INTEGER NOT NULL,
                current_revision INTEGER NOT NULL,
                proposal_json TEXT NOT NULL,
                author TEXT NOT NULL,
                status TEXT NOT NULL,           -- open | resolved_accept | resolved_discard
                resolution_revision INTEGER,
                created_at TEXT NOT NULL
            );
            "#,
        )?;
        let v: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0))
            .ok();
        match v {
            None => {
                self.conn.execute(
                    "INSERT INTO meta(key,value) VALUES('schema_version','1')",
                    [],
                )?;
            }
            Some(s) if s.parse::<i64>().unwrap_or(1) == SCHEMA_VERSION => {}
            Some(other) => bail!("不支持的 schema_version={other}"),
        }
        Ok(())
    }

    /// 旧版本迁移钩子：v0 数据若没有 note 字段等，这里做最小升级并记录迁移事件。
    fn run_old_version_migration(&self) -> Result<()> {
        let migrated: bool = self
            .conn
            .query_row("SELECT value='1' FROM meta WHERE key='legacy_v0'", [], |r| r.get(0))
            .unwrap_or(false);
        if migrated {
            return Ok(());
        }
        // 演示性迁移：旧库可能在 meta 中带 created_by=saxs-v0；存在则写入迁移事件。
        let legacy: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key='created_by'", [], |r| r.get(0))
            .ok();
        if legacy.as_deref() == Some("saxs-v0") {
            self.conn.execute(
                "INSERT INTO meta(key,value) VALUES('legacy_v0','1')",
                [],
            )?;
            self.conn.execute(
                "INSERT INTO events(kind,payload_json,created_at) VALUES(?1,?2,?3)",
                params![
                    "schema_migrated",
                    serde_json::json!({"from":"saxs-v0","to":"1"}).to_string(),
                    now_iso()
                ],
            )?;
        }
        Ok(())
    }

    /// 恢复：两阶段提交只完成阶段一时，current 仍指向旧版本；
    /// pending 修订永远不会被提升，启动时标记为废弃。
    fn recover_pending(&mut self) -> Result<()> {
        let pending: Vec<(i64, String)> = self
            .conn
            .prepare("SELECT id,node_id FROM revisions WHERE status='pending'")?
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|x| x.ok())
            .collect();
        if !pending.is_empty() {
            let tx = self.conn.transaction()?;
            for (id, node) in &pending {
                tx.execute(
                    "UPDATE revisions SET status='aborted_after_crash' WHERE id=?1",
                    params![id],
                )?;
                tx.execute(
                    "INSERT INTO events(kind,payload_json,created_at) VALUES(?1,?2,?3)",
                    params![
                        "revision_aborted",
                        serde_json::json!({"revision_id":id,"node_id":node,"reason":"commit_interrupted"}).to_string(),
                        now_iso()
                    ],
                )?;
            }
            tx.commit()?;
        }
        Ok(())
    }

    /// 测试钩子：把一个旧版库（saxs-v0 标记）写到指定路径，用于迁移测试。
    pub fn write_legacy_v0_database(path: &std::path::Path) -> Result<()> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta(key,value) VALUES('created_by','saxs-v0');",
        )?;
        drop(conn);
        Ok(())
    }

    // ---------- 两阶段提交 ----------

    /// 阶段一：创建新节点（或为已有节点写入新版本），状态 pending 并持久化。
    /// 不修改 current 指针。返回新修订 id。
    pub fn prepare(&mut self, mut draft: Draft) -> Result<i64> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let node_id = match draft.node_id.take() {
            Some(n) => {
                if !node_exists_tx(&tx, &n) {
                    bail!("节点 {n} 不存在");
                }
                n
            }
            None => {
                let seq = next_node_seq_tx(&tx)?;
                let id = make_id(draft.kind.as_str(), seq);
                ensure_node_tx(&tx, &id, draft.kind)?;
                id
            }
        };
        let version = next_version_tx(&tx, &node_id);
        let rid = insert_revision(
            &tx,
            &node_id,
            version,
            draft.kind.as_str(),
            draft.parent,
            "pending",
            &draft.deps,
            &draft.input_hash,
            &draft.data,
            &draft.author,
            draft.base_revision,
        )?;
        tx.commit()?;
        Ok(rid)
    }

    /// 阶段二：原子翻转 current 指针并写事件。提交后新修订才可见。
    pub fn commit_with_event(&mut self, revision_id: i64, event_kind: &str, mut payload: Value) -> Result<String> {
        let tx = self.conn.transaction()?;
        let (node_id, kind): (String, String) = tx.query_row(
            "SELECT node_id,kind FROM revisions WHERE id=?1 AND status IN ('pending','committed')",
            params![revision_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        tx.execute("UPDATE revisions SET status='committed' WHERE id=?1", params![revision_id])?;
        tx.execute(
            "INSERT INTO current(node_id,revision_id) VALUES(?1,?2)
             ON CONFLICT(node_id) DO UPDATE SET revision_id=excluded.revision_id",
            params![node_id, revision_id],
        )?;
        payload["node_id"] = serde_json::json!(node_id);
        payload["revision_id"] = serde_json::json!(revision_id);
        payload["node_kind"] = serde_json::json!(kind);
        tx.execute(
            "INSERT INTO events(kind,payload_json,created_at) VALUES(?1,?2,?3)",
            params![event_kind, payload.to_string(), now_iso()],
        )?;
        tx.commit()?;
        Ok(node_id)
    }

    /// 中止阶段一（测试或回滚分支用）。
    pub fn abort_prepared(&mut self, revision_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE revisions SET status='aborted' WHERE id=?1 AND status='pending'",
            params![revision_id],
        )?;
        Ok(())
    }

    /// 便捷路径：一次原子地 prepare+commit（供批量导入使用，仍走同一两阶段 API）。
    pub fn commit_draft(&mut self, draft: Draft, event_kind: &str, payload: Value) -> Result<(String, i64)> {
        let rid = self.prepare(draft)?;
        let node = self.commit_with_event(rid, event_kind, payload)?;
        Ok((node, rid))
    }

    // ---------- 查询 ----------

    pub fn revision_by_id(&self, id: i64) -> Result<Option<Revision>> {
        self.conn
            .query_row(
                "SELECT * FROM revisions WHERE id=?1",
                params![id],
                row_to_rev,
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other.into()),
            })
    }

    pub fn current_revision(&self, node: &str) -> Result<Option<Revision>> {
        self.conn
            .query_row(
                "SELECT r.* FROM revisions r JOIN current c ON c.revision_id=r.id WHERE c.node_id=?1",
                params![node],
                row_to_rev,
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other.into()),
            })
    }

    pub fn current_all(&self) -> Result<Vec<Revision>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.* FROM revisions r JOIN current c ON c.revision_id=r.id
             ORDER BY (SELECT seq FROM nodes WHERE id=r.node_id)",
        )?;
        let rows = stmt.query_map([], row_to_rev)?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    pub fn history(&self, node: &str) -> Result<Vec<Revision>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM revisions WHERE node_id=?1 AND status='committed' ORDER BY version",
        )?;
        let rows = stmt.query_map(params![node], row_to_rev)?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    pub fn node_seq(&self, node: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row("SELECT seq FROM nodes WHERE id=?1", params![node], |r| r.get(0))
            .ok())
    }

    pub fn events(&self) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare("SELECT seq,kind,payload_json,created_at FROM events ORDER BY seq")?;
        let rows = stmt.query_map([], |r| {
            let payload: String = r.get(2)?;
            Ok(Event {
                seq: r.get(0)?,
                kind: r.get(1)?,
                payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
                created_at: r.get(3)?,
            })
        })?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    /// 判定某项派生为何仍有效或需要重算：逐依赖比较指纹。
    pub fn validity(&self, revision_id: i64) -> Result<Value> {
        let rev = match self.revision_by_id(revision_id)? {
            Some(r) => r,
            None => bail!("修订 {revision_id} 不存在"),
        };
        let mut deps_state = Vec::new();
        let mut stale = Vec::new();
        for dep in &rev.deps {
            let recorded = rev.dep_hashes.get(dep);
            let recorded_hash = recorded.map(|d| d.input_hash.clone()).unwrap_or_default();
            let recorded_rev = recorded.map(|d| d.revision_id);
            let cur = self.current_revision(dep)?;
            let current_hash = cur.as_ref().map(|c| c.input_hash.clone()).unwrap_or_default();
            let unchanged = cur.as_ref().map(|c| c.input_hash == recorded_hash).unwrap_or(false);
            deps_state.push(serde_json::json!({
                "node_id": dep,
                "revision_at_derivation": recorded_rev,
                "current_revision": cur.as_ref().map(|c| c.id),
                "fingerprint_at_derivation": recorded_hash,
                "current_fingerprint": current_hash,
                "unchanged": unchanged
            }));
            if !unchanged {
                stale.push(dep.clone());
            }
        }
        Ok(serde_json::json!({
            "revision_id": revision_id,
            "node_id": rev.node_id,
            "status": if stale.is_empty() { "valid" } else { "stale_needs_recompute" },
            "reason": if stale.is_empty() {
                "全部来源指纹与推导时一致，结果仍有效"
            } else {
                "来源发生变化，依赖图传播标记为需重算"
            },
            "changed_deps": stale,
            "deps": deps_state
        }))
    }
}


impl Store {
    // ---------- 并发裁定：乐观锁 + 可解决冲突 ----------

    /// 提交人工裁定。base_revision 必须是提交者看到的当前修订；
    /// 若期间他人已移动指针，提案写入 conflicts 保留，返回 Err 冲突信息。
    pub fn propose_decision(
        &mut self,
        node_id: &str,
        kind: NodeKind,
        base_revision: i64,
        data: Value,
        author: &str,
        rationale: &str,
    ) -> Result<Result<(String, i64), ConflictRecord>> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<i64> = tx
            .query_row("SELECT revision_id FROM current WHERE node_id=?1", params![node_id], |r| r.get(0))
            .ok();
        let current = match current {
            Some(c) => c,
            None => bail!("节点 {node_id} 尚无当前版本，无法在此基线上裁定"),
        };
        if current != base_revision {
            tx.execute(
                "INSERT INTO conflicts(node_id,attempted_base,current_revision,proposal_json,author,status,created_at)
                 VALUES(?1,?2,?3,?4,?5,'open',?6)",
                params![node_id, base_revision, current, data.to_string(), author, now_iso()],
            )?;
            let cid = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO events(kind,payload_json,created_at) VALUES(?1,?2,?3)",
                params![
                    "decision_conflict",
                    serde_json::json!({
                        "node_id": node_id,
                        "conflict_id": cid,
                        "attempted_base": base_revision,
                        "current_revision": current,
                        "author": author,
                        "rationale": rationale,
                    })
                    .to_string(),
                    now_iso()
                ],
            )?;
            tx.commit()?;
            return Ok(Err(self.conflict(cid)?.expect("刚写入的冲突")));
        }
        let version = next_version_tx(&tx, node_id)?;
        let input_hash = hash_value(&data);
        let rid = insert_revision(
            &tx,
            node_id,
            version,
            kind.as_str(),
            Some(current),
            "pending",
            &[(node_id.to_string(), current, String::new())],
            &input_hash,
            &data,
            author,
            Some(base_revision),
        )?;
        tx.execute("UPDATE revisions SET status='committed' WHERE id=?1", params![rid])?;
        tx.execute("UPDATE current SET revision_id=?1 WHERE node_id=?2", params![rid, node_id])?;
        tx.execute(
            "INSERT INTO events(kind,payload_json,created_at) VALUES(?1,?2,?3)",
            params![
                "decision_committed",
                serde_json::json!({
                    "node_id": node_id,
                    "revision_id": rid,
                    "base_revision": base_revision,
                    "author": author,
                    "rationale": rationale,
                })
                .to_string(),
                now_iso()
            ],
        )?;
        tx.commit()?;
        Ok(Ok((node_id.to_string(), rid)))
    }

    pub fn conflicts(&self) -> Result<Vec<ConflictRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,node_id,attempted_base,current_revision,proposal_json,author,status,created_at
             FROM conflicts ORDER BY id",
        )?;
        let rows = stmt.query_map([], conflict_row)?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    pub fn conflict(&self, id: i64) -> Result<Option<ConflictRecord>> {
        self.conn
            .query_row(
                "SELECT id,node_id,attempted_base,current_revision,proposal_json,author,status,created_at
                 FROM conflicts WHERE id=?1",
                params![id],
                conflict_row,
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other.into()),
            })
    }

    /// 解决冲突：accept=采用旧提案（在新版本上重放），discard=放弃提案。
    /// 冲突记录保留并标记结果。
    pub fn resolve_conflict(
        &mut self,
        conflict_id: i64,
        accept: bool,
        resolver: &str,
    ) -> Result<Option<(String, i64)>> {
        let conf = match self.conflict(conflict_id)? {
            Some(c) if c.status == "open" => c,
            Some(_) => bail!("冲突 {conflict_id} 已解决"),
            None => return Ok(None),
        };
        let current = self
            .current_revision(&conf.node_id)?
            .ok_or_else(|| anyhow::anyhow!("节点当前版本丢失"))?;
        let node_kind: String = self.conn.query_row(
            "SELECT kind FROM nodes WHERE id=?1",
            params![conf.node_id],
            |r| r.get(0),
        )?;
        let mut applied: Option<(String, i64)> = None;
        let tx = self.conn.transaction()?;
        let status = if accept {
            let v = next_version_tx(&tx, &conf.node_id)?;
            let h = hash_value(&conf.proposal);
            let rid = insert_revision(
                &tx,
                &conf.node_id,
                v,
                node_kind.clone(),
                Some(current.id),
                "pending",
                &[(conf.node_id.clone(), current.id, current.input_hash.clone())],
                &h,
                &conf.proposal,
                resolver,
                Some(conf.current_revision),
            )?;
            tx.execute("UPDATE revisions SET status='committed' WHERE id=?1", params![rid])?;
            tx.execute("UPDATE current SET revision_id=?1 WHERE node_id=?2", params![rid, conf.node_id])?;
            applied = Some((conf.node_id.clone(), rid));
            "resolved_accept"
        } else {
            "resolved_discard"
        };
        tx.execute(
            "UPDATE conflicts SET status=?1,resolution_revision=?2 WHERE id=?3",
            params![status, applied.as_ref().map(|x| x.1), conflict_id],
        )?;
        tx.execute(
            "INSERT INTO events(kind,payload_json,created_at) VALUES(?1,?2,?3)",
            params![
                "conflict_resolved",
                serde_json::json!({
                    "conflict_id": conflict_id,
                    "node_id": conf.node_id,
                    "accepted": accept,
                    "resolver": resolver,
                    "resolution_revision": applied.as_ref().map(|x| x.1),
                })
                .to_string(),
                now_iso()
            ],
        )?;
        tx.commit()?;
        Ok(applied)
    }
}

fn conflict_row(r: &rusqlite::Row) -> rusqlite::Result<ConflictRecord> {
    let p: String = r.get(4)?;
    Ok(ConflictRecord {
        id: r.get(0)?,
        node_id: r.get(1)?,
        attempted_base: r.get(2)?,
        current_revision: r.get(3)?,
        proposal: serde_json::from_str(&p).unwrap_or(Value::Null),
        author: r.get(5)?,
        status: r.get(6)?,
        created_at: r.get(7)?,
    })
}
