use crate::domain::*;
use serde_json::json;
use std::collections::BTreeMap;

fn diag(code: &str, severity: &str, message: &str) -> Diagnostic {
    Diagnostic { code: code.into(), severity: severity.into(), message: message.into(), q_min: None, q_max: None }
}

fn curve<'a>(case: &'a CaseMaterials, id: &str) -> Option<&'a CurveRecord> {
    case.curves.iter().find(|c| c.id == id)
}

fn response_at(resp: &InstrumentResponse, q: f64) -> f64 {
    let pts = &resp.points;
    if q <= pts[0].q { return pts[0].response; }
    if q >= pts[pts.len() - 1].q { return pts[pts.len() - 1].response; }
    let pos = pts.partition_point(|p| p.q < q);
    let a = &pts[pos - 1];
    let b = &pts[pos];
    let f = (q - a.q) / (b.q - a.q);
    a.response + f * (b.response - a.response)
}

pub fn normalize(case: &CaseMaterials, curve_id: &str) -> Result<DerivedCurve, String> {
    let raw = curve(case, curve_id).ok_or_else(|| format!("missing curve {curve_id}"))?;
    let resp = case.responses.iter().find(|r| r.id == case.response_id)
        .ok_or("missing instrument response")?;
    let mut points = Vec::new();
    let mut diagnostics = Vec::new();
    for p in &raw.points {
        let response = response_at(resp, p.q);
        let factor = raw.exposure.value * raw.thickness.value * raw.transmission.value * response;
        let mut point_diag = Vec::new();
        if factor <= 0.0 { point_diag.push("non-positive-normalization-factor"); }
        if p.i < 0.0 { point_diag.push("negative-raw-intensity"); }
        let value = if factor > 0.0 { p.i / factor } else { f64::NAN };
        let variance = if factor > 0.0 {
            (p.sigma / factor).powi(2)
                + value.powi(2) * ((raw.exposure.sigma / raw.exposure.value).powi(2)
                    + (raw.thickness.sigma / raw.thickness.value).powi(2)
                    + (raw.transmission.sigma / raw.transmission.value).powi(2))
        } else { f64::NAN };
        if point_diag.iter().any(|x| *x == "negative-raw-intensity") {
            diagnostics.push(diag("NEGATIVE_INTENSITY", "warning", &format!("raw point {} is negative; retained, not clipped", p.index)));
        }
        points.push(Point {
            id: format!("{curve_id}.normalized.p{}", p.index),
            index: p.index,
            q: p.q,
            value,
            sigma: if variance.is_finite() { variance.max(0.0).sqrt() } else { f64::NAN },
            masked: p.masked || factor <= 0.0,
            q_unit: raw.q_unit.clone(),
            value_unit: "1/cm".into(),
            source_point_ids: vec![format!("{curve_id}.raw.p{}", p.index), format!("{}.p{}", resp.id, p.index)],
            interpolation: "instrument-response-linear".into(),
        });
    }
    Ok(DerivedCurve {
        node_id: format!("{curve_id}.normalized"),
        label: format!("{} normalized", raw.label),
        kind: "normalized".into(),
        status: if diagnostics.is_empty() { "ok".into() } else { "warning".into() },
        q_unit: raw.q_unit.clone(),
        value_unit: "1/cm".into(),
        points,
        diagnostics,
        formula: "I_norm = I_raw / (exposure × thickness × transmission × instrument response); sigma includes normalization uncertainties".into(),
    })
}

struct InterpolatedPoint { q: f64, sample: Point, bg_value: f64, bg_sigma: f64, bg_left: usize, bg_right: usize, interpolated: bool }

fn build_subtraction(case: &CaseMaterials, sample_id: &str, buffer_id: &str, label: &str) -> SubtractionCandidate {
    let sample_norm = normalize(case, sample_id);
    let buffer_norm = normalize(case, buffer_id);
    let scale = if case.selected_buffer == buffer_id { case.background_scale.clone() } else {
        BackgroundScale { value: 1.0, sigma: 0.0, source: "unselected-candidate-default".into(), state: "estimated".into() }
    };
    let fallback_sample = DerivedCurve { node_id: String::new(), label: String::new(), kind: String::new(), status: "invalid".into(), q_unit: String::new(), value_unit: String::new(), points: vec![], diagnostics: vec![], formula: String::new() };
    let fallback_bg = fallback_sample.clone();
    let sample = sample_norm.unwrap_or_else(|_| fallback_sample.clone());
    let bg = buffer_norm.unwrap_or_else(|_| fallback_bg.clone());
    let sample_raw = curve(case, sample_id);
    let bg_raw = curve(case, buffer_id);
    let mut diagnostics = Vec::new();
    let mut points = Vec::new();
    let mut residual_squares = 0.0;
    let mut residual_count = 0usize;
    let mut overlap: Option<(f64, f64)> = None;
    if let (Some(s), Some(b)) = (sample_raw, bg_raw) {
        let same = approx_same_grid(&s.points, &b.points);
        overlap = overlap_range(&s.points, &b.points);
        if !same {
            diagnostics.push(Diagnostic { code: "Q_GRID_MISMATCH".into(), severity: "info".into(), message: "q grids differ; direct subtraction is forbidden, linear interpolation is used only inside the overlap".into(), q_min: overlap.map(|x| x.0), q_max: overlap.map(|x| x.1) });
        }
        if overlap.is_none() {
            diagnostics.push(diag("NO_OVERLAP", "error", "no common q range exists; no subtraction points are emitted"));
        }
        for sp in &sample.points {
            let raw_q = s.points[sp.index].q;
            let inside = overlap.map(|(lo, hi)| raw_q >= lo - Q_TOL && raw_q <= hi + Q_TOL).unwrap_or(false);
            if !inside { continue; }
            if let Some((bv, bs, li, ri, interp)) = interpolate_at(&b.points, raw_q) {
                let raw_bg_point_id = if li == ri { format!("{buffer_id}.normalized.p{li}") } else { format!("{buffer_id}.normalized.p{li}..p{ri}") };
                let value = sp.value - scale.value * bv;
                let variance = sp.sigma.powi(2) + (scale.sigma * bv).powi(2)
                    + (scale.value * bs).powi(2);
                let sigma = variance.max(0.0).sqrt();
                if value < 0.0 {
                    diagnostics.push(Diagnostic { code: "NEGATIVE_SUBTRACTED_INTENSITY".into(), severity: "warning".into(), message: format!("subtracted intensity at q={raw_q:.4} is negative and remains visible"), q_min: Some(raw_q), q_max: Some(raw_q) });
                }
                let pid = format!("subtraction.{sample_id}.minus.{buffer_id}.p{}", sp.index);
                points.push(Point {
                    id: pid.clone(),
                    index: sp.index,
                    q: raw_q,
                    value,
                    sigma,
                    masked: sp.masked,
                    q_unit: sp.q_unit.clone(),
                    value_unit: sp.value_unit.clone(),
                    source_point_ids: vec![sp.id.clone(), raw_bg_point_id],
                    interpolation: if same { "shared-grid".into() } else if interp { "linear-overlap".into() } else { "boundary".into() },
                });
                if !sp.masked && value < 0.0 {
                    residual_squares += (value / sigma.max(1e-12)).powi(2);
                    residual_count += 1;
                }
            }
        }
    }
    if scale.sigma > 0.0 {
        diagnostics.push(diag("BACKGROUND_SCALE_UNCERTAINTY", "info", "error band includes relative and absolute buffer scale uncertainty"));
    }
    let chi_square = if residual_count > 0 { residual_squares / residual_count as f64 } else { f64::NAN };
    let rms = if residual_count > 0 { (residual_squares / residual_count as f64).sqrt() } else { f64::NAN };
    let status = if overlap.is_none() { "invalid".into() } else if diagnostics.iter().any(|d| d.severity == "warning") { "warning".into() } else { "ok".into() };
    SubtractionCandidate {
        buffer_id: buffer_id.into(),
        subtraction: DerivedCurve {
            node_id: format!("subtraction.{sample_id}.{buffer_id}"),
            label: label.into(),
            kind: "subtraction".into(),
            status,
            q_unit: sample.q_unit,
            value_unit: sample.value_unit,
            points,
            diagnostics,
            formula: "I_sample(q)-αI_buffer(q); interpolation is restricted to q overlap; sigma² includes sample, background and α".into(),
        },
        scale,
        residual_rms: rms,
        chi_square,
        accepted: false,
        rejection_reason: None,
    }
}

pub fn background(case: &CaseMaterials) -> BackgroundResult {
    let sample_id = "sample".to_string();
    let mut candidates = Vec::new();
    for buffer_id in &case.buffer_candidates {
        candidates.push(build_subtraction(case, &sample_id, buffer_id, &format!("sample minus {buffer_id}")));
    }
    for c in &mut candidates {
        c.accepted = Some(c.buffer_id.as_str()) == case.selected_buffer.as_str().into();
        if !c.accepted { c.rejection_reason = Some("retained candidate; not chosen by current decision".into()); }
    }
    let diagnostics: Vec<_> = candidates.iter().flat_map(|c| c.subtraction.diagnostics.clone()).collect();
    let status = if candidates.iter().all(|c| c.subtraction.status == "invalid") { "invalid".into() } else { "multi-candidate".into() };
    BackgroundResult { node_id: "background-evidence".into(), status, selected_buffer: Some(case.selected_buffer.clone()), candidates, diagnostics }
}

pub fn model_evidence(case: &CaseMaterials, bg: &BackgroundResult) -> ModelEvidence {
    let Some(selected) = bg.candidates.iter().find(|c| c.accepted) else {
        return ModelEvidence { node_id: "model-evidence".into(), status: "invalid".into(), fits: vec![], diagnostics: vec![diag("NO_SELECTED_BACKGROUND", "error", "no selected subtraction branch; no model can be fitted")], note: "Guinier and Porod remain separate evidence and are never joined automatically".into() };
    };
    let cfg = &case.model_windows;
    let mut diagnostics = Vec::new();
    let mut fits = Vec::new();
    if cfg.guinier_q_max > cfg.porod_q_min {
        diagnostics.push(Diagnostic { code: "INTERVAL_OVERLAP".into(), severity: "warning".into(), message: "Guinier and Porod windows overlap; both candidates are retained and no single combined model is emitted".into(), q_min: Some(cfg.guinier_q_min.max(cfg.porod_q_min)), q_max: Some(cfg.guinier_q_max.min(cfg.porod_q_max)) });
    }
    let windows = [("guinier", cfg.guinier_q_min, cfg.guinier_q_max), ("porod", cfg.porod_q_min, cfg.porod_q_max)];
    for (family, qmin, qmax) in windows {
        let rows: Vec<_> = selected.subtraction.points.iter().filter(|p| !p.masked && p.value > 0.0 && p.sigma > 0.0 && p.q >= qmin - Q_TOL && p.q <= qmax + Q_TOL).collect();
        if rows.len() < 3 {
            fits.push(ModelFit { id: format!("fit.{family}"), family: family.into(), status: "insufficient-points".into(), q_min: qmin, q_max: qmax, q_unit: cfg.q_unit.clone(), point_count: rows.len(), intercept: f64::NAN, slope: f64::NAN, derived_parameter: f64::NAN, derived_parameter_unit: if family == "guinier" { "A".into() } else { "dimensionless".into() }, rms: f64::NAN, sse: f64::NAN, residual_point_ids: vec![], diagnostics: vec![diag("INSUFFICIENT_MODEL_POINTS", "warning", "window contains fewer than three valid positive points")] });
            continue;
        }
        if family == "guinier" {
            let xs: Vec<f64> = rows.iter().map(|p| p.q * p.q).collect();
            let ys: Vec<f64> = rows.iter().map(|p| p.value.ln()).collect();
            let ws: Vec<f64> = rows.iter().map(|p| p.sigma / p.value).collect();
            let Some((intercept, slope, rms)) = linear_fit(&xs, &ys, &ws) else { continue; };
            let rg = if slope < 0.0 { (-3.0 * slope).sqrt() } else { f64::NAN };
            if !rg.is_finite() || !rows.iter().all(|p| p.q * rg < 1.3) {
                diagnostics.push(diag("GUINIER_LIMIT_EXCEEDED", "warning", "qRg exceeds the conservative Guinier limit; candidate retained with diagnostic"));
            }
            let sse = rows.iter().map(|p| { let pred = (intercept + slope * p.q * p.q).exp(); ((p.value - pred) / p.sigma).powi(2) }).sum();
            fits.push(ModelFit { id: format!("fit.{family}"), family: family.into(), status: "candidate".into(), q_min: qmin, q_max: qmax, q_unit: cfg.q_unit.clone(), point_count: rows.len(), intercept, slope, derived_parameter: rg, derived_parameter_unit: "A".into(), rms, sse, residual_point_ids: rows.iter().map(|p| p.id.clone()).collect(), diagnostics: vec![] });
        } else {
            let xs: Vec<f64> = rows.iter().map(|p| p.q.ln()).collect();
            let ys: Vec<f64> = rows.iter().map(|p| (p.value * p.q.powi(4)).ln()).collect();
            let ws: Vec<f64> = rows.iter().map(|p| p.sigma / (p.value * p.q.powi(4))).collect();
            let Some((intercept, slope, rms)) = linear_fit(&xs, &ys, &ws) else { continue; };
            let exponent = slope - 4.0;
            let sse = rows.iter().map(|p| { let pred = (intercept + slope * p.q.ln()).exp() / p.q.powi(4); ((p.value - pred) / p.sigma).powi(2) }).sum();
            fits.push(ModelFit { id: format!("fit.{family}"), family: family.into(), status: "candidate".into(), q_min: qmin, q_max: qmax, q_unit: cfg.q_unit.clone(), point_count: rows.len(), intercept, slope, derived_parameter: exponent, derived_parameter_unit: "dimensionless".into(), rms, sse, residual_point_ids: rows.iter().map(|p| p.id.clone()).collect(), diagnostics: vec![] });
        }
    }
    ModelEvidence {
        node_id: "model-evidence".into(),
        status: "multi-candidate".into(),
        fits,
        diagnostics,
        note: "Guinier and Porod are independent interval evidence; overlapping ranges are not automatically stitched".into(),
    }
}

fn node(id: &str, version: u32, kind: &str, label: &str, status: &str, data: serde_json::Value, deps: BTreeMap<String, DependencyRef>, diagnostics: Vec<Diagnostic>, event_id: &str) -> GraphNode {
    let fp_payload = json!({"id": id, "version": version, "kind": kind, "data": data, "deps": deps, "diagnostics": diagnostics});
    GraphNode { id: id.into(), version, kind: kind.into(), label: label.into(), status: status.into(), fingerprint: stable_hash(&fp_payload), data, dependencies: deps, diagnostics, created_by_event: event_id.into() }
}

pub fn rebuild_graph(case: &CaseMaterials, event_id: &str, decisions: &[Decision], conflicts: &[Conflict]) -> BTreeMap<String, GraphNode> {
    let mut nodes = BTreeMap::new();
    for curve in &case.curves {
        let data = json!(curve);
        let fp = stable_hash(&data);
        let mut n = node(&curve.id, 1, "raw-curve", &curve.label, "retained", data, BTreeMap::new(), vec![], event_id);
        n.fingerprint = fp;
        nodes.insert(curve.id.clone(), n);
    }
    for response in &case.responses {
        let data = json!(response);
        let fp = stable_hash(&data);
        let mut n = node(&response.id, 1, "instrument-response", &response.label, "retained", data, BTreeMap::new(), vec![], event_id);
        n.fingerprint = fp;
        nodes.insert(response.id.clone(), n);
    }
    for mask in &case.masks {
        let data = json!(mask);
        let fp = stable_hash(&data);
        let mut n = node(&mask.id, 1, "mask", &mask.label, "retained", data, BTreeMap::new(), vec![], event_id);
        n.fingerprint = fp;
        nodes.insert(mask.id.clone(), n);
    }
    for cid in ["sample", &case.selected_buffer] {
        let derived = normalize(case, cid).expect("normalized curve");
        let deps = BTreeMap::from([(cid.into(), DependencyRef { node_id: cid.into(), version: 1, fingerprint: nodes[cid].fingerprint.clone(), q_min: derived.points.first().map(|p| p.q).unwrap_or(0.0), q_max: derived.points.last().map(|p| p.q).unwrap_or(0.0), role: "raw".into() }), (case.response_id.clone(), DependencyRef { node_id: case.response_id.clone(), version: 1, fingerprint: nodes[&case.response_id].fingerprint.clone(), q_min: 0.0, q_max: f64::INFINITY, role: "response".into() })]);
        nodes.insert(derived.node_id.clone(), node(&derived.node_id, 1, "normalized", &derived.label, &derived.status, json!(derived), deps, vec![], event_id));
    }
    let bg = background(case);
    let mut bg_deps = BTreeMap::new();
    for c in &bg.candidates {
        bg_deps.insert(c.buffer_id.clone(), DependencyRef { node_id: format!("{}.normalized", c.buffer_id), version: 1, fingerprint: nodes.get(&format!("{}.normalized", c.buffer_id)).map(|n| n.fingerprint.clone()).unwrap_or_default(), q_min: c.subtraction.points.first().map(|p| p.q).unwrap_or(0.0), q_max: c.subtraction.points.last().map(|p| p.q).unwrap_or(0.0), role: "normalized-buffer".into() });
    }
    bg_deps.insert("sample.normalized".into(), DependencyRef { node_id: "sample.normalized".into(), version: 1, fingerprint: nodes["sample.normalized"].fingerprint.clone(), q_min: 0.0, q_max: f64::INFINITY, role: "normalized-sample".into() });
    nodes.insert(bg.node_id.clone(), node(&bg.node_id, 1, "background-evidence", "Background candidates", &bg.status, json!(bg), bg_deps, bg.diagnostics.clone(), event_id));
    let models = model_evidence(case, &bg);
    let model_deps = BTreeMap::from([("background-evidence".into(), DependencyRef { node_id: "background-evidence".into(), version: 1, fingerprint: nodes["background-evidence"].fingerprint.clone(), q_min: case.model_windows.guinier_q_min, q_max: case.model_windows.porod_q_max, role: "selected-subtraction".into() })]);
    nodes.insert(models.node_id.clone(), node(&models.node_id, 1, "model-evidence", "Guinier/Porod evidence", &models.status, json!(models), model_deps, models.diagnostics.clone(), event_id));
    for decision in decisions {
        let data = json!(decision);
        let deps = BTreeMap::from([("background-evidence".into(), DependencyRef { node_id: "background-evidence".into(), version: 1, fingerprint: nodes["background-evidence"].fingerprint.clone(), q_min: 0.0, q_max: f64::INFINITY, role: "evidence-at-decision".into() })]);
        nodes.insert(decision.id.clone(), node(&decision.id, 1, "decision", &decision.selected_candidate, &decision.status, data, deps, vec![], &decision.event_id));
    }
    for conflict in conflicts {
        let data = json!(conflict);
        nodes.insert(conflict.id.clone(), node(&conflict.id, 1, "conflict", "Concurrent decision conflict", &conflict.status, data, BTreeMap::new(), vec![], event_id));
    }
    nodes
}
