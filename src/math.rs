//! Scattering math. Encodes the protected semantics:
//!  - different q grids are never subtracted directly; interpolation applies
//!    only inside the overlap range;
//!  - errors carry normalisation AND background-scale uncertainty;
//!  - Guinier and Porod intervals are kept as candidates, never auto-merged.

use crate::model::*;

#[derive(Debug, Clone, PartialEq)]
pub enum SubError {
    NoOverlap { sample: (f64, f64), buffer: (f64, f64) },
    Empty,
}

/// I/(exposure*thickness*transmission); variance gains the relative
/// normalisation uncertainties in quadrature.
pub fn normalize(c: &Curve, p: &NormParams) -> Curve {
    let f = p.exposure_s * p.thickness_mm * p.transmission;
    let rel2 = p.exposure_rel_unc.powi(2)
        + p.thickness_rel_unc.powi(2)
        + p.transmission_rel_unc.powi(2);
    let q = c.q.clone();
    let i = c.i.iter().map(|v| v / f).collect::<Vec<_>>();
    let sigma = c
        .i
        .iter()
        .zip(c.sigma.iter())
        .map(|(v, s)| {
            let v = v / f;
            let s = s / f;
            (s * s + v * v * rel2).sqrt()
        })
        .collect();
    Curve { q, i, sigma, i_unit: c.i_unit.clone() }
}

pub fn range(xs: &[f64]) -> Option<(f64, f64)> {
    if xs.is_empty() {
        None
    } else {
        Some((xs.iter().cloned().fold(f64::INFINITY, f64::min),
              xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)))
    }
}

pub fn overlap(a: &[f64], b: &[f64]) -> Option<(f64, f64)> {
    let (a0, a1) = range(a)?;
    let (b0, b1) = range(b)?;
    let lo = a0.max(b0);
    let hi = a1.min(b1);
    if lo < hi { Some((lo, hi)) } else { None }
}

/// Linear interpolation; `None` outside the tabulated range (never extrapolate).
pub fn interp(xs: &[f64], ys: &[f64], x: f64) -> Option<(f64, (usize, usize))> {
    if xs.len() < 2 || x < xs[0] || x > xs[xs.len() - 1] {
        return None;
    }
    let j = match xs.binary_search_by(|v| v.partial_cmp(&x).unwrap()) {
        Ok(k) => return Some((ys[k], (k, k))),
        Err(k) => k - 1,
    };
    let (x0, x1) = (xs[j], xs[j + 1]);
    let t = if x1 > x0 { (x - x0) / (x1 - x0) } else { 0.0 };
    Some((ys[j] + t * (ys[j + 1] - ys[j]), (j, j + 1)))
}

fn grids_equal(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-12)
}

/// Background subtraction with scale and full error propagation.
/// sigma^2 = sigma_s^2 + scale^2 sigma_b^2 + (b * sigma_scale)^2
pub fn subtract(sample: &Curve, buffer: &Curve, scale: f64, scale_sigma: f64)
    -> Result<Subtraction, SubError>
{
    let ov = overlap(&sample.q, &buffer.q).ok_or(SubError::NoOverlap {
        sample: range(&sample.q).unwrap_or((0.0, 0.0)),
        buffer: range(&buffer.q).unwrap_or((0.0, 0.0)),
    })?;
    let direct = grids_equal(&sample.q, &buffer.q);
    let mut pts = Vec::new();
    for (idx, (&q, &si, &ss)) in sample.q.iter().zip(&sample.i).zip(&sample.sigma)
        .map(|((a, b), c)| (a, b, c)).enumerate()
    {
        if q < ov.0 - 1e-12 || q > ov.1 + 1e-12 {
            continue; // interpolation only inside the overlap
        }
        let (bi, bs, bracket) = if direct {
            (buffer.i[idx], buffer.sigma[idx], (idx, idx))
        } else {
            let bi = interp(&buffer.q, &buffer.i, q).ok_or(SubError::Empty)?;
            let bs = interp(&buffer.q, &buffer.sigma, q).ok_or(SubError::Empty)?.0;
            (bi.0, bs, bi.1)
        };
        let i = si - scale * bi;
        let sigma = (ss * ss + scale * scale * bs * bs + (bi * scale_sigma).powi(2)).sqrt();
        pts.push(SubPoint {
            q, i, sigma,
            sample_idx: idx, sample_i: si, sample_sigma: ss,
            buffer_i: bi, buffer_sigma: bs, buffer_bracket: bracket,
            residual: i, // residual vs zero after subtraction
        });
    }
    if pts.is_empty() {
        return Err(SubError::Empty);
    }
    Ok(Subtraction { scale, scale_sigma, overlap: ov, interpolated: !direct, points: pts })
}

/// Clipped (robust) chi^2 of `sample - scale*buffer` over the fit region.
fn clipped_loss(sample: &Curve, buffer: &Curve, scale: f64, qmin: f64) -> Option<f64> {
    let mut loss = 0.0;
    let mut n = 0;
    for (k, &q) in sample.q.iter().enumerate() {
        if q < qmin {
            continue;
        }
        let bi = interp(&buffer.q, &buffer.i, q)?.0;
        let s = sample.sigma[k].max(1e-9);
        let r = (sample.i[k] - scale * bi) / s;
        loss += r * r.min(9.0); // clip at 3 sigma -> non-convex, minima kept separate
        n += 1;
    }
    if n == 0 { None } else { Some(loss / n as f64) }
}

/// Scan background scale; every local minimum whose loss is within `delta` of
/// the best is kept as a legal candidate (no forced unique answer).
pub fn scale_candidates(sample: &Curve, buffer: &Curve, qmin: f64) -> Vec<ScaleCandidate> {
    let mut grid: Vec<(f64, f64)> = Vec::new();
    let mut k = 0;
    while k <= 100 {
        let s = 0.80 + 0.004 * k as f64;
        if let Some(l) = clipped_loss(sample, buffer, s, qmin) {
            grid.push((s, l));
        }
        k += 1;
    }
    let mut minima: Vec<(f64, f64)> = Vec::new();
    for w in 1..grid.len().saturating_sub(1) {
        let (s, l) = grid[w];
        if l <= grid[w - 1].1 && l <= grid[w + 1].1 {
            if minima.last().map_or(true, |&(ps, _)| (ps - s).abs() > 1e-9) {
                minima.push((s, l));
            }
        }
    }
    if minima.is_empty() {
        if let Some(best) = grid.iter().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()) {
            minima.push(*best);
        }
    }
    let best_loss = minima.iter().map(|m| m.1).fold(f64::INFINITY, f64::min);
    let delta = 0.15 * best_loss.abs().max(1e-9) + 0.05;
    let mut cands: Vec<ScaleCandidate> = minima
        .into_iter()
        .filter(|m| m.1 <= best_loss + delta)
        .map(|(s, l)| ScaleCandidate { scale: s, loss: l, rank: 0 })
        .collect();
    cands.sort_by(|a, b| a.loss.partial_cmp(&b.loss).unwrap());
    for (r, c) in cands.iter_mut().enumerate() {
        c.rank = r;
    }
    cands
}

fn linfit(xs: &[f64], ys: &[f64]) -> Option<(f64, f64, f64)> {
    let n = xs.len() as f64;
    if xs.len() < 3 {
        return None;
    }
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let d = n * sxx - sx * sx;
    if d.abs() < 1e-18 {
        return None;
    }
    let slope = (n * sxy - sx * sy) / d;
    let intercept = (sy - slope * sx) / n;
    let mean = sy / n;
    let ss_tot: f64 = ys.iter().map(|y| (y - mean).powi(2)).sum();
    let ss_res: f64 = xs.iter().zip(ys).map(|(x, y)| (y - intercept - slope * x).powi(2)).sum();
    let r2 = if ss_tot > 1e-18 { 1.0 - ss_res / ss_tot } else { 0.0 };
    Some((slope, intercept, r2))
}

/// Guinier interval candidates: sliding windows, ln I vs q^2.
/// Overlapping windows are all kept; nothing is merged.
pub fn guinier_candidates(c: &Curve) -> Vec<IntervalCandidate> {
    let mut out = Vec::new();
    let n = c.q.len();
    for start in 0..n.min(6) {
        for len in 6..=14usize {
            if start + len > n {
                break;
            }
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for k in start..start + len {
                if c.i[k] <= 0.0 {
                    break; // negative intensities stay in the data but cannot enter ln I
                }
                xs.push(c.q[k] * c.q[k]);
                ys.push(c.i[k].ln());
            }
            if xs.len() < 6 {
                continue;
            }
            if let Some((slope, intercept, r2)) = linfit(&xs, &ys) {
                let rg = (-3.0 * slope).max(0.0).sqrt();
                let qmax = c.q[start + xs.len() - 1];
                if r2 >= 0.985 && qmax * rg <= 1.35 && rg > 0.0 {
                    out.push(IntervalCandidate {
                        kind: "guinier".into(),
                        qmin: c.q[start],
                        qmax,
                        params: serde_json::json!({"rg_nm": rg, "i0": intercept.exp(), "r2": r2}),
                        quality: r2,
                    });
                }
            }
        }
    }
    out
}

/// Porod interval candidates: windows where q^4 I is flat (relative slope small).
pub fn porod_candidates(c: &Curve) -> Vec<IntervalCandidate> {
    let mut out = Vec::new();
    let n = c.q.len();
    for start in 0..n {
        if c.q[start] < 0.15 {
            continue;
        }
        for len in [10usize, 16, 22] {
            if start + len > n {
                break;
            }
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            let mut ok = true;
            for k in start..start + len {
                if c.i[k] <= 0.0 {
                    ok = false;
                    break;
                }
                xs.push(c.q[k]);
                ys.push(c.q[k].powi(4) * c.i[k]);
            }
            if !ok {
                continue;
            }
            if let Some((slope, _, _)) = linfit(&xs, &ys) {
                let mean: f64 = ys.iter().sum::<f64>() / ys.len() as f64;
                let span = xs[xs.len() - 1] - xs[0];
                let rel = (slope * span / mean).abs();
                if rel < 0.35 && mean > 0.0 {
                    out.push(IntervalCandidate {
                        kind: "porod".into(),
                        qmin: xs[0],
                        qmax: xs[xs.len() - 1],
                        params: serde_json::json!({"porod_const": mean, "rel_slope": rel}),
                        quality: 1.0 - rel,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(qs: &[f64], f: impl Fn(f64) -> f64) -> Curve {
        Curve {
            q: qs.to_vec(),
            i: qs.iter().map(|&q| f(q)).collect(),
            sigma: qs.iter().map(|_| 0.1).collect(),
            i_unit: "1/cm".into(),
        }
    }

    #[test]
    fn no_direct_subtract_on_grid_mismatch_and_overlap_only() {
        let s = curve(&[0.1, 0.2, 0.3, 0.4, 0.5], |q| 10.0 + q);
        let b = curve(&[0.15, 0.25, 0.35, 0.45], |q| 1.0 + q);
        let sub = subtract(&s, &b, 1.0, 0.01).unwrap();
        assert!(sub.interpolated);
        assert_eq!(sub.overlap, (0.15, 0.45));
        // sample points outside the overlap are excluded
        assert!(sub.points.iter().all(|p| p.q >= 0.15 && p.q <= 0.45));
        assert_eq!(sub.points.len(), 3); // q = 0.2, 0.3, 0.4
    }

    #[test]
    fn no_overlap_is_an_error_not_an_empty_curve() {
        let s = curve(&[0.1, 0.2], |q| q);
        let b = curve(&[0.7, 0.8], |q| q);
        assert!(matches!(subtract(&s, &b, 1.0, 0.0), Err(SubError::NoOverlap { .. })));
    }

    #[test]
    fn error_includes_scale_uncertainty() {
        let s = curve(&[0.1], |_| 5.0);
        let b = curve(&[0.1], |_| 2.0);
        let no_unc = subtract(&s, &b, 1.0, 0.0).unwrap();
        let with_unc = subtract(&s, &b, 1.0, 0.1).unwrap();
        assert!(with_unc.points[0].sigma > no_unc.points[0].sigma);
        let expect = (0.1f64.powi(2) + 0.1f64.powi(2) + (2.0 * 0.1f64).powi(2)).sqrt();
        assert!((with_unc.points[0].sigma - expect).abs() < 1e-12);
    }

    #[test]
    fn normalisation_uncertainty_propagates() {
        let c = curve(&[0.1], |_| 10.0);
        let p = NormParams {
            exposure_s: 2.0, thickness_mm: 1.0, transmission: 0.5,
            exposure_rel_unc: 0.0, thickness_rel_unc: 0.0, transmission_rel_unc: 0.1,
        };
        let n = normalize(&c, &p);
        assert!((n.i[0] - 10.0).abs() < 1e-9);
        let expect = ((0.1f64).powi(2) + (10.0 * 0.1f64).powi(2)).sqrt();
        assert!((n.sigma[0] - expect).abs() < 1e-9);
    }

    #[test]
    fn guinier_keeps_overlapping_candidates() {
        let qs: Vec<f64> = (0..40).map(|k| 0.02 + 0.008 * k as f64).collect();
        let c = curve(&qs, |q| 1200.0 * (-75.0 * q * q).exp() + 0.001);
        let cands = guinier_candidates(&c);
        assert!(cands.len() >= 2, "expected multiple overlapping candidates");
        let any_overlap = cands.iter().any(|a| {
            cands.iter().any(|b| {
                (a.qmin, a.qmax) != (b.qmin, b.qmax) && a.qmin < b.qmax && b.qmin < a.qmax
            })
        });
        assert!(any_overlap, "overlapping intervals must be preserved, not merged");
    }
}
