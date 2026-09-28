//! 核心散射规则（纯函数，可测试）：
//! - q 网格不同时不直接相减，插值只在重叠范围生效
//! - 误差包括归一化与背景缩放不确定度
//! - Guinier 与 Porod 区间不自动拼成单一模型
//! - 负强度保留并标记，不 clamp
use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurveData {
    pub q: Vec<f64>,     // 规范单位 1/angstrom
    pub i: Vec<f64>,     // 规范单位 1/cm
    pub err: Vec<f64>,   // 1/cm
    pub q_unit: String,  // 原始单位（保留原始表示）
    pub i_unit: String,
}

impl CurveData {
    pub fn len(&self) -> usize { self.q.len() }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormParams {
    pub exposure_s: f64,
    pub thickness_cm: f64,
    pub transmission: f64,
    /// 各归一化因子的相对不确定度（曝光、厚度、透射率）
    pub rel_unc: [f64; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedData {
    pub curve: CurveData,
    pub norm_factor: f64,
    /// 归一化引入的相对不确定度（sqrt 合成）
    pub norm_rel_unc: f64,
}

/// 归一化：I_norm = I_raw / (exposure * thickness * transmission)
/// 归一化不确定度以相对形式合成，随曲线带入后续误差传播。
pub fn normalize(raw: &CurveData, p: &NormParams) -> Result<NormalizedData, String> {
    if p.exposure_s <= 0.0 || p.thickness_cm <= 0.0 || p.transmission <= 0.0 {
        return Err("normalization factors must be positive".into());
    }
    let f = p.exposure_s * p.thickness_cm * p.transmission;
    let rel = (p.rel_unc[0].powi(2) + p.rel_unc[1].powi(2) + p.rel_unc[2].powi(2)).sqrt();
    Ok(NormalizedData {
        curve: CurveData {
            q: raw.q.clone(),
            i: raw.i.iter().map(|v| v / f).collect(),
            err: raw.err.iter().map(|v| v / f).collect(),
            q_unit: raw.q_unit.clone(),
            i_unit: "1/cm".into(),
        },
        norm_factor: f,
        norm_rel_unc: rel,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtractionParams {
    pub scale: f64,
    /// 背景缩放因子的绝对不确定度
    pub scale_unc: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtractedPoint {
    pub q: f64,
    pub i: f64,          // 可能为负：保留并标记
    pub err: f64,        // 含统计 + 归一化 + 背景缩放不确定度
    pub negative: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtractionResult {
    pub points: Vec<SubtractedPoint>,
    pub overlap: Option<(f64, f64)>,
    pub verdicts: Vec<Verdict>,
    pub scale: f64,
    pub scale_unc: f64,
}

fn grids_equal(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-12)
}

/// 线性插值（仅在 x 位于 xs 范围内时调用）。
fn interp(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    match xs.binary_search_by(|v| v.partial_cmp(&x).unwrap()) {
        Ok(k) => ys[k],
        Err(0) => ys[0],
        Err(k) if k >= xs.len() => ys[xs.len() - 1],
        Err(k) => {
            let (x0, x1) = (xs[k - 1], xs[k]);
            let t = (x - x0) / (x1 - x0);
            ys[k - 1] + t * (ys[k] - ys[k - 1])
        }
    }
}

/// 背景扣除。
/// 规则：q 网格不同时不直接相减——把缓冲液插值到样品 q 上，且插值只在重叠范围生效；
/// 重叠范围外的样品点不产出结果（不外推）。无重叠区 → Blocked。
/// 误差：err = sqrt(err_s^2 + (scale*err_b)^2 + (I_b*scale_unc)^2 + (I_s*norm_rel_unc)^2)
pub fn subtract(
    sample: &NormalizedData,
    buffer: &CurveData,
    params: &SubtractionParams,
    sample_ref: &NodeRef,
    buffer_ref: &NodeRef,
) -> SubtractionResult {
    let s = &sample.curve;
    let mut verdicts = Vec::new();
    let qlo = s.q[0].max(buffer.q[0]);
    let qhi = s.q[s.len() - 1].min(buffer.q[buffer.q.len() - 1]);

    if !(qlo < qhi) {
        verdicts.push(Verdict {
            code: "no_overlap".into(),
            status: VerdictStatus::Blocked,
            detail: format!(
                "样品 q 范围 [{:.4}, {:.4}] 与缓冲液 [{:.4}, {:.4}] 无重叠区，禁止相减",
                s.q[0], s.q[s.len() - 1], buffer.q[0], buffer.q[buffer.q.len() - 1]
            ),
            affected: vec![sample_ref.clone(), buffer_ref.clone()],
        });
        return SubtractionResult { points: vec![], overlap: None, verdicts, scale: params.scale, scale_unc: params.scale_unc };
    }

    let same_grid = grids_equal(&s.q, &buffer.q);
    if !same_grid {
        verdicts.push(Verdict {
            code: "grid_mismatch_interpolated".into(),
            status: VerdictStatus::Warning,
            detail: format!(
                "q 网格不同，不直接相减；缓冲液仅在重叠范围 [{:.4}, {:.4}] 1/A 内插值到样品网格",
                qlo, qhi
            ),
            affected: vec![buffer_ref.clone()],
        });
    }

    let mut points = Vec::new();
    let mut neg_count = 0usize;
    for k in 0..s.len() {
        let q = s.q[k];
        if q < qlo || q > qhi {
            continue; // 重叠范围外不产出
        }
        let (bi, berr) = if same_grid {
            (buffer.i[k], buffer.err[k])
        } else {
            (interp(&buffer.q, &buffer.i, q), interp(&buffer.q, &buffer.err, q))
        };
        let i = s.i[k] - params.scale * bi;
        let var = s.err[k].powi(2)
            + (params.scale * berr).powi(2)
            + (bi.abs() * params.scale_unc).powi(2)
            + (s.i[k].abs() * sample.norm_rel_unc).powi(2);
        let negative = i < 0.0;
        if negative { neg_count += 1; }
        points.push(SubtractedPoint { q, i, err: var.sqrt(), negative });
    }

    if neg_count > 0 {
        verdicts.push(Verdict {
            code: "negative_intensity".into(),
            status: VerdictStatus::Warning,
            detail: format!("{} 个点的扣除后强度为负，已保留原值并标记（不 clamp）", neg_count),
            affected: vec![sample_ref.clone()],
        });
    }
    if params.scale_unc > 0.0 {
        verdicts.push(Verdict {
            code: "scale_uncertainty_propagated".into(),
            status: VerdictStatus::Ok,
            detail: format!("背景缩放不确定度 {:.4} 已计入误差带", params.scale_unc),
            affected: vec![buffer_ref.clone()],
        });
    }

    SubtractionResult { points, overlap: Some((qlo, qhi)), verdicts, scale: params.scale, scale_unc: params.scale_unc }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntervalKind { Guinier, Porod }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub kind: IntervalKind,
    pub q_min: f64,
    pub q_max: f64,
    pub params: serde_json::Value, // Guinier: {rg, i0}; Porod: {exponent, const}
    pub fit_ok: bool,
    pub residual: f64,             // 归一化残差指标
    pub overlaps_other: bool,      // 区间重叠：保留，不自动合并
}

/// Guinier 拟合：ln I = ln I0 - (Rg^2/3) q^2，在线性化区间做加权最小二乘。
pub fn fit_guinier(points: &[SubtractedPoint], q_min: f64, q_max: f64, id: &str) -> IntervalCandidate {
    let mut sx = 0.0; let mut sy = 0.0; let mut sxx = 0.0; let mut sxy = 0.0; let mut sw = 0.0;
    let mut n = 0usize;
    for p in points {
        if p.q < q_min || p.q > q_max || p.i <= 0.0 || p.err <= 0.0 { continue; }
        let x = p.q * p.q;
        let y = p.i.ln();
        let w = (p.i / p.err).powi(2); // y 的方差 ≈ (err/I)^2
        sx += w * x; sy += w * y; sxx += w * x * x; sxy += w * x * y; sw += w;
        n += 1;
    }
    let det = sw * sxx - sx * sx;
    let (fit_ok, rg, i0, residual) = if n >= 3 && det.abs() > 1e-30 {
        let slope = (sw * sxy - sx * sy) / det;
        let intercept = (sy - slope * sx) / sw;
        let rg = if slope < 0.0 { (-3.0 * slope).sqrt() } else { f64::NAN };
        // 归一化残差
        let mut chi = 0.0;
        for p in points {
            if p.q < q_min || p.q > q_max || p.i <= 0.0 { continue; }
            let r = p.i.ln() - (intercept + slope * p.q * p.q);
            chi += r * r;
        }
        (rg.is_finite(), rg, intercept.exp(), (chi / n as f64).sqrt())
    } else {
        (false, f64::NAN, f64::NAN, f64::NAN)
    };
    IntervalCandidate {
        id: id.into(),
        kind: IntervalKind::Guinier,
        q_min, q_max,
        params: serde_json::json!({"rg_angstrom": rg, "i0": i0}),
        fit_ok, residual, overlaps_other: false,
    }
}

/// Porod 拟合：ln I = ln C - d ln q。
pub fn fit_porod(points: &[SubtractedPoint], q_min: f64, q_max: f64, id: &str) -> IntervalCandidate {
    let mut sx = 0.0; let mut sy = 0.0; let mut sxx = 0.0; let mut sxy = 0.0; let mut sw = 0.0;
    let mut n = 0usize;
    for p in points {
        if p.q < q_min || p.q > q_max || p.i <= 0.0 || p.err <= 0.0 { continue; }
        let x = p.q.ln();
        let y = p.i.ln();
        let w = (p.i / p.err).powi(2);
        sx += w * x; sy += w * y; sxx += w * x * x; sxy += w * x * y; sw += w;
        n += 1;
    }
    let det = sw * sxx - sx * sx;
    let (fit_ok, d, c, residual) = if n >= 3 && det.abs() > 1e-30 {
        let slope = (sw * sxy - sx * sy) / det;
        let intercept = (sy - slope * sx) / sw;
        let mut chi = 0.0;
        for p in points {
            if p.q < q_min || p.q > q_max || p.i <= 0.0 { continue; }
            let r = p.i.ln() - (intercept + slope * p.q.ln());
            chi += r * r;
        }
        (true, -slope, intercept.exp(), (chi / n as f64).sqrt())
    } else {
        (false, f64::NAN, f64::NAN, f64::NAN)
    };
    IntervalCandidate {
        id: id.into(),
        kind: IntervalKind::Porod,
        q_min, q_max,
        params: serde_json::json!({"porod_exponent": d, "constant": c}),
        fit_ok, residual, overlaps_other: false,
    }
}

/// 区间候选集合：Guinier 与 Porod 不自动拼成单一模型；重叠区间只标记。
/// 多候选残差检验：若多个同类候选残差在 2 倍以内无法区分，产出 Multi 判定，保留全部合法候选。
pub fn evaluate_intervals(mut candidates: Vec<IntervalCandidate>) -> (Vec<IntervalCandidate>, Vec<Verdict>) {
    let mut verdicts = Vec::new();
    for a in 0..candidates.len() {
        for b in (a + 1)..candidates.len() {
            let (ca, cb) = (candidates[a].clone(), candidates[b].clone());
            if ca.q_min < cb.q_max && cb.q_min < ca.q_max {
                candidates[a].overlaps_other = true;
                candidates[b].overlaps_other = true;
            }
        }
    }
    if candidates.iter().any(|c| c.overlaps_other) {
        verdicts.push(Verdict {
            code: "interval_overlap".into(),
            status: VerdictStatus::Warning,
            detail: "Guinier/Porod 区间存在重叠；候选分别保留，不自动拼成单一模型".into(),
            affected: vec![],
        });
    }
    // 多候选残差检验（按类型分组）
    for kind in [IntervalKind::Guinier, IntervalKind::Porod] {
        let mut group: Vec<&IntervalCandidate> = candidates.iter()
            .filter(|c| c.kind == kind && c.fit_ok).collect();
        if group.len() > 1 {
            group.sort_by(|a, b| a.residual.partial_cmp(&b.residual).unwrap());
            let best = group[0].residual;
            let ambiguous: Vec<&&IntervalCandidate> = group.iter().filter(|c| c.residual <= 2.0 * best.max(1e-12)).collect();
            if ambiguous.len() > 1 {
                verdicts.push(Verdict {
                    code: "multi_candidate_residual".into(),
                    status: VerdictStatus::Multi,
                    detail: format!(
                        "{:?} 有 {} 个候选残差在 2 倍以内（最优 {:.4}），无法唯一决定，全部保留",
                        kind, ambiguous.len(), best
                    ),
                    affected: vec![],
                });
            }
        }
    }
    (candidates, verdicts)
}
