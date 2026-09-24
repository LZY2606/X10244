//! Multi-region model evidence.
//!
//! Guinier region: ln I(q) = ln I0 - q^2 Rg^2 / 3  (fit y = a + b x, x = q^2)
//! Porod region:  I(q) = K q^-p                    (fit ln I = lnK - p ln q)
//!
//! Guinier and Porod candidates are fit independently. Overlapping windows are
//! *flagged*, never auto-assembled into one piecewise model, and no single
//! best model is forced: every legal window remains a candidate with its own
//! residuals and evidence.
use crate::domain::ids::hid;
use crate::domain::subtract::{SubCandidate, SubPointStatus};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitWindow {
    pub id: String,
    pub kind: ModelKind,
    pub q_min: f64,
    pub q_max: f64,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Guinier,
    Porod,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitResult {
    pub window_id: String,
    pub kind: ModelKind,
    pub q_min: f64,
    pub q_max: f64,
    pub n_points: usize,
    /// Guinier: (I0, Rg[A]); Porod: (K, exponent)
    pub param_a: f64,
    pub param_b: f64,
    pub sigma_a: f64,
    pub sigma_b: f64,
    pub r_squared: f64,
    pub reduced_chi2: f64,
    pub qr_max: Option<f64>,
    pub qr_min: Option<f64>,
    pub residual: Vec<[f64; 2]>,
    pub evidence_status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Overlap {
    pub a_window: String,
    pub b_window: String,
    pub q_range: [f64; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelReport {
    pub id: String,
    pub subtraction_id: String,
    pub candidate_id: String,
    pub windows: Vec<FitWindow>,
    pub fits: Vec<FitResult>,
    pub overlaps: Vec<Overlap>,
    pub note: String,
}

fn linfit(xs: &[f64], ys: &[f64], ws: &[f64]) -> Option<(f64, f64, f64, f64, f64)> {
    let n = xs.len() as f64;
    if n < 3.0 { return None; }
    let sw: f64 = ws.iter().sum();
    let sx: f64 = xs.iter().zip(ws).map(|(x, w)| x * w).sum();
    let sy: f64 = ys.iter().zip(ws).map(|(y, w)| y * w).sum();
    let sxx: f64 = xs.iter().zip(ws).map(|(x, w)| x * x * w).sum();
    let sxy: f64 = xs.iter().zip(ys).zip(ws).map(|((x, y), w)| x * y * w).sum();
    let det = sw * sxx - sx * sx;
    if det.abs() < 1e-30 { return None; }
    let intercept = (sy * sxx - sx * sxy) / det;
    let slope = (sw * sxy - sx * sy) / det;
    let ybar = sy / sw;
    let ss_tot: f64 = ys.iter().zip(ws).map(|(y, w)| (y - ybar).powi(2) * w).sum();
    let ss_res: f64 = xs.iter().zip(ys).zip(ws).map(|((x, y), w)| {
        let e = y - (intercept + slope * x);
        e * e * w
    }).sum();
    let r2 = if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { f64::NAN };
    let dof = (n - 2.0).max(1.0);
    let var = ss_res / dof;
    let var_b = var * sw / det;
    let var_a = var * sxx / det;
    Some((intercept, slope, var_a.max(0.0).sqrt(), var_b.max(0.0).sqrt(),
          (ss_res / dof).sqrt()))
}

fn fit_window(cand: &SubCandidate, w: &FitWindow) -> Option<FitResult> {
    let pts: Vec<&crate::domain::subtract::SubPoint> = cand.points.iter()
        .filter(|p| p.status == SubPointStatus::Ok && p.intensity > 0.0
                 && p.q >= w.q_min && p.q <= w.q_max).collect();
    if pts.len() < 3 {
        return Some(FitResult { window_id: w.id.clone(), kind: w.kind,
            q_min: w.q_min, q_max: w.q_max, n_points: pts.len(),
            param_a: f64::NAN, param_b: f64::NAN, sigma_a: f64::NAN,
            sigma_b: f64::NAN, r_squared: f64::NAN, reduced_chi2: f64::NAN,
            qr_max: None, qr_min: None, residual: vec![],
            evidence_status: "insufficient_points".into() });
    }
    match w.kind {
        ModelKind::Guinier => {
            let xs: Vec<f64> = pts.iter().map(|p| p.q * p.q).collect();
            let ys: Vec<f64> = pts.iter().map(|p| p.intensity.ln()).collect();
            let ws: Vec<f64> = pts.iter().map(|p|
                (p.sigma / p.intensity).powi(-2).max(1e-30)).collect();
            let (a, b, sa, sb, _) = linfit(&xs, &ys, &ws)?;
            let i0 = a.exp();
            let rg = if b < 0.0 { (-3.0 * b).sqrt() } else { f64::NAN };
            let mean_y: f64 = ys.iter().sum::<f64>() / ys.len() as f64;
            let ss_tot: f64 = ys.iter().map(|y| (y - mean_y).powi(2)).sum();
            let ss_res: f64 = xs.iter().zip(&ys).map(|(x, y)| {
                let e = y - (a + b * x); e * e
            }).sum();
            let r2 = if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { f64::NAN };
            let dof = (pts.len() - 2).max(1) as f64;
            let chi2 = ss_res / dof;
            let residual: Vec<[f64;2]> = pts.iter().map(|p| {
                let pred = (a + b * p.q * p.q).exp();
                [p.q, p.intensity - pred]
            }).collect();
            let qr: Vec<f64> = pts.iter().map(|p| p.q * rg).collect();
            let status = if !rg.is_finite() || b >= 0.0 { "invalid_slope".into() }
                else if qr.iter().cloned().fold(f64::MIN, f64::max) > 1.3 {
                    "guinier_violation_qRg_gt_1.3".into()
                } else { "legal".into() };
            Some(FitResult { window_id: w.id.clone(), kind: w.kind,
                q_min: w.q_min, q_max: w.q_max, n_points: pts.len(),
                param_a: i0, param_b: rg,
                sigma_a: sa * i0, sigma_b: sb * 1.5 / rg.max(1e-30),
                r_squared: r2, reduced_chi2: chi2,
                qr_max: qr.iter().cloned().fold(f64::NEG_INFINITY, f64::max).into(),
                qr_min: qr.iter().cloned().fold(f64::INFINITY, f64::min).into(),
                residual, evidence_status: status })
        }
        ModelKind::Porod => {
            let xs: Vec<f64> = pts.iter().map(|p| p.q.ln()).collect();
            let ys: Vec<f64> = pts.iter().map(|p| p.intensity.ln()).collect();
            let ws: Vec<f64> = pts.iter().map(|p|
                (p.sigma / p.intensity).powi(-2).max(1e-30)).collect();
            let (a, b, sa, sb, _) = linfit(&xs, &ys, &ws)?;
            let k = a.exp();
            let exponent = -b;
            let mean_y: f64 = ys.iter().sum::<f64>() / ys.len() as f64;
            let ss_tot: f64 = ys.iter().map(|y| (y - mean_y).powi(2)).sum();
            let ss_res: f64 = xs.iter().zip(&ys).map(|(x, y)| {
                let e = y - (a + b * x); e * e
            }).sum();
            let r2 = if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { f64::NAN };
            let dof = (pts.len() - 2).max(1) as f64;
            let chi2 = ss_res / dof;
            let residual: Vec<[f64;2]> = pts.iter().map(|p| {
                let pred = (a + b * p.q.ln()).exp();
                [p.q, p.intensity - pred]
            }).collect();
            let status = if exponent >= 2.0 && exponent <= 5.0 { "legal".into() }
                else { "exponent_outside_2_5".into() };
            Some(FitResult { window_id: w.id.clone(), kind: w.kind,
                q_min: w.q_min, q_max: w.q_max, n_points: pts.len(),
                param_a: k, param_b: exponent,
                sigma_a: sa * k, sigma_b: sb,
                r_squared: r2, reduced_chi2: chi2,
                qr_max: None, qr_min: None, residual,
                evidence_status: status })
        }
    }
}

pub fn model_report(subtraction_id: &str, candidate: &SubCandidate,
                    windows: Vec<FitWindow>) -> ModelReport {
    let mut fits = Vec::new();
    for w in &windows {
        if let Some(f) = fit_window(candidate, w) {
            fits.push(f);
        }
    }
    let mut overlaps = Vec::new();
    for i in 0..windows.len() {
        for j in (i + 1)..windows.len() {
            if windows[i].kind == windows[j].kind { continue; }
            let lo = windows[i].q_min.max(windows[j].q_min);
            let hi = windows[i].q_max.min(windows[j].q_max);
            if lo <= hi {
                overlaps.push(Overlap {
                    a_window: windows[i].id.clone(),
                    b_window: windows[j].id.clone(),
                    q_range: [lo, hi],
                });
            }
        }
    }
    let note = if overlaps.is_empty() {
        "regions disjoint; candidates independent; no automatic single model".into()
    } else {
        "overlapping Guinier/Porod windows flagged; not merged into one model".into()
    };
    let payload = json!({"subtraction": subtraction_id, "candidate": candidate.id,
        "windows": windows});
    ModelReport { id: hid("model", &payload), subtraction_id: subtraction_id.into(),
        candidate_id: candidate.id.clone(), windows, fits, overlaps, note }
}
