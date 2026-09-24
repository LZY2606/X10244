//! Derivation engine: single execution path for normalize / subtract /
//! interval-fit derivations, used by seeding, API actions and recompute.

use crate::db::{R, Sql};
use crate::science;
use crate::store;
use rusqlite::Connection;
use serde_json::{json, Value};

pub struct CandidateOut {
    pub kind: String,
    pub label: String,
    pub q_min: f64,
    pub q_max: f64,
    pub fit: Value,
}

pub struct EngineOutput {
    pub kind: String,
    pub params: Value,
    pub inputs: Value,
    pub result: Option<Value>,
    pub diagnostics: Vec<science::Diagnostic>,
    pub status: String, // "complete" | "failed"
    pub candidates: Vec<CandidateOut>,
}

fn input_ref(id: i64, uid: &str, kind: &str, label: &str) -> Value {
    json!({"id": id, "uid": uid, "node": kind, "label": label})
}

pub fn run_normalize(
    conn: &Connection,
    case_id: i64,
    sample_curve_id: i64,
    factors: &science::NormFactors,
) -> R<EngineOutput> {
    let (curve, uid, label, _meta) = store::load_curve(conn, sample_curve_id)?;
    let inputs = json!([input_ref(sample_curve_id, &uid, "curve", &label)]);
    let params = json!({
        "transmission": factors.transmission,
        "transmission_rel_err": factors.transmission_rel_err,
        "exposure_s": factors.exposure_s,
        "thickness_mm": factors.thickness_mm,
        "thickness_rel_err": factors.thickness_rel_err,
    });
    let mut diagnostics = Vec::new();
    let converted = match science::convert_q_unit(&curve, science::CANONICAL_Q_UNIT) {
        Ok(c) => c,
        Err(diag) => {
            diagnostics.push(diag);
            return Ok(EngineOutput {
                kind: "normalize".into(),
                params,
                inputs,
                result: None,
                diagnostics,
                status: "failed".into(),
                candidates: vec![],
            });
        }
    };
    if converted.unit_q != curve.unit_q {
        diagnostics.push(science::Diagnostic {
            code: "unit.converted".into(),
            message: format!(
                "q unit converted {} -> {} at derivation layer; raw import untouched",
                curve.unit_q,
                science::CANONICAL_Q_UNIT
            ),
            indices: vec![],
        });
    }
    let (norm, mut diags) = science::normalize(&converted, factors);
    diagnostics.append(&mut diags);
    let _ = case_id;
    Ok(EngineOutput {
        kind: "normalize".into(),
        params,
        inputs,
        result: Some(json!({
            "curve": norm,
            "factors": factors,
            "source": {"id": sample_curve_id, "uid": uid, "label": label},
        })),
        diagnostics,
        status: "complete".into(),
        candidates: vec![],
    })
}

pub fn run_subtract(
    conn: &Connection,
    _case_id: i64,
    normalize_derivation_id: i64,
    buffer_curve_id: i64,
    scale: f64,
    scale_err: f64,
) -> R<EngineOutput> {
    let norm = store::load_derivation(conn, normalize_derivation_id)?;
    let norm_result = norm
        .result
        .clone()
        .ok_or_else(|| "normalize derivation has no result".to_string())?;
    let sample: science::Curve = serde_json::from_value(norm_result["curve"].clone()).s()?;
    let (buffer_raw, buid, blabel, _m) = store::load_curve(conn, buffer_curve_id)?;
    let inputs = json!([
        input_ref(norm.id, &norm.uid, "derivation", "normalize"),
        input_ref(buffer_curve_id, &buid, "curve", &blabel),
    ]);
    let params = json!({"scale": scale, "scale_err": scale_err});
    let mut diagnostics = Vec::new();
    let buffer = match science::convert_q_unit(&buffer_raw, science::CANONICAL_Q_UNIT) {
        Ok(c) => c,
        Err(diag) => {
            diagnostics.push(diag);
            return Ok(EngineOutput {
                kind: "subtract".into(),
                params,
                inputs,
                result: None,
                diagnostics,
                status: "failed".into(),
                candidates: vec![],
            });
        }
    };
    let aligned = match science::align(&sample, &buffer) {
        Ok(a) => a,
        Err(no) => {
            diagnostics.push(science::Diagnostic {
                code: "grid.no_overlap".into(),
                message: no.message,
                indices: vec![],
            });
            return Ok(EngineOutput {
                kind: "subtract".into(),
                params,
                inputs,
                result: None,
                diagnostics,
                status: "failed".into(),
                candidates: vec![],
            });
        }
    };
    let (s_lo, s_hi) = sample.q_range();
    let (b_lo, b_hi) = buffer.q_range();
    let (lo, hi) = (s_lo.max(b_lo), s_hi.min(b_hi));
    if s_lo != b_lo || s_hi != b_hi {
        diagnostics.push(science::Diagnostic {
            code: "grid.partial_overlap".into(),
            message: format!(
                "q grids differ; interpolation restricted to overlap [{:.4}, {:.4}] (sample [{:.4}, {:.4}], buffer [{:.4}, {:.4}]); no direct subtraction (rule: grid.no_direct_subtract)",
                lo, hi, s_lo, s_hi, b_lo, b_hi
            ),
            indices: vec![],
        });
    }
    let (sub, src) = science::subtract(&aligned, scale, scale_err);
    let negs = sub.negative_indices();
    if !negs.is_empty() {
        diagnostics.push(science::Diagnostic {
            code: "intensity.negative".into(),
            message: format!(
                "{} negative intensity point(s) after subtraction; retained and flagged",
                negs.len()
            ),
            indices: negs,
        });
    }
    diagnostics.push(science::Diagnostic {
        code: "error.model".into(),
        message: "err^2 = err_sample^2 + (scale*err_buffer)^2 + (I_buffer*scale_err)^2; includes normalization and background-scaling uncertainty".into(),
        indices: vec![],
    });
    Ok(EngineOutput {
        kind: "subtract".into(),
        params,
        inputs,
        result: Some(json!({
            "curve": sub,
            "src": src,
            "overlap": [lo, hi],
            "sources": {
                "sample": {"derivation_id": norm.id, "uid": norm.uid, "label": "normalized sample"},
                "buffer": {"id": buffer_curve_id, "uid": buid, "label": blabel},
            },
        })),
        diagnostics,
        status: "complete".into(),
        candidates: vec![],
    })
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RangeSpec {
    pub kind: String, // "guinier" | "porod"
    pub label: String,
    pub q_min: f64,
    pub q_max: f64,
}

pub fn run_intervals(
    conn: &Connection,
    _case_id: i64,
    subtract_derivation_id: i64,
    ranges: &[RangeSpec],
) -> R<EngineOutput> {
    let sub = store::load_derivation(conn, subtract_derivation_id)?;
    let sub_result = sub
        .result
        .clone()
        .ok_or_else(|| "subtract derivation has no result".to_string())?;
    let curve: science::Curve = serde_json::from_value(sub_result["curve"].clone()).s()?;
    let inputs = json!([input_ref(sub.id, &sub.uid, "derivation", "subtract")]);
    let params = json!({"ranges": ranges});
    let mut diagnostics = Vec::new();
    let mut candidates = Vec::new();
    let mut overlaps: Vec<String> = Vec::new();
    for a in ranges.iter() {
        for b in ranges.iter() {
            if a.label < b.label && a.q_min < b.q_max && b.q_min < a.q_max {
                overlaps.push(format!("{} <-> {}", a.label, b.label));
            }
        }
    }
    if !overlaps.is_empty() {
        diagnostics.push(science::Diagnostic {
            code: "intervals.overlap".into(),
            message: format!(
                "overlapping interval candidates retained independently: {}",
                overlaps.join("; ")
            ),
            indices: vec![],
        });
    }
    diagnostics.push(science::Diagnostic {
        code: "intervals.no_auto_merge".into(),
        message: "Guinier and Porod intervals are fitted independently and never auto-merged into a single model; all candidates kept until a reviewer decides".into(),
        indices: vec![],
    });
    for r in ranges {
        let fit = match r.kind.as_str() {
            "guinier" => match science::guinier(&curve, r.q_min, r.q_max) {
                Some(f) => serde_json::to_value(f).s()?,
                None => json!({"failed": "underdetermined or non-decaying Guinier region"}),
            },
            "porod" => match science::porod(&curve, r.q_min, r.q_max) {
                Some(f) => serde_json::to_value(f).s()?,
                None => json!({"failed": "underdetermined Porod region"}),
            },
            other => json!({"failed": format!("unknown interval kind {}", other)}),
        };
        candidates.push(CandidateOut {
            kind: r.kind.clone(),
            label: r.label.clone(),
            q_min: r.q_min,
            q_max: r.q_max,
            fit,
        });
    }
    Ok(EngineOutput {
        kind: "intervals".into(),
        params,
        inputs,
        result: Some(json!({"n_candidates": candidates.len()})),
        diagnostics,
        status: "complete".into(),
        candidates,
    })
}
