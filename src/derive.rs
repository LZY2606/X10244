//! 推导引擎：归一化、背景扣除、Guinier/Porod 区间候选。
//! 语义红线：
//!  - q 网格不同时不直接相减，插值只在重叠范围生效；
//!  - 误差包括归一化与背景缩放不确定度；
//!  - Guinier 与 Porod 区间不自动拼成单一模型。
use crate::model::*;

/// 归一化：I_norm = I / (exposure * thickness * transmission)。
/// sigma 含原始统计误差与归一化相对不确定度（透射率/厚度/曝光合成）。
pub fn normalize(raw: &RawCurve, meta: &ExposureMeta, deps: Vec<String>) -> (NormalizedCurve, Vec<String>) {
    let mut notes = Vec::new();
    let factor = meta.exposure_s * meta.thickness_cm * meta.transmission;
    let rel_unc = (meta.transmission_rel_unc.powi(2)
        + meta.thickness_rel_unc.powi(2)
        + meta.exposure_rel_unc.powi(2))
    .sqrt();
    let mut points = Vec::new();
    for (idx, p) in raw.points.iter().enumerate() {
        let q = match &raw.q_unit {
            Some(u) => u.to_angstrom(p.q),
            None => {
                notes.push(format!("point {idx}: q unit '{}' 无法转换，保留原值", raw.q_unit_raw));
                p.q // 留在原位置，附诊断
            }
        };
        let i = p.i / factor;
        let sigma = ((p.sigma / factor).powi(2) + (i * rel_unc).powi(2)).sqrt();
        points.push(NormPoint { q, i, sigma, src_idx: idx });
    }
    (
        NormalizedCurve { deps, norm_factor: factor, norm_rel_unc: rel_unc, points },
        notes,
    )
}

/// 背景扣除。规则：
///  - q 精确匹配（|dq|<1e-9）才直接相减；
///  - 网格错位时仅在重叠范围内对背景做线性插值；
///  - 重叠区外的样品点不扣除，记入 excluded（不丢点）；
///  - sigma 含背景缩放不确定度项 (I_buffer * scale_unc)^2；
///  - 负强度保留并打 flag。
pub fn subtract(
    sample: &NormalizedCurve,
    buffer: &NormalizedCurve,
    scale: f64,
    scale_unc: f64,
    deps: Vec<String>,
) -> (SubtractedCurve, Vec<serde_json::Value>) {
    let bq: Vec<f64> = buffer.points.iter().map(|p| p.q).collect();
    let mut points = Vec::new();
    let mut excluded = Vec::new();
    let mut n_interp = 0usize;
    let mut n_exact = 0usize;
    for (si, sp) in sample.points.iter().enumerate() {
        let align = find_align(&bq, sp.q);
        let align = match align {
            Some(a) => a,
            None => {
                excluded.push(ExcludedPoint { q: sp.q, reason: "no_overlap".into() });
                continue;
            }
        };
        let (bi, bs) = match &align {
            BufferAlign::Exact { idx } => {
                n_exact += 1;
                (buffer.points[*idx].i, buffer.points[*idx].sigma)
            }
            BufferAlign::Interp { lo, hi, t } => {
                n_interp += 1;
                let a = &buffer.points[*lo];
                let b = &buffer.points[*hi];
                (
                    a.i + (b.i - a.i) * t,
                    (a.sigma.powi(2) * (1.0 - t) + b.sigma.powi(2) * t).sqrt(),
                )
            }
        };
        let i = sp.i - scale * bi;
        // 误差 = 样品归一化误差 + 背景归一化误差(缩放后) + 背景缩放不确定度
        let sigma = (sp.sigma.powi(2) + (scale * bs).powi(2) + (bi * scale_unc).powi(2)).sqrt();
        let flag = if i < 0.0 { Some("negative_intensity".to_string()) } else { None };
        points.push(SubPoint { q: sp.q, i, sigma, flag, sample_idx: si, buffer: align });
    }
    let residual_metric = residual_metric(&points);
    let judgments = vec![
        serde_json::json!({
            "rule": "grid_mismatch_no_direct_subtract",
            "exact": n_exact, "interpolated": n_interp,
            "interp_scope": "overlap_only",
        }),
        serde_json::json!({
            "rule": "interp_only_within_overlap",
            "excluded_no_overlap": excluded.len(),
        }),
        serde_json::json!({
            "rule": "error_includes_norm_and_scale_uncertainty",
            "scale": scale, "scale_unc": scale_unc,
        }),
    ];
    (
        SubtractedCurve { deps, scale, scale_unc, points, excluded, residual_metric },
        judgments,
    )
}

fn find_align(bq: &[f64], q: f64) -> Option<BufferAlign> {
    if bq.is_empty() || q < bq[0] || q > bq[bq.len() - 1] {
        return None;
    }
    for (idx, &bq_i) in bq.iter().enumerate() {
        if (bq_i - q).abs() < 1e-9 {
            return Some(BufferAlign::Exact { idx });
        }
    }
    // 二分查找插值区间
    let hi = bq.partition_point(|&x| x < q);
    if hi == 0 || hi >= bq.len() {
        return None;
    }
    let lo = hi - 1;
    let t = (q - bq[lo]) / (bq[hi] - bq[lo]);
    Some(BufferAlign::Interp { lo, hi, t })
}

/// 残差度量：Porod 区（高 q 后 1/3）q^4*I 的相对起伏，用于多候选比较。
fn residual_metric(points: &[SubPoint]) -> f64 {
    let n = points.len();
    if n < 6 {
        return f64::NAN;
    }
    let tail: Vec<f64> = points[n - n / 3..]
        .iter()
        .map(|p| p.q.powi(4) * p.i)
        .collect();
    let mean = tail.iter().sum::<f64>() / tail.len() as f64;
    if mean.abs() < 1e-30 {
        return f64::NAN;
    }
    (tail.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / tail.len() as f64).sqrt() / mean.abs()
}

/// Guinier 候选：ln I 对 q^2 线性拟合。不与其他区间自动合并。
pub fn guinier(sub: &SubtractedCurve, qmin: f64, qmax: f64, deps: Vec<String>) -> IntervalCandidate {
    let pts: Vec<&SubPoint> = sub.points.iter().filter(|p| p.q >= qmin && p.q <= qmax && p.i > 0.0).collect();
    let xs: Vec<f64> = pts.iter().map(|p| p.q.powi(2)).collect();
    let ys: Vec<f64> = pts.iter().map(|p| p.i.ln()).collect();
    let (slope, intercept, residuals, rms) = linfit(&xs, &ys);
    let rg = if slope < 0.0 { (-3.0 * slope).sqrt() } else { f64::NAN };
    let i0 = intercept.exp();
    let mut notes = Vec::new();
    let mut valid = true;
    if rg.is_nan() {
        valid = false;
        notes.push("slope>=0，无法得到 Rg".to_string());
    } else if qmax * rg > 1.3 {
        valid = false;
        notes.push(format!("qmax*Rg={:.2} > 1.3，超出 Guinier 有效区", qmax * rg));
    }
    notes.push("区间候选独立保存，不与 Porod 或其他区间自动拼接".to_string());
    IntervalCandidate {
        deps,
        model: "guinier".into(),
        qmin,
        qmax,
        params: serde_json::json!({"Rg_A": rg, "I0": i0, "slope": slope, "n_points": pts.len()}),
        residuals,
        rms_residual: rms,
        valid,
        notes,
    }
}

/// Porod 候选：ln I 对 ln q 线性拟合，斜率 ≈ -4。
pub fn porod(sub: &SubtractedCurve, qmin: f64, qmax: f64, deps: Vec<String>) -> IntervalCandidate {
    let pts: Vec<&SubPoint> = sub.points.iter().filter(|p| p.q >= qmin && p.q <= qmax && p.i > 0.0).collect();
    let xs: Vec<f64> = pts.iter().map(|p| p.q.ln()).collect();
    let ys: Vec<f64> = pts.iter().map(|p| p.i.ln()).collect();
    let (slope, intercept, residuals, rms) = linfit(&xs, &ys);
    let mut notes = Vec::new();
    let valid = pts.len() >= 4;
    if !valid {
        notes.push("区间内正强度点不足 4 个".to_string());
    }
    notes.push("区间候选独立保存，不与 Guinier 或其他区间自动拼接".to_string());
    IntervalCandidate {
        deps,
        model: "porod".into(),
        qmin,
        qmax,
        params: serde_json::json!({"porod_exponent": -slope, "log_const": intercept, "n_points": pts.len()}),
        residuals,
        rms_residual: rms,
        valid,
        notes,
    }
}

fn linfit(xs: &[f64], ys: &[f64]) -> (f64, f64, Vec<f64>, f64) {
    let n = xs.len() as f64;
    if xs.is_empty() {
        return (f64::NAN, f64::NAN, vec![], f64::NAN);
    }
    let mx = xs.iter().sum::<f64>() / n;
    let my = ys.iter().sum::<f64>() / n;
    let sxx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| (x - mx) * (y - my)).sum();
    let slope = if sxx.abs() < 1e-30 { 0.0 } else { sxy / sxx };
    let intercept = my - slope * mx;
    let residuals: Vec<f64> = xs.iter().zip(ys).map(|(x, y)| y - (slope * x + intercept)).collect();
    let rms = (residuals.iter().map(|r| r.powi(2)).sum::<f64>() / n).sqrt();
    (slope, intercept, residuals, rms)
}
