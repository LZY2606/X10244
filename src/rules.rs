//! 核心散射规则：归一化、背景扣除、Guinier/Porod 区间拟合。
//! 每条规则都输出机器可读的判定依据（judgments），不吞掉多解或不确定状态。

use crate::model::{Point, STATUS_ANOMALY, STATUS_BLOCKED, STATUS_MULTI, STATUS_OK};
use serde_json::{json, Value};

pub struct RuleOutput {
    pub status: &'static str,
    pub curve: Option<Vec<Point>>,
    pub candidates: Vec<Value>,
    pub judgments: Vec<Value>,
    pub diagnostics: Vec<String>,
    pub extra: Value,
}

impl RuleOutput {
    fn new() -> Self {
        RuleOutput {
            status: STATUS_OK,
            curve: None,
            candidates: Vec::new(),
            judgments: Vec::new(),
            diagnostics: Vec::new(),
            extra: json!({}),
        }
    }
}

/// 单位规范化：原始表示保留在材料/曲线记录里，派生层才转换。
pub fn canonical_q_unit(u: &str) -> Result<&'static str, String> {
    match u {
        "1/angstrom" | "A^-1" | "1/A" | "angstrom^-1" => Ok("A^-1"),
        "1/nm" | "nm^-1" => Ok("nm^-1"),
        other => Err(format!("无法转换的 q 单位: {other}")),
    }
}

pub fn canonical_i_unit(u: &str) -> Result<&'static str, String> {
    match u {
        "1/cm" | "cm^-1" => Ok("cm^-1"),
        "1/m" | "m^-1" => Ok("m^-1"),
        other => Err(format!("无法转换的 I 单位: {other}")),
    }
}

/// 归一化：I_norm = I / (exposure * thickness * transmission)。
/// 误差包含归一化不确定度：sigma^2 = (sigma/f)^2 + (I_norm * u_norm)^2。
pub fn normalize(
    curve: &[Point],
    exposure_s: f64,
    thickness_mm: f64,
    transmission: f64,
    norm_rel_unc: f64,
) -> RuleOutput {
    let mut o = RuleOutput::new();
    let factor = exposure_s * thickness_mm * transmission;
    if !(factor > 0.0) || !factor.is_finite() {
        o.status = STATUS_BLOCKED;
        o.judgments.push(json!({
            "rule": "normalization_factor_positive",
            "basis": {"exposure_s": exposure_s, "thickness_mm": thickness_mm, "transmission": transmission},
            "decision": "blocked",
        }));
        o.diagnostics.push("归一化因子非正，推导被阻止".into());
        return o;
    }
    let pts: Vec<Point> = curve
        .iter()
        .map(|p| {
            let i = p.i / factor;
            let sigma = ((p.sigma / factor).powi(2) + (i * norm_rel_unc).powi(2)).sqrt();
            Point { q: p.q, i, sigma }
        })
        .collect();
    o.judgments.push(json!({
        "rule": "normalization_uncertainty",
        "basis": {"factor": factor, "norm_rel_unc": norm_rel_unc},
        "decision": "error_includes_normalization_uncertainty",
    }));
    o.extra = json!({"factor": factor, "norm_rel_unc": norm_rel_unc});
    o.curve = Some(pts);
    o
}

fn interp(curve: &[Point], q: f64) -> (f64, f64) {
    if q <= curve[0].q {
        return (curve[0].i, curve[0].sigma);
    }
    if q >= curve[curve.len() - 1].q {
        let l = &curve[curve.len() - 1];
        return (l.i, l.sigma);
    }
    for w in curve.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if q >= a.q && q <= b.q {
            let t = (q - a.q) / (b.q - a.q);
            return (a.i + t * (b.i - a.i), a.sigma + t * (b.sigma - a.sigma));
        }
    }
    let l = &curve[curve.len() - 1];
    (l.i, l.sigma)
}

/// 背景扣除。语义：
/// - q 网格不同时不直接相减，插值只在重叠范围生效；
/// - 无重叠区时推导被阻止，不产生结果曲线；
/// - 误差包含背景缩放不确定度：sigma^2 加上 (I_buffer * scale_unc)^2；
/// - 负强度不截断，标记为异常点保留。
pub fn subtract(sample: &[Point], buffer: &[Point], scale: f64, scale_unc: f64) -> RuleOutput {
    let mut o = RuleOutput::new();
    let (s_lo, s_hi) = (sample[0].q, sample[sample.len() - 1].q);
    let (b_lo, b_hi) = (buffer[0].q, buffer[buffer.len() - 1].q);
    let lo = s_lo.max(b_lo);
    let hi = s_hi.min(b_hi);
    if lo >= hi {
        o.status = STATUS_BLOCKED;
        o.judgments.push(json!({
            "rule": "no_overlap",
            "basis": {"sample_range": [s_lo, s_hi], "buffer_range": [b_lo, b_hi]},
            "decision": "blocked_no_result_curve",
        }));
        o.diagnostics.push(format!(
            "样品 q 范围 [{s_lo}, {s_hi}] 与背景 [{b_lo}, {b_hi}] 无重叠，扣除被阻止"
        ));
        o.extra = json!({"overlap": null});
        return o;
    }
    let grids_equal = sample.len() == buffer.len()
        && sample
            .iter()
            .zip(buffer.iter())
            .all(|(a, b)| (a.q - b.q).abs() < 1e-12);
    if !grids_equal {
        o.judgments.push(json!({
            "rule": "grid_mismatch",
            "basis": {"sample_range": [s_lo, s_hi], "buffer_range": [b_lo, b_hi], "overlap": [lo, hi]},
            "decision": "interpolate_within_overlap_only",
        }));
    }
    let mut pts = Vec::new();
    let mut per_point = Vec::new();
    let mut anomalies = Vec::new();
    let mut dropped = 0usize;
    for sp in sample {
        if sp.q < lo || sp.q > hi {
            dropped += 1;
            continue;
        }
        let (bi, bs) = interp(buffer, sp.q);
        let i = sp.i - scale * bi;
        let sigma = (sp.sigma.powi(2) + (scale * bs).powi(2) + (bi * scale_unc).powi(2)).sqrt();
        let idx = pts.len();
        if i < 0.0 {
            anomalies.push(idx);
        }
        pts.push(Point { q: sp.q, i, sigma });
        per_point.push(json!({
            "q": sp.q, "i_sample": sp.i, "sigma_sample": sp.sigma,
            "i_buffer": bi, "sigma_buffer": bs, "scale": scale,
            "i_result": i, "sigma_result": sigma,
        }));
    }
    o.judgments.push(json!({
        "rule": "background_scale_uncertainty",
        "basis": {"scale": scale, "scale_unc": scale_unc},
        "decision": "error_includes_background_scale_uncertainty",
    }));
    if !anomalies.is_empty() {
        o.status = STATUS_ANOMALY;
        o.judgments.push(json!({
            "rule": "negative_intensity",
            "basis": {"point_indices": anomalies, "count": anomalies.len()},
            "decision": "kept_flagged_not_clamped",
        }));
        o.diagnostics
            .push(format!("扣除后出现 {} 个负强度点，已标记保留", anomalies.len()));
    }
    o.extra = json!({
        "overlap": [lo, hi],
        "interpolated": !grids_equal,
        "dropped_points": dropped,
        "anomalies": anomalies,
        "per_point": per_point,
        "scale": scale,
        "scale_unc": scale_unc,
    });
    o.curve = Some(pts);
    o
}

fn linfit(xs: &[f64], ys: &[f64]) -> (f64, f64, f64) {
    let n = xs.len() as f64;
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let denom = n * sxx - sx * sx;
    if denom.abs() < 1e-30 {
        return (0.0, sy / n, f64::NAN);
    }
    let slope = (n * sxy - sx * sy) / denom;
    let intercept = (sy - slope * sx) / n;
    let rmse = (xs
        .iter()
        .zip(ys)
        .map(|(x, y)| {
            let r = y - (slope * x + intercept);
            r * r
        })
        .sum::<f64>()
        / n)
        .sqrt();
    (slope, intercept, rmse)
}

fn intervals_overlap(intervals: &[[f64; 2]]) -> bool {
    for i in 0..intervals.len() {
        for j in (i + 1)..intervals.len() {
            if intervals[i][0] < intervals[j][1] && intervals[j][0] < intervals[i][1] {
                return true;
            }
        }
    }
    false
}

/// Guinier 区间拟合：ln I ~ q^2。区间可重叠；多候选全部保留，不自动合并。
pub fn guinier(curve: &[Point], intervals: &[[f64; 2]]) -> RuleOutput {
    let mut o = RuleOutput::new();
    for iv in intervals {
        let sel: Vec<&Point> = curve
            .iter()
            .filter(|p| p.q >= iv[0] && p.q <= iv[1] && p.i > 0.0)
            .collect();
        if sel.len() < 4 {
            o.diagnostics
                .push(format!("Guinier 区间 [{}, {}] 有效点不足，跳过", iv[0], iv[1]));
            continue;
        }
        let xs: Vec<f64> = sel.iter().map(|p| p.q * p.q).collect();
        let ys: Vec<f64> = sel.iter().map(|p| p.i.ln()).collect();
        let (slope, intercept, rmse) = linfit(&xs, &ys);
        if slope >= 0.0 {
            o.diagnostics
                .push(format!("Guinier 区间 [{}, {}] 斜率非负，无法给出 Rg", iv[0], iv[1]));
            continue;
        }
        o.candidates.push(json!({
            "interval": iv,
            "rg_A": (-3.0 * slope).sqrt(),
            "i0": intercept.exp(),
            "residual": rmse,
            "n_points": sel.len(),
            "slope": slope,
            "intercept": intercept,
        }));
    }
    finish_fit(&mut o, "guinier", intervals);
    o
}

/// Porod 区间拟合：ln I ~ ln q，指数 = -slope。同样不自动合并候选。
pub fn porod(curve: &[Point], intervals: &[[f64; 2]]) -> RuleOutput {
    let mut o = RuleOutput::new();
    for iv in intervals {
        let sel: Vec<&Point> = curve
            .iter()
            .filter(|p| p.q >= iv[0] && p.q <= iv[1] && p.i > 0.0)
            .collect();
        if sel.len() < 4 {
            o.diagnostics
                .push(format!("Porod 区间 [{}, {}] 有效点不足，跳过", iv[0], iv[1]));
            continue;
        }
        let xs: Vec<f64> = sel.iter().map(|p| p.q.ln()).collect();
        let ys: Vec<f64> = sel.iter().map(|p| p.i.ln()).collect();
        let (slope, intercept, rmse) = linfit(&xs, &ys);
        o.candidates.push(json!({
            "interval": iv,
            "porod_exponent": -slope,
            "residual": rmse,
            "n_points": sel.len(),
            "slope": slope,
            "intercept": intercept,
        }));
    }
    finish_fit(&mut o, "porod", intervals);
    o
}

fn finish_fit(o: &mut RuleOutput, kind: &str, intervals: &[[f64; 2]]) {
    o.status = match o.candidates.len() {
        0 => STATUS_BLOCKED,
        1 => STATUS_OK,
        _ => STATUS_MULTI,
    };
    if intervals_overlap(intervals) {
        o.judgments.push(json!({
            "rule": "interval_overlap",
            "basis": {"kind": kind, "intervals": intervals},
            "decision": "keep_all_candidates_no_merge",
        }));
    }
    o.judgments.push(json!({
        "rule": "no_auto_model_merge",
        "basis": {"kind": kind, "candidate_count": o.candidates.len()},
        "decision": "guinier_and_porod_intervals_not_combined_into_single_model",
    }));
    o.extra = json!({"kind": kind, "intervals": intervals});
}
