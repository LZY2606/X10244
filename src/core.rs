//! Core SAXS math: normalization, grid-aware background subtraction,
//! error propagation, Guinier/Porod interval fits.
//! Semantics enforced here:
//!  - mismatched q grids are never subtracted directly; interpolation only
//!    inside the overlapping q range; points outside are kept but excluded.
//!  - errors include normalization AND background-scale uncertainty.
//!  - Guinier and Porod intervals are never auto-merged into one model.
//!  - negative intensities are kept and flagged, never clamped.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawPoint {
    pub q: f64,
    pub i: f64,
    pub err: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurveMeta {
    pub exposure_s: f64,
    pub exposure_rel_unc: f64,
    pub thickness_mm: f64,
    pub thickness_rel_unc: f64,
    pub transmission: f64,
    pub transmission_rel_unc: f64,
    pub q_unit: String,
    pub i_unit: String,
    pub instrument_response_id: String,
    pub mask_id: String,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Curve {
    pub id: String,
    pub kind: String, // "sample" | "buffer"
    pub meta: CurveMeta,
    pub points: Vec<RawPoint>, // raw representation, preserved as imported
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum PointStatus {
    Ok,
    ExcludedNoOverlap,
    Negative,
    SkippedNonPositive,
}

impl PointStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PointStatus::Ok => "ok",
            PointStatus::ExcludedNoOverlap => "excluded_no_overlap",
            PointStatus::Negative => "negative",
            PointStatus::SkippedNonPositive => "skipped_non_positive",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "excluded_no_overlap" => PointStatus::ExcludedNoOverlap,
            "negative" => PointStatus::Negative,
            "skipped_non_positive" => PointStatus::SkippedNonPositive,
            _ => PointStatus::Ok,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DerivedPoint {
    pub q: f64,
    pub i: f64,
    pub err: f64,
    pub status: PointStatus,
    pub diag: String,
    /// provenance: which source points produced this point
    pub src: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Judgment {
    pub rule: String,
    pub detail: String,
}

/// Normalization: I/(exposure*thickness*transmission), error includes
/// counting error plus relative uncertainty of the normalization factor.
pub fn normalize(curve: &Curve) -> (Vec<DerivedPoint>, Vec<Judgment>) {
    let m = &curve.meta;
    let f = m.exposure_s * m.thickness_mm * m.transmission;
    let rel_f = (m.exposure_rel_unc.powi(2)
        + m.thickness_rel_unc.powi(2)
        + m.transmission_rel_unc.powi(2))
    .sqrt();
    let mut pts = Vec::with_capacity(curve.points.len());
    for (idx, p) in curve.points.iter().enumerate() {
        let i = p.i / f;
        let err = ((p.err / f).powi(2) + (i * rel_f).powi(2)).sqrt();
        pts.push(DerivedPoint {
            q: p.q,
            i,
            err,
            status: PointStatus::Ok,
            diag: String::new(),
            src: serde_json::json!({"curve": curve.id, "idx": idx}),
        });
    }
    let j = vec![Judgment {
        rule: "error_includes_normalization_uncertainty".into(),
        detail: format!(
            "factor={:.6} (exposure {} s, thickness {} mm, transmission {}), rel_unc={:.4}",
            f, m.exposure_s, m.thickness_mm, m.transmission, rel_f
        ),
    }];
    (pts, j)
}

fn grids_equal(a: &[DerivedPoint], b: &[DerivedPoint]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| (x.q - y.q).abs() < 1e-12)
}

/// Linear interpolation of buffer value/err at q, returning bracketing indices.
fn interp(buf: &[DerivedPoint], q: f64) -> Option<(f64, f64, usize, usize, f64)> {
    if q < buf.first()?.q || q > buf.last()?.q {
        return None;
    }
    let hi = buf.partition_point(|p| p.q < q);
    if hi < buf.len() && (buf[hi].q - q).abs() < 1e-12 {
        return Some((buf[hi].i, buf[hi].err, hi, hi, 0.0));
    }
    let lo = hi.checked_sub(1)?;
    if hi >= buf.len() {
        return None;
    }
    let (p0, p1) = (&buf[lo], &buf[hi]);
    let w = (q - p0.q) / (p1.q - p0.q);
    Some((p0.i + w * (p1.i - p0.i), p0.err + w * (p1.err - p0.err), lo, hi, w))
}

pub struct SubtractResult {
    pub points: Vec<DerivedPoint>,
    pub judgments: Vec<Judgment>,
    pub failed: bool,
}

/// Background subtraction with scale factor c +/- c_unc.
/// Never subtracts on mismatched grids directly; interpolates buffer onto the
/// sample grid, only inside the overlapping q range.
pub fn subtract(
    sample: &[DerivedPoint],
    buffer: &[DerivedPoint],
    sample_id: &str,
    buffer_id: &str,
    scale: f64,
    scale_unc: f64,
) -> SubtractResult {
    let mut judgments = Vec::new();
    if sample.is_empty() || buffer.is_empty() {
        judgments.push(Judgment {
            rule: "no_overlap_region".into(),
            detail: "empty input curve".into(),
        });
        return SubtractResult { points: vec![], judgments, failed: true };
    }
    let direct = grids_equal(sample, buffer);
    if direct {
        judgments.push(Judgment {
            rule: "q_grid_exact_match".into(),
            detail: "identical grids; direct point-by-point subtraction".into(),
        });
    } else {
        judgments.push(Judgment {
            rule: "q_grid_mismatch_no_direct_subtract".into(),
            detail: format!(
                "grids differ ({} vs {} points); buffer interpolated onto sample grid",
                sample.len(),
                buffer.len()
            ),
        });
    }
    let q_lo = sample.first().unwrap().q.max(buffer.first().unwrap().q);
    let q_hi = sample.last().unwrap().q.min(buffer.last().unwrap().q);
    if q_lo >= q_hi {
        judgments.push(Judgment {
            rule: "no_overlap_region".into(),
            detail: format!(
                "sample q in [{:.4},{:.4}], buffer q in [{:.4},{:.4}]: no overlap, subtraction refused",
                sample.first().unwrap().q, sample.last().unwrap().q,
                buffer.first().unwrap().q, buffer.last().unwrap().q
            ),
        });
        // keep all sample points, marked excluded, so nothing is silently dropped
        let points = sample
            .iter()
            .enumerate()
            .map(|(idx, p)| DerivedPoint {
                q: p.q,
                i: f64::NAN,
                err: f64::NAN,
                status: PointStatus::ExcludedNoOverlap,
                diag: "no overlapping q range with buffer".into(),
                src: serde_json::json!({"sample": sample_id, "sample_idx": idx, "buffer": buffer_id}),
            })
            .collect();
        return SubtractResult { points, judgments, failed: true };
    }
    if !direct {
        judgments.push(Judgment {
            rule: "interpolation_limited_to_overlap".into(),
            detail: format!("interpolation only inside overlapping q range [{:.4}, {:.4}]", q_lo, q_hi),
        });
    }
    judgments.push(Judgment {
        rule: "error_includes_background_scale_uncertainty".into(),
        detail: format!("scale={:.4} +/- {:.4}; sigma^2 = sigma_s^2 + c^2 sigma_b^2 + (I_b sigma_c)^2", scale, scale_unc),
    });
    let mut points = Vec::with_capacity(sample.len());
    let mut neg_count = 0usize;
    for (idx, s) in sample.iter().enumerate() {
        if s.q < q_lo || s.q > q_hi {
            points.push(DerivedPoint {
                q: s.q,
                i: f64::NAN,
                err: f64::NAN,
                status: PointStatus::ExcludedNoOverlap,
                diag: "outside overlapping q range; not interpolated".into(),
                src: serde_json::json!({"sample": sample_id, "sample_idx": idx, "buffer": buffer_id}),
            });
            continue;
        }
        let (bi, berr, lo, hi, w) = if direct {
            (buffer[idx].i, buffer[idx].err, idx, idx, 0.0)
        } else {
            match interp(buffer, s.q) {
                Some(v) => v,
                None => {
                    points.push(DerivedPoint {
                        q: s.q,
                        i: f64::NAN,
                        err: f64::NAN,
                        status: PointStatus::ExcludedNoOverlap,
                        diag: "no bracketing buffer points".into(),
                        src: serde_json::json!({"sample": sample_id, "sample_idx": idx, "buffer": buffer_id}),
                    });
                    continue;
                }
            }
        };
        let i = s.i - scale * bi;
        let err = (s.err.powi(2) + (scale * berr).powi(2) + (bi * scale_unc).powi(2)).sqrt();
        let (status, diag) = if i < 0.0 {
            neg_count += 1;
            (PointStatus::Negative, "negative intensity after subtraction; kept, not clamped".to_string())
        } else {
            (PointStatus::Ok, String::new())
        };
        points.push(DerivedPoint {
            q: s.q,
            i,
            err,
            status,
            diag,
            src: serde_json::json!({
                "sample": sample_id, "sample_idx": idx,
                "buffer": buffer_id, "buffer_lo": lo, "buffer_hi": hi, "weight": w,
                "scale": scale, "scale_unc": scale_unc
            }),
        });
    }
    if neg_count > 0 {
        judgments.push(Judgment {
            rule: "negative_intensity_kept".into(),
            detail: format!("{} points negative after subtraction; retained and flagged", neg_count),
        });
    }
    SubtractResult { points, judgments, failed: false }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FitResult {
    pub kind: String, // "guinier" | "porod"
    pub range: [f64; 2],
    pub params: serde_json::Value,
    /// per-input-point residuals (only for points used in the fit)
    pub residuals: Vec<(usize, f64)>,
    pub used: usize,
    pub skipped: usize,
    pub judgments: Vec<Judgment>,
}

fn wls(xs: &[f64], ys: &[f64], ws: &[f64]) -> (f64, f64) {
    let sw: f64 = ws.iter().sum();
    let sx: f64 = xs.iter().zip(ws).map(|(x, w)| x * w).sum();
    let sy: f64 = ys.iter().zip(ws).map(|(y, w)| y * w).sum();
    let sxx: f64 = xs.iter().zip(ws).map(|(x, w)| x * x * w).sum();
    let sxy: f64 = xs.iter().zip(ys).zip(ws).map(|((x, y), w)| x * y * w).sum();
    let d = sw * sxx - sx * sx;
    let a = (sy * sxx - sx * sxy) / d;
    let b = (sw * sxy - sx * sy) / d;
    (a, b)
}

/// Interval model fit. Guinier: ln I vs q^2. Porod: ln I vs ln q (slope ~ -4).
/// Ranges may overlap other fits; fits are never merged automatically.
pub fn fit_interval(kind: &str, points: &[DerivedPoint], range: [f64; 2]) -> FitResult {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut ws = Vec::new();
    let mut idxs = Vec::new();
    let mut skipped = 0usize;
    for (idx, p) in points.iter().enumerate() {
        if p.q < range[0] || p.q > range[1] || p.status == PointStatus::ExcludedNoOverlap {
            continue;
        }
        if p.i <= 0.0 {
            skipped += 1;
            continue; // ln undefined; point stays in the curve, just not in the fit
        }
        let (x, y, sy) = match kind {
            "guinier" => (p.q * p.q, p.i.ln(), p.err / p.i),
            _ => (p.q.ln(), p.i.ln(), p.err / p.i),
        };
        xs.push(x);
        ys.push(y);
        ws.push(1.0 / (sy * sy).max(1e-12));
        idxs.push(idx);
    }
    let mut judgments = Vec::new();
    if xs.len() < 3 {
        judgments.push(Judgment {
            rule: "fit_underdetermined".into(),
            detail: format!("only {} usable points in range [{:.3},{:.3}]", xs.len(), range[0], range[1]),
        });
        return FitResult {
            kind: kind.into(),
            range,
            params: serde_json::json!({"error": "underdetermined"}),
            residuals: vec![],
            used: xs.len(),
            skipped,
            judgments,
        };
    }
    let (a, b) = wls(&xs, &ys, &ws);
    let residuals: Vec<(usize, f64)> = idxs
        .iter()
        .enumerate()
        .map(|(k, &idx)| (idx, ys[k] - (a + b * xs[k])))
        .collect();
    let chi2: f64 = residuals
        .iter()
        .enumerate()
        .map(|(k, (_, r))| r * r * ws[k])
        .sum::<f64>();
    let dof = (xs.len() - 2).max(1) as f64;
    let params = if kind == "guinier" {
        let rg = if b < 0.0 { (-3.0 * b).sqrt() } else { f64::NAN };
        serde_json::json!({
            "Rg_nm": rg, "I0": a.exp(), "slope": b, "intercept": a,
            "chi2_per_dof": chi2 / dof,
            "qRg_max": range[1] * rg,
        })
    } else {
        serde_json::json!({
            "porod_slope": b, "logI_intercept": a,
            "chi2_per_dof": chi2 / dof,
            "slope_near_minus4": (b + 4.0).abs() < 0.5,
        })
    };
    if skipped > 0 {
        judgments.push(Judgment {
            rule: "non_positive_skipped_in_log_fit".into(),
            detail: format!("{} non-positive points skipped in {} fit (ln undefined), kept in curve", skipped, kind),
        });
    }
    FitResult { kind: kind.into(), range, params, residuals, used: xs.len(), skipped, judgments }
}

/// Residual statistics of a candidate model against the curve it was fit on.
pub fn residual_stats(fit: &FitResult) -> serde_json::Value {
    let n = fit.residuals.len().max(1) as f64;
    let mean_abs: f64 = fit.residuals.iter().map(|(_, r)| r.abs()).sum::<f64>() / n;
    let max_abs: f64 = fit.residuals.iter().map(|(_, r)| r.abs()).fold(0.0, f64::max);
    serde_json::json!({"mean_abs": mean_abs, "max_abs": max_abs, "used": fit.used})
}
