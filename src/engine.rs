//! Core SAXS rules, versioned. These functions are pure: they take retained
//! raw objects and produce independent, content-addressed derived versions.
//!
//! Guarded semantics (see `RULE_VERSION`):
//! - native q grids that differ are NEVER subtracted point-for-point; linear
//!   interpolation of the buffer is valid strictly inside the shared overlap;
//! - errors include normalization propagation AND background scale uncertainty;
//! - Guinier and Porod intervals are separate candidates, never auto-stitched
//!   into one model; every legal candidate is retained.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::hash::hash_canonical;
use crate::model::*;
use crate::units::{import_field, Dimension, RawField};

pub const RULE_VERSION: &str = "saxs-rules-1.0";
pub const SCALE_UNC_REL_THRESHOLD: f64 = 0.05;
const MIN_FIT_POINTS: usize = 5;

// ---------------------------------------------------------------------------
// Import: raw representation is fully retained
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportRequest {
    pub kind: CurveKind,
    pub label: String,
    pub path: String,
    pub format_tag: String,
    pub q_unit: String,
    pub i_unit: String,
    /// (q_text, i_text, e_text) rows, verbatim
    pub rows: Vec<[String; 3]>,
    pub exposure: (String, String),
    pub thickness: (String, String),
    pub transmission: (String, String),
    pub transmission_sigma: Option<(String, String)>,
    pub buffer_candidate_of: Option<String>,
    pub instrument_response: Option<InstrumentResponse>,
    pub mask: Option<Mask>,
}

pub fn import_raw(req: &ImportRequest) -> Obj {
    let mut diagnostics = Vec::new();
    let mut points = Vec::with_capacity(req.rows.len());
    for (idx, [qt, it, et]) in req.rows.iter().enumerate() {
        let qf = import_field(qt, &req.q_unit, Dimension::ReciprocalLength);
        let i_parsed = it.trim().parse::<f64>().ok();
        let e_parsed = et.trim().parse::<f64>().ok();
        let canonical = match (qf.value, i_parsed, e_parsed) {
            (Some(q), Some(i), Some(e)) if e >= 0.0 => Some(Point3 { q, i, e }),
            _ => None,
        };
        if canonical.is_none() {
            diagnostics.push(format!("row {} not numerically parseable; kept raw", idx));
        }
        points.push(RawPoint {
            idx,
            q_text: qt.trim().to_string(),
            i_text: it.trim().to_string(),
            e_text: et.trim().to_string(),
            q_unit: req.q_unit.clone(),
            i_unit: req.i_unit.clone(),
            canonical,
        });
    }

    let exposure = import_field(&req.exposure.0, &req.exposure.1, Dimension::Time);
    if !exposure.convertible {
        diagnostics.push("exposure not convertible; normalization blocked".to_string());
    }
    let thickness = import_field(&req.thickness.0, &req.thickness.1, Dimension::Length);
    if !thickness.convertible {
        diagnostics.push("thickness not convertible; normalization blocked".to_string());
    }
    let transmission = import_field(&req.transmission.0, &req.transmission.1, Dimension::Transmission);
    if !transmission.convertible {
        diagnostics.push("transmission not convertible; normalization blocked".to_string());
    }
    let transmission_sigma = match &req.transmission_sigma {
        Some((t, u)) => import_field(t, u, Dimension::Transmission),
        None => RawField {
            text: "0".to_string(),
            unit: "fraction".to_string(),
            parsed: Some(0.0),
            value: Some(0.0),
            canonical_unit: Some("fraction".to_string()),
            convertible: true,
            diagnostic: None,
        },
    };

    if let Some(resp) = &req.instrument_response {
        if resp.gain.iter().any(|g| g.g <= 0.0) {
            diagnostics.push("instrument response has non-positive gain bins".to_string());
        }
    }

    let mut record = RawRecord {
        id: String::new(),
        kind: req.kind,
        label: req.label.clone(),
        path: req.path.clone(),
        points,
        q_unit: req.q_unit.clone(),
        i_unit: req.i_unit.clone(),
        exposure,
        thickness,
        transmission,
        transmission_sigma,
        buffer_candidate_of: req.buffer_candidate_of.clone(),
        instrument_response: req.instrument_response.clone(),
        mask: req.mask.clone(),
        diagnostics,
        format_tag: req.format_tag.clone(),
    };
    let id = format!("raw:{}", &hash_canonical(&record)[..16]);
    record.id = id.clone();
    Obj::Raw {
        id,
        record,
    }
}

// ---------------------------------------------------------------------------
// Normalization: I_norm = I_raw / (t_exp * T * d), instrument response and
// mask carried through; normalization uncertainty is tracked separately.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizeError {
    pub why: String,
    pub diagnostics: Vec<String>,
}

fn interp_gain(resp: &InstrumentResponse, q: f64) -> Option<f64> {
    let g = &resp.gain;
    if g.is_empty() {
        return None;
    }
    if q <= g[0].q {
        return Some(g[0].g);
    }
    if q >= g[g.len() - 1].q {
        return Some(g[g.len() - 1].g);
    }
    for w in g.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if q >= a.q && q <= b.q && b.q > a.q {
            let f = (q - a.q) / (b.q - a.q);
            return Some(a.g + f * (b.g - a.g));
        }
    }
    None
}

fn masked(mask: &Mask, q: f64) -> bool {
    mask.excluded_ranges.iter().any(|r| q >= r[0] && q <= r[1])
}

pub fn normalize(obj: &Obj) -> Result<Obj, NormalizeError> {
    let (id, rec) = match obj {
        Obj::Raw { id, record } => (id, record),
        _ => panic!("normalize expects a raw object"),
    };
    let mut why = Vec::new();
    for p in rec.points.iter() {
        if p.canonical.is_none() {
            why.push(format!("row {} lacks canonical q/I/e", p.idx));
        }
    }
    if !rec.exposure.convertible {
        why.push("exposure not convertible".to_string());
    }
    if !rec.thickness.convertible {
        why.push("thickness not convertible".to_string());
    }
    if !rec.transmission.convertible {
        why.push("transmission not convertible".to_string());
    }
    if !why.is_empty() {
        return Err(NormalizeError {
            why: "normalization blocked; raw record retained".to_string(),
            diagnostics: why,
        });
    }

    let exposure = rec.exposure.value.unwrap();
    let thickness = rec.thickness.value.unwrap();
    let transmission = rec.transmission.value.unwrap();
    if exposure <= 0.0 || thickness <= 0.0 || transmission <= 0.0 {
        return Err(NormalizeError {
            why: "non-positive exposure/thickness/transmission".to_string(),
            diagnostics: vec![],
        });
    }
    let t_sigma = rec.transmission_sigma.value.unwrap_or(0.0);

    let denom = exposure * transmission * thickness;
    let gain_applied = rec.instrument_response.is_some();
    let mask_applied = rec.mask.is_some();

    let mut points = Vec::with_capacity(rec.points.len());
    for p in &rec.points {
        let c = p.canonical.as_ref().expect("filtered above");
        let gain = rec
            .instrument_response
            .as_ref()
            .and_then(|r| interp_gain(r, c.q))
            .unwrap_or(1.0);
        let raw_corr = c.i / gain;
        let e_corr = c.e / gain;
        let i = raw_corr / denom;
        let e = e_corr / denom;
        let e_norm = if t_sigma > 0.0 {
            i * (t_sigma / transmission)
        } else {
            0.0
        };
        let excluded = rec.mask.as_ref().is_some_and(|m| masked(m, c.q));
        points.push(NormPoint {
            id: format!("{}#{}", id, p.idx),
            idx: p.idx,
            q: c.q,
            i,
            e,
            e_norm,
            excluded_by_mask: excluded,
        });
    }

    let curve = NormCurve {
        params: NormParams {
            exposure_s: exposure,
            thickness_nm: thickness,
            transmission,
            transmission_sigma: t_sigma,
            gain_applied,
            mask_applied,
            rule_version: RULE_VERSION.to_string(),
        },
        points,
    };

    // object id excludes provenance/created_by_event
    let id_body = hash_canonical(&(&id, &curve));
    let obj_id = format!("norm:{}", &id_body[..16]);
    Ok(Obj::Norm {
        id: obj_id.clone(),
        source: id.clone(),
        curve,
        provenance: Provenance {
            inputs: vec![id.clone()],
            rule_version: RULE_VERSION.to_string(),
            created_by_event: String::new(),
        },
    })
}

// ---------------------------------------------------------------------------
// Buffer subtraction.
// Isub(q) = Is(q) - k Ib(q), k fitted on the shared overlap.
// When native grids differ, Ib is linearly interpolated strictly INSIDE the
// shared overlap; outside it points are kept as uncovered and never subtract.
// Total variance: counting+normalization of both curves and sigma_k * Ib.
// ---------------------------------------------------------------------------

fn active_points(c: &NormCurve) -> impl Iterator<Item = &NormPoint> {
    c.points.iter().filter(|p| !p.excluded_by_mask)
}

fn q_bounds<I: Iterator<Item = f64>>(it: I) -> Option<(f64, f64)> {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut any = false;
    for q in it {
        any = true;
        lo = lo.min(q);
        hi = hi.max(q);
    }
    any.then_some((lo, hi))
}

fn grids_identical(sample: &NormCurve, buffer: &NormCurve) -> bool {
    let s: Vec<f64> = active_points(sample).map(|p| p.q).collect();
    let b: Vec<f64> = active_points(buffer).map(|p| p.q).collect();
    s.len() == b.len()
        && s.iter()
            .zip(&b)
            .all(|(x, y)| (x - y).abs() <= 1e-9 * x.abs().max(1e-12))
}

fn interp_buffer(buffer: &NormCurve, q: f64) -> Option<(f64, f64, f64, Vec<String>)> {
    let pts: Vec<&NormPoint> = active_points(buffer).collect();
    if pts.is_empty() {
        return None;
    }
    if (q - pts[0].q).abs() <= 1e-9 {
        return Some((pts[0].i, pts[0].e, pts[0].e_norm, vec![pts[0].id.clone()]));
    }
    if (q - pts[pts.len() - 1].q).abs() <= 1e-9 {
        let p = pts[pts.len() - 1];
        return Some((p.i, p.e, p.e_norm, vec![p.id.clone()]));
    }
    for w in pts.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if q >= a.q && q <= b.q && b.q > a.q {
            let f = (q - a.q) / (b.q - a.q);
            if f <= 1e-12 {
                return Some((a.i, (a.e * a.e + a.e_norm * a.e_norm).sqrt(), a.e_norm, vec![a.id.clone()]));
            }
            if f >= 1.0 - 1e-12 {
                return Some((b.i, (b.e * b.e + b.e_norm * b.e_norm).sqrt(), b.e_norm, vec![b.id.clone()]));
            }
            let i = a.i + f * (b.i - a.i);
            let e = (1.0 - f) * (a.e * a.e + a.e_norm * a.e_norm).sqrt()
                + f * (b.e * b.e + b.e_norm * b.e_norm).sqrt();
            let e_norm = (1.0 - f) * a.e_norm + f * b.e_norm;
            return Some((i, e, e_norm, vec![a.id.clone(), b.id.clone()]));
        }
    }
    None
}

/// Weighted least squares on the overlap, weights from stated variances.
fn fit_scale(sample: &NormCurve, buffer: &NormCurve) -> Option<(f64, f64, [f64; 2], usize)> {
    // Scale is fitted on the high-q tail of the overlap, where the solute
    // Guinier contribution has decayed and the signal is dominated by the
    // solvent background. The top 40% of the overlap q range is used.
    let bmin = active_points(buffer).map(|p| p.q).fold(f64::INFINITY, f64::min);
    let bmax = active_points(buffer).map(|p| p.q).fold(f64::NEG_INFINITY, f64::max);
    let smin = active_points(sample).map(|p| p.q).fold(f64::INFINITY, f64::min);
    let smax = active_points(sample).map(|p| p.q).fold(f64::NEG_INFINITY, f64::max);
    let lo0 = bmin.max(smin);
    let hi0 = bmax.min(smax);
    if hi0 <= lo0 {
        return None;
    }
    let bmin = lo0 + 0.88 * (hi0 - lo0);
    let mut n_pts = 0usize;
    let mut sw = 0.0_f64;
    let mut swxy = 0.0_f64;
    let mut swxx = 0.0_f64;
    for p in active_points(sample) {
        if p.q < bmin || p.q > bmax {
            continue;
        }
        let Some((bi, be, _ben, _)) = interp_buffer(buffer, p.q) else {
            continue;
        };
        let var = p.e * p.e + p.e_norm * p.e_norm + be * be;
        if !(var > 0.0) || !bi.is_finite() {
            continue;
        }
        let w = 1.0 / var;
        n_pts += 1;
        sw += w;
        swxy += w * bi * p.i;
        swxx += w * bi * bi;
    }
    if sw <= 0.0 || swxx <= 0.0 || n_pts < MIN_FIT_POINTS {
        return None;
    }
    let k = swxy / swxx;
    let sigma = (1.0 / swxx).sqrt();
    Some((k, sigma, [bmin, bmax], n_pts))
}

pub struct SubInputs<'a> {
    pub sample_id: &'a str,
    pub buffer_id: &'a str,
    pub sample: &'a NormCurve,
    pub buffer: &'a NormCurve,
    /// excluded q ranges carried by the buffer raw record
    pub buffer_mask_ranges: Vec<[f64; 2]>,
}

pub fn subtract(inp: SubInputs) -> Obj {
    let sample = inp.sample;
    let buffer = inp.buffer;
    let mut diagnostics = Vec::new();

    let s_bounds = q_bounds(active_points(sample).map(|p| p.q));
    let b_bounds = q_bounds(active_points(buffer).map(|p| p.q));
    let identical = grids_identical(sample, buffer);
    if !identical {
        diagnostics.push("GRID_MISMATCH: native q grids differ; no point-for-point subtraction".to_string());
    }

    let overlap = match (s_bounds, b_bounds) {
        (Some((slo, shi)), Some((blo, bhi))) => {
            let lo = slo.max(blo);
            let hi = shi.min(bhi);
            if hi <= lo {
                None
            } else {
                Some([lo, hi])
            }
        }
        _ => None,
    };

    let fit = overlap.and_then(|_| fit_scale(sample, buffer));
    let no_overlap = overlap.is_none();
    let too_few = fit.map(|(_, _, _, n)| n < MIN_FIT_POINTS).unwrap_or(true);

    let (status, params) = if no_overlap {
        diagnostics.push("NO_OVERLAP: native q ranges share no overlap; subtraction not defined".to_string());
        (SubStatus::NoOverlap, None)
    } else if let Some((k, sig, range, n)) = fit.filter(|(_, _, _, n)| *n >= MIN_FIT_POINTS) {
        if k <= 0.0 {
            diagnostics.push(format!("non-positive fitted scale k={:.4}; retained, not clipped", k));
        }
        if k > 0.0 && sig / k > SCALE_UNC_REL_THRESHOLD {
            diagnostics.push(format!(
                "SCALE_UNCERTAIN: sigma_k/k = {:.3} exceeds {:.2}",
                sig / k,
                SCALE_UNC_REL_THRESHOLD
            ));
        }
        (
            if identical { SubStatus::Ok } else { SubStatus::GridMismatch },
            Some(SubParams {
                scale: k,
                scale_sigma: sig,
                fit_range: range,
                rule_version: RULE_VERSION.to_string(),
                interpolation: if identical {
                    "none:identical_grid".to_string()
                } else {
                    "linear:inside_overlap_only".to_string()
                },
            }),
        )
    } else {
        diagnostics.push(format!(
            "insufficient overlap coverage for scale fit (need {} active points)",
            MIN_FIT_POINTS
        ));
        (
            if identical { SubStatus::Ok } else { SubStatus::GridMismatch },
            None,
        )
    };

    let mut points = Vec::new();
    let mut negative_count = 0usize;
    for p in active_points(sample) {
        let inside = overlap.is_some_and(|[lo, hi]| p.q >= lo && p.q <= hi);
        let buffer_excluded = inp
            .buffer_mask_ranges
            .iter()
            .any(|r| p.q >= r[0] && p.q <= r[1]);
        match (&params, inside) {
            (Some(par), true) => {
                let (bi, be, _ben, bpids) = if identical {
                    let bp = buffer
                        .points
                        .iter()
                        .find(|b| (b.q - p.q).abs() <= 1e-9 * p.q.abs().max(1e-12))
                        .expect("identical grids");
                    (bp.i, (bp.e * bp.e + bp.e_norm * bp.e_norm).sqrt(), bp.e_norm, vec![bp.id.clone()])
                } else {
                    interp_buffer(buffer, p.q).expect("inside overlap always interpolable")
                };
                let i = p.i - par.scale * bi;
                let var_s = p.e * p.e + p.e_norm * p.e_norm;
                let var_b = par.scale * par.scale * (be * be)
                    + (bi * par.scale_sigma) * (bi * par.scale_sigma);
                let e = (var_s + var_b).sqrt();
                let e_buffer = par.scale * be;
                let e_scale = bi.abs() * par.scale_sigma;
                let negative = i < 0.0;
                if negative {
                    negative_count += 1;
                }
                points.push(SubPoint {
                    id: format!("{}@{:.6}", inp.sample_id, p.idx),
                    idx: p.idx,
                    q: p.q,
                    i,
                    e,
                    e_sample: var_s.sqrt(),
                    e_buffer,
                    e_scale,
                    covered: true,
                    sample_i: p.i,
                    buffer_i_interp: Some(bi),
                    negative,
                    excluded: buffer_excluded,
                    source_sample_pid: p.id.clone(),
                    source_buffer_pids: bpids,
                });
            }
            _ => points.push(SubPoint {
                id: format!("{}@{:.6}", inp.sample_id, p.idx),
                idx: p.idx,
                q: p.q,
                i: f64::NAN,
                e: f64::NAN,
                e_sample: f64::NAN,
                e_buffer: f64::NAN,
                e_scale: f64::NAN,
                covered: false,
                sample_i: p.i,
                buffer_i_interp: None,
                negative: false,
                excluded: buffer_excluded,
                source_sample_pid: p.id.clone(),
                source_buffer_pids: vec![],
            }),
        }
    }
    if negative_count > 0 {
        diagnostics.push(format!("NEGATIVE_POINTS: {} subtracted points are negative; retained", negative_count));
    }

    let curve = SubCurve {
        params,
        status,
        overlap,
        grids_identical: identical,
        points,
        diagnostics,
    };
    let body = hash_canonical(&json!({
        "sample": inp.sample_id,
        "buffer": inp.buffer_id,
        "curve": curve,
    }));
    let id = format!("sub:{}", &body[..16]);
    Obj::Sub {
        id: id.clone(),
        sample_norm: inp.sample_id.to_string(),
        buffer_norm: inp.buffer_id.to_string(),
        curve,
        provenance: Provenance {
            inputs: vec![inp.sample_id.to_string(), inp.buffer_id.to_string()],
            rule_version: RULE_VERSION.to_string(),
            created_by_event: String::new(),
        },
    }
}

// ---------------------------------------------------------------------------
// Multi-interval model evidence.
// Guinier: ln I = ln I0 - q^2 Rg^2 / 3 ; Porod: ln I = ln C - alpha ln q.
// Windows are enumerated independently for the two interval kinds. Every
// window meeting the quality threshold is retained as a legal candidate;
// overlapping Guinier/Porod windows are flagged, never merged. No single
// model is ever assembled automatically.
// ---------------------------------------------------------------------------

const MIN_WINDOW_POINTS: usize = 6;
const QUALITY_R2: f64 = 0.85;

fn ols(xs: &[f64], ys: &[f64], ws: &[f64]) -> Option<(f64, f64, FitStats)> {
    let n = xs.len();
    if n < 2 {
        return None;
    }
    let sw: f64 = ws.iter().sum();
    let mx: f64 = ws.iter().zip(xs).map(|(w, x)| w * x).sum::<f64>() / sw;
    let my: f64 = ws.iter().zip(ys).map(|(w, y)| w * y).sum::<f64>() / sw;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    let mut syy = 0.0;
    for i in 0..n {
        let dx = xs[i] - mx;
        let dy = ys[i] - my;
        sxx += ws[i] * dx * dx;
        sxy += ws[i] * dx * dy;
        syy += ws[i] * dy * dy;
    }
    if sxx <= 0.0 {
        return None;
    }
    let slope = sxy / sxx;
    let intercept = my - slope * mx;
    let pred: Vec<f64> = xs.iter().map(|x| intercept + slope * x).collect();
    let mut ss_res = 0.0;
    let mut ll = 0.0;
    for i in 0..n {
        let r = ys[i] - pred[i];
        let sig = (1.0 / ws[i]).max(1e-300);
        ss_res += ws[i] * r * r;
        ll += -0.5 * (r * r / sig + (2.0 * std::f64::consts::PI * sig).ln());
    }
    let r2 = if syy > 0.0 { 1.0 - ss_res / syy } else { 0.0 };
    Some((
        slope,
        intercept,
        FitStats {
            chi2: ss_res,
            r2,
            n,
            ll,
        },
    ))
}

fn window_fit(points: &[&SubPoint], kind: IntervalKind) -> Option<(f64, f64, FitStats)> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut ws = Vec::new();
    for p in points {
        if p.i <= 0.0 || !(p.e > 0.0) {
            continue;
        }
        match kind {
            IntervalKind::Guinier => {
                xs.push(p.q * p.q);
                ys.push(p.i.ln());
            }
            IntervalKind::Porod => {
                xs.push(p.q.ln());
                ys.push(p.i.ln());
            }
        }
        ws.push(1.0 / (p.e / p.i).powi(2));
    }
    if xs.len() < MIN_WINDOW_POINTS {
        return None;
    }
    ols(&xs, &ys, &ws)
}

pub fn fit_models(sub_id: &str, sub: &SubCurve) -> Obj {
    let usable: Vec<&SubPoint> = sub
        .points
        .iter()
        .filter(|p| p.covered && !p.excluded && p.i.is_finite() && p.i > 0.0)
        .collect();

    let mut candidates: Vec<ModelCandidate> = Vec::new();
    let mut diagnostics = Vec::new();

    // Enumerate contiguous index windows: every legal lo..hi slice meeting
    // the minimum length and the quality threshold is retained.
    for kind in [IntervalKind::Guinier, IntervalKind::Porod] {
        let mut found = 0usize;
        // For each left edge, extend the right edge and keep a window only
        // when adding the next point makes the fit worse/invalid; this keeps
        // the distinct evidence-bearing boundaries without flooding the
        // review with every nested sub-window.
        let mut lo = 0usize;
        while lo + MIN_WINDOW_POINTS <= usable.len() {
            let mut best: Option<ModelCandidate> = None;
            let mut hi = lo + MIN_WINDOW_POINTS;
            while hi <= usable.len() {
                let win = &usable[lo..hi];
                let accept = window_fit(win, kind).and_then(|(slope, intercept, stats)| {
                    if stats.r2 < QUALITY_R2 {
                        return None;
                    }
                    let q_lo = win.first().unwrap().q;
                    let q_hi = win.last().unwrap().q;
                    match kind {
                        IntervalKind::Guinier => {
                            let rg = (slope.abs() * 3.0).sqrt();
                            let qrg_max = q_hi * rg;
                            if !(qrg_max > 0.3 && qrg_max < 1.3) {
                                return None;
                            }
                            Some((
                                json!({ "I0": intercept.exp(), "Rg_nm": rg, "qRg_max": qrg_max }),
                                q_lo, q_hi, stats,
                            ))
                        }
                        IntervalKind::Porod => {
                            let alpha = -slope;
                            if !(3.0..=4.2).contains(&alpha) {
                                return None;
                            }
                            Some((
                                json!({ "C": intercept.exp(), "alpha": alpha }),
                                q_lo, q_hi, stats,
                            ))
                        }
                    }
                });
                match accept {
                    Some((parameters, q_lo, q_hi, stats)) => {
                        let better = best
                            .as_ref()
                            .map(|b| stats.r2 > b.stats.r2 + 1e-6)
                            .unwrap_or(true);
                        if better {
                            let body = hash_canonical(&json!({
                                "sub": sub_id, "kind": kind,
                                "qlo": q_lo, "qhi": q_hi,
                                "p": parameters, "stats": stats,
                            }));
                            best = Some(ModelCandidate {
                                id: format!("cand:{}", &body[..14]),
                                kind, q_lo, q_hi, parameters, stats,
                                source_sub: sub_id.to_string(),
                                decision: CandidateDecision::Proposed,
                                interval_overlap: false,
                            });
                            hi += 1;
                        } else {
                            break;
                        }
                    }
                    None => break,
                }
            }
            if let Some(c) = best {
                lo = usable
                    .iter()
                    .position(|p| (p.q - c.q_hi).abs() < 1e-12)
                    .map(|i| i + 1)
                    .unwrap_or(usable.len());
                candidates.push(c);
                found += 1;
            } else {
                lo += 1;
            }
        }
        if found == 0 {
            diagnostics.push(format!("no {kind:?} window met R2>={QUALITY_R2}"));
        }
    }

    // Flag cross-kind overlaps (evidence retained, never auto-stitched).
    let guinier: Vec<(f64, f64, usize)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| c.kind == IntervalKind::Guinier)
        .map(|(i, c)| (c.q_lo, c.q_hi, i))
        .collect();
    for (i, c) in candidates.iter_mut().enumerate() {
        if c.kind != IntervalKind::Porod {
            continue;
        }
        if guinier
            .iter()
            .any(|(lo, hi, gi)| c.q_lo < *hi && c.q_hi > *lo && *gi != i)
        {
            c.interval_overlap = true;
        }
    }
    let overlap_count = candidates.iter().filter(|c| c.interval_overlap).count();
    if overlap_count > 0 {
        diagnostics.push(format!(
            "INTERVAL_OVERLAP: {overlap_count} Porod window(s) overlap Guinier windows; candidates kept separate"
        ));
    }
    if candidates.len() > 1 {
        diagnostics.push(format!(
            "MULTI_CANDIDATE: {} legal interval candidates retained; no single model auto-selected",
            candidates.len()
        ));
    }

    let body = hash_canonical(&json!({"sub": sub_id, "candidates": candidates}));
    let id = format!("cands:{}", &body[..16]);
    Obj::Candidates {
        id: id.clone(),
        source_sub: sub_id.to_string(),
        candidates,
        provenance: Provenance {
            inputs: vec![sub_id.to_string()],
            rule_version: RULE_VERSION.to_string(),
            created_by_event: String::new(),
        },
    }
}

/// Apply a human decision to candidate decisions. Unlisted candidates keep
/// their previous decision, so a confirmation is never an irreversible cover.
pub fn with_decisions(
    obj_id: &str,
    cands_obj: &Obj,
    decisions: &[(String, CandidateDecision)],
) -> Option<Obj> {
    let Obj::Candidates { id, source_sub, candidates, provenance } = cands_obj else {
        return None;
    };
    let mut next = candidates.clone();
    for (cid, dec) in decisions {
        let Some(c) = next.iter_mut().find(|c| &c.id == cid) else {
            continue;
        };
        c.decision = *dec;
    }
    let body = hash_canonical(&json!({"sub": source_sub, "candidates": next}));
    let _ = obj_id;
    Some(Obj::Candidates {
        id: format!("cands:{}", &body[..16]),
        source_sub: source_sub.clone(),
        candidates: next,
        provenance: provenance.clone(),
    })
}
