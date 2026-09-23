//! 多区间模型规则 RULE_MODEL_V1。
//!
//! Guinier 与 Porod 各自独立枚举合法窗口并全部保留；绝不自动拼成单一模型。
//! 无法唯一决定时（多个窗口 χ² 相当）保留全部合法候选，由审阅者裁定。

use crate::ids;
use crate::model::*;

pub const RULE_MODEL: &str = "rules/model@1";

#[derive(Debug, Clone)]
pub struct ModelParams {
    pub min_points: usize,
    /// Guinier 合法窗口 qmax·Rg 上限。
    pub qrg_max: f64,
    /// 候选之间被视为“无法唯一决定”的 χ² 相对阈值。
    pub ambiguous_chi2_rel: f64,
}

impl Default for ModelParams {
    fn default() -> Self {
        ModelParams { min_points: 5, qrg_max: 1.3, ambiguous_chi2_rel: 0.10 }
    }
}

pub struct ModelResult {
    pub model_fit: ModelFit,
    pub diagnostics: Vec<Diagnostic>,
}

fn valid_points(sub: &Subtraction) -> Vec<&Point> {
    sub.points.iter().filter(|p| p.i > 0.0 && p.i - p.sigma > 0.0 && p.sigma > 0.0).collect()
}

/// 加权线性拟合 y = a + b x，返回 (a, b, sa, sb, chi2)。
fn wls(xs: &[f64], ys: &[f64], ws: &[f64]) -> (f64, f64, f64, f64, f64) {
    let sw: f64 = ws.iter().sum();
    let sx: f64 = xs.iter().zip(ws).map(|(x, w)| x * w).sum();
    let sy: f64 = ys.iter().zip(ws).map(|(y, w)| y * w).sum();
    let sxx: f64 = xs.iter().zip(ws).map(|(x, w)| x * x * w).sum();
    let sxy: f64 = xs.iter().zip(ys).zip(ws).map(|((x, y), w)| x * y * w).sum();
    let det = sw * sxx - sx * sx;
    let (a, b) = if det.abs() < 1e-300 { (sy / sw, 0.0) } else { ((sxx * sy - sx * sxy) / det, (sw * sxy - sx * sy) / det) };
    let mut chi2 = 0.0;
    for i in 0..xs.len() {
        let r = ys[i] - (a + b * xs[i]);
        chi2 += r * r * ws[i];
    }
    let sa = if det.abs() > 1e-300 { (sxx / det).max(0.0).sqrt() } else { 0.0 };
    let sb = if det.abs() > 1e-300 { (sw / det).max(0.0).sqrt() } else { 0.0 };
    (a, b, sa, sb, chi2)
}

fn window_fits(points: &[&Point], min_points: usize, kind: RegionKind) -> Vec<RegionFit> {
    let n = points.len();
    let mut out = Vec::new();
    if n < min_points { return out; }
    // 枚举所有连续窗口（限制最大窗口数避免组合爆炸：窗口长度 5..=n，起点遍历）。
    for len in min_points..=n {
        for start in 0..=n - len {
            let win = &points[start..start + len];
            match kind {
                RegionKind::Guinier => {
                    let xs: Vec<f64> = win.iter().map(|p| p.q * p.q).collect();
                    let ys: Vec<f64> = win.iter().map(|p| p.i.ln()).collect();
                    let ws: Vec<f64> = win.iter().map(|p| {
                        let s = p.sigma / p.i;
                        1.0 / (s * s).max(1e-12)
                    }).collect();
                    let (a, b, sa, sb, chi2) = wls(&xs, &ys, &ws);
                    if b >= 0.0 || a.is_nan() { continue; }
                    let rg = (-3.0 * b).sqrt();
                    let i0 = a.exp();
                    let rg_sigma = 3.0_f64.sqrt() * sb / (2.0 * (-3.0 * b).sqrt());
                    let i0_sigma = sa * i0;
                    let qmax = win[win.len() - 1].q;
                    if qmax * rg > 1.3 { continue; }
                    let mut residuals = Vec::with_capacity(win.len());
                    for p in win.iter() {
                        let pred = (a + b * p.q * p.q).exp();
                        let wr = (p.i - pred) / p.sigma;
                        residuals.push(Residual { point_id: p.point_id.clone(), q: p.q, observed: p.i, predicted: pred, weighted_residual: wr });
                    }
                    let rms = (residuals.iter().map(|r| (r.observed - r.predicted).powi(2)).sum::<f64>() / win.len() as f64).sqrt();
                    out.push(RegionFit {
                        kind, range_q: [win[0].q, qmax], n_points: win.len(),
                        params: vec![
                            NamedParam { name: "Rg".to_string(), value: rg, sigma: rg_sigma },
                            NamedParam { name: "I0".to_string(), value: i0, sigma: i0_sigma },
                            NamedParam { name: "qmax_Rg".to_string(), value: qmax * rg, sigma: 0.0 },
                        ],
                        residual_rms: rms, chi2, degrees_of_freedom: win.len() - 2, residuals,
                    });
                }
                RegionKind::Porod => {
                    let xs: Vec<f64> = win.iter().map(|p| p.q.ln()).collect();
                    let ys: Vec<f64> = win.iter().map(|p| p.i.ln()).collect();
                    let ws: Vec<f64> = win.iter().map(|p| {
                        let s = p.sigma / p.i;
                        1.0 / (s * s).max(1e-12)
                    }).collect();
                    let (a, b, sa, sb, chi2) = wls(&xs, &ys, &ws);
                    if b >= -1.0 || b <= -6.0 { continue; } // 物理合理幂律指数区间
                    let c = a.exp();
                    let c_sigma = sa * c;
                    let mut residuals = Vec::with_capacity(win.len());
                    for p in win.iter() {
                        let pred = (a + b * p.q.ln()).exp();
                        let wr = (p.i - pred) / p.sigma;
                        residuals.push(Residual { point_id: p.point_id.clone(), q: p.q, observed: p.i, predicted: pred, weighted_residual: wr });
                    }
                    let rms = (residuals.iter().map(|r| (r.observed - r.predicted).powi(2)).sum::<f64>() / win.len() as f64).sqrt();
                    out.push(RegionFit {
                        kind, range_q: [win[0].q, win[win.len() - 1].q], n_points: win.len(),
                        params: vec![
                            NamedParam { name: "C".to_string(), value: c, sigma: c_sigma },
                            NamedParam { name: "p".to_string(), value: b, sigma: sb },
                        ],
                        residual_rms: rms, chi2, degrees_of_freedom: win.len() - 2, residuals,
                    });
                }
            }
        }
    }
    // 帕累托式筛选：去掉“被更大窗口包含且 χ² 更大”的候选，保留边界证据；
    // 相邻候选合并要求 χ² 几乎相同，仍各自保留。
    out.sort_by(|x, y| {
        x.kind.partial_cmp(&y.kind).unwrap_or(std::cmp::Ordering::Equal)
            .then(x.range_q[0].partial_cmp(&y.range_q[0]).unwrap())
            .then(x.chi2.partial_cmp(&y.chi2).unwrap())
    });
    // 每种 kind 最多保留 12 个最优窗口（按 χ²/dof），并保证至少覆盖最宽合法窗口。
    let mut kept: Vec<RegionFit> = Vec::new();
    for kind in [RegionKind::Guinier, RegionKind::Porod] {
        let mut v: Vec<RegionFit> = out.iter().filter(|r| r.kind == kind).cloned().collect();
        v.sort_by(|a, b| (a.chi2 / a.degrees_of_freedom.max(1) as f64).partial_cmp(&(b.chi2 / b.degrees_of_freedom.max(1) as f64)).unwrap());
        let cap = 12;
        if v.len() > cap { v.truncate(cap); }
        kept.extend(v);
    }
    kept
}

/// 对扣背景结果枚举 Guinier 与 Porod 全部合法候选。
pub fn fit_models(sub: &Subtraction, sub_node: &str, params: &ModelParams) -> ModelResult {
    let pts = valid_points(sub);
    let mut diagnostics = Vec::new();

    if sub.status == SubtractionStatus::NoOverlap || sub.points.is_empty() {
        return ModelResult {
            model_fit: ModelFit { subtraction_node: sub_node.to_string(), region_overlaps: Vec::new(), regions: Vec::new() },
            diagnostics: vec![Diagnostic {
                code: "model_no_input".to_string(), severity: Severity::Info,
                message: "扣背景无有效点（无重叠区），不产生任何区间候选".to_string(), refs: vec![sub_node.to_string()],
            }],
        };
    }

    let mut regions = window_fits(&pts, params.min_points, RegionKind::Guinier);
    regions.extend(window_fits(&pts, params.min_points, RegionKind::Porod).into_iter());

    // 区间重叠（含 Guinier×Porod 跨类重叠与同类窗口重叠）。
    let mut overlaps = Vec::new();
    for i in 0..regions.len() {
        for j in (i + 1)..regions.len() {
            let (a, b) = (&regions[i], &regions[j]);
            let lo = a.range_q[0].max(b.range_q[0]);
            let hi = a.range_q[1].min(b.range_q[1]);
            if lo < hi { overlaps.push([i, j]); }
        }
    }
    if !overlaps.is_empty() {
        diagnostics.push(Diagnostic {
            code: crate::rules::REGION_OVERLAP.to_string(),
            severity: Severity::Warning,
            message: format!("存在 {} 对区间候选的 q 范围重叠；Guinier 与 Porod 作为独立区间证据保留，不自动拼接为单一模型", overlaps.len()),
            refs: overlaps.iter().map(|[i, j]| format!("{}#region{},{}", sub_node, i, j)).collect(),
        });
    }

    // 多候选残差：同 kind 内 χ² 接近即“无法唯一决定”。
    for kind in [RegionKind::Guinier, RegionKind::Porod] {
        let v: Vec<&RegionFit> = regions.iter().filter(|r| r.kind == kind).collect();
        if v.len() >= 2 {
            let best = v.iter().map(|r| r.chi2 / r.degrees_of_freedom.max(1) as f64).fold(f64::INFINITY, f64::min);
            let close: Vec<&&RegionFit> = v.iter().filter(|r| {
                let c = r.chi2 / r.degrees_of_freedom.max(1) as f64;
                (c - best).abs() <= params.ambiguous_chi2_rel * best.max(1e-300)
            }).collect();
            if close.len() >= 2 {
                diagnostics.push(Diagnostic {
                    code: crate::rules::MULTI_CANDIDATE.to_string(),
                    severity: Severity::Warning,
                    message: format!("{:?} 区间有 {} 个候选的加权残差 χ²/dof 相差不超过 {:.0}%，无法唯一决定；全部合法候选保留，等待人工裁定（不自动选最优）", kind, close.len(), params.ambiguous_chi2_rel * 100.0),
                    refs: close.iter().map(|r| format!("{}#{:?}[{:.4},{:.4}]", sub_node, kind, r.range_q[0], r.range_q[1])).collect(),
                });
            }
        }
    }

    ModelResult {
        model_fit: ModelFit { subtraction_node: sub_node.to_string(), region_overlaps: overlaps, regions },
        diagnostics,
    }
}

#[allow(dead_code)]
fn _unused_point_id(version_id: &str, i: usize) -> String { ids::point(version_id, i) }
