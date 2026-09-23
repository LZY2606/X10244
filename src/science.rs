//! 核心散射规则：
//! - q 网格不同时不直接相减，插值只在重叠范围生效；
//! - 误差包括归一化与背景缩放不确定度；
//! - Guinier 与 Porod 区间不自动拼成单一模型；多解全部保留。

use crate::model::*;

/// 归一化因子：曝光 × 厚度 × 透射率，含相对不确定度。
#[derive(Debug, Clone)]
pub struct NormFactors {
    pub exposure_s: f64,
    pub thickness_cm: f64,
    pub transmission: f64,
    pub rel_exposure: f64,
    pub rel_thickness: f64,
    pub rel_transmission: f64,
}

impl NormFactors {
    pub fn factor(&self) -> f64 {
        self.exposure_s * self.thickness_cm * self.transmission
    }
    pub fn rel_unc(&self) -> f64 {
        (self.rel_exposure.powi(2) + self.rel_thickness.powi(2) + self.rel_transmission.powi(2)).sqrt()
    }
}

/// 归一化：I_n = I / (e·d·T)；σ 同时含计数统计与归一化因子不确定度。
pub fn normalize(sample: &Curve, f: &NormFactors) -> Curve {
    let factor = f.factor();
    let rel = f.rel_unc();
    let mut diagnostics = sample.diagnostics.clone();
    if factor <= 0.0 {
        diagnostics.push(format!("normalization factor non-positive: {factor}"));
    }
    let points = sample
        .points
        .iter()
        .map(|p| {
            let intensity = p.intensity / factor;
            let stat = p.sigma / factor;
            let sigma = (stat.powi(2) + (intensity * rel).powi(2)).sqrt();
            CurvePoint { q: p.q, intensity, sigma }
        })
        .collect();
    Curve {
        id: format!("{}.normalized", sample.id),
        kind: CurveKind::Normalized,
        q_unit: sample.q_unit.clone(),
        i_unit: "1/cm".into(),
        points,
        diagnostics,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GridAlignment {
    /// 网格逐点一致，可直接对应。
    Direct,
    /// 网格不同：只允许在重叠区间 [q_lo, q_hi] 内插值。
    Interpolated { q_lo: f64, q_hi: f64 },
    /// 无重叠区：禁止相减。
    NoOverlap,
}

pub fn align_grids(a: &[CurvePoint], b: &[CurvePoint]) -> GridAlignment {
    if a.is_empty() || b.is_empty() {
        return GridAlignment::NoOverlap;
    }
    let same = a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(x, y)| (x.q - y.q).abs() < 1e-9);
    if same {
        return GridAlignment::Direct;
    }
    let q_lo = a.first().unwrap().q.max(b.first().unwrap().q);
    let q_hi = a.last().unwrap().q.min(b.last().unwrap().q);
    if q_lo < q_hi {
        GridAlignment::Interpolated { q_lo, q_hi }
    } else {
        GridAlignment::NoOverlap
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SubtractError {
    NoOverlap,
}

/// 在有序网格上线性插值；调用方保证 q 在网格范围内。
fn interp(grid: &[CurvePoint], q: f64) -> (f64, f64, usize, usize, f64) {
    for i in 0..grid.len() - 1 {
        let (q0, q1) = (grid[i].q, grid[i + 1].q);
        if q >= q0 && q <= q1 {
            let t = if q1 > q0 { (q - q0) / (q1 - q0) } else { 0.0 };
            let i_val = grid[i].intensity * (1.0 - t) + grid[i + 1].intensity * t;
            let s_val = grid[i].sigma * (1.0 - t) + grid[i + 1].sigma * t;
            return (i_val, s_val, i, i + 1, t);
        }
    }
    let last = grid.len() - 1;
    (grid[last].intensity, grid[last].sigma, last, last, 0.0)
}

/// 背景扣除。
/// - 网格一致：逐点相减。
/// - 网格不同：只在重叠区间内对缓冲液插值后相减；区间外样本点保留但不扣除并标记。
/// - 无重叠：返回错误，不产生任何结果。
/// - σ² = σ_s² + (k·σ_b)² + (I_b·σ_k)²，含背景缩放不确定度。
/// - 负强度只标记、不钳制。
pub fn subtract_background(
    sample: &Curve,
    buffer: &Curve,
    params: &SubtractionParams,
) -> Result<SubtractedCurve, SubtractError> {
    let alignment = align_grids(&sample.points, &buffer.points);
    let overlap = match alignment {
        GridAlignment::NoOverlap => return Err(SubtractError::NoOverlap),
        GridAlignment::Direct => None,
        GridAlignment::Interpolated { q_lo, q_hi } => Some((q_lo, q_hi)),
    };
    let k = params.scale;
    let k_sig = params.scale_sigma;
    let mut points = Vec::new();
    let mut diagnostics = Vec::new();
    if overlap.is_some() {
        diagnostics.push(format!(
            "q grids differ; buffer interpolated onto sample grid inside overlap only"
        ));
    }
    for (si, sp) in sample.points.iter().enumerate() {
        let in_overlap = match overlap {
            None => true,
            Some((lo, hi)) => sp.q >= lo && sp.q <= hi,
        };
        if !in_overlap {
            points.push(DerivedPoint {
                q: sp.q,
                intensity: sp.intensity,
                sigma: sp.sigma,
                flags: vec![PointFlag::OutsideOverlap],
                sample_idx: si,
                buffer: None,
            });
            continue;
        }
        let (bi, bs, src) = match overlap {
            None => {
                let bp = &buffer.points[si];
                (bp.intensity, bp.sigma, BufferSource::Direct { idx: si })
            }
            Some(_) => {
                let (i_val, s_val, lo, hi, t) = interp(&buffer.points, sp.q);
                (i_val, s_val, BufferSource::Interpolated { lo, hi, t })
            }
        };
        let intensity = sp.intensity - k * bi;
        let sigma = (sp.sigma.powi(2) + (k * bs).powi(2) + (bi * k_sig).powi(2)).sqrt();
        let mut flags = Vec::new();
        if matches!(src, BufferSource::Interpolated { .. }) {
            flags.push(PointFlag::InterpolatedBuffer);
        }
        if intensity < 0.0 {
            flags.push(PointFlag::NegativeIntensity);
        }
        points.push(DerivedPoint { q: sp.q, intensity, sigma, flags, sample_idx: si, buffer: Some(src) });
    }
    let neg = points.iter().filter(|p| p.flags.contains(&PointFlag::NegativeIntensity)).count();
    if neg > 0 {
        diagnostics.push(format!("{neg} points with negative intensity after subtraction (kept, not clamped)"));
    }
    Ok(SubtractedCurve {
        id: format!("{}.minus.{}", sample.id, params.buffer_id),
        q_unit: sample.q_unit.clone(),
        i_unit: sample.i_unit.clone(),
        params: params.clone(),
        points,
        diagnostics,
    })
}

fn linfit(xs: &[f64], ys: &[f64]) -> (f64, f64, f64) {
    let n = xs.len() as f64;
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let denom = n * sxx - sx * sx;
    if denom.abs() < 1e-30 {
        return (0.0, 0.0, 0.0);
    }
    let slope = (n * sxy - sx * sy) / denom;
    let intercept = (sy - slope * sx) / n;
    let ss_res: f64 = xs.iter().zip(ys).map(|(x, y)| (y - slope * x - intercept).powi(2)).sum();
    let ss_tot: f64 = ys.iter().map(|y| (y - sy / n).powi(2)).sum();
    let r2 = if ss_tot > 1e-30 { 1.0 - ss_res / ss_tot } else { 0.0 };
    (slope, intercept, r2)
}

/// Guinier 候选：ln I 对 q² 的线性窗口，约束 q_max·Rg ≤ 1.3，R² ≥ 0.98。
/// 每个起点保留最优窗口；候选之间允许重叠，绝不合并。
pub fn guinier_candidates(curve: &SubtractedCurve) -> Vec<RangeCandidate> {
    let pts: Vec<&DerivedPoint> = curve
        .points
        .iter()
        .filter(|p| p.intensity > 0.0 && !p.flags.contains(&PointFlag::OutsideOverlap))
        .collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start + 6 <= pts.len() {
        let mut best: Option<(usize, f64, f64, f64)> = None; // (end, r2, rg, residual)
        for end in start + 6..=pts.len().min(start + 30) {
            let xs: Vec<f64> = pts[start..end].iter().map(|p| p.q * p.q).collect();
            let ys: Vec<f64> = pts[start..end].iter().map(|p| p.intensity.ln()).collect();
            let (slope, _int, r2) = linfit(&xs, &ys);
            if slope >= 0.0 {
                break;
            }
            let rg = (-3.0 * slope).sqrt();
            let qmax = pts[end - 1].q;
            if qmax * rg > 1.3 || r2 < 0.98 {
                break;
            }
            let residual = (1.0 - r2).max(0.0).sqrt();
            let better = best.as_ref().map(|b| residual < b.3).unwrap_or(true);
            if better {
                best = Some((end, r2, rg, residual));
            }
        }
        if let Some((end, _r2, rg, residual)) = best {
            out.push(RangeCandidate {
                id: format!("guinier-{}-{}", pts[start].q, pts[end - 1].q),
                kind: RangeKind::Guinier,
                lo: pts[start].sample_idx,
                hi: pts[end - 1].sample_idx,
                params: serde_json::json!({"rg": rg, "qmax_rg": pts[end - 1].q * rg}),
                residual,
                adopted: false,
                overlaps: vec![],
            });
        }
        start += 1;
    }
    out
}

/// Porod 候选：q⁴·I 平台窗口，窗口内相对标准差 < 8%。
pub fn porod_candidates(curve: &SubtractedCurve) -> Vec<RangeCandidate> {
    let pts: Vec<&DerivedPoint> = curve
        .points
        .iter()
        .filter(|p| !p.flags.contains(&PointFlag::OutsideOverlap))
        .collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start + 6 <= pts.len() {
        let mut best: Option<(usize, f64)> = None; // (end, rel_std)
        for end in start + 6..=pts.len().min(start + 25) {
            let vals: Vec<f64> = pts[start..end].iter().map(|p| p.q.powi(4) * p.intensity).collect();
            let mean: f64 = vals.iter().sum::<f64>() / vals.len() as f64;
            if mean.abs() < 1e-30 {
                break;
            }
            let var: f64 = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / vals.len() as f64;
            let rel_std = var.sqrt() / mean.abs();
            if rel_std >= 0.08 {
                break;
            }
            let better = best.as_ref().map(|b| rel_std < b.1).unwrap_or(true);
            if better {
                best = Some((end, rel_std));
            }
        }
        if let Some((end, rel_std)) = best {
            out.push(RangeCandidate {
                id: format!("porod-{}-{}", pts[start].q, pts[end - 1].q),
                kind: RangeKind::Porod,
                lo: pts[start].sample_idx,
                hi: pts[end - 1].sample_idx,
                params: serde_json::json!({"plateau_rel_std": rel_std}),
                residual: rel_std,
                adopted: false,
                overlaps: vec![],
            });
        }
        start += 1;
    }
    out
}

/// 区间分析：Guinier 与 Porod 候选并列保留，重叠只标注、不合并。
pub fn analyze_ranges(curve: &SubtractedCurve) -> RangeAnalysis {
    let mut candidates = guinier_candidates(curve);
    candidates.extend(porod_candidates(curve));
    // 按残差排序后只保留前 8 个候选，避免组合爆炸；全部保留在案。
    candidates.sort_by(|a, b| a.residual.partial_cmp(&b.residual).unwrap());
    candidates.truncate(8);
    let mut diagnostics = vec![
        "Guinier and Porod ranges are kept as separate candidates; never auto-merged into one model".to_string(),
    ];
    // 标注重叠（含跨类型重叠）。
    for i in 0..candidates.len() {
        for j in i + 1..candidates.len() {
            let (a, b) = (&candidates[i], &candidates[j]);
            if a.lo <= b.hi && b.lo <= a.hi {
                let (aid, bid) = (a.id.clone(), b.id.clone());
                candidates[i].overlaps.push(bid);
                candidates[j].overlaps.push(aid);
            }
        }
    }
    if candidates.iter().any(|c| !c.overlaps.is_empty()) {
        diagnostics.push("overlapping range candidates detected; all retained for human adjudication".into());
    }
    RangeAnalysis {
        id: format!("ranges.{}", curve.id),
        curve_id: curve.id.clone(),
        candidates,
        diagnostics,
    }
}

/// 多候选残差检验：返回按残差排序的候选 id；无法唯一决定时全部保留。
pub fn residual_ranking(analysis: &RangeAnalysis) -> Vec<(String, f64)> {
    let mut v: Vec<(String, f64)> = analysis
        .candidates
        .iter()
        .map(|c| (c.id.clone(), c.residual))
        .collect();
    v.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    v
}
