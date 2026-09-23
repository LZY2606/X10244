//! 核心散射规则：
//! - q 网格不同时不直接相减，插值只在重叠范围生效
//! - 误差包括归一化与背景缩放不确定度
//! - Guinier 与 Porod 区间不自动拼成单一模型；多解全部保留
use crate::model::*;
use crate::units;

pub struct NormInput {
    pub raw: Curve, // 原始 q/I/σ（已换算到 Å⁻¹；I 可为任意单位计数）
    pub exposure_s: f64,
    pub thickness_cm: f64,
    pub transmission: f64,
    pub transmission_unc: f64,
    /// 仪器响应 R(q)，可选；网格不同则在重叠范围内插值。
    pub response: Option<Curve>,
}

pub struct NormOutput {
    pub curve: Curve,
    pub diags: Vec<Diag>,
}

/// 归一化：I_norm = I_raw / (E · d · T · R(q))。
/// 误差：σ² = (σ_raw/norm)² + (I_norm · σ_T/T)² —— 归一化不确定度进入误差。
pub fn normalize(input: &NormInput) -> NormOutput {
    let mut diags = Vec::new();
    let base = input.exposure_s * input.thickness_cm * input.transmission;
    let mut points = Vec::new();
    let resp = input.response.as_ref();
    for p in &input.raw.points {
        let r = match resp {
            Some(rc) => match interp_at(rc, p.q) {
                Some(v) => v,
                None => {
                    diags.push(Diag {
                        code: DiagCode::NoOverlap,
                        message: format!("仪器响应在 q={:.4} 处无覆盖，该点保留原值未归一化", p.q),
                        subject: format!("q={:.4}", p.q),
                    });
                    1.0
                }
            },
            None => 1.0,
        };
        let norm = base * r;
        let i = p.i / norm;
        let sigma_shape = p.di / norm;
        let sigma_norm = (i * input.transmission_unc / input.transmission).abs();
        let di = (sigma_shape.powi(2) + sigma_norm.powi(2)).sqrt();
        points.push(CurvePoint { q: p.q, i, di });
    }
    if input.transmission_unc > 0.0 {
        diags.push(Diag {
            code: DiagCode::NormUncertainty,
            message: format!(
                "归一化不确定度 σ_T/T = {:.4} 已并入误差",
                input.transmission_unc / input.transmission
            ),
            subject: "normalization".into(),
        });
    }
    NormOutput { curve: Curve::new(points), diags }
}

pub struct SubtractOutput {
    /// 无重叠区时为 None —— 不产出曲线，只有诊断。
    pub curve: Option<Curve>,
    pub diags: Vec<Diag>,
    pub overlap: Option<(f64, f64)>,
    pub interpolated: bool,
}

/// 背景扣除：I_sub = I_s - k·I_b。
/// σ² = σ_s² + (k·σ_b)² + (I_b·σ_k)² —— 背景缩放不确定度进入误差。
/// 网格不同不直接相减；只在重叠范围 [max(qmin), min(qmax)] 内对背景线性插值。
pub fn subtract(sample: &Curve, buffer: &Curve, scale: f64, scale_unc: f64) -> SubtractOutput {
    let mut diags = Vec::new();
    let lo = sample.q_min().max(buffer.q_min());
    let hi = sample.q_max().min(buffer.q_max());
    if !(lo < hi) {
        diags.push(Diag {
            code: DiagCode::NoOverlap,
            message: format!(
                "样品 q 范围 [{:.4},{:.4}] 与背景 [{:.4},{:.4}] 无重叠区，不执行扣除",
                sample.q_min(), sample.q_max(), buffer.q_min(), buffer.q_max()
            ),
            subject: "subtract".into(),
        });
        return SubtractOutput { curve: None, diags, overlap: None, interpolated: false };
    }
    let grids_identical = sample.points.len() == buffer.points.len()
        && sample
            .points
            .iter()
            .zip(buffer.points.iter())
            .all(|(a, b)| (a.q - b.q).abs() < 1e-12);
    let interpolated = !grids_identical;
    if interpolated {
        diags.push(Diag {
            code: DiagCode::QGridMismatchInterpolated,
            message: format!(
                "q 网格错位：不直接相减，背景仅在重叠区 [{:.4},{:.4}] 内插值到样品网格",
                lo, hi
            ),
            subject: "subtract".into(),
        });
    }
    let mut points = Vec::new();
    for p in &sample.points {
        if p.q < lo - 1e-12 || p.q > hi + 1e-12 {
            continue; // 重叠区之外的样品点不进入扣除结果
        }
        let (bi, bdi) = if grids_identical {
            let b = &buffer.points[sample.points.iter().position(|x| x.q == p.q).unwrap()];
            (b.i, b.di)
        } else {
            match interp_pair_at(buffer, p.q) {
                Some(v) => v,
                None => continue,
            }
        };
        let i = p.i - scale * bi;
        let di = (p.di.powi(2) + (scale * bdi).powi(2) + (bi * scale_unc).powi(2)).sqrt();
        if i < 0.0 {
            diags.push(Diag {
                code: DiagCode::NegativeIntensity,
                message: format!("q={:.4} 处扣背景后强度为负 ({:.4e})，保留原值并标记", p.q, i),
                subject: format!("q={:.4}", p.q),
            });
        }
        points.push(CurvePoint { q: p.q, i, di });
    }
    if scale_unc > 0.0 {
        diags.push(Diag {
            code: DiagCode::BackgroundScaleUncertainty,
            message: format!("背景缩放不确定度 σ_k = {:.4} 已并入误差", scale_unc),
            subject: "subtract".into(),
        });
    }
    SubtractOutput { curve: Some(Curve::new(points)), diags, overlap: Some((lo, hi)), interpolated }
}

/// 线性插值（仅曲线范围内）。
pub fn interp_at(c: &Curve, q: f64) -> Option<f64> {
    interp_pair_at(c, q).map(|(i, _)| i)
}

pub fn interp_pair_at(c: &Curve, q: f64) -> Option<(f64, f64)> {
    let pts = &c.points;
    if pts.is_empty() || q < pts[0].q - 1e-12 || q > pts[pts.len() - 1].q + 1e-12 {
        return None;
    }
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if q >= a.q - 1e-12 && q <= b.q + 1e-12 {
            let t = if (b.q - a.q).abs() < 1e-15 { 0.0 } else { (q - a.q) / (b.q - a.q) };
            return Some((a.i + t * (b.i - a.i), a.di + t * (b.di - a.di)));
        }
    }
    let last = pts[pts.len() - 1];
    Some((last.i, last.di))
}

fn linfit(xs: &[f64], ys: &[f64]) -> Option<(f64, f64, f64)> {
    let n = xs.len() as f64;
    if xs.len() < 3 {
        return None;
    }
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let det = n * sxx - sx * sx;
    if det.abs() < 1e-20 {
        return None;
    }
    let slope = (n * sxy - sx * sy) / det;
    let intercept = (sy - slope * sx) / n;
    let rss: f64 = xs
        .iter()
        .zip(ys)
        .map(|(x, y)| {
            let r = y - (slope * x + intercept);
            r * r
        })
        .sum();
    Some((slope, intercept, (rss / n).sqrt()))
}

/// Guinier 候选：ln I 对 q² 线性拟合 → Rg, I0。
pub fn guinier_fit(c: &Curve, q_min: f64, q_max: f64) -> Option<(f64, f64, f64)> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for p in &c.points {
        if p.q >= q_min && p.q <= q_max && p.i > 0.0 {
            xs.push(p.q * p.q);
            ys.push(p.i.ln());
        }
    }
    let (slope, intercept, rms) = linfit(&xs, &ys)?;
    if slope >= 0.0 {
        return None;
    }
    Some(((-3.0 * slope).sqrt(), intercept.exp(), rms))
}

/// Porod 候选：ln I 对 ln q 线性拟合 → 指数。
pub fn porod_fit(c: &Curve, q_min: f64, q_max: f64) -> Option<(f64, f64, f64)> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for p in &c.points {
        if p.q >= q_min && p.q <= q_max && p.i > 0.0 && p.q > 0.0 {
            xs.push(p.q.ln());
            ys.push(p.i.ln());
        }
    }
    let (slope, intercept, rms) = linfit(&xs, &ys)?;
    Some((slope, intercept.exp(), rms))
}

pub struct CandidateSet {
    pub candidates: Vec<FitCandidate>,
    pub diags: Vec<Diag>,
}

/// 多区间候选评估。区间重叠时不合并为单一模型；残差接近时保留全部合法候选。
pub fn evaluate_candidates(
    curve: &Curve,
    guinier_intervals: &[(f64, f64)],
    porod_intervals: &[(f64, f64)],
    id_prefix: &str,
) -> CandidateSet {
    let mut candidates = Vec::new();
    let mut diags = Vec::new();
    for (idx, &(lo, hi)) in guinier_intervals.iter().enumerate() {
        if let Some((rg, i0, rms)) = guinier_fit(curve, lo, hi) {
            candidates.push(FitCandidate {
                id: format!("{}-g{}", id_prefix, idx),
                kind: "Guinier".into(),
                q_min: lo,
                q_max: hi,
                params: serde_json::json!({"rg": rg, "i0": i0}),
                residual_rms: rms,
                accepted: None,
            });
        }
    }
    for (idx, &(lo, hi)) in porod_intervals.iter().enumerate() {
        if let Some((exp, scale, rms)) = porod_fit(curve, lo, hi) {
            candidates.push(FitCandidate {
                id: format!("{}-p{}", id_prefix, idx),
                kind: "Porod".into(),
                q_min: lo,
                q_max: hi,
                params: serde_json::json!({"exponent": exp, "scale": scale}),
                residual_rms: rms,
                accepted: None,
            });
        }
    }
    // 区间重叠：Guinier 与 Porod 区间不自动拼成单一模型
    for a in &candidates {
        for b in &candidates {
            if a.id < b.id && a.q_min < b.q_max && b.q_min < a.q_max {
                diags.push(Diag {
                    code: DiagCode::IntervalOverlap,
                    message: format!(
                        "区间 [{:.4},{:.4}] 与 [{:.4},{:.4}] 重叠（{} / {}），保留为独立候选，不拼成单一模型",
                        a.q_min, a.q_max, b.q_min, b.q_max, a.kind, b.kind
                    ),
                    subject: format!("{}+{}", a.id, b.id),
                });
            }
        }
    }
    // 多候选残差检验：最优两个残差接近 → 无法唯一决定，全部保留
    let mut by_kind: Vec<&FitCandidate> = candidates.iter().collect();
    by_kind.sort_by(|a, b| a.residual_rms.partial_cmp(&b.residual_rms).unwrap());
    if by_kind.len() >= 2 {
        let best = by_kind[0].residual_rms;
        let second = by_kind[1].residual_rms;
        if best > 0.0 && (second - best) / best < 0.15 {
            diags.push(Diag {
                code: DiagCode::MultiCandidateAmbiguous,
                message: format!(
                    "候选 {} 与 {} 残差接近（{:.4e} vs {:.4e}），无法唯一决定，全部保留",
                    by_kind[0].id, by_kind[1].id, best, second
                ),
                subject: format!("{}+{}", by_kind[0].id, by_kind[1].id),
            });
        }
    }
    CandidateSet { candidates, diags }
}

/// 从原始 CSV 文本解析曲线并换算 q 到 Å⁻¹。I 单位不可换算时保留原值并给诊断。
pub fn parse_curve_csv(
    raw: &str,
    q_unit: &str,
    i_unit: &str,
) -> (Curve, Vec<Diag>) {
    let mut diags = Vec::new();
    let qu = units::parse_q_unit(q_unit);
    let iu = units::parse_i_unit(i_unit);
    if qu.is_none() {
        diags.push(Diag {
            code: DiagCode::UnitUnconvertible,
            message: format!("未知 q 单位 '{}'，q 值保留原始表示", q_unit),
            subject: "q_unit".into(),
        });
    }
    if iu.is_none() || iu == Some(units::IUnit::Arbitrary) {
        diags.push(Diag {
            code: DiagCode::UnitUnconvertible,
            message: format!("I 单位 '{}' 不可换算为 cm^-1，保留原始计数", i_unit),
            subject: "i_unit".into(),
        });
    }
    let mut points = Vec::new();
    for (ln, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() < 3 {
            diags.push(Diag {
                code: DiagCode::UnitUnconvertible,
                message: format!("第 {} 行列数不足，跳过但保留在原始记录中", ln + 1),
                subject: format!("line {}", ln + 1),
            });
            continue;
        }
        let (Ok(q), Ok(i), Ok(di)) = (
            cols[0].parse::<f64>(),
            cols[1].parse::<f64>(),
            cols[2].parse::<f64>(),
        ) else {
            diags.push(Diag {
                code: DiagCode::UnitUnconvertible,
                message: format!("第 {} 行无法解析，跳过但保留在原始记录中", ln + 1),
                subject: format!("line {}", ln + 1),
            });
            continue;
        };
        let q_conv = qu.map(|u| units::q_to_inv_angstrom(q, u)).unwrap_or(q);
        points.push(CurvePoint { q: q_conv, i, di });
    }
    points.sort_by(|a, b| a.q.partial_cmp(&b.q).unwrap());
    (Curve::new(points), diags)
}
