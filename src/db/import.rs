//! Archive export/import. The archive is a plain JSON snapshot of every
//! provenance table with explicit ordering. Importing into an empty database
//! preserves ids, sequence numbers and source relations; importing on top of
//! existing data is rejected so identities cannot collide.

use rusqlite::Connection;

use crate::db::error::app_err;
use serde_json::{json, Value};

const TABLES: &[(&str, &str)] = &[
    ("meta", "key"),
    ("projects", "project_id"),
    ("instruments", "instrument_id"),
    ("masks", "mask_id"),
    ("raw_records", "record_id,version"),
    ("raw_points", "record_id,version,ordinal"),
    ("buffer_candidates", "project_id,sample_record_id,buffer_record_id"),
    ("derivations", "node"),
    ("node_config", "node"),
    ("derivation_versions", "node,version"),
    ("version_deps", "node,version,dep_kind,ref_id"),
    ("artifacts", "artifact_id"),
    ("node_decisions", "node"),
    ("events", "project_id,seq"),
    ("pointers_log", "project_id,seq,node"),
    ("point_lineage", "point_id"),
];

fn columns(c: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut s = c.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = s.query_map([], |r| r.get::<_, String>(1))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn dump(c: &Connection) -> rusqlite::Result<Value> {
    let mut tables = serde_json::Map::new();
    for (table, order) in TABLES {
        let cols = columns(c, table)?;
        let sql = format!("SELECT {} FROM {} ORDER BY {}", cols.join(","), table, order);
        let mut s = c.prepare(&sql)?;
        let rows = s.query_map([], |r| {
            let mut obj = serde_json::Map::new();
            for (i, col) in cols.iter().enumerate() {
                let v: Value = match r.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => json!(n),
                    rusqlite::types::ValueRef::Real(f) => json!(f),
                    rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t).as_ref()),
                    rusqlite::types::ValueRef::Blob(b) => json!(format!("blob:{}", b.len())),
                };
                obj.insert(col.clone(), v);
            }
            Ok(Value::Object(obj))
        })?;
        let collected = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        tables.insert((*table).to_string(), Value::Array(collected));
    }
    Ok(json!({
        "archive": "saxs-bench",
        "schema_version": c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
        "tables": Value::Object(tables),
    }))
}

pub fn load(c: &mut Connection, archive: &Value) -> rusqlite::Result<()> {
    let existing: i64 = c.query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))?;
    if existing > 0 {
        return Err(app_err("import target database is not empty"));
    }
    let tables = archive
        .get("tables")
        .and_then(|t| t.as_object())
        .ok_or_else(|| app_err("archive missing tables"))?;
    let tx = c.transaction()?;
    // Insert in foreign-key-safe declaration order (TABLES already respects it).
    for (table, _order) in TABLES {
        let Some(rows) = tables.get(*table).and_then(|v| v.as_array()) else { continue };
        for row in rows {
            let obj = row.as_object().ok_or_else(|| app_err("row not object"))?;
            let cols: Vec<&String> = obj.keys().collect();
            let col_sql = cols.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(",");
            let placeholders = (1..=cols.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(",");
            let sql = format!("INSERT INTO {table}({col_sql}) VALUES ({placeholders})");
            let values: Vec<Box<dyn rusqlite::types::ToSql>> = cols
                .iter()
                .map(|k| {
                    let b: Box<dyn rusqlite::types::ToSql> = match &obj[*k] {
                        Value::Null => Box::new(None::<String>),
                        Value::Bool(b) => Box::new(if *b { 1i64 } else { 0i64 }),
                        Value::Number(n) => Box::new(n.as_f64().unwrap_or(0.0)),
                        Value::String(s) => Box::new(s.clone()),
                        other => Box::new(other.to_string()),
                    };
                    b
                })
                .collect();
            let refs: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|b| b.as_ref()).collect();
            tx.execute(&sql, refs.as_slice())?;
        }
    }
    if let Some(v) = archive.get("schema_version").and_then(|v| v.as_i64()) {
        tx.execute_batch(&format!("PRAGMA user_version = {v}"))?;
    }
    tx.commit()?;
    Ok(())
}
