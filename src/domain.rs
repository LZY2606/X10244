//! SAXS 领域语义：归一化、网格对齐、背景扣除、Guinier/Porod 区间候选。
//! 核心规则：
//! - q 网格不同时不直接相减，插值只在重叠范围生效；
//! - 误差包括归一化与背景缩放不确定度；
//! - Guinier 与 Porod 区间不自动拼成单一模型，多候选全部保留。
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub sigma: Vec<f64>,
    pub q_unit: String,
    pub i_unit: String,
    pub label: String,
}

impl Curve {
    pub fn q_range(&self) -> [f64; 2] {
        [
            self.q.first().copied().unwrap_or(f64::NAN),
            self.q.last().copied().unwrap_or(f64::NAN),
        ]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormFactors {
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub transmission: f64,
    pub rel_exposure: f64,
    pub rel_thickness: f64,
    pub rel_transmission: f64,
}

impl NormFactors {
    pub fn scale(&self) -> f64 {
        1.0 / (self.exposure_s * self.thickness_mm * self.transmission)
    }
    /// 归一化因子的合成相对不确定度。
    pub fn rel_unc(&self) -> f64 {
        (self.rel_exposure.powi(2) + self.rel_thickness.powi(2) + self.rel_transmission.powi(2)).sqrt()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub detail: serde_json::Value,
}

impl Diagnostic {
    pub fn new(code: &str, message: impl Into<String>, detail: serde_json::Value) -> Self {
        Diagnostic { code: code.to_string(), message: message.into(), detail }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "relation")]
pub enum GridRelation {
    Identical,
    Mismatch { overlap: [f64; 2], n_sample_in_overlap: usize },
    NoOverlap { sample_range: [f64; 2], buffer_range: [f64; 2] },
}

/// 判定两条 q 网格的关系。假定 q 均按升序。
pub fn grid_relation(sample_q: &[f64], buffer_q: &[f64]) -> GridRelation {
    let s_range = [
        sample_q.first().copied().unwrap_or(f64::NAN),
        sample_q.last().copied().unwrap_or(f64::NAN),
    ];
    let b_range = [
        buffer_q.first().copied().unwrap_or(f64::NAN),
        buffer_q.last().copied().unwrap_or(f64::NAN),
    ];
    let identical = sample_q.len() == buffer_q.len()
        && sample_q.iter().zip(buffer_q).all(|(a, b)| (a - b).abs() < 1e-12);
    if identical {
        return GridRelation::Identical;
    }
    let lo = s_range[0].max(b_range[0]);
    let hi = s_range[1].min(b_range[1]);
    if lo >= hi {
        return GridRelation::NoOverlap { sample_range: s_range, buffer_range: b_range };
    }
    let n = sample_q.iter().filter(|q| **q >= lo && **q <= hi).count();
    GridRelation::Mismatch { overlap: [lo, hi], n_sample_in_overlap: n }
}

/// 归一化：I' = I / (曝光 * 厚度 * 透射率)。统计误差同比缩放，
/// 归一化因子的相对不确定度单独返回，供扣除步骤合成。
pub fn normalize(curve: &Curve, f: &NormFactors) -> (Curve, f64) {
    let s = f.scale();
    let out = Curve {
        q: curve.q.clone(),
        i: curve.i.iter().map(|v| v * s).collect(),
        sigma: curve.sigma.iter().map(|v| v * s).collect(),
        q_unit: curve.q_unit.clone(),
        i_unit: "1/cm (absolute, normalized)".to_string(),
        label: format!("{} [normalized]", curve.label),
    };
    (out, f.rel_unc())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "outcome")]
pub enum SubtractOutcome {
    /// 无重叠区：拒绝相减，不产生派生曲线。
    NoOverlap { relation: GridRelation, diagnostic: Diagnostic },
    Done {
        curve: Curve,
        relation: GridRelation,
        interpolated: bool,
        negative_indices: Vec<usize>,
        diagnostics: Vec<Diagnostic>,
    },
}

fn lerp_curve(q: &[f64], v: &[f64], x: f64) -> f64 {
    match q.binary_search_by(|p| p.partial_cmp(&x).unwrap()) {
        Ok(k) => v[k],
        Err(0) => v[0],
        Err(k) if k >= q.len() => v[q.len() - 1],
        Err(k) => {
            let (q0, q1) = (q[k - 1], q[k]);
            let t = (x - q0) / (q1 - q0);
            v[k - 1] + t * (v[k] - v[k - 1])
        }
    }
}

/// 背景扣除。网格不同则仅在重叠范围内对缓冲液做线性插值；
/// 误差合成 = 统计 + 归一化不确定度 + 背景缩放不确定度。负强度保留并标记。
pub fn subtract(
    sample: &Curve,
    buffer: &Curve,
    scale: f64,
    scale_sigma: f64,
    rel_norm_sample: f64,
    rel_norm_buffer: f64,
) -> SubtractOutcome {
    let relation = grid_relation(&sample.q, &buffer.q);
    let (lo, hi) = match &relation {
        GridRelation::NoOverlap { .. } => {
            return SubtractOutcome::NoOverlap {
                relation: relation.clone(),
                diagnostic: Diagnostic::new(
                    "no_overlap",
                    "样品与缓冲液 q 网格无重叠区，禁止直接相减，未产生派生曲线",
                    serde_json::json!({ "relation": relation }),
                ),
            };
        }
        GridRelation::Identical => (sample.q[0], *sample.q.last().unwrap()),
        GridRelation::Mismatch { overlap, .. } => (overlap[0], overlap[1]),
    };
    let interpolated = !matches!(relation, GridRelation::Identical);
    let mut q = Vec::new();
    let mut i = Vec::new();
    let mut sig = Vec::new();
    let mut negatives = Vec::new();
    let mut diagnostics = Vec::new();
    if interpolated {
        diagnostics.push(Diagnostic::new(
            "grid_mismatch_interpolated",
            "q 网格不同：未直接相减，缓冲液仅在重叠范围内线性插值到样品网格",
            serde_json::json!({ "overlap": [lo, hi] }),
        ));
    }
    for (idx, (&qs, &is, &ss)) in sample.q.iter().zip(&sample.i).zip(&sample.sigma).map(|((a, b), c)| (a, b, c)).enumerate().collect::<Vec<_>>().into_iter().map(|x| x).enumerate() {
        let _ = idx;
        if qs < lo || qs > hi {
            continue; // 重叠范围之外的样品点被排除
        }
        let (ib, sb) = if interpolated {
            (lerp_curve(&buffer.q, &buffer.i, qs), lerp_curve(&buffer.q, &buffer.sigma, qs))
        } else {
            let pos = buffer.q.iter().position(|x| (*x - qs).abs() < 1e-12).unwrap();
            (buffer.i[pos], buffer.sigma[pos])
        };
        let val = is - scale * ib;
        let var = ss.powi(2)
            + (scale * sb).powi(2)
            + (ib * scale_sigma).powi(2)
            + (rel_norm_sample * is).powi(2)
            + (rel_norm_buffer * scale * ib).powi(2);
        if val < 0.0 {
            negatives.push(q.len());
        }
        q.push(qs);
        i.push(val);
        sig.push(var.sqrt());
    }
    if !negatives.is_empty() {
        diagnostics.push(Diagnostic::new(
            "negative_intensity",
            "扣背景后出现负强度点：保留原值并标记，不做截断",
            serde_json::json!({ "indices": negatives, "count": negatives.len() }),
        ));
    }
    SubtractOutcome::Done {
        curve: Curve {
            q,
            i,
            sigma: sig,
            q_unit: sample.q_unit.clone(),
            i_unit: sample.i_unit.clone(),
            label: format!("{} - buffer", sample.label),
        },
        relation,
        interpolated,
        negative_indices: negatives,
        diagnostics,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinFit {
    pub slope: f64,
    pub intercept: f64,
    pub slope_sigma: f64,
    pub intercept_sigma: f64,
    pub residuals: Vec<f64>,
    pub chi2: f64,
}

pub fn weighted_linfit(x: &[f64], y: &[f64], w: &[f64]) -> LinFit {
    let sw: f64 = w.iter().sum();
    let sx: f64 = x.iter().zip(w).map(|(a, b)| a * b).sum();
    let sy: f64 = y.iter().zip(w).map(|(a, b)| a * b).sum();
    let sxx: f64 = x.iter().zip(w).map(|(a, b)| a * a * b).sum();
    let sxy: f64 = x.iter().zip(y).zip(w).map(|((a, b), c)| a * b * c).sum();
    let d = sw * sxx - sx * sx;
    let slope = (sw * sxy - sx * sy) / d;
    let intercept = (sxx * sy - sx * sxy) / d;
    let residuals: Vec<f64> = x.iter().zip(y).map(|(a, b)| b - (slope * a + intercept)).collect();
    let chi2: f64 = residuals.iter().zip(w).map(|(r, wgt)| r * r * wgt).sum();
    LinFit {
        slope,
        intercept,
        slope_sigma: (sw / d).sqrt(),
        intercept_sigma: (sxx / d).sqrt(),
        residuals,
        chi2,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub kind: String, // "guinier" | "porod"
    pub q_lo: f64,
    pub q_hi: f64,
    pub n_points: usize,
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub params: serde_json::Value,
    pub residuals: Vec<f64>,
    pub chi2: f64,
}

/// Guinier 候选区间：在低 q 区滑动窗口拟合 ln I ~ q^2。
/// 含非正强度的窗口保留为无效候选并附诊断，不被丢弃；区间互相重叠也不合并。
pub fn guinier_candidates(c: &Curve, q_max: f64, min_pts: usize, max_pts: usize) -> Vec<IntervalCandidate> {
    let idx: Vec<usize> = (0..c.q.len()).filter(|&k| c.q[k] <= q_max).collect();
    let mut out = Vec::new();
    for si in 0..idx.len() {
        for ei in (si + min_pts)..=(idx.len().min(si + max_pts)) {
            let win = &idx[si..ei];
            let mut diagnostics = Vec::new();
            let mut valid = true;
            if win.iter().any(|&k| c.i[k] <= 0.0) {
                valid = false;
                diagnostics.push(Diagnostic::new(
                    "nonpositive_intensity",
                    "窗口内含非正强度点，ln I 无定义，候选保留但标记为无效",
                    serde_json::json!({ "window": [c.q[win[0]], c.q[*win.last().unwrap()]] }),
                ));
            }
            let (params, residuals, chi2) = if valid {
                let x: Vec<f64> = win.iter().map(|&k| c.q[k] * c.q[k]).collect();
                let y: Vec<f64> = win.iter().map(|&k| c.i[k].ln()).collect();
                let w: Vec<f64> = win.iter().map(|&k| 1.0 / (c.sigma[k] / c.i[k]).powi(2)).collect();
                let fit = weighted_linfit(&x, &y, &w);
                let rg = (-3.0 * fit.slope).max(0.0).sqrt();
                let i0 = fit.intercept.exp();
                let qrg_max = c.q[*win.last().unwrap()] * rg;
                if qrg_max > 1.3 {
                    valid = false;
                    diagnostics.push(Diagnostic::new(
                        "guinier_range",
                        "q_max*Rg > 1.3，超出 Guinier 有效区，候选保留但标记为无效",
                        serde_json::json!({ "q_max_rg": qrg_max }),
                    ));
                }
                (serde_json::json!({ "rg_angstrom": rg, "i0": i0, "rg_sigma": (3.0 * fit.slope_sigma / (2.0 * rg.max(1e-12))) }), fit.residuals, fit.chi2)
            } else {
                (serde_json::json!({}), vec![], f64::NAN)
            };
            out.push(IntervalCandidate {
                id: format!("g-{}-{}", si, ei - 1),
                kind: "guinier".into(),
                q_lo: c.q[win[0]],
                q_hi: c.q[*win.last().unwrap()],
                n_points: win.len(),
                valid,
                diagnostics,
                params,
                residuals,
                chi2,
            });
        }
    }
    out
}

/// Porod 候选区间：在高 q 区拟合 I*q^4 ~ q^4（截距=Porod 常数，斜率=平背景）。
pub fn porod_candidates(c: &Curve, q_min: f64, min_pts: usize, max_pts: usize) -> Vec<IntervalCandidate> {
    let idx: Vec<usize> = (0..c.q.len()).filter(|&k| c.q[k] >= q_min).collect();
    let mut out = Vec::new();
    for si in 0..idx.len() {
        for ei in (si + min_pts)..=(idx.len().min(si + max_pts)) {
            let win = &idx[si..ei];
            let x: Vec<f64> = win.iter().map(|&k| c.q[k].powi(4)).collect();
            let y: Vec<f64> = win.iter().map(|&k| c.i[k] * c.q[k].powi(4)).collect();
            let w: Vec<f64> = win.iter().map(|&k| 1.0 / (c.sigma[k] * c.q[k].powi(4)).powi(2)).collect();
            let fit = weighted_linfit(&x, &y, &w);
            out.push(IntervalCandidate {
                id: format!("p-{}-{}", si, ei - 1),
                kind: "porod".into(),
                q_lo: c.q[win[0]],
                q_hi: c.q[*win.last().unwrap()],
                n_points: win.len(),
                valid: true,
                diagnostics: vec![],
                params: serde_json::json!({
                    "porod_constant": fit.intercept,
                    "flat_background": fit.slope,
                    "porod_constant_sigma": fit.intercept_sigma,
                }),
                residuals: fit.residuals,
                chi2: fit.chi2,
            });
        }
    }
    out
}
