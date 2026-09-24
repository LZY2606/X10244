//! Multi-interval model evidence (Guinier low-q, Porod high-q).
//!
//! Candidate windows are evaluated independently: Guinier and Porod evidence
//! is never auto-stitched into one model, and overlapping windows are
//! reported rather than silently merged.

use serde::{Deserialize, Serialize};

use crate::domain::subtract::SubPoint;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitWindow {
    pub q_lo: f64,
    pub q_hi: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearFit {
    pub slope: f64,
    pub intercept: f64,
    pub slope_sigma: f64,
    pub r2: f64,
    pub n: usize,
}

/// Weighted linear regression y = a + b x.
pub fn wls(xs: &[f64], ys: &[f64], ws: &[f64]) -> Option<LinearFit> {
    let n = xs.len();
    if n < 3 {
        return None;
    }
    let mut sw = 0.0;
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    for i in 0..n {
        let w = ws[i];
        sw += w;
        sx += w * xs[i];
        sy += w * ys[i];
        sxx += w * xs[i] * xs[i];
        sxy += w * xs[i] * ys[i];
    }
    let det = sw * sxx - sx * sx;
    if det.abs() < 1e-18 {
        return None;
    }
    let intercept = (sy * sxx - sx * sxy) / det;
    let slope = (sw * sxy - sx * sy) / det;
    let ybar = sy / sw;
    let mut ss_tot = 0.0;
    let mut ss_res = 0.0;
    let mut qxx = 0.0;
    for i in 0..n {
        let yhat = intercept + slope * xs[i];
        ss_tot += ws[i] * (ys[i] - ybar) * (ys[i] - ybar);
        ss_res += ws[i] * (ys[i] - yhat) * (ys[i] - yhat);
        qxx += ws[i] * (xs[i] - sx / sw) * (xs[i] - sx / sw);
    }
    let r2 = if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { 0.0 };
    let var = if n > 2 { ss_res / (n as f64 - 2.0) } else { 0.0 };
    let slope_sigma = (var / qxx.max(1e-18)).sqrt();
    Some(LinearFit { slope, intercept, slope_sigma, r2, n })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuinierCandidate {
    pub window: FitWindow,
    pub rg: f64,
    pub rg_sigma: f64,
    pub i0: f64,
    pub r2: f64,
    pub n: usize,
    pub accepted_hint: bool,
    pub reason: String,
}

/// Guinier: ln I vs q^2 linear; Rg = sqrt(-3 slope).
/// A window is merely a *hint* candidate: the UI keeps every candidate and a
/// reviewer decides. q_max*Rg must stay within the linearity guideline.
pub fn guinier(pts: &[SubPoint], window: &FitWindow) -> Option<GuinierCandidate> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut ws = Vec::new();
    for p in pts {
        let usable = p.i_sub.is_finite() && p.sigma_sub > 0.0 && p.i_sub > 0.0
            && p.q >= window.q_lo && p.q <= window.q_hi;
        if usable {
            xs.push(p.q * p.q);
            ys.push(p.i_sub.ln());
            ws.push(1.0 / (p.sigma_sub / p.i_sub).powi(2));
        }
    }
    let fit = wls(&xs, &ys, &ws)?;
    if fit.slope >= 0.0 {
        return Some(GuinierCandidate {
            window: window.clone(),
            rg: f64::NAN,
            rg_sigma: f64::NAN,
            i0: fit.intercept.exp(),
            r2: fit.r2,
            n: fit.n,
            accepted_hint: false,
            reason: "non-negative slope of ln I vs q^2".into(),
        });
    }
    let rg = (-3.0 * fit.slope).sqrt();
    let rg_sigma = 3.0_f64.sqrt() * fit.slope_sigma / (2.0 * rg);
    let qmax = window.q_hi;
    let (hint, reason) = if qmax * rg > 1.3 {
        (false, format!("q_max Rg = {:.2} > 1.3 linearity guideline", qmax * rg))
    } else if fit.r2 < 0.9 {
        (false, format!("R^2 = {:.3} below 0.9", fit.r2))
    } else {
        (true, format!("q_max Rg = {:.2} <= 1.3 and R^2 = {:.3}", qmax * rg, fit.r2))
    };
    Some(GuinierCandidate { window: window.clone(), rg, rg_sigma, i0: fit.intercept.exp(), r2: fit.r2, n: fit.n, accepted_hint: hint, reason })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PorodCandidate {
    pub window: FitWindow,
    pub exponent: f64,
    pub exponent_sigma: f64,
    pub r2: f64,
    pub n: usize,
    pub law: String,
    pub accepted_hint: bool,
    pub reason: String,
}

/// Porod: fit ln I vs ln q. Exponent near -4 => Porod-like decay.
/// "porod-plateau" mode fits I q^4 vs q^4 for a constant+background tail.
pub fn porod(pts: &[SubPoint], window: &FitWindow, mode: &str) -> Option<PorodCandidate> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut ws = Vec::new();
    for p in pts {
        let usable = p.i_sub.is_finite() && p.sigma_sub > 0.0 && p.i_sub > 0.0
            && p.q >= window.q_lo && p.q <= window.q_hi;
        if !usable {
            continue;
        }
        match mode {
            "power" => {
                xs.push(p.q.ln());
                ys.push(p.i_sub.ln());
                ws.push(1.0 / (p.sigma_sub / p.i_sub).powi(2));
            }
            _ => {
                xs.push(p.q.powi(4));
                ys.push(p.i_sub * p.q.powi(4));
                ws.push(1.0 / (p.sigma_sub * p.q.powi(4)).powi(2));
            }
        }
    }
    let fit = wls(&xs, &ys, &ws)?;
    if mode == "power" {
        let (hint, reason) = if (fit.slope + 4.0).abs() <= 0.5 {
            (true, format!("decay exponent {:.2} consistent with Porod (-4)", fit.slope))
        } else {
            (false, format!("decay exponent {:.2} not within +/-0.5 of -4", fit.slope))
        };
        Some(PorodCandidate {
            window: window.clone(),
            exponent: fit.slope,
            exponent_sigma: fit.slope_sigma,
            r2: fit.r2,
            n: fit.n,
            law: "power".into(),
            accepted_hint: hint,
            reason,
        })
    } else {
        Some(PorodCandidate {
            window: window.clone(),
            exponent: fit.intercept,
            exponent_sigma: fit.slope_sigma,
            r2: fit.r2,
            n: fit.n,
            law: "porod-plateau".into(),
            accepted_hint: fit.r2 >= 0.9,
            reason: format!("I q^4 vs q^4 linear R^2 = {:.3}", fit.r2),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntervalOverlap {
    pub a_kind: String,
    pub a_lo: f64,
    pub a_hi: f64,
    pub b_kind: String,
    pub b_lo: f64,
    pub b_hi: f64,
    pub overlap: f64,
}

/// Report every Guinier/Porod window pair that shares q range. These
/// overlaps are evidence for the reviewer, never a reason to merge models.
pub fn interval_overlaps(guinier_windows: &[FitWindow], porod_windows: &[FitWindow]) -> Vec<IntervalOverlap> {
    let mut out = Vec::new();
    for g in guinier_windows {
        for p in porod_windows {
            let lo = g.q_lo.max(p.q_lo);
            let hi = g.q_hi.min(p.q_hi);
            if hi > lo {
                out.push(IntervalOverlap {
                    a_kind: "guinier".into(),
                    a_lo: g.q_lo,
                    a_hi: g.q_hi,
                    b_kind: "porod".into(),
                    b_lo: p.q_lo,
                    b_hi: p.q_hi,
                    overlap: hi - lo,
                });
            }
        }
    }
    out
}

/// Sliding candidate windows across the finite positive part of a curve.
/// `side = "low"` widens from the lowest q, `"high"` from the highest.
pub fn candidate_windows(pts: &[SubPoint], min_points: usize, side: &str) -> Vec<FitWindow> {
    let mut qs: Vec<f64> = pts
        .iter()
        .filter(|p| p.i_sub.is_finite() && p.i_sub > 0.0)
        .map(|p| p.q)
        .collect();
    qs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut windows = Vec::new();
    if qs.len() < min_points {
        return windows;
    }
    // Fixed-length stride windows plus a few expanding ones, enough to expose
    // ambiguity without exploding the candidate set.
    let n = qs.len();
    for len in [min_points, min_points + 2].into_iter().filter(|l| *l <= n) {
        let starts: Vec<usize> = match side {
            "low" => (0..=n.saturating_sub(len)).step_by(2.max(len / 3)).collect(),
            _ => (0..=n.saturating_sub(len)).rev().step_by(2.max(len / 3)).collect(),
        };
        for s in starts {
            let w = FitWindow { q_lo: qs[s], q_hi: qs[s + len - 1] };
            if !windows.iter().any(|x: &FitWindow| x.q_lo == w.q_lo && x.q_hi == w.q_hi) {
                windows.push(w);
            }
        }
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::subtract::SubPoint;

    fn sp(q: f64, i: f64, s: f64) -> SubPoint {
        SubPoint {
            ordinal: q.to_bits() as u32,
            q,
            i_sub: i,
            sigma_sub: s,
            interp_buffer: false,
            weight_lo: 1.0,
            buffer_lo_ordinal: None,
            buffer_hi_ordinal: None,
            sample_ordinal: 1,
            flags: vec![],
        }
    }

    fn guinier_like(qmax: f64) -> Vec<SubPoint> {
        // I(q) = exp(-q^2 Rg^2/3), Rg = 3 nm.
        (0..10)
            .map(|k| {
                let q = 0.02 + (qmax - 0.02) * k as f64 / 9.0;
                sp(q, (-(q * q) * 9.0 / 3.0).exp(), 1e-4)
            })
            .collect()
    }

    #[test]
    fn guinier_recovers_rg() {
        let pts = guinier_like(0.4);
        let c = guinier(&pts, &FitWindow { q_lo: 0.02, q_hi: 0.35 }).unwrap();
        assert!((c.rg - 3.0).abs() < 0.05, "rg={}", c.rg);
        assert!(c.accepted_hint);
    }

    #[test]
    fn guinier_rejects_window_violating_qrg_guideline() {
        let pts = guinier_like(0.8);
        let c = guinier(&pts, &FitWindow { q_lo: 0.02, q_hi: 0.8 }).unwrap();
        assert!(!c.accepted_hint);
        assert!(c.reason.contains("q_max Rg"));
    }

    #[test]
    fn porod_detects_minus_four_decay() {
        let pts: Vec<SubPoint> = (0..10)
            .map(|k| {
                let q = 0.5 + 0.5 * k as f64;
                sp(q, 100.0 * q.powi(-4), 1e-3)
            })
            .collect();
        let c = porod(&pts, &FitWindow { q_lo: 0.5, q_hi: 5.0 }, "power").unwrap();
        assert!((c.exponent + 4.0).abs() < 1e-6);
        assert!(c.accepted_hint);
    }

    #[test]
    fn windows_are_not_merged_when_overlapping() {
        let g = vec![FitWindow { q_lo: 0.02, q_hi: 0.3 }];
        let p = vec![FitWindow { q_lo: 0.25, q_hi: 1.0 }];
        let ov = interval_overlaps(&g, &p);
        assert_eq!(ov.len(), 1);
        assert!((ov[0].overlap - 0.05).abs() < 1e-12);
    }

    #[test]
    fn sliding_windows_stay_inside_curve() {
        let pts: Vec<SubPoint> = (0..12).map(|k| sp(0.1 * k as f64 + 0.1, 1.0 / (k as f64 + 1.0), 1e-3)).collect();
        let lows = candidate_windows(&pts, 4, "low");
        assert!(lows.iter().all(|w| w.q_hi <= pts.last().unwrap().q));
        assert!(!lows.is_empty());
    }
}
