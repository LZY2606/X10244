//! Pure SAXS rule engine.
//!
//! No SQLite, no HTTP here: these functions take records and return derivations
//! plus machine-readable judgments, so the invariants are unit testable.

use crate::ids::{canon, deriv_id, deriv_point_id, fnv64};
use crate::model::*;
use crate::quantity::Measure;
use crate::RULE_VERSION;

pub fn build_raw_curve(cmd: RegisterCurve, imported_at: String) -> (RawCurve, Vec<Judgment>) {
    let mut judgments = Vec::new();
    let mut diagnostics = Vec::new();
    let role_s = cmd.role.as_str();
    let mut points = Vec::with_capacity(cmd.rows.len());
    for (idx, row) in cmd.rows.iter().enumerate() {
        let q = Measure::imported(&row.q, &cmd.q_unit, "angstrom^-1", "q[1/angstrom]");
        let i = Measure::imported(&row.i, &cmd.i_unit, "counts/s", "I[counts/s]");
        let sigma = Measure::imported(&row.sigma, &cmd.i_unit, "counts/s", "sigma[counts/s]");
        for m in [&q, &i, &sigma].iter().flat_map(|x| x.diagnostics.iter()) {
            diagnostics.push(format!("point {idx}: {m}"));
        }
        let mut mask_reasons = Vec::new();
        for ms in &cmd.mask_sources {
            if ms.point_indexes.contains(&idx) {
                mask_reasons.push(format!("{}: {}", ms.name, ms.reason));
            }
        }
        points.push(RawPoint {
            idx,
            q_raw: row.q.trim().to_string(),
            i_raw: row.i.trim().to_string(),
            sigma_raw: row.sigma.trim().to_string(),
            q,
            i,
            sigma,
            masked: !mask_reasons.is_empty(),
            mask_reasons,
        });
    }
    let exposure = Measure::imported(&cmd.exposure_raw, &cmd.exposure_unit, "s", "exposure[s]");
    let thickness =
        Measure::imported(&cmd.thickness_raw, &cmd.thickness_unit, "mm", "thickness[mm]");
    let transmission =
        Measure::imported(&cmd.transmission_raw, &cmd.transmission_unit, "1", "transmission[1]");
    for m in [&exposure, &thickness, &transmission]
        .iter()
        .flat_map(|x| x.diagnostics.iter())
    {
        diagnostics.push(m.clone());
    }
    if !exposure.convertible || exposure.value <= 0.0 || exposure.value.is_nan() {
        judgments.push(Judgment {
            code: "unconvertible_exposure".into(),
            status: "error".into(),
            message: "normalization impossible: exposure missing/non-positive/unconvertible; raw values retained".into(),
            affected: vec![],
            evidence: serde_json::json!({"raw": exposure.raw, "unit": exposure.unit}),
        });
    }
    if !transmission.convertible || !(0.0..=1.0).contains(&transmission.value) {
        judgments.push(Judgment {
            code: "transmission_out_of_range".into(),
            status: "warning".into(),
            message: "transmission outside [0,1] or unconvertible; kept with diagnostic".into(),
            affected: vec![],
            evidence: serde_json::json!({"raw": transmission.raw, "unit": transmission.unit}),
        });
    }
    if !diagnostics.is_empty() {
        judgments.push(Judgment {
            code: "raw_preserved_with_diagnostics".into(),
            status: "warning".into(),
            message: "some imported values could not be normalized; originals retained in place".into(),
            affected: vec![],
            evidence: serde_json::json!({"diagnostics": diagnostics}),
        });
    }
    let first = points.first().map(|p| p.q_raw.clone()).unwrap_or_default();
    let last = points.last().map(|p| p.q_raw.clone()).unwrap_or_default();
    let id = crate::ids::curve_id(&cmd.payload_key, points.len(), &first, &last, role_s);
    (
        RawCurve {
            id,
            name: cmd.name,
            role: cmd.role,
            payload_key: cmd.payload_key,
            q_unit: cmd.q_unit,
            i_unit: cmd.i_unit,
            points,
            exposure,
            thickness,
            transmission,
            mask_sources: cmd.mask_sources,
            response_id: cmd.response_id,
            supersedes: cmd.supersedes,
            imported_at,
            diagnostics,
        },
        judgments,
    )
}

fn gain_at(response: Option<&InstrumentResponse>, idx: usize, n: usize) -> f64 {
    match response.and_then(|r| r.gain.get(idx)) {
        Some(g) if *g > 0.0 => *g,
        Some(_) => 1.0,
        None => {
            let _ = n;
            1.0
        }
    }
}

fn config_signature(parts: &[String]) -> String {
    fnv64(&parts.iter().map(|s| s.as_str()).collect::<Vec<_>>())
}

/// Normalize a raw curve: I/[t * T * d] with instrument response correction.
/// Masked/unconvertible points are retained but excluded with explicit reasons;
/// sigma includes the configured normalization relative uncertainty.
pub fn normalize(
    curve: &RawCurve,
    response: Option<&InstrumentResponse>,
    params: &Params,
    event_seq: i64,
) -> Derivation {
    let mut judgments = Vec::new();
    let mut points = Vec::new();
    let t = curve.exposure.value;
    let tr = curve.transmission.value;
    let d = thickness_factor(curve);
    let feasible = curve.exposure.convertible && t > 0.0 && !t.is_nan()
        && curve.transmission.convertible
        && tr > 0.0
        && d.is_finite()
        && d > 0.0;
    if !feasible {
        judgments.push(Judgment {
            code: "normalization_infeasible".into(),
            status: "error".into(),
            message: "normalization denominator not available; points retained unnormalized".into(),
            affected: vec![curve.id.clone()],
            evidence: serde_json::json!({
                "exposure": curve.exposure.value,
                "transmission": curve.transmission.value,
                "thickness_mm": d,
            }),
        });
    }
    let n = curve.points.len();
    let sig = config_signature(&[
        curve.id.clone(),
        format!("{t}"),
        format!("{tr}"),
        format!("{d}"),
        curve.response_id.clone().unwrap_or_else(|| "flat".into()),
        params.version.clone(),
        format!("{}", params.u_norm_rel),
    ]);
    let id = deriv_id("normalized", &sig);
    for p in &curve.points {
        let rid = raw_point_id(curve, p.idx, &p.q_raw, &p.i_raw);
        let gain = gain_at(response, p.idx, n);
        let mut excluded = false;
        let mut reasons = Vec::new();
        if p.masked {
            excluded = true;
            reasons.push("masked: ".to_string() + &p.mask_reasons.join("; "));
        }
        if !p.i.convertible || !p.q.convertible {
            excluded = true;
            reasons.push("unconvertible imported representation".into());
        }
        let q = p.q.value;
        let denom = if feasible { t * tr * d * gain } else { 1.0 };
        let value = if feasible { p.i.value / denom } else { p.i.value };
        let raw_rel = if p.sigma.value > 0.0 && p.i.value != 0.0 {
            Some(p.sigma.value / p.i.value.abs())
        } else {
            None
        };
        if raw_rel.is_none() {
            reasons.push("counting error unavailable; normalization uncertainty only".into());
        }
        let rel2 = raw_rel.map(|r| r * r).unwrap_or(0.0) + params.u_norm_rel * params.u_norm_rel;
        let sigma = value.abs() * rel2.sqrt();
        points.push(DerivedPoint {
            idx: p.idx,
            id: deriv_point_id(&id, p.idx, q),
            q,
            value,
            sigma,
            source_point_ids: vec![rid],
            excluded,
            reasons,
        });
    }
    let status = if judgments.iter().any(|j| j.status == "error") {
        "error"
    } else if judgments.iter().any(|j| j.status == "warning") || points.iter().any(|p| p.excluded)
    {
        "warning"
    } else {
        "ok"
    };
    Derivation {
        id,
        kind: DerivationKind::Normalized.as_str().into(),
        status: status.into(),
        rule_version: RULE_VERSION.into(),
        config_sig: sig,
        inputs: vec![curve.id.clone()],
        points,
        judgments,
        candidates: vec![],
        created_event_seq: event_seq,
    }
}

fn thickness_factor(curve: &RawCurve) -> f64 {
    if curve.thickness.convertible && curve.thickness.value > 0.0 {
        curve.thickness.value
    } else {
        f64::NAN
    }
}

fn raw_point_id(curve: &RawCurve, idx: usize, q_raw: &str, i_raw: &str) -> String {
    crate::ids::raw_point_id(&curve.id, idx, q_raw, i_raw)
}

fn usable_points(d: &Derivation) -> Vec<&DerivedPoint> {
    d.points.iter().filter(|p| !p.excluded && p.value.is_finite()).collect()
}

fn same_grid(a: &[f64], b: &[f64], tol: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol * x.abs().max(1e-12))
}

fn interp(xs: &[f64], ys: &[f64], x: f64) -> Option<(usize, usize, f64)> {
    if xs.len() < 2 || x < xs[0] || x > xs[xs.len() - 1] {
        return None;
    }
    let mut hi = match xs.binary_search_by(|v| v.partial_cmp(&x).unwrap()) {
        Ok(i) => {
            return Some((i, i, 0.0));
        }
        Err(i) => i,
    };
    if hi == 0 {
        hi = 1;
    }
    let lo = hi - 1;
    let f = (x - xs[lo]) / (xs[lo + 1] - xs[lo]);
    Some((lo, hi, f))
}

/// Buffer subtraction.
///
/// Different q grids are never subtracted directly: the buffer is linearly
/// interpolated, and only inside the overlapping q range. The variance includes
/// counting/normalization terms plus the buffer scale uncertainty. Negative
/// results are retained and flagged, never clipped.
pub fn subtract(
    sample: &Derivation,
    buffer: &Derivation,
    params: &Params,
    event_seq: i64,
    scenario_tag: &str,
) -> Derivation {
    let mut judgments = Vec::new();
    let sp = usable_points(sample);
    let bp = usable_points(buffer);
    let sqs: Vec<f64> = sp.iter().map(|p| p.q).collect();
    let bqs: Vec<f64> = bp.iter().map(|p| p.q).collect();
    let tol = 1e-9;
    let grids_equal = same_grid(&sqs, &bqs, tol);
    let overlap_lo = sqs.first().copied().unwrap_or(0.0).max(bqs.first().copied().unwrap_or(0.0));
    let overlap_hi = sqs.last().copied().unwrap_or(0.0).min(bqs.last().copied().unwrap_or(0.0));
    let mut overlap_count = 0usize;
    for q in &sqs {
        if *q >= overlap_lo - 1e-12 && *q <= overlap_hi + 1e-12 {
            overlap_count += 1;
        }
    }
    if !grids_equal {
        judgments.push(Judgment {
            code: "q_grid_mismatch".into(),
            status: "warning".into(),
            message: "q grids differ; buffer is interpolated and subtraction is valid only inside the overlapping range".into(),
            affected: vec![sample.id.clone(), buffer.id.clone()],
            evidence: serde_json::json!({
                "sample_n": sqs.len(), "buffer_n": bqs.len(),
                "overlap_qmin": overlap_lo, "overlap_qmax": overlap_hi,
            }),
        });
    }
    let no_overlap = overlap_count < params.min_overlap_points || overlap_hi <= overlap_lo;
    if no_overlap {
        judgments.push(Judgment {
            code: "no_overlap".into(),
            status: "error".into(),
            message: "fewer than the required overlapping q points; no direct subtraction is produced".into(),
            affected: vec![sample.id.clone(), buffer.id.clone()],
            evidence: serde_json::json!({
                "overlap_points": overlap_count,
                "required": params.min_overlap_points,
                "overlap_qmin": overlap_lo, "overlap_qmax": overlap_hi,
            }),
        });
    }

    let bvals: Vec<f64> = bp.iter().map(|p| p.value).collect();
    let bsigs: Vec<f64> = bp.iter().map(|p| p.sigma).collect();

    // Buffer scale: k=1 unless estimation is requested (weighted LS in a q region).
    let (scale_k, scale_estimated) = if params.estimate_buffer_scale && !no_overlap {
        let mut sw = 0.0;
        let mut swxy = 0.0;
        let mut swyy = 0.0;
        for p in &sp {
            if p.q < params.scale_region_qmin || p.q > params.scale_region_qmax {
                continue;
            }
            if let Some((lo, hi, f)) = interp(&bqs, &bvals, p.q) {
                let bv = bvals[lo] * (1.0 - f) + bvals[hi] * f;
                let bs = bsigs[lo] * (1.0 - f) + bsigs[hi] * f;
                let w = 1.0 / (p.sigma * p.sigma + bs * bs + 1e-30);
                sw += w;
                swxy += w * bv * p.value;
                swyy += w * bv * bv;
            }
        }
        if sw > 0.0 && swyy > 0.0 {
            (swxy / swyy, true)
        } else {
            (1.0, false)
        }
    } else {
        (1.0, false)
    };

    let u_scale = if params.estimate_buffer_scale && scale_estimated {
        params.u_scale_rel
    } else {
        // k fixed at 1: reviewers may still carry a declared scale uncertainty.
        params.u_scale_rel
    };
    if u_scale > 0.0 && !no_overlap {
        judgments.push(Judgment {
            code: "scale_uncertainty".into(),
            status: "warning".into(),
            message: "buffer scale uncertainty propagated into every subtracted error band".into(),
            affected: vec![buffer.id.clone()],
            evidence: serde_json::json!({"k": scale_k, "u_scale_rel": u_scale, "estimated": scale_estimated}),
        });
    }

    let sig = config_signature(&[
        sample.id.clone(),
        buffer.id.clone(),
        params.version.clone(),
        format!("{u_scale}"),
        format!("{scale_k}"),
        scenario_tag,
    ]);
    let id = deriv_id("subtracted", &sig);
    let mut points = Vec::new();
    let mut negative = 0usize;
    for p in &sp {
        let inter = interp(&bqs, &bvals, p.q);
        let inside = p.q >= overlap_lo - 1e-12 && p.q <= overlap_hi + 1e-12;
        let mut excluded = false;
        let mut reasons = Vec::new();
        let (value, sigma, sources) = if no_overlap || inter.is_none() || !inside {
            excluded = true;
            reasons.push(if no_overlap {
                "no overlapping region; no subtraction".into()
            } else {
                "sample point outside buffer overlap; interpolation not applied".into()
            });
            judgments.retain(|j| j.code != "point_outside_overlap");
            (p.value, p.sigma, vec![p.id.clone()])
        } else {
            let (lo, hi, f) = inter.unwrap();
            let bv = bvals[lo] * (1.0 - f) + bvals[hi] * f;
            let bs = bsigs[lo] * (1.0 - f) + bsigs[hi] * f;
            let val = p.value - scale_k * bv;
            // var = sigma_s^2 + k^2 sigma_b^2 + (u_scale * k * bv)^2
            let var = p.sigma * p.sigma
                + scale_k * scale_k * bs * bs
                + (u_scale * scale_k * bv).powi(2);
            let mut srcs = vec![p.id.clone()];
            if f == 0.0 {
                srcs.push(bp[lo].id.clone());
            } else {
                srcs.push(bp[lo].id.clone());
                srcs.push(bp[hi].id.clone());
            }
            (val, var.sqrt(), srcs)
        };
        if !excluded && value < 0.0 {
            negative += 1;
            reasons.push("negative subtracted intensity retained (not clipped)".into());
        }
        points.push(DerivedPoint {
            idx: p.idx,
            id: deriv_point_id(&id, p.idx, p.q),
            q: p.q,
            value,
            sigma,
            source_point_ids: sources,
            excluded,
            reasons,
        });
    }
    if negative > 0 {
        judgments.push(Judgment {
            code: "negative_intensity".into(),
            status: "warning".into(),
            message: "subtracted points are negative; kept for review rather than clipped to zero".into(),
            affected: vec![id.clone()],
            evidence: serde_json::json!({"negative_points": negative, "floor_rel": params.negative_flag_rel}),
        });
    }
    let status = if no_overlap {
        "error"
    } else if !judgments.is_empty() || points.iter().any(|p| p.excluded) {
        "warning"
    } else {
        "ok"
    };
    Derivation {
        id,
        kind: DerivationKind::Subtracted.as_str().into(),
        status: status.into(),
        rule_version: RULE_VERSION.into(),
        config_sig: sig,
        inputs: vec![sample.id.clone(), buffer.id.clone()],
        points,
        judgments,
        candidates: vec![],
        created_event_seq: event_seq,
    }
}

fn linfit_w(xs: &[f64], ys: &[f64], ws: &[f64]) -> Option<(f64, f64, f64)> {
    let n = xs.len() as f64;
    let sw: f64 = ws.iter().sum();
    if sw <= 0.0 {
        return None;
    }
    let sx: f64 = xs.iter().zip(ws).map(|(x, w)| x * w).sum();
    let sy: f64 = ys.iter().zip(ws).map(|(y, w)| y * w).sum();
    let sxx: f64 = xs.iter().zip(ws).map(|(x, w)| x * x * w).sum();
    let sxy: f64 = xs.iter().zip(ys).zip(ws).map(|((x, y), w)| x * y * w).sum();
    let den = sw * sxx - sx * sx;
    if den.abs() < 1e-30 {
        return None;
    }
    let slope = (sw * sxy - sx * sy) / den;
    let intercept = (sy - slope * sx) / sw;
    // R^2 weighted on y
    let ybar = sy / sw;
    let mut ss_tot = 0.0;
    let mut ss_res = 0.0;
    let mut chi2 = 0.0;
    for i in 0..xs.len() {
        let pred = intercept + slope * xs[i];
        ss_tot += ws[i] * (ys[i] - ybar).powi(2);
        ss_res += ws[i] * (ys[i] - pred).powi(2);
        if ws[i] > 0.0 {
            chi2 += (ys[i] - pred).powi(2) * ws[i];
        }
    }
    let r2 = if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { f64::NAN };
    let _ = n;
    Some((slope, intercept, chi2.max(0.0).sqrt()))
}

#[derive(Clone)]
struct Fit {
    q_min: f64,
    q_max: f64,
    r2: f64,
    chi2: f64,
    rms: f64,
    point_ids: Vec<String>,
    residuals: Vec<f64>,
    params: serde_json::Value,
}

/// Produce Guinier and Porod interval candidates on a subtracted curve.
///
/// Multiple windows survive; overlapping windows and Guinier/Porod overlap are
/// recorded as judgments. The two model families are never joined here.
pub fn interval_candidates(
    sub: &Derivation,
    params: &Params,
    event_seq: i64,
) -> Derivation {
    let mut judgments = Vec::new();
    let pts: Vec<&DerivedPoint> =
        sub.points.iter().filter(|p| !p.excluded && p.value > 0.0).collect();
    let mut candidates: Vec<IntervalCandidate> = Vec::new();
    let mut fits_guinier: Vec<Fit> = Vec::new();
    let m = pts.len();

    // Guinier: ln I vs q^2, nested windows starting at the lowest usable q.
    let start = 0usize;
    for end in (params.guinier_min_points..=m).rev() {
        let window = &pts[start..end.min(m)];
        if window.len() < params.guinier_min_points {
            break;
        }
        let xs: Vec<f64> = window.iter().map(|p| p.q * p.q).collect();
        let ys: Vec<f64> = window.iter().map(|p| p.value.ln()).collect();
        let ws: Vec<f64> = window
            .iter()
            .map(|p| (p.value / p.sigma.max(1e-12)).powi(2))
            .collect();
        if let Some((slope, intercept, chi2)) = linfit_w(&xs, &ys, &ws) {
            if slope >= 0.0 {
                continue;
            }
            let rg = (-3.0 * slope).sqrt();
            let qmax = window.last().unwrap().q;
            let qrg = qmax * rg;
            let residuals: Vec<f64> = window
                .iter()
                .zip(&ys)
                .zip(&xs)
                .map(|((p, y), x)| y - (intercept + slope * x))
                .collect();
            let rms = (residuals.iter().map(|r| r * r).sum::<f64>() / residuals.len() as f64).sqrt();
            let ybar = ys.iter().sum::<f64>() / ys.len() as f64;
            let ss_tot: f64 = ys.iter().map(|y| (y - ybar).powi(2)).sum();
            let ss_res: f64 = residuals.iter().map(|r| r * r).sum();
            let r2 = if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { 0.0 };
            fits_guinier.push(Fit {
                q_min: window.first().unwrap().q,
                q_max: qmax,
                r2,
                chi2,
                rms,
                point_ids: window.iter().map(|p| p.id.clone()).collect(),
                residuals,
                params: serde_json::json!({
                    "slope": slope, "intercept_lnI0": intercept,
                    "Rg": rg, "qmax_Rg": qrg, "within_qRg_cap": qrg <= params.guinier_qrg_max,
                }),
            });
        }
    }
    // Keep all legal nested windows, mark those exceeding the q*Rg cap rather
    // than deleting them; retain up to the 8 longest distinct ends.
    fits_guinier.sort_by(|a, b| b.q_max.partial_cmp(&a.q_max).unwrap());
    fits_guinier.truncate(8);
    let mut cap_violations = 0usize;
    for f in &fits_guinier {
        let qrg = f.params["qmax_Rg"].as_f64().unwrap_or(0.0);
        if qrg > params.guinier_qrg_max {
            cap_violations += 1;
        }
        candidates.push(IntervalCandidate {
            id: format!("cand_guinier_{}", fnv64(&["guinier", &sub.id, &canon(f.q_min), &canon(f.q_max)])),
            model: "guinier".into(),
            q_min: f.q_min,
            q_max: f.q_max,
            qrg_max: Some(qrg),
            r_squared: f.r2,
            chi2: f.chi2,
            residual_rms: f.rms,
            point_ids: f.point_ids.clone(),
            residuals: f.residuals.clone(),
            params: f.params.clone(),
            adopted: false,
            rejected: false,
            reject_reason: None,
        });
    }
    if cap_violations > 0 {
        judgments.push(Judgment {
            code: "guinier_qrg_cap".into(),
            status: "warning".into(),
            message: "windows extend beyond q*Rg cap; retained with evidence, not silently removed".into(),
            affected: candidates.iter().filter(|c| c.qrg_max.unwrap_or(0.0) > params.guinier_qrg_max).map(|c| c.id.clone()).collect(),
            evidence: serde_json::json!({"cap": params.guinier_qrg_max, "violations": cap_violations}),
        });
    }

    // Porod: high-q tail, ln I vs ln q => I = C q^{-n}.
    let start_idx = pts.partition_point(|p| p.q < params.porod_qmin);
    let mut fits_porod: Vec<Fit> = Vec::new();
    for start in start_idx..m.saturating_sub(params.porod_min_points - 1) {
        let window = &pts[start..m];
        if window.len() < params.porod_min_points {
            break;
        }
        let xs: Vec<f64> = window.iter().map(|p| p.q.ln()).collect();
        let ys: Vec<f64> = window.iter().map(|p| p.value.ln()).collect();
        let ws: Vec<f64> = window.iter().map(|p| (p.value / p.sigma.max(1e-12)).powi(2)).collect();
        if let Some((slope, intercept, chi2)) = linfit_w(&xs, &ys, &ws) {
            let residuals: Vec<f64> = window
                .iter()
                .zip(&ys)
                .zip(&xs)
                .map(|((p, y), x)| y - (intercept + slope * x))
                .collect();
            let rms = (residuals.iter().map(|r| r * r).sum::<f64>() / residuals.len() as f64).sqrt();
            let ybar = ys.iter().sum::<f64>() / ys.len() as f64;
            let ss_tot: f64 = ys.iter().map(|y| (y - ybar).powi(2)).sum();
            let ss_res: f64 = residuals.iter().map(|r| r * r).sum();
            let r2 = if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { 0.0 };
            fits_porod.push(Fit {
                q_min: window.first().unwrap().q,
                q_max: window.last().unwrap().q,
                r2,
                chi2,
                rms,
                point_ids: window.iter().map(|p| p.id.clone()).collect(),
                residuals,
                params: serde_json::json!({"exponent_n": -slope, "ln_C": intercept}),
            });
        }
    }
    // Retain distinct starts (coarse-grained) up to 8.
    fits_porod.sort_by(|a, b| a.q_min.partial_cmp(&b.q_min).unwrap());
    fits_porod.truncate(8);
    for f in &fits_porod {
        candidates.push(IntervalCandidate {
            id: format!("cand_porod_{}", fnv64(&["porod", &sub.id, &canon(f.q_min), &canon(f.q_max)])),
            model: "porod".into(),
            q_min: f.q_min,
            q_max: f.q_max,
            qrg_max: None,
            r_squared: f.r2,
            chi2: f.chi2,
            residual_rms: f.rms,
            point_ids: f.point_ids.clone(),
            residuals: f.residuals.clone(),
            params: f.params.clone(),
            adopted: false,
            rejected: false,
            reject_reason: None,
        });
    }

    // Multi-candidate residual evidence: rank without collapsing to one.
    if candidates.len() > 1 {
        let mut ranking: Vec<serde_json::Value> = candidates
            .iter()
            .map(|c| serde_json::json!({"id": c.id, "model": c.model, "r2": c.r_squared, "residual_rms": c.residual_rms}))
            .collect();
        ranking.sort_by(|a, b| {
            b["residual_rms"].as_f64().unwrap().partial_cmp(&a["residual_rms"].as_f64().unwrap()).unwrap()
        });
        judgments.push(Judgment {
            code: "multi_candidate_residuals".into(),
            status: "info".into(),
            message: "multiple legal intervals survive; residuals are listed for review, no single winner is forced".into(),
            affected: candidates.iter().map(|c| c.id.clone()).collect(),
            evidence: serde_json::json!({"candidates": ranking}),
        });
    }

    // Interval overlap: within-family and Guinier/Porod.
    let mut overlaps: Vec<(String, String)> = Vec::new();
    for i in 0..candidates.len() {
        for j in (i + 1)..candidates.len() {
            let a = &candidates[i];
            let b = &candidates[j];
            if a.q_min < b.q_max && b.q_min < a.q_max {
                overlaps.push((a.id.clone(), b.id.clone()));
            }
        }
    }
    if !overlaps.is_empty() {
        judgments.push(Judgment {
            code: "interval_overlap".into(),
            status: "warning".into(),
            message: "candidate intervals overlap; both retained and shown together".into(),
            affected: overlaps.iter().flat_map(|(a, b)| vec![a.clone(), b.clone()]).collect(),
            evidence: serde_json::json!({"pairs": overlaps}),
        });
    }
    let cross = candidates.iter().any(|g| g.model == "guinier")
        && candidates.iter().any(|p| p.model == "porod")
        && candidates
            .iter()
            .filter(|g| g.model == "guinier")
            .any(|g| candidates.iter().filter(|p| p.model == "porod").any(|p| g.q_min < p.q_max && p.q_min < g.q_max));
    judgments.push(Judgment {
        code: "no_automatic_join".into(),
        status: if cross { "warning" } else { "info" }.into(),
        message: "Guinier and Porod intervals are never joined automatically into one piecewise model".into(),
        affected: candidates.iter().map(|c| c.id.clone()).collect(),
        evidence: serde_json::json!({"cross_family_overlap": cross, "joined_model_emitted": false}),
    });

    let sig = config_signature(&[
        sub.id.clone(),
        params.version.clone(),
        format!("{}", params.guinier_qrg_max),
        format!("{}", params.porod_qmin),
        format!("{}", params.guinier_min_points),
        format!("{}", params.porod_min_points),
    ]);
    let id = deriv_id("interval_candidates", &sig);
    Derivation {
        id,
        kind: DerivationKind::IntervalCandidates.as_str().into(),
        status: if judgments.iter().any(|j| j.status == "error") {
            "error"
        } else if judgments.iter().any(|j| j.status == "warning") {
            "warning"
        } else {
            "ok"
        },
        rule_version: RULE_VERSION.into(),
        config_sig: sig,
        inputs: vec![sub.id.clone()],
        points: sub.points.clone(),
        judgments,
        candidates,
        created_event_seq: event_seq,
    }
}
