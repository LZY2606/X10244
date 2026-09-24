//! Core SAXS science rules.
//!
//! Enforced semantics:
//!  * q grids that differ are never subtracted directly; interpolation only
//!    happens inside the overlapping q range.
//!  * Uncertainties include normalization (transmission/thickness) and
//!    background-scaling uncertainty.
//!  * Guinier and Porod intervals are fitted independently and are never
//!    merged into a single model automatically.
//!  * Negative intensities are retained and flagged, never silently dropped.

use serde::{Deserialize, Serialize};

pub const CANONICAL_Q_UNIT: &str = "1/angstrom";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub err: Vec<f64>,
    pub unit_q: String,
    pub unit_i: String,
}

impl Curve {
    pub fn len(&self) -> usize {
        self.q.len()
    }
    pub fn q_range(&self) -> (f64, f64) {
        (
            self.q.first().copied().unwrap_or(0.0),
            self.q.last().copied().unwrap_or(0.0),
        )
    }
    pub fn negative_indices(&self) -> Vec<usize> {
        self.i
            .iter()
            .enumerate()
            .filter(|(_, v)| **v < 0.0)
            .map(|(k, _)| k)
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub indices: Vec<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormFactors {
    pub transmission: f64,
    pub transmission_rel_err: f64,
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub thickness_rel_err: f64,
}

impl NormFactors {
    pub fn factor(&self) -> f64 {
        self.transmission * self.exposure_s * self.thickness_mm
    }
    /// Relative normalization uncertainty (transmission + thickness terms).
    pub fn rel_err(&self) -> f64 {
        (self.transmission_rel_err.powi(2) + self.thickness_rel_err.powi(2)).sqrt()
    }
}

/// Convert q unit at the derivation layer. Raw imports are never rewritten;
/// unconvertible units stay in place and produce a diagnostic instead.
pub fn convert_q_unit(curve: &Curve, target: &str) -> Result<Curve, Diagnostic> {
    if curve.unit_q == target {
        return Ok(curve.clone());
    }
    let factor = match (curve.unit_q.as_str(), target) {
        ("1/nm", "1/angstrom") => 0.1,
        ("1/angstrom", "1/nm") => 10.0,
        _ => {
            return Err(Diagnostic {
                code: "unit.unconvertible".into(),
                message: format!(
                    "cannot convert q unit '{}' to '{}'; value left in original position",
                    curve.unit_q, target
                ),
                indices: vec![],
            })
        }
    };
    let mut out = curve.clone();
    for q in out.q.iter_mut() {
        *q *= factor;
    }
    out.unit_q = target.to_string();
    Ok(out)
}

/// I_norm = I / (T * exposure * thickness); uncertainty gains the
/// normalization relative-uncertainty term in quadrature.
pub fn normalize(curve: &Curve, f: &NormFactors) -> (Curve, Vec<Diagnostic>) {
    let factor = f.factor();
    let rel = f.rel_err();
    let mut out = curve.clone();
    for k in 0..curve.len() {
        let i_norm = curve.i[k] / factor;
        let stat = curve.err[k] / factor;
        let norm_term = i_norm.abs() * rel;
        out.i[k] = i_norm;
        out.err[k] = (stat * stat + norm_term * norm_term).sqrt();
    }
    let mut diags = vec![Diagnostic {
        code: "norm.applied".into(),
        message: format!(
            "normalized by T*exposure*thickness = {:.6e}; normalization rel. uncertainty {:.4} folded into err",
            factor, rel
        ),
        indices: vec![],
    }];
    let negs = curve.negative_indices();
    if !negs.is_empty() {
        diags.push(Diagnostic {
            code: "intensity.negative".into(),
            message: format!(
                "{} negative intensity point(s) retained (never discarded)",
                negs.len()
            ),
            indices: negs,
        });
    }
    (out, diags)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlignedPoint {
    pub q: f64,
    pub sample_q: f64,
    pub sample_i: f64,
    pub sample_err: f64,
    pub buffer_q: f64,
    pub buffer_i: f64,
    pub buffer_err: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoOverlap {
    pub sample_range: (f64, f64),
    pub buffer_range: (f64, f64),
    pub message: String,
}

fn interp(q: &[f64], v: &[f64], x: f64) -> Option<f64> {
    if q.is_empty() || x < q[0] || x > q[q.len() - 1] {
        return None;
    }
    match q.binary_search_by(|a| a.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal)) {
        Ok(k) => Some(v[k]),
        Err(k) => {
            if k == 0 || k >= q.len() {
                return None;
            }
            let (a, b) = (q[k - 1], q[k]);
            if (b - a).abs() < 1e-15 {
                return Some(v[k]);
            }
            let t = (x - a) / (b - a);
            Some(v[k - 1] + t * (v[k] - v[k - 1]))
        }
    }
}

/// Align sample and buffer on the sample grid, restricted to the overlapping
/// q range. Points outside the overlap are dropped; if no usable overlap
/// exists the caller gets a `NoOverlap` diagnostic instead of a subtraction.
pub fn align(sample: &Curve, buffer: &Curve) -> Result<Vec<AlignedPoint>, NoOverlap> {
    let (s_lo, s_hi) = sample.q_range();
    let (b_lo, b_hi) = buffer.q_range();
    let lo = s_lo.max(b_lo);
    let hi = s_hi.min(b_hi);
    let no_overlap = || NoOverlap {
        sample_range: (s_lo, s_hi),
        buffer_range: (b_lo, b_hi),
        message: format!(
            "no usable q overlap between sample [{:.4}, {:.4}] and buffer [{:.4}, {:.4}]; subtraction refused (rule: grid.no_direct_subtract)",
            s_lo, s_hi, b_lo, b_hi
        ),
    };
    if !(lo < hi) {
        return Err(no_overlap());
    }
    let mut pts = Vec::new();
    for k in 0..sample.len() {
        let q = sample.q[k];
        if q < lo || q > hi {
            continue;
        }
        match (interp(&buffer.q, &buffer.i, q), interp(&buffer.q, &buffer.err, q)) {
            (Some(bi), Some(be)) => pts.push(AlignedPoint {
                q,
                sample_q: q,
                sample_i: sample.i[k],
                sample_err: sample.err[k],
                buffer_q: q,
                buffer_i: bi,
                buffer_err: be,
            }),
            _ => continue,
        }
    }
    if pts.len() < 3 {
        return Err(no_overlap());
    }
    Ok(pts)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SrcPoint {
    pub q: f64,
    pub sample_i: f64,
    pub sample_err: f64,
    pub buffer_i: f64,
    pub buffer_err: f64,
    pub scale: f64,
}

/// I_sub = I_s - scale * I_b with
/// var = err_s^2 + (scale*err_b)^2 + (I_b*scale_err)^2
/// so the background-scaling uncertainty is part of every output error bar.
pub fn subtract(pts: &[AlignedPoint], scale: f64, scale_err: f64) -> (Curve, Vec<SrcPoint>) {
    let mut q = Vec::with_capacity(pts.len());
    let mut ii = Vec::with_capacity(pts.len());
    let mut ee = Vec::with_capacity(pts.len());
    let mut src = Vec::with_capacity(pts.len());
    for p in pts {
        let val = p.sample_i - scale * p.buffer_i;
        let var = p.sample_err.powi(2)
            + (scale * p.buffer_err).powi(2)
            + (p.buffer_i * scale_err).powi(2);
        q.push(p.q);
        ii.push(val);
        ee.push(var.sqrt());
        src.push(SrcPoint {
            q: p.q,
            sample_i: p.sample_i,
            sample_err: p.sample_err,
            buffer_i: p.buffer_i,
            buffer_err: p.buffer_err,
            scale,
        });
    }
    (
        Curve {
            q,
            i: ii,
            err: ee,
            unit_q: CANONICAL_Q_UNIT.into(),
            unit_i: "arb".into(),
        },
        src,
    )
}

/// Weighted least squares: returns (intercept, slope, intercept_err,
/// slope_err, rms_residual) or None when the fit is underdetermined.
fn wls(x: &[f64], y: &[f64], w: &[f64]) -> Option<(f64, f64, f64, f64, f64)> {
    if x.len() < 3 {
        return None;
    }
    let sw: f64 = w.iter().sum();
    let sx: f64 = w.iter().zip(x).map(|(w, x)| w * x).sum();
    let sy: f64 = w.iter().zip(y).map(|(w, y)| w * y).sum();
    let sxx: f64 = w.iter().zip(x).map(|(w, x)| w * x * x).sum();
    let sxy: f64 = w.iter().zip(x.zip(y)).map(|(w, (x, y))| w * x * y).sum();
    let delta = sw * sxx - sx * sx;
    if delta.abs() < 1e-30 || sw <= 0.0 {
        return None;
    }
    let intercept = (sxx * sy - sx * sxy) / delta;
    let slope = (sw * sxy - sx * sy) / delta;
    let intercept_err = (sxx / delta).sqrt();
    let slope_err = (sw / delta).sqrt();
    let rms = (w
        .iter()
        .zip(x.zip(y))
        .map(|(w, (x, y))| {
            let r = y - (intercept + slope * x);
            w * r * r
        })
        .sum::<f64>()
        / sw)
        .sqrt();
    Some((intercept, slope, intercept_err, slope_err, rms))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuinierFit {
    pub q_min: f64,
    pub q_max: f64,
    pub rg: f64,
    pub rg_err: f64,
    pub i0: f64,
    pub i0_err: f64,
    pub residual_rms: f64,
    pub n_points: usize,
    pub skipped_nonpositive: usize,
}

/// Guinier fit on [q_min, q_max]: ln I = ln I0 - Rg^2 q^2 / 3, weighted.
/// Non-positive intensities are skipped for the log but reported.
pub fn guinier(curve: &Curve, q_min: f64, q_max: f64) -> Option<GuinierFit> {
    let (mut x, mut y, mut w) = (Vec::new(), Vec::new(), Vec::new());
    let mut skipped = 0usize;
    for k in 0..curve.len() {
        let q = curve.q[k];
        if q < q_min || q > q_max {
            continue;
        }
        let (iv, ev) = (curve.i[k], curve.err[k]);
        if iv > 0.0 && ev > 0.0 {
            x.push(q * q);
            y.push(iv.ln());
            w.push((iv / ev).powi(2));
        } else {
            skipped += 1;
        }
    }
    let n = x.len();
    let (a, b, ae, be, rms) = wls(&x, &y, &w)?;
    if b >= 0.0 {
        return None; // not a decaying Guinier region
    }
    let rg = (-3.0 * b).sqrt();
    Some(GuinierFit {
        q_min,
        q_max,
        rg,
        rg_err: 1.5 * be / rg,
        i0: a.exp(),
        i0_err: a.exp() * ae,
        residual_rms: rms,
        n_points: n,
        skipped_nonpositive: skipped,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PorodFit {
    pub q_min: f64,
    pub q_max: f64,
    pub exponent: f64,
    pub exponent_err: f64,
    pub residual_rms: f64,
    pub n_points: usize,
    pub skipped_nonpositive: usize,
}

/// Porod fit on [q_min, q_max]: ln I vs ln q, slope = Porod exponent.
pub fn porod(curve: &Curve, q_min: f64, q_max: f64) -> Option<PorodFit> {
    let (mut x, mut y, mut w) = (Vec::new(), Vec::new(), Vec::new());
    let mut skipped = 0usize;
    for k in 0..curve.len() {
        let q = curve.q[k];
        if q < q_min || q > q_max || q <= 0.0 {
            continue;
        }
        let (iv, ev) = (curve.i[k], curve.err[k]);
        if iv > 0.0 && ev > 0.0 {
            x.push(q.ln());
            y.push(iv.ln());
            w.push((iv / ev).powi(2));
        } else {
            skipped += 1;
        }
    }
    let n = x.len();
    let (_, slope, _, slope_err, rms) = wls(&x, &y, &w)?;
    Some(PorodFit {
        q_min,
        q_max,
        exponent: slope,
        exponent_err: slope_err,
        residual_rms: rms,
        n_points: n,
        skipped_nonpositive: skipped,
    })
}
