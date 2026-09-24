use crate::engine::apply;
use crate::store::{Event, Store};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

impl Store {
    pub fn open_memory() -> rusqlite::Result<Self> {
        let conn = rusqlite::Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let store = Store { conn: std::sync::Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }
}

/// 从空库重放全部事件（用于导入与时间旅行）。
pub fn replay(store: &Store, events: &[Event]) -> Result<(), String> {
    store.wipe().map_err(|e| e.to_string())?;
    for ev in events {
        apply(store, &ev.actor, &ev.kind, &ev.payload, ev.base_seq)?;
    }
    Ok(())
}

/// 构建给复核界面的完整状态。at_seq 给定时在内存中重放到该事件（时间旅行）。
pub fn build_state(store: &Store, at_seq: Option<i64>) -> Result<Value, String> {
    let (target, events) = match at_seq {
        Some(seq) => {
            let mem = Store::open_memory().map_err(|e| e.to_string())?;
            let all = store.events().map_err(|e| e.to_string())?;
            let cut: Vec<Event> = all.into_iter().filter(|e| e.seq <= seq).collect();
            replay(&mem, &cut)?;
            (mem, cut)
        }
        None => {
            let all = store.events().map_err(|e| e.to_string())?;
            return build_from(store, &all);
        }
    };
    build_from(&target, &events)
}

fn build_from(store: &Store, events: &[Event]) -> Result<Value, String> {
    let conn = store.conn.lock().unwrap();

    let pointers: Vec<(String, String, i64)> = {
        let mut st = conn.prepare("SELECT key,value,updated_seq FROM pointers").map_err(|e| e.to_string())?;
        let mapped = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(|e| e.to_string())?;
        mapped.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())?
    };
    let current: std::collections::HashSet<String> = pointers.iter().map(|(_, v, _)| v.clone()).collect();

    let sources = {
        let mut st = conn
            .prepare("SELECT id,slot,kind,version,supersedes,payload,created_seq FROM sources ORDER BY slot,version")
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map([], |r| {
                let payload: String = r.get(5)?;
                let sup: Option<String> = r.get(4)?;
                Ok(json!({
                    "id": r.get::<_,String>(0)?,
                    "slot": r.get::<_,String>(1)?,
                    "source_kind": r.get::<_,String>(2)?,
                    "version": r.get::<_,i64>(3)?,
                    "supersedes": sup,
                    "current": current.contains(&r.get::<_,String>(0)?),
                    "payload": serde_json::from_str::<Value>(&payload).unwrap_or(json!({})),
                    "created_seq": r.get::<_,i64>(6)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        let mut v = Vec::new();
        for r in rows { v.push(r.map_err(|e| e.to_string())?); }
        v
    };

    let recomputes = {
        let mut st = conn.prepare("SELECT seq,old_id,new_id,reason FROM recomputes ORDER BY seq").map_err(|e| e.to_string())?;
        let rows = st.query_map([], |r| {
            Ok(json!({"seq": r.get::<_,i64>(0)?, "old_id": r.get::<_,String>(1)?,
                      "new_id": r.get::<_,String>(2)?, "reason": r.get::<_,String>(3)?}))
        }).map_err(|e| e.to_string())?;
        let mut v = Vec::new();
        for r in rows { v.push(r.map_err(|e| e.to_string())?); }
        v
    };
    drop(conn);

    let derived = {
        let conn = store.conn.lock().unwrap();
        let mut st = conn
            .prepare("SELECT id,kind,inputs,params,payload,diagnostics,candidates,created_seq FROM derived ORDER BY created_seq,id")
            .map_err(|e| e.to_string())?;
        let rows = st.query_map([], |r| {
            let inputs: String = r.get(2)?;
            let params_s: String = r.get(3)?;
            let payload: String = r.get(4)?;
            let diags: String = r.get(5)?;
            let cands: String = r.get(6)?;
            Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?,
                serde_json::from_str::<Value>(&inputs).unwrap_or(json!({})),
                serde_json::from_str::<Value>(&params_s).unwrap_or(json!({})),
                serde_json::from_str::<Value>(&payload).unwrap_or(json!({})),
                serde_json::from_str::<Value>(&diags).unwrap_or(json!([])),
                serde_json::from_str::<Value>(&cands).unwrap_or(json!([])),
                r.get::<_,i64>(7)?))
        }).map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        for r in rows { raw.push(r.map_err(|e| e.to_string())?); }
        raw
    };

    let reasons = {
        let mut m = std::collections::HashMap::<String, Value>::new();
        for r in &recomputes {
            m.insert(r["old_id"].as_str().unwrap_or("").to_string(), json!({
                "state": "superseded",
                "reason": format!("seq {} 因 {} 重算，新身份 {}", r["seq"], r["reason"], r["new_id"]),
                "replaced_by": r["new_id"]
            }));
            m.insert(r["new_id"].as_str().unwrap_or("").to_string(), json!({
                "state": "recomputed",
                "reason": format!("seq {} 因 {} 重算产生", r["seq"], r["reason"])
            }));
        }
        m
    };

    let derived_v: Vec<Value> = derived
        .iter()
        .map(|(id, kind, inputs, params, payload, diags, cands, created)| {
            let validity = reasons.get(id).cloned().unwrap_or_else(|| json!({
                "state": "valid",
                "reason": format!("自 seq {} 创建以来输入指纹未变，结果仍然有效", created)
            }));
            json!({
                "id": id, "kind": kind, "inputs": inputs, "params": params,
                "payload": payload, "diagnostics": diags, "candidates": cands,
                "created_seq": created, "validity": validity
            })
        })
        .collect();

    let decisions = {
        let conn = store.conn.lock().unwrap();
        let mut st = conn
            .prepare("SELECT id,target,action,branch_base_seq,note,conflict,resolved_by,created_seq FROM decisions ORDER BY created_seq")
            .map_err(|e| e.to_string())?;
        let rows = st.query_map([], |r| {
            let resolved: Option<String> = r.get(6)?;
            Ok(json!({
                "id": r.get::<_,String>(0)?, "target": r.get::<_,String>(1)?,
                "action": r.get::<_,String>(2)?, "branch_base_seq": r.get::<_,i64>(3)?,
                "note": r.get::<_,String>(4)?, "conflict": r.get::<_,i64>(5)? == 1,
                "resolved_by": resolved, "created_seq": r.get::<_,i64>(7)?
            }))
        }).map_err(|e| e.to_string())?;
        let mut v = Vec::new();
        for r in rows { v.push(r.map_err(|e| e.to_string())?); }
        v
    };

    let event_list: Vec<Value> = events
        .iter()
        .map(|e| json!({"seq": e.seq, "id": e.id, "actor": e.actor, "kind": e.kind,
                        "payload": e.payload, "base_seq": e.base_seq, "ts": e.ts}))
        .collect();

    Ok(json!({
        "head_seq": events.last().map(|e| e.seq).unwrap_or(0),
        "at_seq": events.last().map(|e| e.seq).unwrap_or(0),
        "events": event_list,
        "sources": sources,
        "derived": derived_v,
        "decisions": decisions,
        "pointers": pointers.iter().map(|(k,v,s)| json!({"key":k,"source_id":v,"updated_seq":s})).collect::<Vec<_>>(),
        "recomputes": recomputes
    }))
}

/// 来源追溯：返回派生节点及其直接输入（点击派生点时界面据此高亮两条源曲线）。
pub fn provenance(store: &Store, derived_id: &str) -> Result<Value, String> {
    let conn = store.conn.lock().unwrap();
    let row: Option<(String, String, String, String, String, String, i64)> = conn
        .query_row(
            "SELECT kind,inputs,params,payload,diagnostics,candidates,created_seq FROM derived WHERE id=?",
            params![derived_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let (kind, inputs, params, payload, diags, cands, created) =
        row.ok_or_else(|| format!("派生 {derived_id} 不存在"))?;
    drop(conn);

    let inputs_v: Value = serde_json::from_str(&inputs).unwrap_or(json!({}));
    let mut resolved = Vec::new();
    for (key, val) in inputs_v.as_object().unwrap_or(&serde_json::Map::new()) {
        if let Some(id) = val.as_str() {
            resolved.push(json!({
                "slot": key, "id": id, "record": input_record(store, id)?
            }));
        }
    }

    Ok(json!({
        "id": derived_id, "kind": kind,
        "params": serde_json::from_str::<Value>(&params).unwrap_or(json!({})),
        "inputs": inputs_v,
        "payload": serde_json::from_str::<Value>(&payload).unwrap_or(json!({})),
        "diagnostics": serde_json::from_str::<Value>(&diags).unwrap_or(json!([])),
        "candidates": serde_json::from_str::<Value>(&cands).unwrap_or(json!([])),
        "created_seq": created,
        "resolved_inputs": resolved
    }))
}

fn input_record(store: &Store, id: &str) -> Result<Value, String> {
    let conn = store.conn.lock().unwrap();
    if let Ok(Some(s)) = conn
        .query_row("SELECT slot,kind,version,payload FROM sources WHERE id=?", params![id], |r| {
            let p: String = r.get(3)?;
            Ok(json!({"type":"source","slot":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,
                      "version":r.get::<_,i64>(2)?,
                      "payload":serde_json::from_str::<Value>(&p).unwrap_or(json!({}))}))
        })
        .optional()
    {
        return Ok(s);
    }
    conn.query_row(
        "SELECT kind,payload,diagnostics,candidates FROM derived WHERE id=?",
        params![id],
        |r| {
            let p: String = r.get(1)?;
            let d: String = r.get(2)?;
            let c: String = r.get(3)?;
            Ok(json!({"type":"derived","kind":r.get::<_,String>(0)?,
                      "payload":serde_json::from_str::<Value>(&p).unwrap_or(json!({})),
                      "diagnostics":serde_json::from_str::<Value>(&d).unwrap_or(json!([])),
                      "candidates":serde_json::from_str::<Value>(&c).unwrap_or(json!([]))}))
        },
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("记录 {id} 不存在"))
}
