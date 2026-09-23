//! Core scattering rules. These functions are pure and fully tested:
//!
//! * normalization with uncertainty propagation (exposure, thickness,
//!   transmission),
//! * q-grid reconciliation: no direct subtraction across different grids;
//!   interpolation is only valid inside the overlap range,
//! * background subtraction whose error budget includes normalization and
//!   background-scale uncertainty,
//! * negative-intensity flagging,
//! * Guinier / Porod interval candidates that are never auto-merged,
//! * multi-candidate residual comparison that preserves ambiguity.

use crate::domain::*;

/// Normalize a raw curve by exposure * thickness * transmission.
/// The returned relative uncertainty of the normalization factor is carried
/// forward so subtraction can include it in the error budget.
pub fn normalization_factors(
    exposure: &RawQty,
    thickness: &RawQty,
    transmission: &RawQty,
) -> Result<NormFactors, Diagnostic> {
    let exp = exposure.parsed.as_ref().ok_or_else(|| Diagnostic {
        code: "exposure-unparseable".into(),
        message: format!("exposure kept raw: {:?}", exposure.diagnostic),
        point_index: None,
    })?;
    let exp_s = to_seconds(exp).ok_or_else(|| Diagnostic {
        code: "exposure-unit".into(),
        message: format!("cannot convert exposure unit '{}' to seconds", exp.unit),
        point_index: None,
    })?;
    let th = thickness.parsed.as_ref().ok_or_else(|| Diagnostic {
        code: "thickness-unparseable".into(),
        message: format!("thickness kept raw: {:?}", thickness.diagnostic),
        point_index: None,
    })?;
    let th_cm = to_cm(th).ok_or_else(|| Diagnostic {
        code: "thickness-unit".into(),
        message: format!("cannot convert thickness unit '{}' to cm", th.unit),
        point_index: None,
    })?;
    let tr = transmission.parsed.as_ref().ok_or_else(|| Diagnostic {
        code: "transmission-unparseable".into(),
        message: format!("transmission kept raw: {:?}", transmission.diagnostic),
        point_index: None,
    })?;
    if !(tr.unit.is_empty() || tr.unit == "1" || tr.unit == "dimensionless") {
        return Err(Diagnostic {
            code: "transmission-unit".into(),
            message: format!("transmission must be dimensionless, got '{}'", tr.unit),
            point_index: None,
        });
    }
    if exp_s <= 0.0 || th_cm <= 0.0 || tr.value <= 0.0 {
        return Err(Diagnostic {
            code: "normalization-nonpositive".into(),
            message: format!(
                "normalization factors must be positive (exposure={exp_s} s, thickness={th_cm} cm, transmission={})", tr.value
            ),
            point_index: None,
        });
    }
    let tr_rel_unc = match tr.uncertainty {
        Some(u) => u / tr.value,
        None => 0.0,
    };
    let factor = exp_s * th_cm * tr.value;
    Ok(NormFactors {
        exposure_s: exp_s,
        thickness_cm: th_cm,
        transmission: tr.value,
        transmission_rel_unc: tr_rel_unc,
        factor,
        factor_rel_unc: tr_rel_unc,
    })
}

/// Apply normalization factors to a curve. Statistical sigmas are scaled;
/// the factor's own relative uncertainty is *not* folded into each point here
/// (it is a correlated term and is applied at subtraction time).
pub fn apply_normalization(curve: &Curve, factors: &NormFactors) -> Curve {
    Curve {
        q: curve.q.clone(),
        i: curve.i.iter().map(|v| v / factors.factor).collect(),
        sigma: curve.sigma.iter().map(|s| s / factors.factor).collect(),
        q_unit: curve.q_unit.clone(),
        i_unit: CANONICAL_I_UNIT.to_string(),
    }
}

/// Linear interpolation of `(xq, xi)` onto `x`. Caller must guarantee `x` is
/// inside the tabulated range (we clamp to the nearest segment anyway).
fn interp(xq: &[f64], xi: &[f64], x: f64) -> f64 {
    if xq.is_empty() {
        return f64::NAN;
    }
    if x <= xq[0] {
        return xi[0];
    }
    if x >= xq[xq.len() - 1] {
        return xi[xi.len() - 1];
    }
    let mut lo = 0usize;
    let mut hi = xq.len() - 1;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if xq[mid] <= x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let t = (x - xq[lo]) / (xq[hi] - xq[lo]);
    xi[lo] + t * (xi[hi] - xi[lo])
}

/// Reconcile the sample and buffer q grids.
///
/// * Identical grids -> exact point-wise combination.
/// * Different grids -> buffer is interpolated onto the sample grid, but only
///   sample points strictly inside the q overlap survive; points outside the
///   overlap are dropped (never extrapolated).
/// * Disjoint ranges -> `GridDecision::NoOverlap`, subtraction refused.
pub fn reconcile_grids(sample: &Curve, buffer: &Curve) -> (GridDecision, Option<(Curve, Curve)>) {
    let exact = sample.q.len() == buffer.q.len()
        && sample
            .q
            .iter()
            .zip(buffer.q.iter())
            .all(|(a, b)| (a - b).abs() <= 1e-12);
    if exact {
        return (
            GridDecision::ExactMatch,
            Some((sample.clone(), buffer.clone())),
        );
    }
    let (s_lo, s_hi) = match sample.q_range() {
        Some(r) => r,
        None => return (GridDecision::NoOverlap, None),
    };
    let (b_lo, b_hi) = match buffer.q_range() {
        Some(r) => r,
        None => return (GridDecision::NoOverlap, None),
    };
    let lo = s_lo.max(b_lo);
    let hi = s_hi.min(b_hi);
    if lo >= hi {
        return (GridDecision::NoOverlap, None);
    }
    let mut qs = Vec::new();
    let mut si = Vec::new();
    let mut ss = Vec::new();
    let mut bi = Vec::new();
    let mut bs = Vec::new();
    let mut dropped = 0usize;
    for k in 0..sample.q.len() {
        let q = sample.q[k];
        if q >= lo && q <= hi {
            qs.push(q);
            si.push(sample.i[k]);
            ss.push(sample.sigma[k]);
            bi.push(interp(&buffer.q, &buffer.i, q));
            bs.push(interp(&buffer.q, &buffer.sigma, q));
        } else {
            dropped += 1;
        }
    }
    if qs.is_empty() {
        return (GridDecision::NoOverlap, None);
    }
    let kept = qs.len();
    let s = Curve {
        q: qs.clone(),
        i: si,
        sigma: ss,
        q_unit: sample.q_unit.clone(),
        i_unit: sample.i_unit.clone(),
    };
    let b = Curve {
        q: qs,
        i: bi,
        sigma: bs,
        q_unit: sample.q_unit.clone(),
        i_unit: buffer.i_unit.clone(),
    };
    (
        GridDecision::InterpolatedOverlap { overlap: (lo, hi), kept, dropped },
        Some((s, b)),
    )
}

/// Subtract `scale * buffer` from sample.
///
/// Error budget per point:
///   sigma^2 = sigma_s^2 + scale^2 * sigma_b^2
///             + (buffer * scale * scale_rel_unc)^2        (background scale)
///             + (sample * norm_rel_unc)^2                 (normalization, correlated)
///             + (buffer * scale * buffer_norm_rel_unc)^2  (buffer normalization)
///
/// Negative resulting intensities are flagged, not clamped.
pub fn subtract(
    sample: &Curve,
    buffer: &Curve,
    scale: f64,
    scale_rel_unc: f64,
    sample_norm_rel_unc: f64,
    buffer_norm_rel_unc: f64,
) -> SubtractionResult {
    let (decision, aligned) = reconcile_grids(sample, buffer);
    let Some((s, b)) = aligned else {
        return SubtractionResult {
            curve: Curve {
                q: vec![],
                i: vec![],
                sigma: vec![],
                q_unit: sample.q_unit.clone(),
                i_unit: sample.i_unit.clone(),
            },
            grid_decision: decision,
            scale,
            scale_rel_unc,
            negative_points: vec![],
            diagnostics: vec![Diagnostic {
                code: "no-overlap".into(),
                message: "sample and buffer q ranges do not overlap; subtraction refused".into(),
                point_index: None,
            }],
        };
    };
    let mut i_out = Vec::with_capacity(s.q.len());
    let mut s_out = Vec::with_capacity(s.q.len());
    let mut neg = Vec::new();
    for k in 0..s.q.len() {
        let sv = s.i[k];
        let bv = b.i[k] * scale;
        let v = sv - bv;
        if v < 0.0 {
            neg.push(k);
        }
        let var = s.sigma[k] * s.sigma[k]
            + scale * scale * b.sigma[k] * b.sigma[k]
            + (bv * scale_rel_unc) * (bv * scale_rel_unc)
            + (sv * sample_norm_rel_unc) * (sv * sample_norm_rel_unc)
            + (bv * buffer_norm_rel_unc) * (bv * buffer_norm_rel_unc);
        i_out.push(v);
        s_out.push(var.sqrt());
    }
    let mut diagnostics = Vec::new();
    if !neg.is_empty() {
        diagnostics.push(Diagnostic {
            code: "negative-intensity".into(),
            message: format!("{} point(s) with negative intensity after subtraction", neg.len()),
            point_index: Some(neg[0]),
        });
    }
    SubtractionResult {
        curve: Curve {
            q: s.q,
            i: i_out,
            sigma: s_out,
            q_unit: s.q_unit,
            i_unit: s.i_unit,
        },
        grid_decision: decision,
        scale,
        scale_rel_unc,
        negative_points: neg,
        diagnostics,
    }
}

/// Weighted linear least squares of y = a + b*x with per-point sigma.
/// Returns (a, b, chi2_reduced).
fn wlinfit(x: &[f64], y: &[f64], s: &[f64]) -> Option<(f64, f64, f64)> {
    let mut sw = 0.0;
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    for k in 0..x.len() {
        let w = 1.0 / (s[k] * s[k]).max(1e-30);
        sw += w;
        sx += w * x[k];
        sy += w * y[k];
        sxx += w * x[k] * x[k];
        sxy += w * x[k] * y[k];
    }
    let d = sw * sxx - sx * sx;
    if d.abs() < 1e-30 {
        return None;
    }
    let a = (sxx * sy - sx * sxy) / d;
    let b = (sw * sxy - sx * sy) / d;
    let mut chi2 = 0.0;
    for k in 0..x.len() {
        let r = (y[k] - a - b * x[k]) / s[k];
        chi2 += r * r;
    }
    let dof = (x.len() as f64 - 2.0).max(1.0);
    Some((a, b, chi2 / dof))
}

/// Indices of points whose q falls inside [q_min, q_max].
fn window(q: &[f64], q_min: f64, q_max: f64) -> Vec<usize> {
    (0..q.len()).filter(|&k| q[k] >= q_min && q[k] <= q_max).collect()
}

/// Build a Guinier candidate: fit ln I vs q^2 on the interval.
pub fn guinier_candidate(curve: &Curve, q_min: f64, q_max: f64) -> Option<IntervalCandidate> {
    let idx = window(&curve.q, q_min, q_max);
    if idx.len() < 3 {
        return None;
    }
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut s = Vec::new();
    for &k in &idx {
        if curve.i[k] <= 0.0 {
            return None; // Guinier undefined on non-positive intensity
        }
        x.push(curve.q[k] * curve.q[k]);
        y.push(curve.i[k].ln());
        s.push(curve.sigma[k] / curve.i[k]);
    }
    let (a, b, chi2) = wlinfit(&x, &y, &s)?;
    let rg = (-3.0 * b).max(0.0).sqrt();
    let i0 = a.exp();
    Some(IntervalCandidate {
        kind: IntervalKind::Guinier,
        q_min,
        q_max,
        params: vec![("Rg".into(), rg), ("I0".into(), i0)],
        residual_chi2: chi2,
        overlaps_other: false,
        status: CandidateStatus::Open,
    })
}

/// Build a Porod candidate: fit log-log slope on the interval.
pub fn porod_candidate(curve: &Curve, q_min: f64, q_max: f64) -> Option<IntervalCandidate> {
    let idx = window(&curve.q, q_min, q_max);
    if idx.len() < 3 {
        return None;
    }
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut s = Vec::new();
    for &k in &idx {
        if curve.i[k] <= 0.0 || curve.q[k] <= 0.0 {
            return None;
        }
        x.push(curve.q[k].ln());
        y.push(curve.i[k].ln());
        s.push(curve.sigma[k] / curve.i[k]);
    }
    let (a, b, chi2) = wlinfit(&x, &y, &s)?;
    Some(IntervalCandidate {
        kind: IntervalKind::Porod,
        q_min,
        q_max,
        params: vec![("slope".into(), b), ("logK".into(), a)],
        residual_chi2: chi2,
        overlaps_other: false,
        status: CandidateStatus::Open,
    })
}

/// Mark interval overlap between candidates of different kinds. Overlapping
/// Guinier and Porod intervals stay separate candidates; they are flagged so
/// reviewers see the overlap, but they are never merged into one model.
pub fn flag_overlaps(candidates: &mut [IntervalCandidate]) {
    for a in 0..candidates.len() {
        for b in (a + 1)..candidates.len() {
            let (ca, cb) = (candidates[a].clone(), candidates[b].clone());
            let overlap = ca.q_min < cb.q_max && cb.q_min < ca.q_max;
            if overlap {
                candidates[a].overlaps_other = true;
                candidates[b].overlaps_other = true;
            }
        }
    }
}

/// Chi2 of a subtracted curve inside its overlap range (goodness of the
/// background match, judged on the high-q tail where sample signal ~ 0).
pub fn tail_chi2(curve: &Curve, tail_fraction: f64) -> f64 {
    let n = curve.q.len();
    if n < 4 {
        return f64::NAN;
    }
    let start = ((n as f64) * (1.0 - tail_fraction)) as usize;
    let mut chi2 = 0.0;
    let mut count = 0usize;
    for k in start..n {
        if curve.sigma[k] > 0.0 {
            let r = curve.i[k] / curve.sigma[k];
            chi2 += r * r;
            count += 1;
        }
    }
    if count == 0 {
        return f64::NAN;
    }
    chi2 / count as f64
}

/// Compare subtraction results across buffer candidates. All candidates are
/// kept; if the best two are statistically indistinguishable the comparison
/// is reported as ambiguous instead of picking a winner.
pub fn compare_residuals(entries: Vec<ResidualEntry>) -> ResidualComparison {
    let mut sorted = entries.clone();
    sorted.sort_by(|a, b| a.chi2.partial_cmp(&b.chi2).unwrap_or(std::cmp::Ordering::Equal));
    let ambiguous = sorted.len() >= 2
        && sorted[0].chi2.is_finite()
        && sorted[1].chi2.is_finite()
        && (sorted[1].chi2 - sorted[0].chi2).abs() < 0.5;
    ResidualComparison { entries, ambiguous }
}
