//! 核心规则（纯函数，可独立测试）：
//! 1. 归一化 I_norm = I_raw / (t_exp * d * T * R)，误差含归一化不确定度；
//! 2. 背景扣除：q 网格不同时不直接相减，缓冲液只在重叠范围内线性插值；
//!    缩放不确定度进入每个点的误差；多缩放候选全部保留；
//! 3. 多区间模型：Guinier 与 Porod 各自在滑窗上拟合，永不自动拼接。

use crate::core::id::short_hash;
use crate::core::model::{
    Curve, Dataset, Diagnostic, ModelCandidate, ModelFit, ModelKind, Normalized, ScaleCandidate,
    SubPoint, Subtraction,
};
use crate::units::CanonicalUnit;

fn diag(code: &str, severity: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic { code: code.into(), severity: severity.into(), message: message.into() }
}

/// q 坐标按曲线声明的基准换算到 1/A（nm^-1 → 除以 10）。
fn q_to_inv_a(dataset: &Dataset, q: f64) -> f64 {
    if dataset.q_unit_raw.to_lowercase().contains("nm") {
        q * 0.1
    } else {
        q
    }
}

/// 原始计数 → 归一化绝对/相对尺度。
/// 返回 Err 仅当必需的转换缺失；此时原始材料仍保留，调用方把诊断挂到结果上。
pub fn normalize(dataset: &Dataset) -> Result<Normalized, Vec<Diagnostic>> {
    let mut problems = Vec::new();
    let exposure = match dataset.exposure.value {
        Some(v) if v > 0.0 => v,
        _ => {
            problems.push(diag("EXPOSURE_INVALID", "error", "曝光时间缺失或非正，无法归一化"));
            f64::NAN
        }
    };
    let thickness = match (dataset.role, dataset.thickness.value) {
        (crate::core::model::DatasetRole::Sample, Some(v)) if v > 0.0 => v,
        (crate::core::model::DatasetRole::Buffer, _) => 1.0,
        (_, _) => {
            problems.push(diag("THICKNESS_INVALID", "error", "样品厚度缺失或非正，无法归一化"));
            f64::NAN
        }
    };
    let transmission = match dataset.transmission.value {
        Some(v) if v > 0.0 => v,
        _ => {
            problems.push(diag("TRANSMISSION_INVALID", "error", "透射率缺失或非正，无法归一化"));
            f64::NAN
        }
    };
    let response = dataset.response.value.unwrap_or(1.0).max(1e-12);
    if !problems.is_empty() {
        return Err(problems);
    }

    let monitor = exposure * thickness * transmission * response;
    let abs = dataset.absolute.factor.unwrap_or(1.0);
    let norm_factor = abs / monitor;

    // 归一化相对不确定度：透射率相对误差（曝光/厚度/响应按零处理，除非原始声明）。
    let norm_rel_sigma = dataset.transmission_rel_err.max(0.0);

    let mut q = Vec::with_capacity(dataset.points.len());
    let mut i = Vec::new();
    let mut sigma = Vec::new();
    let mut masked = Vec::new();
    let mut source_line = Vec::new();
    let mut diagnostics = Vec::new();
    for (idx, p) in dataset.points.iter().enumerate() {
        let qv = q_to_inv_a(dataset, p.q);
        if !qv.is_finite() || !p.i.is_finite() || !p.sigma.is_finite() {
            diagnostics.push(diag(
                "NONFINITE_POINT",
                "warn",
                format!("第 {} 行存在非有限值，派生层标记为遮罩并保留原始串", p.line_no),
            ));
        }
        q.push(qv);
        let iv = p.i * norm_factor;
        // sigma_norm 同时包含统计误差与归一化尺度不确定度。
        let sv = (p.sigma * norm_factor).hypot(iv * norm_rel_sigma);
        i.push(iv);
        sigma.push(if sv.is_finite() { sv } else { f64::NAN });
        masked.push(p.masked || !qv.is_finite() || !p.i.is_finite() || !p.sigma.is_finite());
        source_line.push(p.line_no);
        let _ = idx;
    }

    let i_unit = match dataset.absolute.factor {
        Some(_) => CanonicalUnit::IcmInv.label().to_string(),
        None => format!("{} (norm. rel.)", dataset.i_unit_raw),
    };
    Ok(Normalized {
        curve: Curve { q, i, sigma, masked, i_unit, source_line },
        monitor,
        formula: "I_norm = I_raw * abs / (exposure * thickness * transmission * response)".into(),
        norm_factor,
        norm_rel_sigma,
    })
}

/// 在严格单调网格上做线性插值（只用于重叠范围内部）。
fn interpolate(xs: &[f64], ys: &[f64], x: f64) -> Option<(f64, usize, usize)> {
    if xs.len() < 2 || x < xs[0] || x > xs[xs.len() - 1] {
        return None;
    }
    // xs 单调递增，二分查找
    let mut lo = 0usize;
    let mut hi = xs.len() - 1;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if xs[mid] <= x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let (x0, x1) = (xs[lo], xs[hi]);
    if (x1 - x0).abs() < f64::EPSILON {
        return None;
    }
    let w = (x - x0) / (x1 - x0);
    Some((ys[lo] * (1.0 - w) + ys[hi] * w, lo, hi))
}

fn monotonic(q: &[f64]) -> bool {
    q.windows(2).all(|w| w[1] > w[0])
}

/// 在指定窗口内以加权最小二乘估计背景缩放 k：min Σ (S_i - k B_i)^2 / σS_i²。
/// 缓冲液点由插值得到；只统计未遮罩样品点。
fn estimate_scale(
    sample: &Curve,
    buffer: &Curve,
    q_min: f64,
    q_max: f64,
    method: &str,
) -> Option<ScaleCandidate> {
    let mut num = 0.0;
    let mut den = 0.0;
    let mut n = 0usize;
    for (j, &qs) in sample.q.iter().enumerate() {
        if sample.masked[j] || qs < q_min || qs > q_max {
            continue;
        }
        let (bi, _, _) = interpolate(&buffer.q, &buffer.i, qs)?;
        let (bsi, _, _) = interpolate(&buffer.q, &buffer.sigma, qs)?;
        let s2 = sample.sigma[j].powi(2);
        // 背景统计误差通过插值近似进入权重，但 scale 用样品误差加权（标准做法）。
        let w = 1.0 / (s2 + bsi.powi(2).max(1e-30));
        num += w * sample.i[j] * bi;
        den += w * bi * bi;
        n += 1;
    }
    if n < 3 || den <= 0.0 {
        return None;
    }
    let scale = num / den;
    // k 的方差 = 1 / Σ w B²（线性 WLS）。
    let scale_var = 1.0 / den;
    // 在同一窗口评估残差 chi2（含两边统计误差）。
    let mut chi2 = 0.0;
    for (j, &qs) in sample.q.iter().enumerate() {
        if sample.masked[j] || qs < q_min || qs > q_max {
            continue;
        }
        let (bi, _, _) = interpolate(&buffer.q, &buffer.i, qs)?;
        let (bsi, _, _) = interpolate(&buffer.q, &buffer.sigma, qs)?;
        let var = sample.sigma[j].powi(2) + (scale * bsi).powi(2);
        let r = sample.i[j] - scale * bi;
        chi2 += r * r / var.max(1e-30);
    }
    Some(ScaleCandidate {
        id: short_hash(&[method, &q_min.to_bits().to_string(), &q_max.to_bits().to_string()]),
        method: method.into(),
        q_min,
        q_max,
        scale,
        scale_sigma: scale_var.sqrt(),
        chi2,
        dof: n.saturating_sub(1),
        n_points: n,
    })
}

/// 估计重叠区内所有背景缩放候选，并返回建议采纳下标。
/// 候选 1：高 q 尾 55% 段；候选 2：重叠区整体。诊断随候选返回。
pub fn scale_candidates(
    sample: &Normalized,
    buffer: &Normalized,
) -> Result<(Vec<ScaleCandidate>, usize, Vec<Diagnostic>), Vec<Diagnostic>> {
    let (sc, bc) = (&sample.curve, &buffer.curve);
    if !monotonic(&sc.q) || !monotonic(&bc.q) {
        return Err(vec![diag(
            "GRID_NOT_MONOTONIC",
            "error",
            "q 网格非严格单调，拒绝插值与相减",
        )]);
    }
    let lo = sc.q[0].max(bc.q[0]);
    let hi = sc.q[sc.q.len() - 1].min(bc.q[bc.q.len() - 1]);
    if hi <= lo {
        return Err(vec![diag(
            "NO_OVERLAP",
            "error",
            format!(
                "样品[{},{}] 与缓冲液[{},{}] 无重叠 q 范围，拒绝背景扣除",
                sc.q[0], sc.q[sc.q.len()-1], bc.q[0], bc.q[bc.q.len()-1]
            ),
        )]);
    }
    let span = hi - lo;
    let mut candidates = Vec::new();
    if let Some(c) = estimate_scale(sc, bc, lo + span * 0.55, hi, "highq_overlap") {
        candidates.push(c);
    }
    if let Some(c) = estimate_scale(sc, bc, lo, hi, "full_overlap") {
        candidates.push(c);
    }
    if candidates.is_empty() {
        return Err(vec![diag(
            "SCALE_UNDETERMINED",
            "error",
            "重叠范围内有效点不足，无法估计背景缩放",
        )]);
    }
    let mut diagnostics = Vec::new();
    let best_idx = candidates
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (a.1.chi2 / a.1.dof.max(1) as f64)
                .partial_cmp(&(b.1.chi2 / b.1.dof.max(1) as f64))
                .unwrap()
        })
        .unwrap()
        .0;
    let best_red = candidates[best_idx].chi2 / candidates[best_idx].dof.max(1) as f64;
    let ambiguous: Vec<&ScaleCandidate> = candidates
        .iter()
        .filter(|c| {
            let red = c.chi2 / c.dof.max(1) as f64;
            (red - best_red).abs() <= 0.2 * best_red.max(1e-12)
        })
        .collect();
    if ambiguous.len() > 1 {
        let ids: Vec<String> = ambiguous.iter().map(|c| c.id.clone()).collect();
        diagnostics.push(diag(
            "SCALE_MULTI_CANDIDATE",
            "warn",
            format!(
                "背景缩放存在 {} 个难分高下的候选 {:?}（归约卡方相差 ≤20%），全部保留待裁定",
                ambiguous.len(), ids
            ),
        ));
    }
    let rel = candidates[best_idx].scale_sigma / candidates[best_idx].scale.abs().max(1e-12);
    if rel > 0.10 {
        diagnostics.push(diag(
            "SCALE_UNCERTAIN",
            "warn",
            format!("背景缩放相对不确定度 {:.1}% > 10%，已进入逐点误差", rel * 100.0),
        ));
    }
    Ok((candidates, best_idx, diagnostics))
}

/// 用指定缩放候选在样品网格上执行扣除（插值仅在重叠范围）。
pub fn apply_subtraction(
    sample: &Normalized,
    buffer: &Normalized,
    chosen: &ScaleCandidate,
    sample_role_name: &str,
    buffer_name: &str,
    mut diagnostics: Vec<Diagnostic>,
) -> Subtraction {
    let (sc, bc) = (&sample.curve, &buffer.curve);
    let lo = sc.q[0].max(bc.q[0]);
    let hi = sc.q[sc.q.len() - 1].min(bc.q[bc.q.len() - 1]);
    let same_grid = sc.q.len() == bc.q.len()
        && sc.q.iter().zip(&bc.q).all(|(a, b)| (a - b).abs() < 1e-9);
    let mut points = Vec::with_capacity(sc.q.len());
    for (j, &qs) in sc.q.iter().enumerate() {
        let mut flags = Vec::new();
        if qs < lo || qs > hi {
            flags.push("OUTSIDE_OVERLAP".into());
        }
        if sc.masked[j] {
            flags.push("SAMPLE_MASKED".into());
        }
        let interp = interpolate(&bc.q, &bc.i, qs);
        let interp_s = interpolate(&bc.q, &bc.sigma, qs);
        let buffer_lines = interp.map(|(_, a, b)| (bc.source_line[a], bc.source_line[b]));
        let interpolated = interp.is_some() && !same_grid;
        let (i_sub, sigma_sub) = match (interp, interp_s) {
            (Some((bi, _, _)), Some((bs, _, _)))
                if !sc.masked[j] && (lo..=hi).contains(&qs) =>
            {
                let i_sub = sc.i[j] - chosen.scale * bi;
                let var_sample = sc.sigma[j].powi(2);
                let var_buffer = chosen.scale.powi(2) * bs.powi(2);
                let scale_term = (chosen.scale_sigma * bi).powi(2);
                let sigma_sub = (var_sample + var_buffer + scale_term).max(0.0).sqrt();
                if i_sub < 0.0 {
                    flags.push("NEGATIVE_INTENSITY".into());
                }
                (i_sub, sigma_sub)
            }
            _ => {
                if qs < lo || qs > hi {
                    diagnostics.push(diag(
                        "POINT_OUTSIDE_OVERLAP",
                        "info",
                        format!("q={:.5} 不在重叠范围，保留未扣除样品强度", qs),
                    ));
                }
                (sc.i[j], sc.sigma[j])
            }
        };
        points.push(SubPoint {
            q: qs,
            i: i_sub,
            sigma: sigma_sub,
            masked: sc.masked[j],
            sample_line: sc.source_line[j],
            buffer_lines,
            interpolated,
            flags,
        });
    }
    diagnostics.push(diag(
        "SCALE_ADOPTED",
        "info",
        format!(
            "采纳缩放候选 {}（{}）：k={:.5}±{:.5}，依据 {} vs {}",
            chosen.id, chosen.method, chosen.scale, chosen.scale_sigma,
            sample_role_name, buffer_name
        ),
    ));
    Subtraction {
        scale_candidate_id: chosen.id.clone(),
        scale: chosen.scale,
        scale_sigma: chosen.scale_sigma,
        points,
        overlap: Some((lo, hi)),
        diagnostics,
    }
}

/// 背景扣除。
///
/// 语义保证：
/// - q 网格不同时**不直接相减**：缓冲液在样品网格上线性插值；
/// - 插值只在两条曲线的重叠 q 范围生效，范围外的样品点保留为未扣除（带诊断）；
/// - 逐点误差同时包含统计、样品归一化、缓冲液归一化与**背景缩放不确定度**；
/// - 缩放候选 ≥1；多个难分高下的候选全部保留（不吞掉多解）；
/// - 负值点不裁剪、不删除，只打标记。
pub fn subtract_background(
    sample: &Normalized,
    buffer: &Normalized,
    sample_role_name: &str,
    buffer_name: &str,
) -> Result<Subtraction, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let (sc, bc) = (&sample.curve, &buffer.curve);
    if !monotonic(&sc.q) || !monotonic(&bc.q) {
        return Err(vec![diag(
            "GRID_NOT_MONOTONIC",
            "error",
            "q 网格非严格单调，拒绝插值与相减",
        )]);
    }

    let lo = sc.q[0].max(bc.q[0]);
    let hi = sc.q[sc.q.len() - 1].min(bc.q[bc.q.len() - 1]);
    if hi <= lo {
        return Err(vec![diag(
            "NO_OVERLAP",
            "error",
            format!(
                "样品[{},{}] 与缓冲液[{},{}] 无重叠 q 范围，拒绝背景扣除",
                sc.q[0],
                sc.q[sc.q.len() - 1],
                bc.q[0],
                bc.q[bc.q.len() - 1]
            ),
        )]);
    }
    let same_grid = sc.q.len() == bc.q.len()
        && sc.q.iter().zip(&bc.q).all(|(a, b)| (a - b).abs() < 1e-9);
    if !same_grid {
        diagnostics.push(diag(
            "GRID_MISMATCH",
            "warn",
            format!(
                "q 网格不同（样品 {} 点 vs 缓冲液 {} 点）：不直接相减，缓冲液仅在重叠 [{:.5},{:.5}] 内插值",
                sc.q.len(),
                bc.q.len(),
                lo,
                hi
            ),
        ));
    }

    // 候选 1：重叠区高 q 尾（Porod 区，受分子散射影响小）。
    let span = hi - lo;
    let hiwin_lo = lo + span * 0.55;
    // 候选 2：保守的最尾段。
    let tail_lo = lo + span * 0.8;
    let mut candidates = Vec::new();
    if let Some(c) = estimate_scale(sc, bc, hiwin_lo, hi, "highq_overlap") {
        candidates.push(c);
    }
    if let Some(c) = estimate_scale(sc, bc, tail_lo, hi, "tail_overlap") {
        candidates.push(c);
    }
    let adopted = match candidates.len() {
        0 => {
            return Err(vec![diag(
                "SCALE_UNDETERMINED",
                "error",
                "重叠范围内有效点不足，无法估计背景缩放",
            )])
        }
        _ => {
            // 多候选：归约卡方最优者被采纳；相对差异 ≤20% 时报告多解。
            let best_idx = candidates
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    (a.1.chi2 / a.1.dof.max(1) as f64)
                        .partial_cmp(&(b.1.chi2 / b.1.dof.max(1) as f64))
                        .unwrap()
                })
                .unwrap()
                .0;
            let best_red = candidates[best_idx].chi2 / candidates[best_idx].dof.max(1) as f64;
            let ambiguous: Vec<&ScaleCandidate> = candidates
                .iter()
                .filter(|c| {
                    let red = c.chi2 / c.dof.max(1) as f64;
                    (red - best_red).abs() <= 0.2 * best_red.max(1e-12)
                })
                .collect();
            if ambiguous.len() > 1 {
                let ids: Vec<String> = ambiguous.iter().map(|c| c.id.clone()).collect();
                diagnostics.push(diag(
                    "SCALE_MULTI_CANDIDATE",
                    "warn",
                    format!(
                        "背景缩放存在 {} 个难分高下的候选 {:?}（归约卡方相差 ≤20%），全部保留待裁定",
                        ambiguous.len(),
                        ids
                    ),
                ));
            }
            let rel = candidates[best_idx].scale_sigma / candidates[best_idx].scale.abs().max(1e-12);
            if rel > 0.10 {
                diagnostics.push(diag(
                    "SCALE_UNCERTAIN",
                    "warn",
                    format!("背景缩放相对不确定度 {:.1}% > 10%，已进入逐点误差", rel * 100.0),
                ));
            }
            best_idx
        }
    };

    let chosen = candidates[adopted].clone();
    let mut points = Vec::with_capacity(sc.q.len());
    for (j, &qs) in sc.q.iter().enumerate() {
        let mut flags = Vec::new();
        if qs < lo || qs > hi {
            flags.push("OUTSIDE_OVERLAP".into());
        }
        if sc.masked[j] {
            flags.push("SAMPLE_MASKED".into());
        }
        let interp = interpolate(&bc.q, &bc.i, qs);
        let interp_s = interpolate(&bc.q, &bc.sigma, qs);
        let buffer_lines = interp.map(|(_, a, b)| (bc.source_line[a], bc.source_line[b]));
        let interpolated = interp.is_some() && !same_grid;
        let (i_sub, sigma_sub) = match (interp, interp_s) {
            (Some((bi, _, _)), Some((bs, _, _))) if !sc.masked[j] && (lo..=hi).contains(&qs) => {
                let i_sub = sc.i[j] - chosen.scale * bi;
                // 归一化曲线上的 σ 已含统计误差与归一化尺度不确定度。
                let var_sample = sc.sigma[j].powi(2);
                let var_buffer = chosen.scale.powi(2) * bs.powi(2);
                // 背景缩放不确定度：(σk · B)²
                let scale_term = (chosen.scale_sigma * bi).powi(2);
                let sigma_sub = (var_sample + var_buffer + scale_term).max(0.0).sqrt();
                if i_sub < 0.0 {
                    flags.push("NEGATIVE_INTENSITY".into());
                }
                (i_sub, sigma_sub)
            }
            _ => {
                // 无重叠/遮罩：保留样品归一化值，标记未扣除，绝不伪造背景。
                if qs < lo || qs > hi {
                    diagnostics.push(diag(
                        "POINT_OUTSIDE_OVERLAP",
                        "info",
                        format!("q={:.5} 不在重叠范围，保留未扣除样品强度", qs),
                    ));
                }
                (sc.i[j], sc.sigma[j])
            }
        };
        points.push(SubPoint {
            q: qs,
            i: i_sub,
            sigma: sigma_sub,
            masked: sc.masked[j],
            sample_line: sc.source_line[j],
            buffer_lines,
            interpolated,
            flags,
        });
    }

    diagnostics.push(diag(
        "SCALE_ADOPTED",
        "info",
        format!(
            "采纳缩放候选 {}（{}）：k={:.5}±{:.5}，依据 {} vs {}",
            chosen.id, chosen.method, chosen.scale, chosen.scale_sigma, sample_role_name, buffer_name
        ),
    ));
    Ok(Subtraction {
        scale_candidate_id: chosen.id,
        scale: chosen.scale,
        scale_sigma: chosen.scale_sigma,
        points,
        overlap: Some((lo, hi)),
        diagnostics,
    })
}

/// 加权线性回归 y = a + b x，返回 (a, b, σa, σb, chi2)。
fn wls(x: &[f64], y: &[f64], w: &[f64]) -> Option<(f64, f64, f64, f64, f64)> {
    let n = x.len() as f64;
    let (mut sw, mut swx, mut swy, mut swxx, mut swxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for k in 0..x.len() {
        sw += w[k];
        swx += w[k] * x[k];
        swy += w[k] * y[k];
        swxx += w[k] * x[k] * x[k];
        swxy += w[k] * x[k] * y[k];
    }
    let det = sw * swxx - swx * swx;
    if det.abs() < 1e-18 {
        return None;
    }
    let a = (swy * swxx - swx * swxy) / det;
    let b = (sw * swxy - swx * swy) / det;
    let var_a = swxx / det;
    let var_b = sw / det;
    let mut chi2 = 0.0;
    for k in 0..x.len() {
        let r = y[k] - (a + b * x[k]);
        chi2 += w[k] * r * r;
    }
    Some((a, b, var_a.sqrt(), var_b.sqrt(), chi2 * n / (n - 2.0).max(1.0)))
}

/// 收集一条扣除曲线上所有未遮罩、非负、有限的点（负值不能进 log 拟合）。
fn positive_points(sub: &Subtraction) -> Vec<(f64, f64, f64)> {
    sub.points
        .iter()
        .filter(|p| {
            !p.masked
                && p.i > 0.0
                && p.sigma > 0.0
                && p.q.is_finite()
                && !p.flags.contains(&"OUTSIDE_OVERLAP".to_string())
        })
        .map(|p| (p.q, p.i, p.sigma))
        .collect()
}

/// 多区间模型证据：Guinier lnI = lnI0 - q²Rg²/3；Porod lnI = lnC - p·lnq。
/// 两个族各自枚举滑窗候选，区间重叠被显式标注，族之间不做任何自动拼接。
pub fn fit_models(sub: &Subtraction) -> ModelFit {
    let mut diagnostics = Vec::new();
    let pts = positive_points(sub);
    if pts.iter().any(|(_, i, _)| *i <= 0.0) || pts.len() < 5 {
        diagnostics.push(diag(
            "FIT_TOO_FEW_POINTS",
            "warn",
            "扣除曲线有效正强度点不足 5 个，模型候选可能缺失",
        ));
    }
    let neg = sub.points.iter().filter(|p| p.flags.contains(&"NEGATIVE_INTENSITY".to_string())).count();
    if neg > 0 {
        diagnostics.push(diag(
            "NEGATIVE_EXCLUDED_FROM_FIT",
            "info",
            format!("{} 个负强度点保留在曲线上但不进入 log 域拟合", neg),
        ));
    }

    let mut candidates: Vec<ModelCandidate> = Vec::new();

    // ---- Guinier 滑窗：qRg <= 1.3 的自洽约束在拟合后检查 ----
    let qs: Vec<f64> = pts.iter().map(|p| p.0).collect();
    let max_lo = qs.len().saturating_sub(5);
    for start in 0..=max_lo {
        for len in [5usize, 7, 9] {
            let end = start + len;
            if end > qs.len() {
                continue;
            }
            let win = &pts[start..end];
            let x: Vec<f64> = win.iter().map(|p| p.1.powi(2)).collect();
            let y: Vec<f64> = win.iter().map(|p| p.2.ln()).collect();
            let w: Vec<f64> = win
                .iter()
                .map(|p| (p.1 / p.2).powi(2))
                .collect();
            let Some((lni0, slope, sl, ss, chi2)) = wls(&x, &y, &w) else { continue };
            if slope >= 0.0 {
                continue;
            }
            let rg = (-3.0 * slope).sqrt();
            let i0 = lni0.exp();
            let qmax = win[win.len() - 1].0;
            let mut checks = Vec::new();
            let qrg = qmax * rg;
            if qrg <= 1.3 {
                checks.push(format!("q_max*Rg={:.3} <= 1.3", qrg));
            } else {
                checks.push(format!("q_max*Rg={:.3} > 1.3（越出线性 Guinier 区）", qrg));
            }
            candidates.push(ModelCandidate {
                id: short_hash(&["guinier", &start.to_string(), &len.to_string()]),
                kind: ModelKind::Guinier,
                q_min: win[0].0,
                q_max: qmax,
                n_points: len,
                chi2,
                dof: len - 2,
                param_a: rg,
                param_b: i0,
                param_a_sigma: 3.0 * ss / (2.0 * rg.max(1e-12)),
                param_b_sigma: i0 * sl,
                checks,
                overlaps_with: vec![],
            });
        }
    }

    // ---- Porod 滑窗：高 q 段，指数 p ----
    for start in 0..qs.len() {
        for len in [5usize, 7, 9] {
            let end = start + len;
            if end > qs.len() {
                continue;
            }
            let win = &pts[start..end];
            // 只在高 q 一半枚举，避免把 Guinier 区当 Porod。
            if win[0].0 < sub.overlap.map(|(l, h)| l + (h - l) * 0.45).unwrap_or(0.0) {
                continue;
            }
            let x: Vec<f64> = win.iter().map(|p| p.0.ln()).collect();
            let y: Vec<f64> = win.iter().map(|p| p.1.ln()).collect();
            let w: Vec<f64> = win.iter().map(|p| (p.1 / p.2).powi(2)).collect();
            let Some((lnc, slope, slc, ssp, chi2)) = wls(&x, &y, &w) else { continue };
            if slope >= 0.0 {
                continue;
            }
            candidates.push(ModelCandidate {
                id: short_hash(&["porod", &start.to_string(), &len.to_string()]),
                kind: ModelKind::Porod,
                q_min: win[0].0,
                q_max: win[win.len() - 1].0,
                n_points: len,
                chi2,
                dof: len - 2,
                param_a: -slope,      // 指数 p
                param_b: lnc.exp(),   // 前置因子 C
                param_a_sigma: ssp,
                param_b_sigma: lnc.exp() * slc,
                checks: vec![format!("lnI-lnq 线性，p={:.3}", -slope)],
                overlaps_with: vec![],
            });
        }
    }

    // 同族区间重叠标注（合法但需审阅者知晓）。
    let ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
    for a in 0..candidates.len() {
        for b in (a + 1)..candidates.len() {
            if candidates[a].kind == candidates[b].kind {
                let ov = candidates[a].q_min < candidates[b].q_max
                    && candidates[b].q_min < candidates[a].q_max;
                if ov {
                    let (idb, ida) = (ids[b].clone(), ids[a].clone());
                    candidates[a].overlaps_with.push(idb);
                    candidates[b].overlaps_with.push(ida);
                }
            }
        }
    }
    let overlap_pairs: usize =
        candidates.iter().map(|c| c.overlaps_with.len()).sum::<usize>() / 2;
    if overlap_pairs > 0 {
        diagnostics.push(diag(
            "REGION_OVERLAP",
            "warn",
            format!("存在 {} 对区间重叠的同族候选，已互相标注，不自动取舍", overlap_pairs),
        ));
    }
    diagnostics.push(diag(
        "NO_AUTO_STITCH",
        "info",
        "Guinier 与 Porod 候选分别给出，系统不自动拼成单一分段模型".into(),
    ));
    ModelFit { candidates, diagnostics }
}
