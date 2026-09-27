//! Core SAXS rules. Pure functions; all domain quantities carry units in
//! `CurveData` and every judgment returns machine-readable diagnostics.
use crate::model::{CurveData, CurvePoint, Diagnostic};

pub const RULE_IMPORT: &str = "import.v1";
pub const RULE_NORMALIZE: &str = "normalize.v1";
pub const RULE_SUBTRACT: &str = "subtract.v1";
pub const RULE_INTERVALS: &str = "intervals.v1";
pub const RULE_ASSESS: &str = "assess.v1";

#[derive(Clone, Debug, PartialEq)]
pub enum GridCheck {
    Identical,
    /// Grids differ; interpolation is allowed only inside this overlap.
    Misaligned { overlap: (f64, f64) },
    /// No shared q range: subtraction must not happen.
    Disjoint,
}

pub fn check_grid(a: &CurveData, b: &CurveData) -> GridCheck {
    let (a0, a1) = match a.q_range() { Some(r) => r, None => return GridCheck::Disjoint };
    let (b0, b1) = match b.q_range() { Some(r) => r, None => return GridCheck::Disjoint };
    let lo = a0.max(b0);
    let hi = a1.min(b1);
    if lo >= hi {
        return GridCheck::Disjoint;
    }
    let same_len = a.points.len() == b.points.len();
    let same_grid = same_len
        && a.points.iter().zip(b.points.iter()).all(|(x, y)| (x.q - y.q).abs() < 1e-9);
    if same_grid {
        GridCheck::Identical
    } else {
        GridCheck::Misaligned { overlap: (lo, hi) }
    }
}

/// Normalization by exposure * thickness * transmission. Uncertainty budget
/// includes the relative uncertainties of every normalization factor.
pub fn normalize(
    curve: &CurveData,
    exposure_s: f64,
    thickness_mm: f64,
    transmission: f64,
    u_exposure: f64,
    u_thickness: f64,
    u_transmission: f64,
) -> (CurveData, Vec<Diagnostic>) {
    let factor = exposure_s * thickness_mm * transmission;
    let rel = (u_exposure * u_exposure + u_thickness * u_thickness + u_transmission * u_transmission).sqrt();
    let mut diags = vec![Diagnostic::new(
        "normalization.uncertainty",
        format!(
            "归一化因子={:.6} (exposure={}s, thickness={}mm, transmission={}), 相对不确定度={:.4}",
            factor, exposure_s, thickness_mm, transmission, rel
        ),
    )];
    let mut pts = Vec::with_capacity(curve.points.len());
    for p in &curve.points {
        let i = p.i / factor;
        let rel_stat = if p.i.abs() > 1e-12 { p.err / p.i.abs() } else { 0.0 };
        let err = i.abs() * (rel_stat * rel_stat + rel * rel).sqrt();
        if p.i < 0.0 {
            diags.push(Diagnostic::at(
                "negative_intensity.raw",
                format!("原始强度为负 ({:.4})，保留该点并标记", p.i),
                p.q,
            ));
        }
        pts.push(CurvePoint { q: p.q, i, err });
    }
    (
        CurveData { unit_q: curve.unit_q.clone(), unit_i: curve.unit_i.clone(), points: pts },
        diags,
    )
}

fn interp(points: &[CurvePoint], q: f64) -> Option<(f64, f64)> {
    if q < points.first()?.q || q > points.last()?.q {
        return None;
    }
    let idx = points.partition_point(|p| p.q < q);
    if idx == 0 {
        return Some((points[0].i, points[0].err));
    }
    let (p0, p1) = (&points[idx - 1], &points[idx]);
    let w = if (p1.q - p0.q).abs() < 1e-15 { 0.0 } else { (q - p0.q) / (p1.q - p0.q) };
    Some((p0.i + w * (p1.i - p0.i), p0.err + w * (p1.err - p0.err)))
}

#[derive(Clone, Debug)]
pub struct Subtraction {
    pub curve: CurveData,
    pub diagnostics: Vec<Diagnostic>,
    /// true when grids were identical and no interpolation was needed.
    pub direct: bool,
    pub overlap: (f64, f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum SubtractError {
    /// q grids share no overlap: direct subtraction and interpolation are
    /// both refused; inputs must be preserved and judged by a human.
    NoOverlap,
}

/// Background subtraction. Semantics:
/// - different q grids are never subtracted pointwise;
/// - interpolation happens only inside the overlap range;
/// - the error budget includes the background scaling uncertainty;
/// - negative intensities are kept and flagged, never silently dropped.
pub fn subtract(
    sample: &CurveData,
    buffer: &CurveData,
    scale: f64,
    scale_rel_unc: f64,
) -> Result<Subtraction, SubtractError> {
    let grid = check_grid(sample, buffer);
    let overlap = match &grid {
        GridCheck::Disjoint => return Err(SubtractError::NoOverlap),
        GridCheck::Identical => sample.q_range().unwrap(),
        GridCheck::Misaligned { overlap } => *overlap,
    };
    let mut diags = Vec::new();
    match &grid {
        GridCheck::Identical => {}
        GridCheck::Misaligned { overlap } => diags.push(Diagnostic::new(
            "grid.misaligned",
            format!(
                "q 网格不同，不直接相减；仅在重叠区 [{:.4}, {:.4}] {} 内插值",
                overlap.0, overlap.1, sample.unit_q
            ),
        )),
        GridCheck::Disjoint => unreachable!(),
    }
    diags.push(Diagnostic::new(
        "background.scale_uncertainty",
        format!("背景缩放={:.4}，相对不确定度={:.4} 已计入误差", scale, scale_rel_unc),
    ));
    let mut pts = Vec::new();
    for p in &sample.points {
        if p.q < overlap.0 - 1e-12 || p.q > overlap.1 + 1e-12 {
            continue; // outside overlap: no subtraction is attempted
        }
        let (bi, berr) = match interp(&buffer.points, p.q) {
            Some(v) => v,
            None => continue,
        };
        let i = p.i - scale * bi;
        // err^2 = sample^2 + (scale*buffer_err)^2 + (scale_unc*buffer)^2
        let err = (p.err * p.err
            + (scale * berr) * (scale * berr)
            + (scale_rel_unc * scale * bi) * (scale_rel_unc * scale * bi))
            .sqrt();
        if i < 0.0 {
            diags.push(Diagnostic::at(
                "negative_intensity.subtracted",
                format!("扣背景后强度为负 ({:.4})，保留该点并标记", i),
                p.q,
            ));
        }
        pts.push(CurvePoint { q: p.q, i, err });
    }
    Ok(Subtraction {
        curve: CurveData { unit_q: sample.unit_q.clone(), unit_i: sample.unit_i.clone(), points: pts },
        diagnostics: diags,
        direct: matches!(grid, GridCheck::Identical),
        overlap,
    })
}

fn linfit(xs: &[f64], ys: &[f64], ws: &[f64]) -> Option<(f64, f64)> {
    let n = xs.len();
    if n < 2 {
        return None;
    }
    let sw: f64 = ws.iter().sum();
    let sx: f64 = xs.iter().zip(ws).map(|(x, w)| x * w).sum();
    let sy: f64 = ys.iter().zip(ws).map(|(y, w)| y * w).sum();
    let sxx: f64 = xs.iter().zip(ws).map(|(x, w)| x * x * w).sum();
    let sxy: f64 = xs.iter().zip(ys).zip(ws).map(|((x, y), w)| x * y * w).sum();
    let det = sw * sxx - sx * sx;
    if det.abs() < 1e-30 {
        return None;
    }
    let slope = (sw * sxy - sx * sy) / det;
    let intercept = (sy - slope * sx) / sw;
    Some((slope, intercept))
}

#[derive(Clone, Debug)]
pub struct IntervalFit {
    pub kind: String,
    pub qmin: f64,
    pub qmax: f64,
    pub params: serde_json::Value,
    pub residuals: Vec<(f64, f64)>,
    pub notes: Vec<String>,
}

/// Guinier fit: ln I vs q^2 over [qmin, qmax]. Points with I<=0 are excluded
/// from the log fit but reported.
pub fn guinier(curve: &CurveData, qmin: f64, qmax: f64) -> Option<IntervalFit> {
    let mut xs = vec![];
    let mut ys = vec![];
    let mut ws = vec![];
    let mut notes = vec![];
    for p in &curve.points {
        if p.q < qmin || p.q > qmax {
            continue;
        }
        if p.i <= 0.0 {
            notes.push(format!("q={:.4} 强度非正，Guinier 对数拟合排除该点", p.q));
            continue;
        }
        xs.push(p.q * p.q);
        ys.push(p.i.ln());
        let w = if p.err > 0.0 { p.i / p.err } else { 1.0 };
        ws.push(w * w);
    }
    let (slope, intercept) = linfit(&xs, &ys, &ws)?;
    let rg = (-3.0 * slope).max(0.0).sqrt();
    let i0 = intercept.exp();
    let qmax_rg = qmax * rg;
    let mut notes2 = notes.clone();
    notes2.push(if qmax_rg <= 1.3 {
        format!("qmax*Rg={:.3} <= 1.3，Guinier 区间有效", qmax_rg)
    } else {
        format!("qmax*Rg={:.3} > 1.3，Guinier 区间存疑，保留为候选", qmax_rg)
    });
    let residuals = xs
        .iter()
        .zip(ys.iter())
        .map(|(x, y)| (x.sqrt(), y - (slope * x + intercept)))
        .collect();
    Some(IntervalFit {
        kind: "guinier".into(),
        qmin,
        qmax,
        params: serde_json::json!({"Rg": rg, "I0": i0, "unit_q": curve.unit_q}),
        residuals,
        notes: notes2,
    })
}

/// Porod fit: ln I vs ln q slope over [qmin, qmax].
pub fn porod(curve: &CurveData, qmin: f64, qmax: f64) -> Option<IntervalFit> {
    let mut xs = vec![];
    let mut ys = vec![];
    let mut ws = vec![];
    let mut notes = vec![];
    for p in &curve.points {
        if p.q < qmin || p.q > qmax {
            continue;
        }
        if p.i <= 0.0 {
            notes.push(format!("q={:.4} 强度非正，Porod 对数拟合排除该点", p.q));
            continue;
        }
        xs.push(p.q.ln());
        ys.push(p.i.ln());
        ws.push(1.0);
    }
    let (slope, intercept) = linfit(&xs, &ys, &ws)?;
    let residuals = xs
        .iter()
        .zip(ys.iter())
        .map(|(x, y)| (x.exp(), y - (slope * x + intercept)))
        .collect();
    Some(IntervalFit {
        kind: "porod".into(),
        qmin,
        qmax,
        params: serde_json::json!({"porod_exponent": slope, "log_scale": intercept, "unit_q": curve.unit_q}),
        residuals,
        notes,
    })
}

/// Residual metric of a subtracted curve against zero in a high-q window,
/// used to rank buffer candidates. All candidates are kept; this only
/// produces evidence, never an automatic decision.
pub fn residual_metric(curve: &CurveData, qmin: f64) -> serde_json::Value {
    let mut chi2 = 0.0;
    let mut n = 0usize;
    let mut max_abs = 0.0f64;
    for p in &curve.points {
        if p.q < qmin {
            continue;
        }
        let r = p.i / p.err.max(1e-12);
        chi2 += r * r;
        max_abs = max_abs.max(r.abs());
        n += 1;
    }
    serde_json::json!({
        "window_qmin": qmin,
        "n_points": n,
        "chi2": chi2,
        "reduced_chi2": if n > 0 { chi2 / n as f64 } else { f64::NAN },
        "max_abs_residual_sigma": max_abs,
    })
}
