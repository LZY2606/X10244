//! Core scattering rules. These are pure functions so they can be tested
//! against repo fixtures and replayed deterministically by the case pack.
//!
//! Semantics enforced here (also emitted as machine-readable judgments):
//!  * q grids differ -> never subtract directly; interpolate only in overlap.
//!  * errors include normalization and background-scale uncertainty.
//!  * Guinier and Porod intervals are never auto-merged into one model.
//!  * multi-solution / uncertain states are kept, not swallowed.

use crate::model::{content_id, Curve, CurvePoint, Diagnostic, GridJudgment, IntervalCandidate, Quantity};

pub const RULE_VERSION: u32 = 2;

/// I_norm = I / (T * t * d). Error includes the normalization uncertainty
/// from transmission (relative sigma of T propagated to every point).
pub fn normalize(
    raw: &Curve,
    exposure: &Quantity,     // seconds
    thickness: &Quantity,    // mm
    transmission: &Quantity, // dimensionless, may carry sigma
) -> (Curve, Vec<Diagnostic>) {
    let mut diags = vec![];
    let mut factor = exposure.value * thickness.value * transmission.value;
    if factor <= 0.0 {
        diags.push(Diagnostic {
            code: "normalization_nonpositive_factor".into(),
            message: format!("normalization factor {factor} <= 0; curve kept unnormalized"),
            affected: vec![raw.id.clone()],
        });
        factor = 1.0;
    }
    let rel_t = transmission
        .sigma
        .filter(|s| transmission.value.abs() > 0.0)
        .map(|s| (s / transmission.value).abs())
        .unwrap_or(0.0);
    let points = raw
        .points
        .iter()
        .map(|p| {
            let i = p.i / factor;
            // sigma^2 = (sigma_I/f)^2 + (I_norm * rel_T)^2
            let sigma = ((p.sigma / factor).powi(2) + (i * rel_t).powi(2)).sqrt();
            CurvePoint { q: p.q, i, sigma, negative: p.i < 0.0 }
        })
        .collect::<Vec<_>>();
    let id = content_id("nrm", &(&raw.id, factor, RULE_VERSION));
    let curve = Curve {
        id: id.clone(),
        kind: "derived:normalized".into(),
        q_unit: raw.q_unit.clone(),
        i_unit: format!("{}/(s*mm)", raw.i_unit),
        points,
        raw_text: String::new(),
        meta: serde_json::json!({
            "source": raw.id,
            "exposure_s": exposure.value,
            "thickness_mm": thickness.value,
            "transmission": transmission.value,
            "transmission_sigma": transmission.sigma,
            "rule_version": RULE_VERSION,
        }),
    };
    diags.push(Diagnostic {
        code: "normalization_uncertainty_included".into(),
        message: format!("relative transmission uncertainty {:.4} folded into sigma", rel_t),
        affected: vec![id],
    });
    (curve, diags)
}

fn grids_identical(a: &Curve, b: &Curve) -> bool {
    a.points.len() == b.points.len()
        && a.points.iter().zip(b.points.iter()).all(|(x, y)| x.q == y.q)
}

/// Linear interpolation of `buf` at q. Only valid inside buf's q range.
fn interp(buf: &Curve, q: f64) -> Option<(f64, f64)> {
    let pts = &buf.points;
    if pts.is_empty() || q < pts[0].q || q > pts[pts.len() - 1].q {
        return None;
    }
    for w in pts.windows(2) {
        let (p0, p1) = (w[0], w[1]);
        if q >= p0.q && q <= p1.q {
            let t = if p1.q > p0.q { (q - p0.q) / (p1.q - p0.q) } else { 0.0 };
            return Some((p0.i + t * (p1.i - p0.i), p0.sigma + t * (p1.sigma - p0.sigma)));
        }
    }
    let last = pts[pts.len() - 1];
    Some((last.i, last.sigma))
}

pub struct Subtraction {
    pub curve: Option<Curve>,
    pub judgment: GridJudgment,
    pub diagnostics: Vec<Diagnostic>,
}

/// Background subtraction with background scale factor `scale +/- scale_sigma`.
/// sigma^2 = sigma_sample^2 + scale^2 * sigma_buf^2 + (I_buf * sigma_scale)^2
pub fn subtract(sample: &Curve, buffer: &Curve, scale: f64, scale_sigma: f64) -> Subtraction {
    let mut diagnostics = vec![];
    if sample.q_unit != buffer.q_unit {
        diagnostics.push(Diagnostic {
            code: "q_unit_mismatch".into(),
            message: format!("q units differ: '{}' vs '{}'; subtraction refused", sample.q_unit, buffer.q_unit),
            affected: vec![sample.id.clone(), buffer.id.clone()],
        });
        return Subtraction {
            curve: None,
            judgment: GridJudgment::NoOverlap,
            diagnostics,
        };
    }

    if grids_identical(sample, buffer) {
        let points = sample
            .points
            .iter()
            .zip(buffer.points.iter())
            .map(|(s, b)| mk_sub_point(s.q, s.i, s.sigma, b.i, b.sigma, scale, scale_sigma))
            .collect::<Vec<_>>();
        let curve = finish_sub_curve(sample, buffer, scale, scale_sigma, points, "direct");
        return Subtraction { curve: Some(curve), judgment: GridJudgment::Identical, diagnostics };
    }

    // Grids differ: interpolate buffer onto sample grid, overlap only.
    let (s_min, s_max) = sample.q_range().unwrap();
    let (b_min, b_max) = match buffer.q_range() {
        Some(r) => r,
        None => {
            diagnostics.push(Diagnostic {
                code: "buffer_empty".into(),
                message: "buffer curve has no points".into(),
                affected: vec![buffer.id.clone()],
            });
            return Subtraction { curve: None, judgment: GridJudgment::NoOverlap, diagnostics };
        }
    };
    let (lo, hi) = (s_min.max(b_min), s_max.min(b_max));
    if lo >= hi {
        diagnostics.push(Diagnostic {
            code: "grid_no_overlap".into(),
            message: format!(
                "sample q [{:.3},{:.3}] and buffer q [{:.3},{:.3}] do not overlap; subtraction refused",
                s_min, s_max, b_min, b_max
            ),
            affected: vec![sample.id.clone(), buffer.id.clone()],
        });
        return Subtraction { curve: None, judgment: GridJudgment::NoOverlap, diagnostics };
    }

    let mut points = vec![];
    let mut n_dropped = 0usize;
    for s in &sample.points {
        if s.q < lo || s.q > hi {
            n_dropped += 1;
            continue; // outside overlap: dropped, never extrapolated
        }
        let (bi, bs) = interp(buffer, s.q).expect("inside overlap");
        points.push(mk_sub_point(s.q, s.i, s.sigma, bi, bs, scale, scale_sigma));
    }
    let n_used = points.len();
    diagnostics.push(Diagnostic {
        code: "grid_misaligned".into(),
        message: format!(
            "q grids differ; buffer interpolated onto sample grid within overlap [{:.3},{:.3}] only; {} points used, {} dropped",
            lo, hi, n_used, n_dropped
        ),
        affected: vec![sample.id.clone(), buffer.id.clone()],
    });
    let judgment = GridJudgment::Misaligned { overlap_min: lo, overlap_max: hi, n_used, n_dropped };
    let curve = finish_sub_curve(sample, buffer, scale, scale_sigma, points, "interpolated_overlap");
    Subtraction { curve: Some(curve), judgment, diagnostics }
}

fn mk_sub_point(q: f64, si: f64, ss: f64, bi: f64, bs: f64, scale: f64, scale_sigma: f64) -> CurvePoint {
    let i = si - scale * bi;
    let sigma = (ss.powi(2) + (scale * bs).powi(2) + (bi * scale_sigma).powi(2)).sqrt();
    CurvePoint { q, i, sigma, negative: i < 0.0 }
}

fn finish_sub_curve(
    sample: &Curve,
    buffer: &Curve,
    scale: f64,
    scale_sigma: f64,
    points: Vec<CurvePoint>,
    mode: &str,
) -> Curve {
    let n_negative = points.iter().filter(|p| p.negative).count();
    let id = content_id("sub", &(&sample.id, &buffer.id, scale, scale_sigma, mode, RULE_VERSION));
    Curve {
        id,
        kind: "derived:subtracted".into(),
        q_unit: sample.q_unit.clone(),
        i_unit: sample.i_unit.clone(),
        points,
        raw_text: String::new(),
        meta: serde_json::json!({
            "sample": sample.id,
            "buffer": buffer.id,
            "scale": scale,
            "scale_sigma": scale_sigma,
            "mode": mode,
            "n_negative_points": n_negative,
            "rule_version": RULE_VERSION,
        }),
    }
}

fn linfit(xs: &[f64], ys: &[f64], ws: &[f64]) -> (f64, f64) {
    let sw: f64 = ws.iter().sum();
    let sx: f64 = xs.iter().zip(ws).map(|(x, w)| x * w).sum();
    let sy: f64 = ys.iter().zip(ws).map(|(y, w)| y * w).sum();
    let sxx: f64 = xs.iter().zip(ws).map(|(x, w)| x * x * w).sum();
    let sxy: f64 = xs.iter().zip(ys).zip(ws).map(|((x, y), w)| x * y * w).sum();
    let denom = sw * sxx - sx * sx;
    if denom.abs() < 1e-30 {
        return (0.0, 0.0);
    }
    let slope = (sw * sxy - sx * sy) / denom;
    let intercept = (sy - slope * sx) / sw;
    (slope, intercept)
}

/// Guinier fit: ln I = ln I0 - Rg^2 q^2 / 3 over [q_min, q_max].
/// Points with non-positive intensity are excluded from the fit but kept in
/// the curve; the candidate records how many were excluded.
pub fn guinier(curve: &Curve, q_min: f64, q_max: f64) -> (IntervalCandidate, Vec<Diagnostic>) {
    let mut xs = vec![];
    let mut ys = vec![];
    let mut ws = vec![];
    let mut excluded = 0usize;
    for p in &curve.points {
        if p.q < q_min || p.q > q_max {
            continue;
        }
        if p.i <= 0.0 {
            excluded += 1;
            continue;
        }
        xs.push(p.q * p.q);
        ys.push(p.i.ln());
        ws.push(1.0 / (p.sigma / p.i).powi(2).max(1e-12));
    }
    let (slope, intercept) = linfit(&xs, &ys, &ws);
    let rg = if slope < 0.0 { (-3.0 * slope).sqrt() } else { f64::NAN };
    let i0 = intercept.exp();
    let residuals: Vec<f64> = xs.iter().zip(ys.iter()).map(|(x, y)| y - (slope * x + intercept)).collect();
    let rms = rms(&residuals);
    let id = content_id("fit", &("guinier", &curve.id, q_min, q_max, RULE_VERSION));
    let diags = if excluded > 0 {
        vec![Diagnostic {
            code: "guinier_excluded_nonpositive".into(),
            message: format!("{excluded} non-positive points excluded from Guinier fit (kept in curve)"),
            affected: vec![curve.id.clone()],
        }]
    } else {
        vec![]
    };
    (
        IntervalCandidate {
            id,
            model: "guinier".into(),
            q_min,
            q_max,
            params: serde_json::json!({ "Rg": rg, "I0": i0, "n_points": xs.len(), "excluded_nonpositive": excluded }),
            residuals,
            rms_residual: rms,
            adopted: false,
        },
        diags,
    )
}

/// Porod fit: plateau of I*q^4 over [q_min, q_max].
pub fn porod(curve: &Curve, q_min: f64, q_max: f64) -> (IntervalCandidate, Vec<Diagnostic>) {
    let mut vals = vec![];
    let mut ws = vec![];
    let mut excluded = 0usize;
    for p in &curve.points {
        if p.q < q_min || p.q > q_max {
            continue;
        }
        if p.i <= 0.0 {
            excluded += 1;
            continue;
        }
        vals.push(p.i * p.q.powi(4));
        ws.push(1.0 / (p.sigma * p.q.powi(4)).powi(2).max(1e-30));
    }
    let sw: f64 = ws.iter().sum();
    let plateau = if sw > 0.0 { vals.iter().zip(&ws).map(|(v, w)| v * w).sum::<f64>() / sw } else { f64::NAN };
    let residuals: Vec<f64> = vals.iter().map(|v| v - plateau).collect();
    let rms = rms(&residuals);
    let id = content_id("fit", &("porod", &curve.id, q_min, q_max, RULE_VERSION));
    let diags = if excluded > 0 {
        vec![Diagnostic {
            code: "porod_excluded_nonpositive".into(),
            message: format!("{excluded} non-positive points excluded from Porod fit (kept in curve)"),
            affected: vec![curve.id.clone()],
        }]
    } else {
        vec![]
    };
    (
        IntervalCandidate {
            id,
            model: "porod".into(),
            q_min,
            q_max,
            params: serde_json::json!({ "plateau": plateau, "n_points": vals.len(), "excluded_nonpositive": excluded }),
            residuals,
            rms_residual: rms,
            adopted: false,
        },
        diags,
    )
}

fn rms(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    (v.iter().map(|x| x * x).sum::<f64>() / v.len() as f64).sqrt()
}

/// Machine-readable judgment emitted whenever interval candidates coexist:
/// overlapping Guinier/Porod intervals are kept as separate candidates and
/// never auto-merged into a single model.
pub fn interval_judgment(candidates: &[IntervalCandidate]) -> serde_json::Value {
    let mut overlaps = vec![];
    for i in 0..candidates.len() {
        for j in (i + 1)..candidates.len() {
            let (a, b) = (&candidates[i], &candidates[j]);
            if a.q_min <= b.q_max && b.q_min <= a.q_max {
                overlaps.push(serde_json::json!({
                    "a": a.id, "b": b.id,
                    "overlap": [a.q_min.max(b.q_min), a.q_max.min(b.q_max)],
                }));
            }
        }
    }
    serde_json::json!({
        "rule": "intervals_not_auto_merged",
        "candidates": candidates.iter().map(|c| &c.id).collect::<Vec<_>>(),
        "overlapping_pairs": overlaps,
        "policy": "guinier_and_porod_kept_as_separate_candidates",
    })
}
