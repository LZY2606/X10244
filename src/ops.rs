//! 核心散射规则：
//! - q 网格不同时不直接相减，插值只在重叠范围生效
//! - 误差包括归一化与背景缩放不确定度
//! - Guinier 与 Porod 区间不自动拼成单一模型；多候选全部保留

use crate::domain::*;

/// 导入 -> 规范化坐标（仅派生层做单位换算；不可转换的值留在原位置并附诊断）
pub fn canonicalize(raw: &RawCurve) -> (Vec<Point>, Vec<Diag>) {
    let mut pts = Vec::new();
    let mut diags = Vec::new();
    for (idx, p) in raw.points.iter().enumerate() {
        match q_to_nm(p.q, &raw.q_unit) {
            Some(q) => pts.push(Point {
                q, i: p.i, err: p.err,
                flags: vec![],
                src: Some(PointSrc { sample_idx: idx, buffer_idx_lo: None, buffer_idx_hi: None }),
            }),
            None => diags.push(Diag::new(
                "unit_unconvertible",
                format!("q 单位 '{}' 无法转换为 nm^-1，值保留在原位置，不参与派生计算", raw.q_unit),
                format!("point#{}", idx),
            )),
        }
    }
    pts.sort_by(|a, b| a.q.partial_cmp(&b.q).unwrap());
    (pts, diags)
}

/// 归一化：I_norm = I / (T * t_exp * d)
/// 误差含归一化不确定度：σ_norm² = (I·σ_T/(T²·t·d))² + (σ_I/(T·t·d))²
pub fn normalize(points: &[Point], meta: &ExposureMeta) -> (Vec<Point>, Vec<Diag>) {
    let mut diags = Vec::new();
    let denom = meta.transmission * meta.exposure_s * meta.thickness_mm;
    if denom <= 0.0 {
        diags.push(Diag::new("normalization_invalid",
            "透射率/曝光/厚度乘积非正，无法归一化", "exposure_meta"));
        return (vec![], diags);
    }
    let rel_t = meta.transmission_err / meta.transmission;
    let out = points.iter().map(|p| {
        let i = p.i / denom;
        let err = ((p.err / denom).powi(2) + (i * rel_t).powi(2)).sqrt();
        Point { q: p.q, i, err, flags: p.flags.clone(), src: p.src.clone() }
    }).collect();
    diags.push(Diag::new("normalization_applied",
        format!("归一化因子 1/(T·t·d)=1/{:.6}，透射率相对不确定度 {:.4}", denom, rel_t),
        "exposure_meta"));
    (out, diags)
}

pub const GRID_TOL: f64 = 1e-9;

pub fn grids_equal(a: &[Point], b: &[Point]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(x, y)| (x.q - y.q).abs() < GRID_TOL)
}

/// 线性插值：在 buf 上求 q 处的值，返回 (i, err, lo_idx, hi_idx)；q 越界返回 None
fn interp(buf: &[Point], q: f64) -> Option<(f64, f64, usize, usize)> {
    if buf.is_empty() || q < buf[0].q || q > buf[buf.len() - 1].q { return None; }
    let hi = buf.partition_point(|p| p.q < q - GRID_TOL).min(buf.len() - 1);
    let lo = hi.saturating_sub(1);
    let (a, b) = (&buf[lo], (&buf[hi],)).0;
    let b = &buf[hi];
    if (b.q - a.q).abs() < GRID_TOL { return Some((a.i, a.err, lo, lo)); }
    let w = (q - a.q) / (b.q - a.q);
    Some((a.i + w * (b.i - a.i),
          a.err + w * (b.err - a.err).abs(),
          lo, hi))
}

pub struct SubtractionResult {
    pub points: Vec<Point>,
    pub diags: Vec<Diag>,
    pub status: String, // "ok" | "no_overlap"
}

/// 背景扣除：I_sub = I_s - scale·I_b
/// σ² = σ_s² + (scale·σ_b)² + (I_b·σ_scale)²   （含背景缩放不确定度）
/// q 网格不同 -> 不直接相减，仅在重叠区间内对缓冲液插值；无重叠 -> 状态 no_overlap，不产出点
pub fn subtract(sample: &[Point], buffer: &[Point], scale: f64, scale_err: f64) -> SubtractionResult {
    let mut diags = Vec::new();
    let mut out = Vec::new();

    if grids_equal(sample, buffer) {
        diags.push(Diag::new("grid_equal", "q 网格一致，逐点相减", "grid"));
        for (idx, (s, b)) in sample.iter().zip(buffer.iter()).enumerate() {
            out.push(sub_point(s, b.i, b.err, scale, scale_err,
                PointSrc { sample_idx: idx, buffer_idx_lo: Some(idx), buffer_idx_hi: Some(idx) }));
        }
    } else {
        let (smin, smax) = (sample.first().map(|p| p.q), sample.last().map(|p| p.q));
        let (bmin, bmax) = (buffer.first().map(|p| p.q), buffer.last().map(|p| p.q));
        let (lo, hi) = match (smin, smax, bmin, bmax) {
            (Some(a), Some(b), Some(c), Some(d)) => (a.max(c), b.min(d)),
            _ => { 
                return SubtractionResult { points: vec![], status: "no_overlap".into(),
                    diags: vec![Diag::new("no_overlap", "样本或缓冲液为空，无重叠区", "grid")] };
            }
        };
        if lo >= hi {
            return SubtractionResult { points: vec![], status: "no_overlap".into(),
                diags: vec![Diag::new("no_overlap",
                    format!("q 网格错位且重叠区为空（样本 [{:?},{:?}]，缓冲液 [{:?},{:?}]），不执行相减", smin, smax, bmin, bmax),
                    "grid")] };
        }
        diags.push(Diag::new("grid_mismatch_interpolated",
            format!("q 网格不同，不直接相减；缓冲液仅在重叠区 [{:.4}, {:.4}] nm^-1 内线性插值", lo, hi),
            "grid"));
        let mut n_outside = 0usize;
        for (idx, s) in sample.iter().enumerate() {
            if s.q < lo - GRID_TOL || s.q > hi + GRID_TOL { n_outside += 1; continue; }
            match interp(buffer, s.q) {
                Some((bi, berr, blo, bhi)) => out.push(sub_point(s, bi, berr, scale, scale_err,
                    PointSrc { sample_idx: idx, buffer_idx_lo: Some(blo), buffer_idx_hi: Some(bhi) })),
                None => n_outside += 1,
            }
        }
        if n_outside > 0 {
            diags.push(Diag::new("outside_overlap",
                format!("{} 个样本点位于重叠区之外，保留在源曲线但不进入扣背景结果", n_outside),
                "grid"));
        }
    }
    diags.push(Diag::new("background_scale_uncertainty",
        format!("背景缩放 scale={:.4} ± {:.4}，其不确定度已并入误差带", scale, scale_err),
        "background_scale"));
    SubtractionResult { points: out, diags, status: "ok".into() }
}

fn sub_point(s: &Point, bi: f64, berr: f64, scale: f64, scale_err: f64, src: PointSrc) -> Point {
    let i = s.i - scale * bi;
    let err = (s.err.powi(2) + (scale * berr).powi(2) + (bi * scale_err).powi(2)).sqrt();
    let mut flags = s.flags.clone();
    if i < 0.0 { flags.push("negative_intensity".into()); }
    Point { q: s.q, i, err, flags, src: Some(src) }
}

/// 线性最小二乘 y = a + b·x，返回 (a, b, 每点残差)
fn linfit(xs: &[f64], ys: &[f64]) -> (f64, f64, Vec<f64>) {
    let n = xs.len() as f64;
    let (sx, sy) = (xs.iter().sum::<f64>(), ys.iter().sum::<f64>());
    let sxx = xs.iter().map(|x| x * x).sum::<f64>();
    let sxy = xs.iter().zip(ys).map(|(x, y)| x * y).sum::<f64>();
    let det = n * sxx - sx * sx;
    let b = (n * sxy - sx * sy) / det;
    let a = (sy - b * sx) / n;
    let res = xs.iter().zip(ys).map(|(x, y)| y - a - b * x).collect();
    (a, b, res)
}

/// Guinier 区间候选：ln I vs q² -> Rg, I(0)。只产出候选，不与其他区间合并。
pub fn guinier(id: &str, pts: &[Point], qmin: f64, qmax: f64) -> Result<IntervalCandidate, Diag> {
    let sel: Vec<&Point> = pts.iter()
        .filter(|p| p.q >= qmin && p.q <= qmax && p.i > 0.0).collect();
    if sel.len() < 3 {
        return Err(Diag::new("interval_too_sparse",
            format!("Guinier 区间 [{:.4},{:.4}] 内正强度点不足（{} 个）", qmin, qmax, sel.len()),
            id));
    }
    let xs: Vec<f64> = sel.iter().map(|p| p.q * p.q).collect();
    let ys: Vec<f64> = sel.iter().map(|p| p.i.ln()).collect();
    let (a, b, res) = linfit(&xs, &ys);
    let rms = (res.iter().map(|r| r * r).sum::<f64>() / res.len() as f64).sqrt();
    Ok(IntervalCandidate {
        id: id.into(), kind: "guinier".into(), qmin, qmax,
        params: serde_json::json!({"rg": (-3.0 * b).max(0.0).sqrt(), "i0": a.exp()}),
        residuals: res, rms, selected: false,
        note: "Guinier 候选；区间独立评估，不与 Porod 自动拼接".into(),
    })
}

/// Porod 区间候选：ln I vs ln q -> 斜率（期望约 -4）
pub fn porod(id: &str, pts: &[Point], qmin: f64, qmax: f64) -> Result<IntervalCandidate, Diag> {
    let sel: Vec<&Point> = pts.iter()
        .filter(|p| p.q >= qmin && p.q <= qmax && p.i > 0.0).collect();
    if sel.len() < 3 {
        return Err(Diag::new("interval_too_sparse",
            format!("Porod 区间 [{:.4},{:.4}] 内正强度点不足（{} 个）", qmin, qmax, sel.len()),
            id));
    }
    let xs: Vec<f64> = sel.iter().map(|p| p.q.ln()).collect();
    let ys: Vec<f64> = sel.iter().map(|p| p.i.ln()).collect();
    let (a, b, res) = linfit(&xs, &ys);
    let rms = (res.iter().map(|r| r * r).sum::<f64>() / res.len() as f64).sqrt();
    Ok(IntervalCandidate {
        id: id.into(), kind: "porod".into(), qmin, qmax,
        params: serde_json::json!({"slope": b, "exponent": -b, "prefactor": a.exp()}),
        residuals: res, rms, selected: false,
        note: "Porod 候选；区间独立评估，不与 Guinier 自动拼接".into(),
    })
}
