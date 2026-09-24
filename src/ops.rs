//! Core scattering rules.
//!
//! Semantics enforced here:
//! * different q grids are never subtracted directly; the buffer is
//!   interpolated and only inside the overlapping q range;
//! * the error model includes counting statistics, normalization
//!   (transmission) uncertainty and background scale uncertainty;
//! * Guinier and Porod intervals are candidate sets: overlapping intervals
//!   stay as separate candidates and are never merged into a single model.

use crate::model::*;

/// Normalize a raw curve by transmission, exposure time and sample thickness.
/// Unit conversion happens here (derived layer), not at import. Values whose
/// basis cannot be converted stay in place and get a diagnostic.
pub fn normalize(raw: &RawCurve, source_version: i64, meta: &ExperimentMeta) -> NormalizedCurve {
    let norm_factor = 1.0 / (meta.transmission * meta.exposure_s * meta.thickness_mm);
    let norm_rel_err = meta.transmission_err / meta.transmission;
    let mut diagnostics = Vec::new();
    let (q, canonical) = match raw.q_unit.q_to_canonical() {
        Some(f) => (raw.points.iter().map(|p| p.q * f).collect(), true),
        None => {
            diagnostics.push(Diagnostic {
                code: "unit-unconvertible".into(),
                message: format!(
                    "q unit '{}' has no conversion to nm^-1; values kept in raw basis, curve excluded from subtraction",
                    raw.q_unit.label()
                ),
                affected: (0..raw.points.len()).collect(),
            });
            (raw.points.iter().map(|p| p.q).collect(), false)
        }
    };
    let i: Vec<f64> = raw.points.iter().map(|p| p.i * norm_factor).collect();
    let err: Vec<f64> = raw
        .points
        .iter()
        .zip(i.iter())
        .map(|(p, ni)| {
            let counting = p.err * norm_factor;
            (counting * counting + (ni * norm_rel_err) * (ni * norm_rel_err)).sqrt()
        })
        .collect();
    NormalizedCurve {
        source_id: raw.id.clone(),
        source_version,
        q,
        i,
        err,
        q_unit: if canonical { Unit::InvNm } else { raw.q_unit.clone() },
        norm_factor,
        norm_rel_err,
        canonical,
        diagnostics,
    }
}

fn grids_match(a: &[f64], b: &[f64]) -> bool {
    // grids "match" if every shared-range sample point coincides with a buffer point
    let lo = a[0].max(b[0]);
    let hi = a[a.len() - 1].min(b[b.len() - 1]);
    a.iter()
        .filter(|&&q| q >= lo && q <= hi)
        .all(|&q| b.iter().any(|&bq| (bq - q).abs() < 1e-9))
}

/// Linear interpolation of (xs, ys) at x. Caller guarantees x is inside range.
fn interp(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    match xs.binary_search_by(|v| v.partial_cmp(&x).unwrap()) {
        Ok(k) => ys[k],
        Err(0) => ys[0],
        Err(k) if k >= xs.len() => ys[xs.len() - 1],
        Err(k) => {
            let (x0, x1) = (xs[k - 1], xs[k]);
            let (y0, y1) = (ys[k - 1], ys[k]);
            let t = (x - x0) / (x1 - x0);
            y0 + t * (y1 - y0)
        }
    }
}

/// Subtract a normalized buffer from a normalized sample.
///
/// The grids are never subtracted point-by-point unless they coincide; the
/// buffer is interpolated onto the sample grid strictly inside the overlap
/// range. Sample points outside the overlap are returned as dropped, not
/// extrapolated. The error budget adds the background scale uncertainty.
pub fn subtract(
    sample: &NormalizedCurve,
    sample_version: i64,
    buffer: &NormalizedCurve,
    buffer_version: i64,
    scale: f64,
    scale_err: f64,
) -> Result<SubtractedCurve, SubtractFailure> {
    if !sample.canonical {
        return Err(SubtractFailure::NotCanonical {
            which: sample.source_id.clone(),
            reason: "sample q basis not convertible to nm^-1".into(),
        });
    }
    if !buffer.canonical {
        return Err(SubtractFailure::NotCanonical {
            which: buffer.source_id.clone(),
            reason: "buffer q basis not convertible to nm^-1".into(),
        });
    }
    let sample_range = (sample.q[0], sample.q[sample.q.len() - 1]);
    let buffer_range = (buffer.q[0], buffer.q[buffer.q.len() - 1]);
    let lo = sample_range.0.max(buffer_range.0);
    let hi = sample_range.1.min(buffer_range.1);
    if lo >= hi {
        return Err(SubtractFailure::NoOverlap { sample_range, buffer_range });
    }
    let matched = grids_match(&sample.q, &buffer.q);
    let mut q = Vec::new();
    let mut i = Vec::new();
    let mut err = Vec::new();
    let mut dropped = Vec::new();
    let mut diagnostics = Vec::new();
    if !matched {
        diagnostics.push(Diagnostic {
            code: "q-grid-mismatch".into(),
            message: "q grids differ: buffer interpolated onto sample grid, restricted to overlap range; no direct point-wise subtraction".into(),
            affected: vec![],
        });
    }
    for (k, &sq) in sample.q.iter().enumerate() {
        if sq < lo || sq > hi {
            dropped.push(sq);
            continue;
        }
        let bi = interp(&buffer.q, &buffer.i, sq);
        let be = interp(&buffer.q, &buffer.err, sq);
        let si = sample.i[k];
        let se = sample.err[k];
        let val = si - scale * bi;
        // error model: sample counting+normalization, buffer statistics, and
        // the background scale uncertainty applied to the buffer level
        let var = se * se + (scale * be) * (scale * be) + (bi * scale_err) * (bi * scale_err);
        q.push(sq);
        i.push(val);
        err.push(var.sqrt());
    }
    let negative_indices: Vec<usize> = i
        .iter()
        .enumerate()
        .filter(|(_, v)| **v < 0.0)
        .map(|(k, _)| k)
        .collect();
    if !negative_indices.is_empty() {
        diagnostics.push(Diagnostic {
            code: "negative-intensity".into(),
            message: format!(
                "{} negative intensities after subtraction; kept (not clipped) and flagged for review",
                negative_indices.len()
            ),
            affected: negative_indices.clone(),
        });
    }
    Ok(SubtractedCurve {
        sample_id: sample.source_id.clone(),
        sample_version,
        buffer_id: buffer.source_id.clone(),
        buffer_version,
        scale,
        scale_err,
        q,
        i,
        err,
        overlap: (lo, hi),
        dropped_sample_q: dropped,
        negative_indices,
        grids_matched: matched,
        diagnostics,
    })
}

/// Weighted linear fit y = slope*x + intercept.
/// Returns (slope, slope_err, intercept, intercept_err, rms_residual).
pub fn linfit(x: &[f64], y: &[f64], w: &[f64]) -> Option<(f64, f64, f64, f64, f64)> {
    let n = x.len();
    if n < 3 {
        return None;
    }
    let sw: f64 = w.iter().sum();
    let sx: f64 = x.iter().zip(w).map(|(a, b)| a * b).sum();
    let sy: f64 = y.iter().zip(w).map(|(a, b)| a * b).sum();
    let sxx: f64 = x.iter().zip(w).map(|(a, b)| a * a * b).sum();
    let sxy: f64 = x.iter().zip(y).zip(w).map(|((a, b), c)| a * b * c).sum();
    let delta = sw * sxx - sx * sx;
    if delta.abs() < 1e-30 {
        return None;
    }
    let slope = (sw * sxy - sx * sy) / delta;
    let intercept = (sxx * sy - sx * sxy) / delta;
    let slope_err = (sw / delta).sqrt();
    let intercept_err = (sxx / delta).sqrt();
    let rms = (x.iter().zip(y.iter())
        .map(|(a, b)| {
            let r = b - (slope * a + intercept);
            r * r
        })
        .sum::<f64>()
        / n as f64)
        .sqrt();
    Some((slope, slope_err, intercept, intercept_err, rms))
}

/// Guinier candidates: ln I vs q^2 linear fit per window. Overlapping windows
/// remain separate candidates; nothing is merged.
pub fn guinier_candidates(
    curve_id: &str,
    curve_version: i64,
    curve: &SubtractedCurve,
    windows: &[(f64, f64)],
) -> Vec<IntervalCandidate> {
    let mut out = Vec::new();
    for (n, &(lo, hi)) in windows.iter().enumerate() {
        let mut x = Vec::new();
        let mut y = Vec::new();
        let mut w = Vec::new();
        for (k, &q) in curve.q.iter().enumerate() {
            if q >= lo && q <= hi && curve.i[k] > 0.0 {
                x.push(q * q);
                y.push(curve.i[k].ln());
                let wgt = curve.i[k] / curve.err[k].max(1e-12);
                w.push(wgt * wgt);
            }
        }
        if let Some((slope, slope_err, intercept, intercept_err, rms)) = linfit(&x, &y, &w) {
            let rg = (-3.0 * slope).max(0.0).sqrt();
            let rg_err = if rg > 0.0 { 1.5 * slope_err / rg } else { 0.0 };
            out.push(IntervalCandidate {
                id: format!("guinier:{curve_id}:{n}"),
                curve_id: curve_id.into(),
                curve_version,
                kind: IntervalKind::Guinier,
                q_min: lo,
                q_max: hi,
                params: vec![
                    ("Rg".into(), rg, rg_err),
                    ("I0".into(), intercept.exp(), intercept.exp() * intercept_err),
                ],
                rms_residual: rms,
                n_points: x.len(),
                status: CandidateStatus::Pending,
            });
        }
    }
    out
}

/// Porod candidates: ln I vs ln q fit per window, exponent near -4 expected.
pub fn porod_candidates(
    curve_id: &str,
    curve_version: i64,
    curve: &SubtractedCurve,
    windows: &[(f64, f64)],
) -> Vec<IntervalCandidate> {
    let mut out = Vec::new();
    for (n, &(lo, hi)) in windows.iter().enumerate() {
        let mut x = Vec::new();
        let mut y = Vec::new();
        let mut w = Vec::new();
        for (k, &q) in curve.q.iter().enumerate() {
            if q >= lo && q <= hi && curve.i[k] > 0.0 {
                x.push(q.ln());
                y.push(curve.i[k].ln());
                let wgt = curve.i[k] / curve.err[k].max(1e-12);
                w.push(wgt * wgt);
            }
        }
        if let Some((slope, slope_err, intercept, intercept_err, rms)) = linfit(&x, &y, &w) {
            out.push(IntervalCandidate {
                id: format!("porod:{curve_id}:{n}"),
                curve_id: curve_id.into(),
                curve_version,
                kind: IntervalKind::Porod,
                q_min: lo,
                q_max: hi,
                params: vec![
                    ("exponent".into(), slope, slope_err),
                    ("K".into(), intercept.exp(), intercept.exp() * intercept_err),
                ],
                rms_residual: rms,
                n_points: x.len(),
                status: CandidateStatus::Pending,
            });
        }
    }
    out
}

/// Multi-candidate residual check: each background scale hypothesis produces
/// its own subtracted curve and RMS residual over the fit range. All legal
/// candidates are returned; the caller decides, nothing is collapsed.
pub fn residual_candidates(
    sample: &NormalizedCurve,
    sample_version: i64,
    buffer: &NormalizedCurve,
    buffer_version: i64,
    scales: &[(f64, f64)],
    fit_range: (f64, f64),
) -> Vec<(ResidualCandidate, SubtractedCurve)> {
    let mut out = Vec::new();
    for (n, &(scale, scale_err)) in scales.iter().enumerate() {
        if let Ok(sub) = subtract(sample, sample_version, buffer, buffer_version, scale, scale_err) {
            let mut acc = 0.0;
            let mut cnt = 0usize;
            for (k, &q) in sub.q.iter().enumerate() {
                if q >= fit_range.0 && q <= fit_range.1 {
                    acc += sub.i[k] * sub.i[k];
                    cnt += 1;
                }
            }
            let rms = if cnt > 0 { (acc / cnt as f64).sqrt() } else { f64::NAN };
            let cand = ResidualCandidate {
                id: format!("residual:{}:{}:{n}", sample.source_id, buffer.source_id),
                scale,
                scale_err,
                subtracted_node: format!("sub:{}:{}", sample.source_id, buffer.source_id),
                subtracted_version: 0, // filled by caller after persisting
                rms,
                status: CandidateStatus::Pending,
            };
            out.push((cand, sub));
        }
    }
    out
}
