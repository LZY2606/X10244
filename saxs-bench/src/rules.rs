//! Pure domain rules. No I/O, no SQLite: fully unit-testable.
//!
//! Invariants implemented here:
//! * curves on different q grids are never subtracted point-wise; the buffer is
//!   linearly interpolated and subtraction is only defined on the overlap;
//! * uncertainty includes normalization propagation and background scale sigma;
//! * negative subtracted intensities are retained, never clipped;
//! * Guinier and Porod remain separate candidate lists and are never merged.

use crate::derived::{
    GuinierFit, ModelAnalysis, NormPoint, NormalizedCurve, PorodFit, ScaleCandidate, SubPoint,
    Subtraction,
};
use crate::domain::{
    CurveKind, Findings, InstrumentResponse, MaskSource, Quantity, RawCurve, RawPoint,
};
use crate::ids;

// ---------------------------------------------------------------------------
// small numeric helpers
// ---------------------------------------------------------------------------

fn canonical_finite(q: &Quantity) -> Option<f64> {
    q.canonical.filter(|v| v.is_finite())
}

/// Linear interpolation of a sorted (x,y) table at `xq`. Returns the value, the
/// bracketing indices and the weight toward the upper point. Only valid inside
/// the tabulated range (caller guarantees overlap).
pub fn interp(xs: &[f64], ys: &[f64], xq: f64) -> (f64, usize, usize, f64) {
    if xs.len() == 1 {
        return (ys[0], 0, 0, 0.0);
    }
    let hi = xs.partition_point(|x| *x < xq).min(xs.len() - 1);
    let lo = hi.saturating_sub(1);
    let denom = xs[hi] - xs[lo];
    let w = if denom == 0.0 { 0.0 } else { (xq - xs[lo]) / denom };
    let y = ys[lo] * (1.0 - w) + ys[hi] * w;
    (y, lo, hi, w)
}

fn weighted_slope_intercept(
    xs: &[f64],
    ys: &[f64],
    ws: &[f64],
) -> Option<(f64, f64)> {
    let (mut sw, mut sx, mut sy, mut sxx, mut sxy) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
    for ((x, y), w) in xs.iter().zip(ys).zip(ws) {
        if !(w > 0.0) || !x.is_finite() || !y.is_finite() {
            continue;
        }
        sw += w;
        sx += w * x;
        sy += w * y;
        sxx += w * x * x;
        sxy += w * x * y;
    }
    if sw <= 0.0 {
        return None;
    }
    let det = sw * sxx - sx * sx;
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let intercept = (sy * sxx - sx * sxy) / det;
    let slope = (sw * sxy - sx * sy) / det;
    Some((slope, intercept))
}

// ---------------------------------------------------------------------------
// normalization
// ---------------------------------------------------------------------------

fn norm_meta_factors(raw: &RawCurve) -> Result<(f64, f64, f64), Vec<String>> {
    let mut errs = Vec::new();
    let exp = canonical_finite(&raw.exposure)
        .filter(|v| *v > 0.0)
        .ok_or_else(|| "exposure missing, <=0, or in an unknown unit".to_string());
    let tr = canonical_finite(&raw.transmission)
        .filter(|v| *v > 0.0)
        .ok_or_else(|| "transmission missing, <=0, or in an unknown unit".to_string());
    let th = if raw.kind == CurveKind::Sample {
        canonical_finite(&raw.thickness)
            .filter(|v| *v > 0.0)
            .ok_or_else(|| "sample thickness missing, <=0, or in an unknown unit".to_string())
    } else {
        // buffers need no thickness; if supplied and valid use it, else neutral
        canonical_finite(&raw.thickness).filter(|v| *v > 0.0).ok_or(())
    };
    if let Err(e) = &exp {
        errs.push(e.clone());
    }
    if let Err(e) = &tr {
        errs.push(e.clone());
    }
    let th = match th {
        Ok(v) => v,
        Err(_) => {
            if raw.kind == CurveKind::Sample {
                errs.push("sample thickness missing".to_string());
                1.0
            } else {
                1.0
            }
        }
    };
    if errs.is_empty() {
        Ok((exp.unwrap(), tr.unwrap(), th))
    } else {
        Err(errs)
    }
}

fn point_q_and_i(p: &RawPoint) -> Result<(f64, f64, f64), String> {
    let q = canonical_finite(&p.q).ok_or("point q not on a convertible grid")?;
    let i = p.i.canonical.filter(|v| v.is_finite()).ok_or("point I not numeric")?;
    let sigma = p
        .sigma
        .canonical
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(0.0);
    Ok((q, i, sigma))
}

/// Normalize one raw curve against detector response and mask provenance.
pub fn normalize(
    raw: &RawCurve,
    response: Option<&InstrumentResponse>,
    mask: Option<&MaskSource>,
) -> NormalizedCurve {
    let mut findings = Findings::ok();
    let (exp, tr, th) = match norm_meta_factors(raw) {
        Ok(v) => v,
        Err(errs) => {
            for e in errs {
                findings = findings.fail("NORMALIZATION_META_INVALID", e);
            }
            (f64::NAN, f64::NAN, f64::NAN)
        }
    };
    let meta_ok = findings.ok;

    let mask_id = mask.map(|m| m.id.clone()).unwrap_or_default();
    let response_id = response.map(|r| r.id.clone()).unwrap_or_default();

    let mut points = Vec::new();
    for (idx, p) in raw.points.iter().enumerate() {
        let (q_nm, i_raw, sigma_raw) = match point_q_and_i(p) {
            Ok(v) => v,
            Err(e) => {
                findings = findings.fail("POINT_UNCONVERTED", format!("index {idx}: {e}"));
                continue;
            }
        };
        let masked = p.masked;
        let (gain, uncovered) = match response {
            Some(r) => {
                let inside = q_nm >= r.q_nm[0] && q_nm <= *r.q_nm.last().unwrap();
                if inside {
                    (interp(&r.q_nm, &r.gain, q_nm).0.max(1e-12), false)
                } else {
                    (1.0, true)
                }
            }
            None => (1.0, false),
        };
        if uncovered {
            findings = findings.warn(
                "RESPONSE_UNCOVERED",
                format!("q={q_nm:.4} outside response calibration; retained, excluded from fits"),
            );
        }
        if meta_ok {
            // exposure to count/s, divide transmission and thickness, apply gain
            let norm = i_raw / (exp * tr * th) / gain;
            let sigma = if i_raw > 0.0 {
                (sigma_raw / (exp * tr * th) / gain).max(norm.sqrt())
            } else {
                sigma_raw / (exp * tr * th) / gain
            };
            let id = ids::point_id(&raw.id, idx, &format!("{q_nm:.8}"));
            points.push(NormPoint {
                id,
                index: idx,
                q_nm,
                i: norm,
                sigma,
                masked,
                uncovered_response: uncovered,
                note: p.mask_reason.clone(),
            });
        } else {
            // keep the point in original position but flag it; cannot normalize
            let id = ids::point_id(&raw.id, idx, &format!("{q_nm:.8}"));
            points.push(NormPoint {
                id,
                index: idx,
                q_nm,
                i: f64::NAN,
                sigma: f64::NAN,
                masked,
                uncovered_response: uncovered,
                note: Some("not normalized: invalid metadata".to_string()),
            });
        }
    }

    let body = serde_json::to_string(&(&raw.id, &response_id, &mask_id, &points)).unwrap_or_default();
    let id = ids::norm_id(&raw.id, &response_id, &mask_id, &body);
    NormalizedCurve {
        id,
        raw_id: raw.id.clone(),
        logical: raw.logical.clone(),
        version: raw.version.clone(),
        response_id,
        mask_id,
        points,
        exposure_s: exp,
        transmission: tr,
        thickness_mm: th,
        findings,
    }
}

// ---------------------------------------------------------------------------
// overlap + background subtraction
// ---------------------------------------------------------------------------

fn usable(p: &NormPoint) -> bool {
    !p.masked && !p.uncovered_response && p.i.is_finite() && p.i > 0.0
}

/// Inclusive overlap [lo, hi] of the usable sample grid with the buffer grid.
pub fn overlap_range(sample: &NormalizedCurve, buffer: &NormalizedCurve) -> Option<(f64, f64)> {
    let s: Vec<f64> = sample.points.iter().filter(|p| usable(p)).map(|p| p.q_nm).collect();
    let b: Vec<f64> = buffer
        .points
        .iter()
        .filter(|p| !p.masked && p.i.is_finite())
        .map(|p| p.q_nm)
        .collect();
    if s.is_empty() || b.is_empty() {
        return None;
    }
    let lo = s[0].max(b[0]);
    let hi = s[s.len() - 1].min(b[b.len() - 1]);
    if lo > hi {
        return None;
    }
    Some((lo, hi))
}

struct ScaleFit {
    alpha: f64,
    sigma: f64,
    used: usize,
    chi2: f64,
    q_lo: f64,
    q_hi: f64,
}

/// Weighted least squares of sample = alpha * buffer over a q window.
/// Minimises sum w (I_s - alpha I_b)^2 with weights 1/sigma_s^2.
fn fit_alpha(
    sample: &NormalizedCurve,
    buffer: &NormalizedCurve,
    q_lo: f64,
    q_hi: f64,
) -> Option<ScaleFit> {
    let bq: Vec<f64> = buffer.points.iter().filter(|p| !p.masked && p.i.is_finite()).map(|p| p.q_nm).collect();
    let bi: Vec<f64> = buffer.points.iter().filter(|p| !p.masked && p.i.is_finite()).map(|p| p.i).collect();
    let mut num = 0.0;
    let mut den = 0.0;
    let mut chi2 = 0.0;
    let mut used = 0usize;
    let mut actual_lo = f64::INFINITY;
    let mut actual_hi = f64::NEG_INFINITY;
    for sp in sample.points.iter().filter(|p| usable(p) && p.q_nm >= q_lo && p.q_nm <= q_hi) {
        if sp.q_nm < bq[0] || sp.q_nm > bq[bq.len() - 1] {
            continue;
        }
        let ib = interp(&bq, &bi, sp.q_nm).0;
        if !(ib > 0.0) {
            continue;
        }
        let w = 1.0 / sp.sigma.max(1e-9).powi(2);
        num += w * sp.i * ib;
        den += w * ib * ib;
        used += 1;
        actual_lo = actual_lo.min(sp.q_nm);
        actual_hi = actual_hi.max(sp.q_nm);
    }
    if used < 2 || den <= 0.0 {
        return None;
    }
    let alpha = num / den;
    // sigma_alpha from curvature of chi2: var = 2 / (d2chi2/dalpha2) = 1/den,
    // scaled by reduced chi2 to absorb mismatch (background scale uncertainty)
    for sp in sample.points.iter().filter(|p| usable(p) && p.q_nm >= actual_lo && p.q_nm <= actual_hi) {
        let ib = interp(&bq, &bi, sp.q_nm).0;
        let w = 1.0 / sp.sigma.max(1e-9).powi(2);
        chi2 += w * (sp.i - alpha * ib).powi(2);
    }
    let reduced = chi2 / (used as f64 - 1.0);
    let sigma = (reduced / den).sqrt();
    Some(ScaleFit { alpha, sigma, used, chi2, q_lo: actual_lo, q_hi: actual_hi })
}

/// Subtract buffer from sample. Produces points only on the overlap; negatives
/// are kept. Emits all legal scale candidates plus uncertainty.
pub fn subtract(sample: &NormalizedCurve, buffer: &NormalizedCurve) -> Subtraction {
    let mut findings = Findings::ok();
    let Some((lo, hi)) = overlap_range(sample, buffer) else {
        return Subtraction {
            id: ids::sub_id(&sample.id, &buffer.id, "no-overlap"),
            sample_norm_id: sample.id.clone(),
            buffer_norm_id: buffer.id.clone(),
            alpha: f64::NAN,
            sigma_alpha: f64::NAN,
            scale_candidates: vec![],
            overlap_lo: f64::NAN,
            overlap_hi: f64::NAN,
            points: vec![],
            findings: Findings::ok().fail(
                "NO_OVERLAP",
                "q grids have no overlapping region; subtraction is undefined and no direct subtraction was attempted",
            ),
        };
    };

    // Candidate 1: full overlap. Candidates 2..: thirds of the overlap, which
    // lets a scale drift / non-flat background surface multiple legal values.
    let full = fit_alpha(sample, buffer, lo, hi);
    let mut fits: Vec<(&str, ScaleFit)> = Vec::new();
    if let Some(f) = full {
        fits.push(("full-overlap", f));
    }
    let span = hi - lo;
    let cuts = [
        (lo, lo + span / 2.0),
        (lo + span / 4.0, lo + 3.0 * span / 4.0),
        (lo + span / 2.0, hi),
    ];
    for (i, (a, b)) in cuts.iter().enumerate() {
        if let Some(f) = fit_alpha(sample, buffer, *a, *b) {
            fits.push((["low-half", "middle", "high-half"][i], f));
        }
    }
    if fits.is_empty() {
        return Subtraction {
            id: ids::sub_id(&sample.id, &buffer.id, "indeterminate"),
            sample_norm_id: sample.id.clone(),
            buffer_norm_id: buffer.id.clone(),
            alpha: f64::NAN,
            sigma_alpha: f64::NAN,
            scale_candidates: vec![],
            overlap_lo: lo,
            overlap_hi: hi,
            points: vec![],
            findings: Findings::ok().fail(
                "SCALE_INDETERMINATE",
                "fewer than two usable overlap points; background scale cannot be estimated",
            ),
        };
    }

    let mut candidates: Vec<ScaleCandidate> = Vec::new();
    for (i, (method, f)) in fits.iter().enumerate() {
        candidates.push(ScaleCandidate {
            id: ids::fnv1a(&[
                "scale",
                &sample.id,
                &buffer.id,
                method,
                &format!("{:.8}", f.alpha),
            ]),
            method: method.to_string(),
            alpha: f.alpha,
            sigma_alpha: f.sigma,
            q_lo: f.q_lo,
            q_hi: f.q_hi,
            points_used: f.used,
            chi2: f.chi2,
            adopted: i == 0,
            note: String::new(),
        });
    }
    let chosen = candidates.iter().find(|c| c.adopted).unwrap().clone();

    // If partial-window estimates disagree beyond their joint uncertainty, the
    // background scale is not uniquely determined: keep all, flag ambiguity.
    let alphas: Vec<f64> = candidates.iter().map(|c| c.alpha).collect();
    let spread = alphas.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        - alphas.iter().cloned().fold(f64::INFINITY, f64::min);
    let joint_sigma = candidates
        .iter()
        .map(|c| c.sigma_alpha)
        .fold(0.0f64, |a, s| a.hypot(s));
    if spread > 2.0 * joint_sigma.max(chosen.sigma_alpha) && candidates.len() > 1 {
        findings = findings.warn(
            "SCALE_AMBIGUOUS",
            format!(
                "background scale candidates span {:.3} beyond joint sigma {:.3}; all candidates retained",
                spread, joint_sigma
            ),
        );
    }

    // Build subtracted points on the overlap, interpolating the buffer only.
    let bq: Vec<f64> = buffer.points.iter().filter(|p| !p.masked && p.i.is_finite()).map(|p| p.q_nm).collect();
    let bi: Vec<f64> = buffer.points.iter().filter(|p| !p.masked && p.i.is_finite()).map(|p| p.i).collect();
    let mut points = Vec::new();
    let mut negatives = 0usize;
    for sp in sample.points.iter().filter(|p| usable(p) && p.q_nm >= lo && p.q_nm <= hi) {
        let (ib, blo, bhi, w) = interp(&bq, &bi, sp.q_nm);
        let i_sub = sp.i - chosen.alpha * ib;
        if i_sub < 0.0 {
            negatives += 1;
        }
        // variance: measurement/normalization term + background scale term.
        let var = sp.sigma.powi(2) + (ib * chosen.sigma_alpha).powi(2);
        let id = ids::point_id(
            &format!("sub-{}-{}", sample.id, buffer.id),
            sp.index,
            &format!("{:.8}", sp.q_nm),
        );
        points.push(SubPoint {
            id,
            index: sp.index,
            q_nm: sp.q_nm,
            i: i_sub,
            sigma: var.sqrt(),
            negative: i_sub < 0.0,
            sample_index: sp.index,
            buffer_lo: blo,
            buffer_hi: bhi,
            interp_weight: w,
        });
    }
    if negatives > 0 {
        findings = findings.warn(
            "NEGATIVE_POINTS",
            format!("{negatives} subtracted points are negative; retained without clipping"),
        );
    }
    let body = serde_json::to_string(&(&sample.id, &buffer.id, &points, chosen.alpha)).unwrap_or_default();
    Subtraction {
        id: ids::sub_id(&sample.id, &buffer.id, &body),
        sample_norm_id: sample.id.clone(),
        buffer_norm_id: buffer.id.clone(),
        alpha: chosen.alpha,
        sigma_alpha: chosen.sigma_alpha,
        scale_candidates: candidates,
        overlap_lo: lo,
        overlap_hi: hi,
        points,
        findings,
    }
}
