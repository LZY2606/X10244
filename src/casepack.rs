//! Offline case pack: "散射曲线台 — SAXS 归一化、背景扣除与多区间模型证据".
//!
//! Replays a full review narrative against the fixtures in `casepack/`:
//! raw imports (q/I/err, exposure, thickness, transmission, buffer candidates,
//! instrument response, mask source all preserved), then one parameter
//! revision, one local-dependency source change and one conflicting human
//! decision. Every boundary case (grid misalignment, no overlap, background
//! scale uncertainty, negative intensity, interval overlap, buffer version
//! switch, multi-candidate residuals) is exercised and recorded as a
//! machine-readable event.

use serde_json::{json, Value};

use crate::domain::*;
use crate::graph;
use crate::store::{Node, Store};

const SAMPLE: &str = include_str!("../casepack/sample_s1.json");
const BUFFER_V1: &str = include_str!("../casepack/buffer_b1_v1.json");
const BUFFER_V2: &str = include_str!("../casepack/buffer_b1_v2.json");
const BUFFER_FAR: &str = include_str!("../casepack/buffer_far.json");
const INSTRUMENT: &str = include_str!("../casepack/instrument_inst1.json");

pub fn fixture_json(name: &str) -> Value {
    let raw = match name {
        "sample.s1" => SAMPLE,
        "buffer.b1.v1" => BUFFER_V1,
        "buffer.b1.v2" => BUFFER_V2,
        "buffer.far" => BUFFER_FAR,
        "instrument.inst1" => INSTRUMENT,
        _ => panic!("unknown fixture {}", name),
    };
    serde_json::from_str(raw).unwrap()
}

fn fixture_curve(name: &str) -> Curve {
    let f = fixture_json(name);
    Curve {
        id: f["dataset_id"].as_str().unwrap().to_string(),
        kind: f["kind"].as_str().unwrap().to_string(),
        q_unit: f["q_unit"].as_str().unwrap().to_string(),
        i_unit: f["i_unit"].as_str().unwrap().to_string(),
        points: f["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| CurvePoint {
                id: p["id"].as_str().unwrap().to_string(),
                q: p["q"].as_f64().unwrap(),
                i: p["i"].as_f64().unwrap(),
                sigma: p["sigma"].as_f64().unwrap(),
                status: PointStatus::Ok,
                sources: vec![],
            })
            .collect(),
        meta: f["meta"].clone(),
    }
}

fn qty(meta: &Value, key: &str) -> Quantity {
    Quantity {
        value: meta[key]["value"].as_f64().unwrap(),
        unit: meta[key]["unit"].as_str().unwrap().to_string(),
        sigma: meta[key]["sigma"].as_f64().unwrap_or(0.0),
    }
}

fn add_node(store: &mut Store, id: &str, kind: &str, name: &str, deps: &[&str]) {
    store
        .add_node(&Node {
            id: id.into(),
            kind: kind.into(),
            name: name.into(),
            deps: deps.iter().map(|d| d.to_string()).collect(),
        })
        .unwrap();
}

fn import_raw(store: &mut Store, fixture: &str, node_id: &str, raw_text: &str) -> String {
    let curve = fixture_curve(fixture);
    let state = json!({
        "kind": "raw",
        "raw_json": raw_text, // original representation preserved verbatim
        "curve": curve,
        "units": { "q": curve.q_unit, "i": curve.i_unit },
        "meta": curve.meta,
    });
    let (vid, _) = store
        .save_version(node_id, &state, None, true, "import", "beamline",
            &format!("imported raw dataset {}", node_id),
            &json!({ "node": node_id, "preserved": "raw q/I/err, exposure, thickness, transmission, buffer candidates, instrument response, mask source" }))
        .unwrap();
    vid
}

fn normalize_sample(store: &mut Store, sample_vid: &str, thickness_mm: f64) -> (String, Curve) {
    let sample_state = store.version(sample_vid).unwrap().state;
    let sample: Curve = serde_json::from_value(sample_state["curve"].clone()).unwrap();
    let meta = &sample_state["meta"];
    let mut thickness = qty(meta, "sample_thickness");
    thickness.value = thickness_mm;
    let (mut norm, diags) = domain_normalize(&sample, &qty(meta, "exposure_time"), &qty(meta, "transmission"), &thickness);
    norm.id = "norm.s1".into();
    let state = json!({
        "kind": "normalized",
        "curve": norm,
        "diagnostics": diags,
        "inputs": { "sample.s1": sample_vid },
        "params": { "exposure": qty(meta, "exposure_time"), "transmission": qty(meta, "transmission"), "thickness": thickness },
    });
    let (vid, _) = store
        .save_version("norm.s1", &state, None, true, "normalize", "beamline",
            &format!("normalized sample (thickness {} mm)", thickness_mm),
            &json!({ "node": "norm.s1", "inputs": { "sample.s1": sample_vid }, "diagnostics": diags }))
        .unwrap();
    (vid, norm)
}

fn domain_normalize(s: &Curve, e: &Quantity, t: &Quantity, th: &Quantity) -> (Curve, Vec<Diagnostic>) {
    normalize(s, e, t, th)
}

fn run_subtract(store: &mut Store, norm_vid: &str, buffer_vid: &str, scale: f64, scale_sigma: f64, chosen: bool, kind: &str, actor: &str) -> Option<String> {
    let norm_state = store.version(norm_vid).unwrap().state;
    let norm: Curve = serde_json::from_value(norm_state["curve"].clone()).unwrap();
    let buf_state = store.version(buffer_vid).unwrap().state;
    let buffer: Curve = serde_json::from_value(buf_state["curve"].clone()).unwrap();
    let scale_q = Quantity { value: scale, unit: "dimensionless".into(), sigma: scale_sigma };
    let out = subtract(&norm, &buffer, &scale_q);
    let diag_codes: Vec<&str> = out.diagnostics.iter().map(|d| d.code.as_str()).collect();
    match out.curve {
        Some(mut curve) => {
            curve.id = "sub.s1".into();
            let state = json!({
                "kind": "subtracted",
                "curve": curve,
                "diagnostics": out.diagnostics,
                "inputs": { "norm.s1": norm_vid, "buffer.b1": buffer_vid },
                "scale": scale_q,
            });
            let (vid, _) = store
                .save_version("sub.s1", &state, None, chosen, kind, actor,
                    &format!("background subtraction (scale {} ± {})", scale, scale_sigma),
                    &json!({ "node": "sub.s1", "inputs": { "norm.s1": norm_vid, "buffer.b1": buffer_vid },
                             "scale": scale_q, "diagnostics": out.diagnostics }))
                .unwrap();
            Some(vid)
        }
        None => {
            store.record_event("derivation_failed", actor,
                "subtraction refused: no q overlap",
                &json!({ "node": "sub.s1", "inputs": { "norm.s1": norm_vid, "buffer": buffer_vid },
                         "codes": diag_codes, "diagnostics": out.diagnostics }))
                .unwrap();
            None
        }
    }
}

fn run_intervals(store: &mut Store, sub_vid: &str, chosen_candidate: Option<&str>, base: Option<&str>, chosen: bool, actor: &str) -> Result<String, crate::store::SaveError> {
    let sub_state = store.version(sub_vid).unwrap().state;
    let sub: Curve = serde_json::from_value(sub_state["curve"].clone()).unwrap();
    let specs = vec![
        IntervalSpec { id: "G1".into(), kind: "guinier".into(), q_min: 0.010, q_max: 0.022 },
        IntervalSpec { id: "G2".into(), kind: "guinier".into(), q_min: 0.012, q_max: 0.030 },
        IntervalSpec { id: "P1".into(), kind: "porod".into(), q_min: 0.020, q_max: 0.060 },
    ];
    let (mut cands, diags) = fit_intervals(&sub, &specs);
    for c in &mut cands {
        c.status = match chosen_candidate {
            Some(id) if id == c.id => CandidateStatus::Chosen,
            Some(_) => CandidateStatus::Unchosen,
            None => CandidateStatus::Pending,
        };
    }
    let state = json!({
        "kind": "intervals",
        "candidates": cands,
        "diagnostics": diags,
        "inputs": { "sub.s1": sub_vid },
    });
    let (vid, _) = store.save_version("intervals.s1", &state, base, chosen, "interval_fit", actor,
        "fitted Guinier/Porod interval candidates",
        &json!({ "node": "intervals.s1", "inputs": { "sub.s1": sub_vid },
                 "chosen": chosen_candidate, "diagnostics": diags }))?;
    Ok(vid)
}

fn recompute_after(store: &mut Store, changed_node: &str, actor: &str) {
    let impact = graph::impact(store, changed_node);
    store.record_event("recompute", actor,
        &format!("dependency propagation from {}", changed_node),
        &json!({ "changed": changed_node, "impact": impact })).unwrap();
}

/// Full replay of the case narrative. Returns the ordered event summaries.
pub fn replay(store: &mut Store) -> Vec<String> {
    // Nodes of the dependency graph.
    add_node(store, "sample.s1", "raw", "样品 s1 原始 I(q)", &[]);
    add_node(store, "buffer.b1", "raw", "缓冲液 b1 候选", &[]);
    add_node(store, "buffer.far", "raw", "缓冲液 far（无重叠区）", &[]);
    add_node(store, "instrument.inst1", "raw", "仪器响应与遮罩来源", &[]);
    add_node(store, "norm.s1", "normalized", "归一化样品", &["sample.s1"]);
    add_node(store, "sub.s1", "subtracted", "扣背景结果", &["norm.s1", "buffer.b1"]);
    add_node(store, "intervals.s1", "intervals", "Guinier/Porod 区间候选", &["sub.s1"]);

    // 1. Imports: raw representation preserved verbatim.
    let sample_v1 = import_raw(store, "sample.s1", "sample.s1", SAMPLE);
    let buf_v1 = import_raw(store, "buffer.b1.v1", "buffer.b1", BUFFER_V1);
    let _far_v1 = import_raw(store, "buffer.far", "buffer.far", BUFFER_FAR);
    let _inst_v1 = import_raw(store, "instrument.inst1", "instrument.inst1", INSTRUMENT);

    // 2. Normalization (thickness 1.0 mm).
    let (norm_v1, _norm) = normalize_sample(store, &sample_v1, 1.0);

    // 3. Baseline subtraction: misaligned grid (buffer in 1/nm), scale 1.0±0.02.
    let sub_v1 = run_subtract(store, &norm_v1, &buf_v1, 1.0, 0.02, true, "subtract", "beamline").unwrap();

    // 4. No-overlap buffer: subtraction refused, diagnostic event only.
    let far_head = store.head("buffer.far").unwrap();
    run_subtract(store, &norm_v1, &far_head, 1.0, 0.02, true, "subtract", "beamline");

    // 5. Interval candidates: G1/G2/P1 with overlap and multi-candidate residuals.
    let int_v1 = run_intervals(store, &sub_v1, None, None, true, "beamline").unwrap();

    // 6. Reviewer decision: choose G1; G2/P1 retained as unchosen candidates.
    let _int_v2 = run_intervals(store, &sub_v1, Some("G1"), Some(&int_v1), true, "reviewer.ying").unwrap();

    // 7. Parameter revision: background scale 1.0±0.02 -> 0.98±0.05.
    let sub_v2 = run_subtract(store, &norm_v1, &buf_v1, 0.98, 0.05, true, "revision", "reviewer.ying").unwrap();
    recompute_after(store, "sub.s1", "reviewer.ying");
    let _int_v3 = run_intervals(store, &sub_v2, Some("G1"), None, true, "reviewer.ying").unwrap();

    // 8. Buffer version switch: b1-v2 imported; subtraction recomputed from the
    //    new buffer; the v1-buffer subtraction is kept as an unchosen branch.
    let buf_v2 = import_raw(store, "buffer.b1.v2", "buffer.b1", BUFFER_V2);
    store.record_event("buffer_version_switch", "beamline",
        "buffer b1 switched v1 -> v2",
        &json!({ "node": "buffer.b1", "from": buf_v1, "to": buf_v2 })).unwrap();
    recompute_after(store, "buffer.b1", "beamline");
    let sub_v3 = run_subtract(store, &norm_v1, &buf_v2, 0.98, 0.05, true, "subtract", "beamline").unwrap();
    // Unchosen candidate: same parameters but old buffer version.
    let _sub_branch = run_subtract(store, &norm_v1, &buf_v1, 0.98, 0.05, false, "subtract", "beamline").unwrap();
    let int_head = store.head("intervals.s1").unwrap();
    let int_v4 = run_intervals(store, &sub_v3, Some("G1"), Some(&int_head), true, "reviewer.ying").unwrap();

    // 9. Conflicting human decision: two users edit from the same base head.
    let a = run_intervals(store, &sub_v3, Some("G2"), Some(&int_v4), true, "user.alice").unwrap();
    let conflict = run_intervals(store, &sub_v3, Some("G1"), Some(&int_v4), true, "user.bob");
    let bob_branch = match conflict {
        Err(crate::store::SaveError::Conflict { head }) => {
            store.record_event("conflict", "user.bob",
                "concurrent decision conflict on intervals.s1",
                &json!({ "node": "intervals.s1", "base": int_v4, "head": head,
                         "resolution": "saved as unchosen branch; both candidates retained" })).unwrap();
            run_intervals(store, &sub_v3, Some("G1"), None, false, "user.bob").unwrap()
        }
        Ok(v) => v,
        Err(e) => panic!("unexpected save error: {:?}", e),
    };
    // Resolution: reviewer keeps alice's version as head; bob's branch retained.
    store.choose_version("intervals.s1", &a, "reviewer.ying",
        &format!("alice's G2 choice confirmed; bob's branch {} retained unchosen", bob_branch)).unwrap();

    // 10. Local source change: sample thickness corrected 1.0 -> 1.1 mm.
    //     Only normalization and its dependents are recomputed.
    let (norm_v2, _) = normalize_sample(store, &sample_v1, 1.1);
    store.record_event("source_data_change", "beamline",
        "sample thickness corrected 1.0 -> 1.1 mm",
        &json!({ "node": "norm.s1", "from": norm_v1, "to": norm_v2,
                 "field": "sample_thickness", "unit": "mm" })).unwrap();
    recompute_after(store, "norm.s1", "beamline");
    let sub_v4 = run_subtract(store, &norm_v2, &buf_v2, 0.98, 0.05, true, "subtract", "beamline").unwrap();
    let int_head2 = store.head("intervals.s1").unwrap();
    let _int_v5 = run_intervals(store, &sub_v4, Some("G2"), Some(&int_head2), true, "reviewer.ying").unwrap();

    store.events().iter().map(|e| format!("#{} [{}] {}", e.id, e.kind, e.summary)).collect()
}
