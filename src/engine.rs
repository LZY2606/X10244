use crate::domain::*;
use crate::rules::*;
use crate::store::{commit_event, Event, Store};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct InputRef(pub Value);

pub struct RecomputeEntry {
    pub seq: i64,
    pub old_id: String,
    pub new_id: String,
    pub reason: String,
}

fn db_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn id_of(kind: &str, v: &Value) -> String {
    crate::ids::content_id(kind, v)
}

/// 追加并物化一个事件（同步重算受影响派生）。
pub fn apply(store: &Store, actor: &str, kind: &str, payload: &Value, base_seq: Option<i64>) -> Result<Event, String> {
    commit_event(store, actor, kind, payload, base_seq, |tx, ev| match kind {
        "import_source" => apply_import(tx, ev),
        "derive" => apply_derive(tx, ev),
        "set_pointer" => {
            let (old, new) = switch_pointer(tx, ev)?;
            propagate(
                tx,
                ev.seq,
                &[(old.clone(), new.clone())],
                &format!("缓冲液版本切换（指针 {} 从 {} 移到 {}）", payload_str(payload, "key"), &old[..old.len().min(18)], &new[..new.len().min(18)]),
            )?;
            Ok(())
        }
        "source_revision" => {
            let (old, new) = revision_pair(tx, ev)?;
            propagate(
                tx,
                ev.seq,
                &[(old.clone(), new.clone())],
                "源数据修订（新版本取代旧版本），仅受影响子图重算",
            )?;
            Ok(())
        }
        "decide" => apply_decide(tx, ev),
        other => Err(format!("未知事件类型 {other}")),
    })
}

fn payload_str(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn next_version(tx: &Connection, slot: &str) -> rusqlite::Result<i64> {
    let v: Option<i64> = tx
        .query_row("SELECT MAX(version) FROM sources WHERE slot=?", params![slot], |r| r.get(0))
        .optional()?;
    Ok(v.unwrap_or(0) + 1)
}

fn apply_import(tx: &Connection, ev: &Event) -> Result<(), String> {
    let p = &ev.payload;
    let slot = payload_str(p, "slot");
    let kind = payload_str(p, "source_kind");
    let supersede = p.get("supersede").and_then(|x| x.as_bool()).unwrap_or(false);
    let version = next_version(tx, &slot).map_err(db_err)?;
    let supersedes: Option<String> = if supersede {
        tx.query_row(
            "SELECT id FROM sources WHERE slot=? AND id NOT IN (SELECT supersedes FROM sources WHERE supersedes IS NOT NULL) ORDER BY version DESC LIMIT 1",
            params![slot],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?
    } else {
        None
    };
    let body = p.get("body").cloned().unwrap_or(json!({}));
    let idv = json!({"slot": slot, "kind": kind, "version": version, "body": body});
    let id = id_of("src", &idv);
    tx.execute(
        "INSERT OR REPLACE INTO sources(id,slot,kind,version,supersedes,payload,created_seq) VALUES(?,?,?,?,?,?,?)",
        params![id, slot, kind, version, supersedes, body.to_string(), ev.seq],
    )
    .map_err(db_err)?;
    if version == 1 {
        tx.execute(
            "INSERT OR IGNORE INTO pointers(key,value,updated_seq) VALUES(?,?,?)",
            params![slot, id, ev.seq],
        )
        .map_err(db_err)?;
    }
    Ok(())
}


fn get_source(tx: &Connection, id: &str) -> rusqlite::Result<Option<Value>> {
    let s: Option<String> = tx
        .query_row("SELECT payload FROM sources WHERE id=?", params![id], |r| r.get(0))
        .optional()?;
    Ok(s.map(|s| serde_json::from_str(&s).unwrap_or(json!({}))))
}

fn get_derived(tx: &Connection, id: &str) -> rusqlite::Result<Option<(String, Value, Vec<String>)>> {
    let row: Option<(String, String, String)> = tx
        .query_row(
            "SELECT kind,payload,inputs FROM derived WHERE id=?",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(row.map(|(k, p, ins)| {
        (
            k,
            serde_json::from_str(&p).unwrap_or(json!({})),
            serde_json::from_str(&ins).unwrap_or_default(),
        )
    }))
}

fn raw_curve_from(v: &Value) -> RawCurve {
    RawCurve {
        points: serde_json::from_value(v.get("points").cloned().unwrap_or(json!([]))).unwrap_or_default(),
        exposure_s: v.get("exposure_s").and_then(|x| x.as_f64()).unwrap_or(1.0),
        thickness_cm: v.get("thickness_cm").and_then(|x| x.as_f64()).unwrap_or(0.1),
        transmission: v.get("transmission").and_then(|x| x.as_f64()).unwrap_or(1.0),
        description: v.get("description").and_then(|x| x.as_str()).unwrap_or("").to_string(),
    }
}

/// 执行一次派生计算，返回 (payload, diagnostics, candidates)。
fn compute(tx: &Connection, derivation: &str, inputs: &Value, params: &Value)
    -> Result<(Value, Vec<Diagnostic>, Vec<ScaleCandidate>), String> {
    match derivation {
        "normalize" => {
            let sid = inputs.get("curve").and_then(|x| x.as_str()).ok_or("缺少 inputs.curve")?;
            let body = get_source(tx, sid).map_err(db_err)?.ok_or_else(|| format!("来源 {sid} 不存在"))?;
            let raw = raw_curve_from(&body);
            let p = NormParams { scale_uncertainty: params.get("norm_uncertainty").and_then(|x| x.as_f64()).unwrap_or(0.0) };
            let (pts, diags) = normalize(&raw, &p);
            Ok((json!({"points": pts, "mode": "normalize", "overlap": Value::Null}), diags, vec![]))
        }
        "subtract" => {
            let s = norm_points(tx, inputs, "sample")?;
            let b = norm_points(tx, inputs, "buffer")?;
            let p = SubParams {
                scale: params.get("scale").and_then(|x| x.as_f64()).unwrap_or(1.0),
                scale_sigma: params.get("scale_sigma").and_then(|x| x.as_f64()).unwrap_or(0.0),
            };
            let out = subtract(&s, &b, &p);
            Ok((
                json!({"points": out.points, "mode": out.mode, "overlap": out.overlap}),
                out.diagnostics,
                out.candidates,
            ))
        }
        "fit" => {
            let cid = inputs.get("curve").and_then(|x| x.as_str()).ok_or("缺少 inputs.curve")?;
            let (_, payload, _) = get_derived(tx, cid).map_err(db_err)?.ok_or_else(|| format!("派生 {cid} 不存在"))?;
            let points: Vec<SubPoint> =
                serde_json::from_value(payload.get("points").cloned().unwrap_or(json!([]))).unwrap_or_default();
            let specs: Vec<IntervalSpec> =
                serde_json::from_value(params.get("intervals").cloned().unwrap_or(json!([]))).unwrap_or_default();
            let (fits, diags) = fit_intervals(&points, &specs);
            Ok((json!({"fits": fits}), diags, vec![]))
        }
        other => Err(format!("未知派生类型 {other}")),
    }
}

fn norm_points(tx: &Connection, inputs: &Value, key: &str) -> Result<Vec<NormPoint>, String> {
    let id = inputs.get(key).and_then(|x| x.as_str()).ok_or_else(|| format!("缺少 inputs.{key}"))?;
    let (_, payload, _) = get_derived(tx, id).map_err(db_err)?.ok_or_else(|| format!("归一化曲线 {id} 不存在"))?;
    Ok(serde_json::from_value(payload.get("points").cloned().unwrap_or(json!([]))).unwrap_or_default())
}

fn insert_derived(
    tx: &Connection,
    ev: &Event,
    derivation: &str,
    inputs: &Value,
    params: &Value,
) -> Result<String, String> {
    let (payload, diags, candidates) = compute(tx, derivation, inputs, params)?;
    let idv = json!({"kind": derivation, "inputs": inputs, "params": params});
    let id = id_of("der", &idv);
    tx.execute(
        "INSERT OR REPLACE INTO derived(id,kind,inputs,params,payload,diagnostics,candidates,status,created_seq)
         VALUES(?,?,?,?,?,?,?, 'valid', ?)",
        params![
            id,
            derivation,
            inputs.to_string(),
            params.to_string(),
            payload.to_string(),
            serde_json::to_string(&diags).unwrap_or_default(),
            serde_json::to_string(&candidates).unwrap_or_default(),
            ev.seq
        ],
    )
    .map_err(db_err)?;
    Ok(id)
}

fn apply_derive(tx: &Connection, ev: &Event) -> Result<(), String> {
    let p = &ev.payload;
    let derivation = payload_str(p, "derivation");
    let inputs = p.get("inputs").cloned().unwrap_or(json!({}));
    let params = p.get("params").cloned().unwrap_or(json!({}));
    insert_derived(tx, ev, &derivation, &inputs, &params)?;
    Ok(())
}

fn resolve_refs(inputs: &Value, map: &std::collections::HashMap<String, String>) -> (Value, bool) {
    let mut changed = false;
    let v = inputs
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| {
                    if let Some(s) = v.as_str() {
                        if let Some(n) = map.get(s) {
                            changed = true;
                            return (k.clone(), Value::String(n.clone()));
                        }
                    }
                    (k.clone(), v.clone())
                })
                .collect::<serde_json::Map<String, Value>>()
        })
        .unwrap_or_default();
    (Value::Object(v), changed)
}

fn record_recompute(
    tx: &Connection,
    seq: i64,
    old_id: &str,
    new_id: &str,
    reason: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO recomputes(seq,old_id,new_id,reason) VALUES(?,?,?,?)",
        params![seq, old_id, new_id, reason],
    )?;
    Ok(())
}

/// 依赖图传播：只重算输入闭包中实际受影响的派生节点，顺序沿依赖方向。
fn propagate(
    tx: &Connection,
    seq: i64,
    seeds: &[(String, String)],
    reason: &str,
) -> Result<Vec<(String, String)>, String> {
    if seeds.is_empty() {
        return Ok(vec![]);
    }
    let mut map: std::collections::HashMap<String, String> =
        seeds.iter().map(|(o, n)| (o.clone(), n.clone())).collect();
    let mut report = Vec::new();

    loop {
        let mut rows = tx
            .prepare(
                "SELECT id,kind,inputs,params,created_seq FROM derived
                 WHERE status='valid' ORDER BY created_seq, id",
            )
            .map_err(db_err)?;
        let nodes: Vec<(String, String, Value, Value, i64)> = rows
            .query_map([], |r| {
                let ins: String = r.get(2)?;
                let prm: String = r.get(3)?;
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?,
                    serde_json::from_str(&ins).unwrap_or(json!({})),
                    serde_json::from_str(&prm).unwrap_or(json!({})), r.get(4)?))
            })
            .map_err(db_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?;
        let mut progressed = false;
        for (old_id, kind, inputs, params, created) in nodes {
            if map.contains_key(&old_id) {
                continue;
            }
            let (new_inputs, changed) = resolve_refs(&inputs, &map);
            if !changed {
                continue;
            }
            let fake = Event {
                seq,
                id: format!("recompute@{seq}"),
                ts: String::new(),
                actor: "system".into(),
                kind: "derive".into(),
                payload: json!({}),
                base_seq: None,
            };
            let new_id = insert_derived(tx, &fake, &kind, &new_inputs, &params)?;
            let why = format!("{reason}（原派生创建于 seq {created}）");
            record_recompute(tx, seq, &old_id, &new_id, &why).map_err(db_err)?;
            map.insert(old_id.clone(), new_id.clone());
            report.push((old_id, new_id));
            progressed = true;
        }
        if !progressed {
            break;
        }
    }
    Ok(report)
}

fn switch_pointer(tx: &Connection, ev: &Event) -> Result<(String, String), String> {
    let key = payload_str(&ev.payload, "key");
    let new = payload_str(&ev.payload, "source_id");
    let old: Option<String> = tx
        .query_row("SELECT value FROM pointers WHERE key=?", params![key], |r| r.get(0))
        .optional()
        .map_err(db_err)?;
    let exists: i64 = tx
        .query_row("SELECT COUNT(*) FROM sources WHERE id=?", params![new], |r| r.get(0))
        .map_err(db_err)?;
    if exists == 0 {
        return Err(format!("指针 {key} 指向不存在的来源 {new}"));
    }
    tx.execute(
        "INSERT INTO pointers(key,value,updated_seq) VALUES(?,?,?)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_seq=excluded.updated_seq",
        params![key, new, ev.seq],
    )
    .map_err(db_err)?;
    Ok((old.unwrap_or_default(), new))
}

fn revision_pair(tx: &Connection, ev: &Event) -> Result<(String, String), String> {
    let p = &ev.payload;
    let slot = payload_str(p, "slot");
    let kind = payload_str(p, "source_kind");
    let body = p.get("body").cloned().unwrap_or(json!({}));
    let version = next_version(tx, &slot).map_err(db_err)?;
    // “旧版本”= 当前指针指向的版本（回退到最高版本号）。
    let old: Option<String> = tx
        .query_row("SELECT value FROM pointers WHERE key=?", params![slot], |r| r.get(0))
        .optional()
        .map_err(db_err)?
        .or_else(|| {
            tx.query_row(
                "SELECT id FROM sources WHERE slot=? ORDER BY version DESC LIMIT 1",
                params![slot],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
        });
    let idv = json!({"slot": slot, "kind": kind, "version": version, "body": body});
    let id = id_of("src", &idv);
    tx.execute(
        "INSERT INTO sources(id,slot,kind,version,supersedes,payload,created_seq) VALUES(?,?,?,?,?,?,?)",
        params![id, slot, kind, version, old, body.to_string(), ev.seq],
    )
    .map_err(db_err)?;
    tx.execute(
        "INSERT INTO pointers(key,value,updated_seq) VALUES(?,?,?)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_seq=excluded.updated_seq",
        params![slot, id, ev.seq],
    )
    .map_err(db_err)?;
    Ok((old.unwrap_or_default(), id))
}

fn apply_decide(tx: &Connection, ev: &Event) -> Result<(), String> {
    let p = &ev.payload;
    let target = payload_str(p, "target");
    let action = payload_str(p, "action");
    let note = payload_str(p, "note");
    let idv = json!({"target": target, "action": action, "note": note, "seq": ev.seq});
    let id = id_of("dec", &idv);
    let branch_base = p.get("branch_base_seq").and_then(|x| x.as_i64()).unwrap_or(ev.base_seq.unwrap_or(ev.seq - 1));

    // 与同一目标上尚未解决、且结论不同的最新决定构成可解决冲突；双方都保留。
    let conflicted: Option<String> = tx
        .query_row(
            "SELECT id FROM decisions WHERE target=? AND resolved_by IS NULL AND action<>? ORDER BY created_seq DESC LIMIT 1",
            params![target, action],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;
    let is_conflict = conflicted.is_some();
    if let Some(other) = &conflicted {
        tx.execute("UPDATE decisions SET conflict=1 WHERE id=?", params![other]).map_err(db_err)?;
    }
    tx.execute(
        "INSERT INTO decisions(id,target,action,branch_base_seq,note,payload,conflict,resolved_by,created_seq)
         VALUES(?,?,?,?,?,?,?,NULL,?)",
        params![id, target, action, branch_base, note, p.to_string(), is_conflict as i64, ev.seq],
    )
    .map_err(db_err)?;
    if p.get("resolves").and_then(|x| x.as_str()).is_some() {
        // 解决事件：把同一目标上所有未解决的决定标记为已被本次裁定解决。
        tx.execute(
            "UPDATE decisions SET resolved_by=?, conflict=0 WHERE target=? AND resolved_by IS NULL",
            params![id, target],
        )
        .map_err(db_err)?;
    }
    Ok(())
}
