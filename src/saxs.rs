//! SAXS 核心规则：归一化、背景扣除、Guinier/Porod 区间拟合。
//! 所有判断都产出机器可读的 Diagnostic，规则文本常量化以便审计。

use serde::{Deserialize, Serialize};

pub const RULE_GRID: &str = "q 网格不同时不直接相减，插值只在重叠范围生效";
pub const RULE_ERR: &str = "误差包括归一化与背景缩放不确定度";
pub const RULE_INTERVALS: &str = "Guinier 与 Porod 区间不自动拼成单一模型";
pub const RULE_UNITS: &str = "导入时保留原始表示，派生层才做规范化；无法转换的值留在原位置并附诊断";
pub const RULE_NEGATIVE: &str = "负强度不被吞掉或截断，保留并标记";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub q: f64,
    pub i: f64,
    pub err: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub rule: String,
    pub message: String,
    #[serde(default)]
    pub detail: serde_json::Value,
}

impl Diagnostic {
    pub fn new(code: &str, rule: &str, message: impl Into<String>, detail: serde_json::Value) -> Self {
        Diagnostic { code: code.to_string(), rule: rule.to_string(), message: message.into(), detail }
    }
}

/// q 单位换算到规范坐标 nm^-1。无法识别时返回 None（保留原值并附诊断）。
pub fn q_factor_to_nm(unit: &str) -> Option<f64> {
    match unit {
        "nm^-1" | "1/nm" | "nm-1" => Some(1.0),
        "A^-1" | "1/A" | "A-1" | "angstrom^-1" => Some(10.0),
        _ => None,
    }
}

pub fn thickness_to_cm(value: f64, unit: &str) -> Option<f64> {
    match unit {
        "cm" => Some(value),
        "mm" => Some(value / 10.0),
        _ => None,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormParams {
    pub exposure_s: f64,
    pub exposure_rel_unc: f64,
    pub thickness_cm: f64,
    pub thickness_rel_unc: f64,
    pub transmission: f64,
    pub transmission_rel_unc: f64,
}

/// 归一化 I_norm = I / (曝光·厚度·透射率)。
/// 误差传播：σ² = (σI/f)² + I_norm²·(相对不确定度平方和)，覆盖归一化不确定度。
pub fn normalize(points: &[CurvePoint], p: &NormParams) -> (Vec<CurvePoint>, Vec<Diagnostic>) {
    let factor = p.exposure_s * p.thickness_cm * p.transmission;
    let rel2 = p.exposure_rel_unc.powi(2) + p.thickness_rel_unc.powi(2) + p.transmission_rel_unc.powi(2);
    let out = points
        .iter()
        .map(|pt| {
            let i = pt.i / factor;
            let err = ((pt.err / factor).powi(2) + i * i * rel2).sqrt();
            CurvePoint { q: pt.q, i, err }
        })
        .collect();
    let diag = Diagnostic::new(
        "normalization_uncertainty",
        RULE_ERR,
        format!(
            "归一化因子 f={:.6}（曝光 {:.3}s·厚度 {:.4}cm·透射率 {:.4}），相对不确定度 {:.4} 已并入误差",
            factor, p.exposure_s, p.thickness_cm, p.transmission, rel2.sqrt()
        ),
        serde_json::json!({"factor": factor, "rel_unc": rel2.sqrt()}),
    );
    (out, vec![diag])
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubtractResult {
    pub points: Vec<CurvePoint>,
    pub diagnostics: Vec<Diagnostic>,
    pub overlap: Option<(f64, f64)>,
    pub interpolated: bool,
    pub negative_count: usize,
    pub refused: bool,
}

fn grids_match(a: &[CurvePoint], b: &[CurvePoint]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| (x.q - y.q).abs() < 1e-12)
}

fn interp_at(sorted: &[CurvePoint], q: f64) -> Option<(f64, f64)> {
    if sorted.is_empty() || q < sorted[0].q || q > sorted[sorted.len() - 1].q {
        return None;
    }
    let idx = sorted.partition_point(|p| p.q < q);
    if idx == 0 {
        return Some((sorted[0].i, sorted[0].err));
    }
    let (p0, p1) = (&sorted[idx - 1], &sorted[idx.min(sorted.len() - 1)]);
    if (p1.q - p0.q).abs() < 1e-15 {
        return Some((p0.i, p0.err));
    }
    let t = (q - p0.q) / (p1.q - p0.q);
    Some((p0.i + t * (p1.i - p0.i), p0.err + t * (p1.err - p0.err)))
}

fn sub_pair(s: &CurvePoint, bi: f64, berr: f64, scale: f64, scale_unc: f64) -> CurvePoint {
    let i = s.i - scale * bi;
    // 误差包括背景缩放不确定度：σ² = σs² + (scale·σb)² + (b·σscale)²
    let err = (s.err.powi(2) + (scale * berr).powi(2) + (bi * scale_unc).powi(2)).sqrt();
    CurvePoint { q: s.q, i, err }
}

/// 背景扣除。网格一致才直接相减；不一致时只在重叠区间内对缓冲液插值；
/// 无重叠则拒绝并给出诊断。负强度保留并计数。
pub fn subtract(sample: &[CurvePoint], buffer: &[CurvePoint], scale: f64, scale_unc: f64) -> SubtractResult {
    let mut diagnostics = vec![Diagnostic::new(
        "background_scale_uncertainty",
        RULE_ERR,
        format!("背景缩放 scale={:.4} ± {:.4}，其不确定度已并入误差", scale, scale_unc),
        serde_json::json!({"scale": scale, "scale_unc": scale_unc}),
    )];
    let mut interpolated = false;
    let mut refused = false;
    let points: Vec<CurvePoint>;
    let overlap: Option<(f64, f64)>;

    if grids_match(sample, buffer) {
        overlap = sample.first().map(|f| (f.q, sample[sample.len() - 1].q));
        points = sample
            .iter()
            .zip(buffer.iter())
            .map(|(s, b)| sub_pair(s, b.i, b.err, scale, scale_unc))
            .collect();
    } else {
        interpolated = true;
        let slo = sample.first().map(|p| p.q).unwrap_or(0.0);
        let shi = sample.last().map(|p| p.q).unwrap_or(0.0);
        let blo = buffer.first().map(|p| p.q).unwrap_or(0.0);
        let bhi = buffer.last().map(|p| p.q).unwrap_or(0.0);
        let lo = slo.max(blo);
        let hi = shi.min(bhi);
        if lo >= hi {
            diagnostics.push(Diagnostic::new(
                "no_overlap",
                RULE_GRID,
                format!("样品 q∈[{:.4},{:.4}] 与背景 q∈[{:.4},{:.4}] 无重叠区，拒绝相减", slo, shi, blo, bhi),
                serde_json::json!({"sample_range": [slo, shi], "buffer_range": [blo, bhi]}),
            ));
            return SubtractResult { points: vec![], diagnostics, overlap: None, interpolated, negative_count: 0, refused: true };
        }
        let mut kept = Vec::new();
        let mut dropped = 0usize;
        for s in sample {
            if s.q < lo - 1e-12 || s.q > hi + 1e-12 {
                dropped += 1;
                continue;
            }
            match interp_at(buffer, s.q) {
                Some((bi, berr)) => kept.push(sub_pair(s, bi, berr, scale, scale_unc)),
                None => dropped += 1,
            }
        }
        diagnostics.push(Diagnostic::new(
            "grid_mismatch_interpolated",
            RULE_GRID,
            format!(
                "q 网格不一致，不直接相减；仅在重叠区 [{:.4}, {:.4}] nm^-1 内对背景线性插值，丢弃 {} 个区间外点",
                lo, hi, dropped
            ),
            serde_json::json!({"overlap_nm": [lo, hi], "dropped_points": dropped, "kept_points": kept.len()}),
        ));
        overlap = Some((lo, hi));
        points = kept;
    }

    let negative_count = points.iter().filter(|p| p.i < 0.0).count();
    if negative_count > 0 {
        diagnostics.push(Diagnostic::new(
            "negative_intensity",
            RULE_NEGATIVE,
            format!("扣除后出现 {} 个负强度点，保留原值不截断", negative_count),
            serde_json::json!({"count": negative_count}),
        ));
    }
    let _ = refused;
    refused = false;
    SubtractResult { points, diagnostics, overlap, interpolated, negative_count, refused }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FitResult {
    pub kind: String,
    pub qmin: f64,
    pub qmax: f64,
    pub params: serde_json::Value,
    pub chi2: f64,
    pub ndof: usize,
    pub used: usize,
    pub skipped_nonpositive: usize,
    pub residuals: Vec<(f64, f64)>,
    pub flags: Vec<String>,
}

fn weighted_linreg(xs: &[f64], ys: &[f64], ws: &[f64]) -> Option<(f64, f64)> {
    let sw: f64 = ws.iter().sum();
    if sw <= 0.0 {
        return None;
    }
    let sx: f64 = xs.iter().zip(ws).map(|(x, w)| x * w).sum();
    let sy: f64 = ys.iter().zip(ws).map(|(y, w)| y * w).sum();
    let sxx: f64 = xs.iter().zip(ws).map(|(x, w)| x * x * w).sum();
    let sxy: f64 = xs.iter().zip(ys).zip(ws).map(|((x, y), w)| x * y * w).sum();
    let denom = sw * sxx - sx * sx;
    if denom.abs() < 1e-30 {
        return None;
    }
    let b = (sw * sxy - sx * sy) / denom;
    let a = (sy - b * sx) / sw;
    Some((a, b))
}

/// Guinier 拟合：ln I = ln I0 - Rg²q²/3。I<=0 的点不参与但计入诊断。
pub fn guinier(points: &[CurvePoint], qmin: f64, qmax: f64) -> FitResult {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut ws = Vec::new();
    let mut raw = Vec::new();
    let mut skipped = 0usize;
    for p in points.iter().filter(|p| p.q >= qmin && p.q <= qmax) {
        if p.i <= 0.0 {
            skipped += 1;
            continue;
        }
        let rel = if p.i.abs() > 1e-30 { p.err / p.i } else { 1.0 };
        xs.push(p.q * p.q);
        ys.push(p.i.ln());
        ws.push(1.0 / (rel * rel + 1e-12));
        raw.push(p);
    }
    let (a, b) = weighted_linreg(&xs, &ys, &ws).unwrap_or((f64::NAN, f64::NAN));
    let rg = if b < 0.0 { (-3.0 * b).sqrt() } else { f64::NAN };
    let i0 = a.exp();
    let mut chi2 = 0.0;
    let mut residuals = Vec::new();
    for p in &raw {
        let model = (a + b * p.q * p.q).exp();
        let r = (p.i - model) / p.err.max(1e-12);
        chi2 += r * r;
        residuals.push((p.q, r));
    }
    let ndof = raw.len().saturating_sub(2);
    let mut flags = Vec::new();
    if rg.is_finite() && qmax * rg > 1.3 {
        flags.push(format!("qmax·Rg={:.2} > 1.3，超出 Guinier 有效区", qmax * rg));
    }
    FitResult {
        kind: "guinier".into(),
        qmin,
        qmax,
        params: serde_json::json!({"Rg_nm": rg, "I0": i0, "slope": b, "intercept": a}),
        chi2,
        ndof,
        used: raw.len(),
        skipped_nonpositive: skipped,
        residuals,
        flags,
    }
}

/// Porod 拟合：ln I = ln K - p·ln q，报告 Porod 指数 p。
pub fn porod(points: &[CurvePoint], qmin: f64, qmax: f64) -> FitResult {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut ws = Vec::new();
    let mut raw = Vec::new();
    let mut skipped = 0usize;
    for p in points.iter().filter(|p| p.q >= qmin && p.q <= qmax) {
        if p.i <= 0.0 {
            skipped += 1;
            continue;
        }
        let rel = if p.i.abs() > 1e-30 { p.err / p.i } else { 1.0 };
        xs.push(p.q.ln());
        ys.push(p.i.ln());
        ws.push(1.0 / (rel * rel + 1e-12));
        raw.push(p);
    }
    let (a, b) = weighted_linreg(&xs, &ys, &ws).unwrap_or((f64::NAN, f64::NAN));
    let exponent = -b;
    let k = a.exp();
    let mut chi2 = 0.0;
    let mut residuals = Vec::new();
    for p in &raw {
        let model = (a + b * p.q.ln()).exp();
        let r = (p.i - model) / p.err.max(1e-12);
        chi2 += r * r;
        residuals.push((p.q, r));
    }
    let ndof = raw.len().saturating_sub(2);
    let mut flags = Vec::new();
    if exponent.is_finite() && !(3.0..=4.5).contains(&exponent) {
        flags.push(format!("Porod 指数 {:.2} 偏离两相体系预期 ~4", exponent));
    }
    FitResult {
        kind: "porod".into(),
        qmin,
        qmax,
        params: serde_json::json!({"porod_exponent": exponent, "K": k}),
        chi2,
        ndof,
        used: raw.len(),
        skipped_nonpositive: skipped,
        residuals,
        flags,
    }
}

/// 区间重叠检测：只诊断、不合并。
pub fn intervals_overlap(a: (f64, f64), b: (f64, f64)) -> bool {
    a.0 < b.1 && b.0 < a.1
}
