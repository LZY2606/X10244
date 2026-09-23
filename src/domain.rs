//! SAXS domain rules: units, curves, normalization, background subtraction,
//! Guinier/Porod interval fits. All quantities carry units or a coordinate
//! basis; raw representations are preserved by the import layer and only the
//! derived layer normalizes to canonical units (q in 1/angstrom, I in 1/cm).

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Unit {
    QAngstrom,      // 1/angstrom
    QNm,            // 1/nm
    IPerCm,         // 1/cm
    IPerMm,         // 1/mm
    Counts,         // detector counts, no absolute calibration
    Seconds,
    Cm,
    Mm,
    Dimensionless,
    Unknown(String),
}

impl Unit {
    pub fn parse(s: &str) -> Unit {
        match s {
            "1/angstrom" | "A^-1" | "angstrom^-1" => Unit::QAngstrom,
            "1/nm" | "nm^-1" => Unit::QNm,
            "1/cm" | "cm^-1" => Unit::IPerCm,
            "1/mm" | "mm^-1" => Unit::IPerMm,
            "counts" => Unit::Counts,
            "s" | "seconds" => Unit::Seconds,
            "cm" => Unit::Cm,
            "mm" => Unit::Mm,
            "1" | "dimensionless" | "" => Unit::Dimensionless,
            other => Unit::Unknown(other.to_string()),
        }
    }
    pub fn label(&self) -> String {
        match self {
            Unit::QAngstrom => "1/angstrom".into(),
            Unit::QNm => "1/nm".into(),
            Unit::IPerCm => "1/cm".into(),
            Unit::IPerMm => "1/mm".into(),
            Unit::Counts => "counts".into(),
            Unit::Seconds => "s".into(),
            Unit::Cm => "cm".into(),
            Unit::Mm => "mm".into(),
            Unit::Dimensionless => "1".into(),
            Unit::Unknown(s) => format!("unknown({s})"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub indices: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Curve {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub sigma: Vec<f64>,
    pub q_unit: Unit,
    pub i_unit: Unit,
}

impl Curve {
    pub fn len(&self) -> usize { self.q.len() }
}

/// Convert a curve to canonical units (q: 1/angstrom, I: 1/cm).
/// Values whose unit cannot be converted stay in place (raw) and a
/// diagnostic is attached; nothing is dropped for the sake of plotting.
pub fn to_canonical(c: &Curve) -> (Option<Curve>, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let qf = match c.q_unit {
        Unit::QAngstrom => Some(1.0),
        Unit::QNm => Some(0.1),
        _ => None,
    };
    let ifactor = match c.i_unit {
        Unit::IPerCm => Some(1.0),
        Unit::IPerMm => Some(0.1),
        _ => None,
    };
    if qf.is_none() {
        diags.push(Diagnostic {
            code: "unit_unconvertible_q".into(),
            message: format!("q unit '{}' has no conversion to 1/angstrom; values kept raw", c.q_unit.label()),
            indices: vec![],
        });
    }
    if ifactor.is_none() {
        diags.push(Diagnostic {
            code: "unit_unconvertible_i".into(),
            message: format!("I unit '{}' has no conversion to 1/cm; values kept raw", c.i_unit.label()),
            indices: vec![],
        });
    }
    match (qf, ifactor) {
        (Some(qf), Some(ifactor)) => (
            Some(Curve {
                q: c.q.iter().map(|v| v * qf).collect(),
                i: c.i.iter().map(|v| v * ifactor).collect(),
                sigma: c.sigma.iter().map(|v| v * ifactor).collect(),
                q_unit: Unit::QAngstrom,
                i_unit: Unit::IPerCm,
            }),
            diags,
        ),
        _ => (None, diags),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormFactors {
    pub exposure_s: f64,
    pub thickness_cm: f64,
    pub transmission: f64,
    pub rel_unc_exposure: f64,
    pub rel_unc_thickness: f64,
    pub rel_unc_transmission: f64,
}

/// I_norm = I / (exposure * thickness * transmission).
/// Sigma includes counting error AND normalization uncertainty of the
/// exposure/thickness/transmission factors (propagated in quadrature).
pub fn normalize(c: &Curve, f: &NormFactors) -> (Curve, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let denom = f.exposure_s * f.thickness_cm * f.transmission;
    if denom <= 0.0 {
        diags.push(Diagnostic {
            code: "norm_nonpositive_denom".into(),
            message: format!("normalization denominator {denom} <= 0; curve kept unnormalized"),
            indices: vec![],
        });
        return (c.clone(), diags);
    }
    let rel2 = f.rel_unc_exposure.powi(2)
        + f.rel_unc_thickness.powi(2)
        + f.rel_unc_transmission.powi(2);
    let q = c.q.clone();
    let mut i_out = Vec::with_capacity(c.len());
    let mut s_out = Vec::with_capacity(c.len());
    for k in 0..c.len() {
        let iv = c.i[k] / denom;
        let var = (c.sigma[k] / denom).powi(2) + iv * iv * rel2;
        i_out.push(iv);
        s_out.push(var.sqrt());
    }
    diags.push(Diagnostic {
        code: "norm_uncertainty_included".into(),
        message: format!("sigma includes normalization uncertainty (rel^2 sum = {rel2:.6})"),
        indices: vec![],
    });
    (
        Curve { q, i: i_out, sigma: s_out, q_unit: c.q_unit.clone(), i_unit: c.i_unit.clone() },
        diags,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubPoint {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub sample_idx: usize,
    pub buffer_i: f64,
    pub buffer_sigma: f64,
    pub interpolated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubResult {
    pub points: Vec<SubPoint>,
    pub overlap: [f64; 2],
    pub interpolated: bool,
    pub scale: f64,
    pub scale_sigma: f64,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubError {
    NoOverlap { sample_range: [f64; 2], buffer_range: [f64; 2] },
}

/// Background subtraction.
/// Rule: when q grids differ we never subtract point-by-point directly;
/// the buffer is linearly interpolated onto the sample grid, restricted to
/// the overlapping q range only. Sigma includes counting errors of both
/// curves plus the background scale uncertainty: (I_b * sigma_scale)^2.
pub fn subtract(sample: &Curve, buffer: &Curve, scale: f64, scale_sigma: f64) -> Result<SubResult, SubError> {
    let s_lo = sample.q.iter().cloned().fold(f64::INFINITY, f64::min);
    let s_hi = sample.q.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let b_lo = buffer.q.iter().cloned().fold(f64::INFINITY, f64::min);
    let b_hi = buffer.q.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let lo = s_lo.max(b_lo);
    let hi = s_hi.min(b_hi);
    if !(lo < hi) {
        return Err(SubError::NoOverlap { sample_range: [s_lo, s_hi], buffer_range: [b_lo, b_hi] });
    }
    let grids_equal = sample.q.len() == buffer.q.len()
        && sample.q.iter().zip(buffer.q.iter()).all(|(a, b)| (a - b).abs() < 1e-12);
    let interpolated = !grids_equal;
    let mut points = Vec::new();
    let mut neg_idx = Vec::new();
    for (k, (&q, &iv)) in sample.q.iter().zip(sample.i.iter()).enumerate() {
        if q < lo - 1e-12 || q > hi + 1e-12 {
            continue; // interpolation only valid inside the overlap
        }
        let (bi, bs) = if grids_equal {
            (buffer.i[k], buffer.sigma[k])
        } else {
            match lerp(buffer, q) {
                Some(v) => v,
                None => continue,
            }
        };
        let sub = iv - scale * bi;
        let var = sample.sigma[k].powi(2) + (scale * bs).powi(2) + (bi * scale_sigma).powi(2);
        if sub < 0.0 {
            neg_idx.push(points.len());
        }
        points.push(SubPoint {
            q, i: sub, sigma: var.sqrt(),
            sample_idx: k, buffer_i: bi, buffer_sigma: bs, interpolated,
        });
    }
    let mut diagnostics = vec![Diagnostic {
        code: "bg_scale_uncertainty_included".into(),
        message: format!("sigma includes background scale uncertainty (scale={scale}, sigma={scale_sigma})"),
        indices: vec![],
    }];
    if interpolated {
        diagnostics.push(Diagnostic {
            code: "grid_mismatch_interpolated".into(),
            message: format!("q grids differ; buffer interpolated onto sample grid within overlap [{lo:.6}, {hi:.6}] only"),
            indices: vec![],
        });
    }
    if !neg_idx.is_empty() {
        diagnostics.push(Diagnostic {
            code: "negative_intensity".into(),
            message: format!("{} subtracted point(s) are negative; kept and flagged", neg_idx.len()),
            indices: neg_idx,
        });
    }
    Ok(SubResult { points, overlap: [lo, hi], interpolated, scale, scale_sigma, diagnostics })
}

/// Linear interpolation of (i, sigma) at q. None outside the buffer range.
pub fn lerp(c: &Curve, q: f64) -> Option<(f64, f64)> {
    if c.q.len() < 2 || q < c.q[0] || q > *c.q.last().unwrap() {
        return None;
    }
    let mut lo = 0usize;
    for k in 0..c.q.len() - 1 {
        if c.q[k] <= q && q <= c.q[k + 1] {
            lo = k;
            break;
        }
    }
    let (q0, q1) = (c.q[lo], c.q[lo + 1]);
    let t = if (q1 - q0).abs() < 1e-15 { 0.0 } else { (q - q0) / (q1 - q0) };
    let i = c.i[lo] * (1.0 - t) + c.i[lo + 1] * t;
    let s = c.sigma[lo] * (1.0 - t) + c.sigma[lo + 1] * t;
    Some((i, s))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitResult {
    pub model: String, // "guinier" | "porod"
    pub interval: [f64; 2],
    pub slope: f64,
    pub intercept: f64,
    pub derived: serde_json::Value, // e.g. {"Rg": ..} or {"porod_slope": ..}
    pub residuals: Vec<f64>,
    pub rms: f64,
    pub n_points: usize,
}

fn linfit(xs: &[f64], ys: &[f64]) -> Option<(f64, f64, Vec<f64>, f64)> {
    let n = xs.len();
    if n < 3 {
        return None;
    }
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let denom = n as f64 * sxx - sx * sx;
    if denom.abs() < 1e-20 {
        return None;
    }
    let slope = (n as f64 * sxy - sx * sy) / denom;
    let intercept = (sy - slope * sx) / n as f64;
    let residuals: Vec<f64> = xs.iter().zip(ys).map(|(x, y)| y - (slope * x + intercept)).collect();
    let rms = (residuals.iter().map(|r| r * r).sum::<f64>() / n as f64).sqrt();
    Some((slope, intercept, residuals, rms))
}

/// Guinier fit: ln I vs q^2 over [lo, hi]. Rg = sqrt(-3*slope).
pub fn guinier(q: &[f64], i: &[f64], interval: [f64; 2]) -> Option<FitResult> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for (&qv, &iv) in q.iter().zip(i.iter()) {
        if qv >= interval[0] && qv <= interval[1] && iv > 0.0 {
            xs.push(qv * qv);
            ys.push(iv.ln());
        }
    }
    let (slope, intercept, residuals, rms) = linfit(&xs, &ys)?;
    let rg = if slope < 0.0 { (-3.0 * slope).sqrt() } else { f64::NAN };
    Some(FitResult {
        model: "guinier".into(),
        interval,
        slope, intercept, residuals, rms,
        n_points: xs.len(),
        derived: serde_json::json!({"Rg_angstrom": rg, "I0": intercept.exp()}),
    })
}

/// Porod fit: ln I vs ln q over [lo, hi]; slope near -4 indicates Porod law.
pub fn porod(q: &[f64], i: &[f64], interval: [f64; 2]) -> Option<FitResult> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for (&qv, &iv) in q.iter().zip(i.iter()) {
        if qv >= interval[0] && qv <= interval[1] && iv > 0.0 && qv > 0.0 {
            xs.push(qv.ln());
            ys.push(iv.ln());
        }
    }
    let (slope, intercept, residuals, rms) = linfit(&xs, &ys)?;
    Some(FitResult {
        model: "porod".into(),
        interval,
        slope, intercept, residuals, rms,
        n_points: xs.len(),
        derived: serde_json::json!({"porod_slope": slope}),
    })
}

/// Evaluate all interval candidates. Overlapping candidates are flagged but
/// never merged: Guinier and Porod intervals are not auto-combined into a
/// single model, and every legal candidate is kept with its residuals.
pub fn fit_candidates(
    q: &[f64],
    i: &[f64],
    candidates: &[(String, [f64; 2])],
) -> (Vec<(String, FitResult)>, Vec<Diagnostic>) {
    let mut fits = Vec::new();
    for (name, iv) in candidates {
        let fit = match name.as_str() {
            "guinier" => guinier(q, i, *iv),
            "porod" => porod(q, i, *iv),
            _ => None,
        };
        if let Some(f) = fit {
            fits.push((name.clone(), f));
        }
    }
    let mut diags = Vec::new();
    for a in 0..candidates.len() {
        for b in a + 1..candidates.len() {
            let (na, ia) = &candidates[a];
            let (nb, ib) = &candidates[b];
            if ia[0] < ib[1] && ib[0] < ia[1] {
                diags.push(Diagnostic {
                    code: "interval_overlap".into(),
                    message: format!(
                        "intervals {na}[{:.4},{:.4}] and {nb}[{:.4},{:.4}] overlap; kept as separate candidates, not merged",
                        ia[0], ia[1], ib[0], ib[1]
                    ),
                    indices: vec![],
                });
            }
        }
    }
    diags.push(Diagnostic {
        code: "candidates_not_merged".into(),
        message: "Guinier and Porod intervals are evaluated independently; no single merged model is produced".into(),
        indices: vec![],
    });
    (fits, diags)
}
