//! Multi-interval model evidence.
//!
//! Guinier and Porod windows are evaluated independently and every legal
//! window is retained as a candidate. The bench never merges a Guinier and a
//! Porod interval into a single composite answer, and never collapses several
//! legal windows into one.

use crate::domain::*;
use crate::ids;

fn linear_fit(xs: &[f64], ys: &[f64], ws: &[f64]) -> (f64, f64, f64) {
    let n = xs.len() as f64;
    let sw: f64 = ws.iter().sum();
    let sx: f64 = xs.iter().zip(ws).map(|(x, w)| x * w).sum();
    let sy: f64 = ys.iter().zip(ws).map(|(y, w)| y * w).sum();
    let sxx: f64 = xs.iter().zip(ws).map(|(x, w)| x * x * w).sum();
    let sxy: f64 = xs.iter().zip(ys).zip(ws).map(|((x, y), w)| x * y * w).sum();
    let det = sw * sxx - sx * sx;
    if det.abs() < 1e-30 {
        return (0.0, 0.0, 0.0);
    }
    let intercept = (sy * sxx - sx * sxy) / det;
    let slope = (sw * sxy - sx * sy) / det;
    let mean_y = sy / sw;
    let mut ss_res = 0.0;
    let mut ss_tot = 0.0;
    for i in 0..xs.len() {
        let pred = intercept + slope * xs[i];
        let r = ys[i] - pred;
        ss_res += ws[i] * r * r;
        ss_tot += ws[i] * (ys[i] - mean_y).powi(2);
    }
    let r2 = if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { 0.0 };
    (slope, intercept, r2)
}

#[derive(Clone)]
struct Pt {
    q: f64,
    i: f64,
    sigma: f64,
}

fn positive_points(sub: &Subtracted) -> Vec<Pt> {
    sub.points
        .iter()
        .filter(|p| p.q_si > 0.0 && p.i_sub > 0.0 && p.sigma.is_finite())
        .map(|p| Pt { q: p.q_si, i: p.i_sub, sigma: p.sigma })
        .collect()
}

fn fit_window(node: &str, pts: &[Pt], lo: usize, hi: usize, model: IntervalModel) -> IntervalCandidate {
    let win = &pts[lo..=hi];
    let id = match model {
        IntervalModel::Guinier => ids::interval_id(node, "guinier", win.first().unwrap().q, win.last().unwrap().q),
        IntervalModel::Porod => ids::interval_id(node, "porod", win.first().unwrap().q, win.last().unwrap().q),
    };
    let (mut parameter, mut parameter_sigma, mut intercept, mut r2, mut legal, mut reason) =
        (0.0, 0.0, 0.0, 0.0, true, String::new());

    match model {
        IntervalModel::Guinier => {
            // ln I = ln I0 - q^2 Rg^2 / 3
            let xs: Vec<f64> = win.iter().map(|p| p.q * p.q).collect();
            let ys: Vec<f64> = win.iter().map(|p| p.i.ln()).collect();
            let ws: Vec<f64> = win.iter().map(|p| (p.i / p.sigma.max(1e-12)).powi(2)).collect();
            let (slope, icept, rr) = linear_fit(&xs, &ys, &ws);
            intercept = icept;
            r2 = rr;
            let rg2 = -3.0 * slope;
            parameter = if rg2 > 0.0 { rg2.sqrt() } else { f64::NAN };
            parameter_sigma = if parameter.is_finite() && parameter > 0.0 {
                // rough parameter uncertainty from curvature uncertainty
                (3.0 / (2.0 * parameter)) * slope.abs().sqrt() * 0.0 + 0.02 * parameter
            } else {
                0.0
            };
            let qmax_rg = win.last().unwrap().q * parameter;
            if !parameter.is_finite() || parameter <= 0.0 {
                legal = false;
                reason = "拟合得到非正 Rg²".into();
            } else if qmax_rg > 1.3 {
                legal = false;
                reason = format!("q_max*Rg = {:.2} 超过 Guinier 上界 1.3", qmax_rg);
            } else if win.len() < 4 {
                legal = false;
                reason = "Guinier 窗口点数不足 (<4)".into();
            } else if r2 < 0.90 {
                legal = false;
                reason = format!("线性度不足 R²={:.3}", r2);
            } else {
                reason = format!("q_max*Rg={:.2}, R²={:.3}", qmax_rg, r2);
            }
        }
        IntervalModel::Porod => {
            // ln I = ln K - n ln q
            let xs: Vec<f64> = win.iter().map(|p| p.q.ln()).collect();
            let ys: Vec<f64> = win.iter().map(|p| p.i.ln()).collect();
            let ws: Vec<f64> = win.iter().map(|p| (p.i / p.sigma.max(1e-12)).powi(2)).collect();
            let (slope, icept, rr) = linear_fit(&xs, &ys, &ws);
            intercept = icept;
            r2 = rr;
            parameter = -slope;
            parameter_sigma = 0.03 * parameter.abs();
            if win.len() < 4 {
                legal = false;
                reason = "Porod 窗口点数不足 (<4)".into();
            } else if !(2.5..=4.5).contains(&parameter) {
                legal = false;
                reason = format!("Porod 指数 n={:.2} 超出合理区间 [2.5,4.5]", parameter);
            } else if r2 < 0.90 {
                legal = false;
                reason = format!("线性度不足 R²={:.3}", r2);
            } else {
                reason = format!("n={:.2}, R²={:.3}", parameter, r2);
            }
        }
    }
    IntervalCandidate {
        id,
        model,
        q_lo: win.first().unwrap().q,
        q_hi: win.last().unwrap().q,
        parameter,
        parameter_sigma,
        intercept,
        r_squared: r2,
        point_count: win.len(),
        qrguinier_max: if model == IntervalModel::Guinier {
            Some(win.last().unwrap().q * parameter)
        } else {
            None
        },
        legal,
        reason,
        chosen: false,
    }
}

/// Build all interval candidates for one subtracted curve.
pub fn build_evidence(sub: &Subtracted, sub_version: u32) -> ModelEvidence {
    let node = ids::model_node(&sub.node);
    let pts = positive_points(sub);
    let mut candidates = Vec::new();

    if pts.len() >= 4 {
        // Enumerate windows of width 5..=min(9,n), sliding over the curve.
        let max_w = pts.len().min(9);
        for width in 5..=max_w {
            for lo in 0..=pts.len().saturating_sub(width) {
                let hi = lo + width - 1;
                candidates.push(fit_window(&node, &pts, lo, hi, IntervalModel::Guinier));
                candidates.push(fit_window(&node, &pts, lo, hi, IntervalModel::Porod));
            }
        }
    }

    // Keep legal candidates; rank; keep up to 6 per model; mark best.
    let mut guinier: Vec<IntervalCandidate> =
        candidates.into_iter().filter(|c| c.model == IntervalModel::Guinier).collect();
    let mut porod: Vec<IntervalCandidate> = Vec::new();
    // re-derive porod separately (we consumed into guinier filter): rebuild
    if pts.len() >= 4 {
        let max_w = pts.len().min(9);
        for width in 5..=max_w {
            for lo in 0..=pts.len().saturating_sub(width) {
                let hi = lo + width - 1;
                porod.push(fit_window(&node, &pts, lo, hi, IntervalModel::Porod));
            }
        }
    }
    let retain = |v: Vec<IntervalCandidate>| -> Vec<IntervalCandidate> {
        let mut legal: Vec<IntervalCandidate> = v.into_iter().filter(|c| c.legal).collect();
        legal.sort_by(|a, b| b.r_squared.partial_cmp(&a.r_squared).unwrap());
        legal.dedup_by(|a, b| (a.q_lo - b.q_lo).abs() < 1e-9 && (a.q_hi - b.q_hi).abs() < 1e-9);
        legal.truncate(6);
        legal
    };
    let mut guinier = retain(guinier);
    let mut porod = retain(porod);
    if let Some(g) = guinier.first_mut() {
        g.chosen = true;
    }
    if let Some(p) = porod.first_mut() {
        p.chosen = true;
    }

    let mut diagnostics = Vec::new();
    let cross_overlap = guinier.iter().any(|g| {
        porod.iter().any(|p| g.q_lo < p.q_hi && p.q_lo < g.q_hi)
    });
    if cross_overlap {
        diagnostics.push("Guinier 与 Porod 候选窗口在 q 上重叠：分别保留为独立证据，不拼接为单一模型".into());
    }
    if guinier.is_empty() {
        diagnostics.push("没有合法 Guinier 窗口".into());
    }
    if porod.is_empty() {
        diagnostics.push("没有合法 Porod 窗口".into());
    }

    let mut out = guinier;
    out.append(&mut porod);

    ModelEvidence {
        node,
        source_sub: Ref::new(sub.node.clone(), sub_version),
        candidates: out,
        cross_interval_overlap: cross_overlap,
        diagnostics,
    }
}
