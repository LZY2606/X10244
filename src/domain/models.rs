use super::subtract::SubtractedCurve;
use super::types::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelParams {
    pub guinier_q_max_factor: f64,
    pub porod_q_min: f64,
    pub guinier_min_points: usize,
    pub guinier_max_points: usize,
    pub keep_top: usize,
}

impl Default for ModelParams {
    fn default() -> Self {
        ModelParams {
            guinier_q_max_factor: 1.3,
            porod_q_min: 2.0,
            guinier_min_points: 5,
            guinier_max_points: 12,
            keep_top: 3,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuinierCandidate {
    pub id: String,
    pub q_min: f64,
    pub q_max: f64,
    pub n_points: usize,
    pub rg: f64,
    pub rg_sigma: f64,
    pub i0: f64,
    pub i0_sigma: f64,
    pub chi2: f64,
    pub adopted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PorodCandidate {
    pub id: String,
    pub q_min: f64,
    pub q_max: f64,
    pub n_points: usize,
    pub k: f64,
    pub k_sigma: f64,
    pub b: f64,
    pub b_sigma: f64,
    pub chi2: f64,
    pub adopted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelOutput {
    pub rule_version: String,
    pub source_candidate_id: Option<String>,
    pub guinier: Vec<GuinierCandidate>,
    pub porod: Vec<PorodCandidate>,
    pub diagnostics: Vec<Diagnostic>,
}

fn wls(x: &[f64], y: &[f64], sy: &[f64]) -> Option<WlsFit> {
    let mut sw = 0.0;
    let mut swx = 0.0;
    let mut swy = 0.0;
    let mut swxx = 0.0;
    let mut swxy = 0.0;
    for i in 0..x.len() {
        let w = 1.0 / (sy[i] * sy[i]);
        sw += w;
        swx += w * x[i];
        swy += w * y[i];
        swxx += w * x[i] * x[i];
        swxy += w * x[i] * y[i];
    }
    let det = sw * swxx - swx * swx;
    if det.abs() < 1e-30 {
        return None;
    }
    let slope = (sw * swxy - swx * swy) / det;
    let intercept = (swxx * swy - swx * swxy) / det;
    let mut chi2 = 0.0;
    for i in 0..x.len() {
        let r = (y[i] - (intercept + slope * x[i])) / sy[i];
        chi2 += r * r;
    }
    let dfree = (x.len().saturating_sub(2)).max(1) as f64;
    let reduced = chi2 / dfree;
    Some(WlsFit {
        slope,
        intercept,
        slope_sigma: (sw / det).max(0.0).sqrt(),
        intercept_sigma: (swxx / det).max(0.0).sqrt(),
        chi2: reduced,
    })
}

struct WlsFit {
    slope: f64,
    intercept: f64,
    slope_sigma: f64,
    intercept_sigma: f64,
    chi2: f64,
}

fn fit_guinier_window(
    curve: &SubtractedCurve,
    start: usize,
    len: usize,
) -> Option<GuinierCandidate> {
    let pts = &curve.curve.points;
    if start + len > pts.len() {
        return None;
    }
    let window = &pts[start..start + len];
    if window.iter().any(|p| p.i <= 0.0) {
        return None;
    }
    let x: Vec<f64> = window.iter().map(|p| p.q * p.q).collect();
    let y: Vec<f64> = window.iter().map(|p| p.i.ln()).collect();
    let sy: Vec<f64> = window.iter().map(|p| p.sigma / p.i).collect();
    let fit = wls(&x, &y, &sy)?;
    if fit.slope >= 0.0 {
        return None;
    }
    let rg = (-3.0 * fit.slope).sqrt();
    let i0 = fit.intercept.exp();
    let q_min = window.first().unwrap().q;
    let q_max = window.last().unwrap().q;
    let rg_sigma = 3.0_f64.sqrt() * fit.slope_sigma / (2.0 * rg);
    let i0_sigma = fit.intercept_sigma * i0;
    Some(GuinierCandidate {
        id: format!("{}:g{:03}-{}", curve.id, start, len),
        q_min,
        q_max,
        n_points: len,
        rg,
        rg_sigma,
        i0,
        i0_sigma,
        chi2: fit.chi2,
        adopted: false,
    })
}

fn fit_porod_window(
    curve: &SubtractedCurve,
    start: usize,
    len: usize,
) -> Option<PorodCandidate> {
    let pts = &curve.curve.points;
    if start + len > pts.len() {
        return None;
    }
    let window = &pts[start..start + len];
    let x: Vec<f64> = window.iter().map(|p| p.q.powi(-4)).collect();
    let y: Vec<f64> = window.iter().map(|p| p.i).collect();
    let sy: Vec<f64> = window.iter().map(|p| p.sigma.max(1e-9)).collect();
    let fit = wls(&x, &y, &sy)?;
    Some(PorodCandidate {
        id: format!("{}:p{:03}-{}", curve.id, start, len),
        q_min: window.first().unwrap().q,
        q_max: window.last().unwrap().q,
        n_points: len,
        k: fit.slope,
        k_sigma: fit.slope_sigma,
        b: fit.intercept,
        b_sigma: fit.intercept_sigma,
        chi2: fit.chi2,
        adopted: false,
    })
}

pub fn fit_models(
    adopted: Option<&SubtractedCurve>,
    params: &ModelParams,
    node_id: &str,
    adopted_guinier_id: Option<&str>,
    adopted_porod_id: Option<&str>,
) -> ModelOutput {
    let _ = node_id;
    let mut diagnostics = Vec::new();
    let Some(curve) = adopted else {
        diagnostics.push(Diagnostic::new(
            D_NO_OVERLAP,
            "error",
            "没有可用于拟合的已采纳相减曲线（上游无重叠或未采纳候选），不生成模型候选",
        ));
        return ModelOutput {
            rule_version: RULE_VERSION_MODEL.to_string(),
            source_candidate_id: None,
            guinier: Vec::new(),
            porod: Vec::new(),
            diagnostics,
        };
    };

    let mut excluded = 0usize;
    let pts = &curve.curve.points;

    let mut guinier_all = Vec::new();
    let mut start = 0usize;
    while start < pts.len() {
        if pts[start].i <= 0.0 {
            excluded += 1;
            start += 1;
            continue;
        }
        for len in params.guinier_min_points..=params.guinier_max_points {
            if let Some(c) = fit_guinier_window(curve, start, len) {
                if c.q_max * c.rg <= params.guinier_q_max_factor {
                    guinier_all.push(c);
                }
            }
        }
        start += 1;
    }
    if excluded > 0 {
        diagnostics.push(
            Diagnostic::new(
                D_GUINIER_EXCLUDED,
                "info",
                format!("{excluded} 个非正强度点被排除出 Guinier 线性化（不取对数）"),
            )
            .context(serde_json::json!({ "excluded_points": excluded })),
        );
    }
    guinier_all.sort_by(|a, b| a.chi2.partial_cmp(&b.chi2).unwrap());
    let mut guinier: Vec<GuinierCandidate> = Vec::new();
    for c in guinier_all {
        let duplicate = guinier.iter().any(|k: &GuinierCandidate| {
            (k.q_min - c.q_min).abs() < 1e-12 && (k.q_max - c.q_max).abs() < 1e-12
        });
        if !duplicate {
            guinier.push(c);
        }
        if guinier.len() >= params.keep_top {
            break;
        }
    }
    if guinier.len() > 1 {
        diagnostics.push(
            Diagnostic::new(
                D_MULTI_CANDIDATE,
                "info",
                "Guinier 区间存在多个证据相当的合法候选，全部保留；拒绝为了单一答案而吞掉多解",
            )
            .context(serde_json::json!({
                "candidate_ids": guinier.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
            })),
        );
    }

    let mut porod_all = Vec::new();
    let first_high_q = pts.iter().position(|p| p.q >= params.porod_q_min).unwrap_or(pts.len());
    for start_i in first_high_q..pts.len() {
        for len in 5..=10usize {
            if let Some(c) = fit_porod_window(curve, start_i, len) {
                if c.k > 0.0 {
                    porod_all.push(c);
                }
            }
        }
    }
    porod_all.sort_by(|a, b| a.chi2.partial_cmp(&b.chi2).unwrap());
    let mut porod: Vec<PorodCandidate> = Vec::new();
    for c in porod_all {
        let duplicate = porod.iter().any(|k| {
            (k.q_min - c.q_min).abs() < 1e-12 && (k.q_max - c.q_max).abs() < 1e-12
        });
        if !duplicate {
            porod.push(c);
        }
        if porod.len() >= params.keep_top {
            break;
        }
    }

    let overlaps: Vec<serde_json::Value> = guinier
        .iter()
        .flat_map(|g| {
            porod.iter().filter_map(move |p| {
                if g.q_min <= p.q_max && p.q_min <= g.q_max {
                    Some(serde_json::json!({
                        "guinier_id": g.id,
                        "porod_id": p.id,
                        "guinier_range": [g.q_min, g.q_max],
                        "porod_range": [p.q_min, p.q_max],
                    }))
                } else {
                    None
                }
            })
        })
        .collect();
    if !overlaps.is_empty() {
        diagnostics.push(
            Diagnostic::new(
                D_INTERVAL_OVERLAP,
                "warning",
                "Guinier 与 Porod 候选区间发生重叠：保留为两个独立区间证据，不自动拼成单一模型",
            )
            .context(serde_json::json!({ "overlaps": overlaps })),
        );
    }

    for g in &mut guinier {
        g.adopted = adopted_guinier_id == Some(g.id.as_str());
    }
    for p in &mut porod {
        p.adopted = adopted_porod_id == Some(p.id.as_str());
    }

    ModelOutput {
        rule_version: RULE_VERSION_MODEL.to_string(),
        source_candidate_id: Some(curve.id.clone()),
        guinier,
        porod,
        diagnostics,
    }
}
