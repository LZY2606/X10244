//! 核心规则引擎（纯函数，无 I/O）：
//! - q 网格不同时不直接相减，插值只在重叠范围生效；
//! - 误差包括归一化误差与背景缩放（α）不确定度；
//! - Guinier 与 Porod 区间只作为并列候选，绝不自动拼成单一模型。

use crate::domain::*;
use std::collections::BTreeSet;

pub const GRID_REL_TOL: f64 = 1e-9;
pub const RULE_GRID: &str = "no_direct_subtract_on_mismatched_grids";
pub const RULE_OVERLAP: &str = "interpolation_only_inside_overlap";
pub const RULE_ALPHA: &str = "uncertainty_includes_normalization_and_scale";
pub const RULE_REGIONS: &str = "guinier_porod_are_parallel_candidates";

#[derive(Debug, Clone)]
pub struct ParsedCurve {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub sigma: Vec<f64>,
    pub rows: Vec<usize>,
    pub diagnostics: Vec<String>,
}

/// 从原始表解析曲线：保留原始表示；无法转换的行附诊断并剔除出派生序列。
pub fn parse_curve(raw: &RawTable) -> ParsedCurve {
    let find = |names: &[&str]| -> Option<usize> {
        raw.columns.iter().position(|c| {
            let n = c.name.to_ascii_lowercase();
            names.iter().any(|want| n == *want || n.starts_with(want))
        })
    };
    let mut diagnostics = Vec::new();
    let qi = match find(&["q", "q(1/a)", "qvec"]) {
        Some(i) => i,
        None => return ParsedCurve { q: vec![], i: vec![], sigma: vec![], rows: vec![],
            diagnostics: vec!["缺少 q 列".into()] },
    };
    let ii = match find(&["i", "intensity", "i(q)", "i1"]) {
        Some(i) => i,
        None => return ParsedCurve { q: vec![], i: vec![], sigma: vec![], rows: vec![],
            diagnostics: vec!["缺少 I 列".into()] },
    };
    let si = find(&["sigma", "error", "err", "i_err", "d_i"]);
    let n = raw.columns[qi].cells.len();
    let (mut q, mut int, mut sig, mut rows) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for row in 0..n {
        let qcell = &raw.columns[qi].cells[row];
        let icell = &raw.columns[ii].cells[row];
        let scell = si.and_then(|c| raw.columns[c].cells.get(row));
        let mut bad: Vec<String> = Vec::new();
        let qv = match qcell.canonical {
            Some(v) if v.is_finite() && v > 0.0 => Some(v),
            _ => { bad.push(format!("q 无法转换（原始 '{}'）", qcell.text)); None }
        };
        let iv = match icell.canonical {
            Some(v) if v.is_finite() => Some(v),
            _ => { bad.push(format!("I 无法转换（原始 '{}'）", icell.text)); None }
        };
        let mut sv = scell.and_then(|c| c.canonical).filter(|v| v.is_finite() && *v >= 0.0);
        if sv.is_none() {
            if let Some(v) = iv {
                sv = Some((v.abs().sqrt()).max(1.0));
                bad.push("误差缺失/非法：按 sqrt(|I|) 下限估计".into());
            }
        }
        if let (Some(qv), Some(iv), Some(sv)) = (qv, iv, sv) {
            q.push(qv); int.push(iv); sig.push(sv); rows.push(row);
        }
        for b in bad { diagnostics.push(format!("行 {row}: {b}")); }
    }
    ParsedCurve { q, i: int, sigma: sig, rows, diagnostics }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlapReport {
    pub q_min: f64,
    pub q_max: f64,
    pub n_sample_in_overlap: usize,
    pub grids_match: bool,
    pub max_rel_drift: f64,
}

/// 重叠判定基于实际 q 向量（Å^-1），不假设网格相同。
pub fn overlap_report(sq: &[f64], bq: &[f64]) -> Option<OverlapReport> {
    if sq.len() < 2 || bq.len() < 2 {
        return None;
    }
    let (slo, shi) = (sq[0], sq[sq.len() - 1]);
    let (blo, bhi) = (bq[0], bq[bq.len() - 1]);
    let lo = slo.max(blo);
    let hi = shi.min(bhi);
    if !(hi > lo) {
        return None;
    }
    let mut grids_match = sq.len() == bq.len();
    let mut max_rel = 0.0;
    if grids_match {
        for (a, b) in sq.iter().zip(bq) {
            let rel = (a - b).abs() / a.abs().max(1e-30);
            max_rel = max_rel.max(rel);
            if rel > GRID_REL_TOL { grids_match = false; }
        }
    }
    let n = sq.iter().filter(|&&q| q >= lo && q <= hi).count();
    Some(OverlapReport { q_min: lo, q_max: hi, n_sample_in_overlap: n,
        grids_match, max_rel_drift: max_rel })
}

fn linear_interp(xs: &[f64], ys: &[f64], x: f64) -> Option<(f64, bool, usize, usize)> {
    if xs.len() < 2 || x < xs[0] || x > xs[xs.len() - 1] {
        return None;
    }
    match xs.binary_search_by(|v| v.partial_cmp(&x).unwrap()) {
        Ok(k) => Some((ys[k], false, k, k)),
        Err(k) if k >= 1 && k < xs.len() => {
            let (a, b) = (k - 1, k);
            let t = (x - xs[a]) / (xs[b] - xs[a]);
            Some((ys[a] + t * (ys[b] - ys[a]), true, a, b))
        }
        Err(_) => None,
    }
}

/// 归一化：I_norm = I_raw / (response · thickness · exposure · transmission)。
/// σ_norm 同时包含计数误差传播与归一化因子（透射率/厚度/仪器响应）不确定度。
pub fn normalize(mat: &MaterialVersion) -> DerivedRecord {
    let curve = parse_curve(&mat.raw);
    let mut diagnostics = curve.diagnostics.clone();
    let t_s = mat.exposure.canonical;
    let d_cm = mat.thickness.canonical;
    let tr = mat.transmission.canonical;
    let resp = if mat.instrument.factor.is_finite() && mat.instrument.factor > 0.0 {
        Some(mat.instrument.factor)
    } else {
        None
    };
    for (name, q) in [("曝光", t_s), ("样品厚度", d_cm), ("透射率", tr)] {
        if q.is_none() {
            let txt = match name {
                "曝光" => &mat.exposure.raw_text,
                "样品厚度" => &mat.thickness.raw_text,
                _ => &mat.transmission.raw_text,
            };
            diagnostics.push(format!("{name}无法规范化：原始表示 '{txt}' 保留在导入层，不据此绘图"));
        }
    }
    if resp.is_none() {
        diagnostics.push("仪器响应因子非法".into());
    }
    let ready = t_s.is_some() && d_cm.is_some() && tr.is_some() && resp.is_some();
    if !ready {
        return DerivedRecord::Blocked {
            reason: "normalization_inputs_not_normalizable".into(),
            diagnostics,
        };
    }
    let (t_s, d_cm, tr, resp) = (t_s.unwrap(), d_cm.unwrap(), tr.unwrap(), resp.unwrap());
    let norm = resp * d_cm * t_s * tr;
    let rel_factor2 = 0.02_f64.powi(2) + 0.01_f64.powi(2) + mat.instrument.rel_uncertainty.powi(2);
    let mut points = Vec::with_capacity(curve.q.len());
    for (k, ((&q, &i), &s)) in curve.q.iter().zip(&curve.i).zip(&curve.sigma).enumerate() {
        let i_n = i / norm;
        let rel_counts = if i.abs() > 0.0 { s / i } else { 1.0 };
        let sigma = i_n.abs() * (rel_counts * rel_counts + rel_factor2).sqrt();
        let mut flags = Vec::new();
        if i < 0.0 { flags.push("raw_negative".into()); }
        points.push(NormPoint {
            id: format!("{}#v{}#p{}", mat.id, mat.version, curve.rows[k]),
            q, i: i_n, sigma, sigma_norm: sigma,
            refs: vec![PointRef { material_id: mat.id.clone(),
                material_version: mat.version, row: curve.rows[k], q }],
            flags,
        });
    }
    DerivedRecord::Normalized(NormalizedArtifact {
        points, exposure_s: Some(t_s), thickness_cm: Some(d_cm),
        transmission: Some(tr), response_factor: resp, diagnostics,
    })
}

#[derive(Debug, Clone)]
pub struct ScaledOutput {
    pub artifact: ScaledBufferArtifact,
    pub overlap: OverlapReport,
}

/// 把归一化缓冲液缩放到样品 q 网格；只在重叠区间内产出点。
/// q 网格不一致时走插值路径并显式标注，绝不逐点直接相减。
pub fn scale_buffer(
    sample: &NormalizedArtifact,
    buffer: &NormalizedArtifact,
    buffer_id: &str,
    buffer_version: u32,
    alpha: f64,
    alpha_sigma: f64,
    method: &str,
) -> Result<ScaledOutput, (String, Vec<String>)> {
    let sq: Vec<f64> = sample.points.iter().map(|p| p.q).collect();
    let bq: Vec<f64> = buffer.points.iter().map(|p| p.q).collect();
    let overlap = match overlap_report(&sq, &bq) {
        Some(o) => o,
        None => return Err(("no_overlap_q_range".into(),
            vec!["样品与缓冲液 q 范围无重叠：拒绝背景扣除，不输出曲线".into()])),
    };
    let mut diagnostics = Vec::new();
    if !overlap.grids_match {
        diagnostics.push(format!(
            "q 网格错位（最大相对漂移 {:.2e}，容差 {:.0e}）：不直接相减；仅在重叠区 [{:.4},{:.4}] Å^-1 内对缓冲液线性插值",
            overlap.max_rel_drift, GRID_REL_TOL, overlap.q_min, overlap.q_max));
    }
    let bi: Vec<f64> = buffer.points.iter().map(|p| p.i).collect();
    let bs: Vec<f64> = buffer.points.iter().map(|p| p.sigma).collect();
    let mut points = Vec::new();
    for sp in &sample.points {
        if sp.q < overlap.q_min - 1e-12 || sp.q > overlap.q_max + 1e-12 { continue; }
        let (val, interp, ra, rb) = match linear_interp(&bq, &bi, sp.q) {
            Some(v) => v, None => continue,
        };
        let sigma_pre = if ra == rb { bs[ra] } else {
            let t = (sp.q - bq[ra]) / (bq[rb] - bq[ra]);
            let s1 = bs[ra] * (1.0 - t);
            let s2 = bs[rb] * t;
            (s1 * s1 + s2 * s2).sqrt()
        };
        let mut refs = BTreeSet::new();
        refs.insert(buffer.points[ra].refs[0].clone());
        refs.insert(buffer.points[rb].refs[0].clone());
        points.push(ScaledPoint {
            id: format!("{}#v{}@{:.6}", buffer_id, buffer_version, sp.q),
            q: sp.q, i_buf: alpha * val, sigma_buf: alpha * sigma_pre,
            refs: refs.into_iter().collect(), interpolated: interp,
        });
    }
    diagnostics.push(format!("背景缩放：α = {:.5} ± {:.5}（{}）", alpha, alpha_sigma, method));
    Ok(ScaledOutput {
        artifact: ScaledBufferArtifact {
            alpha, alpha_sigma, method: method.into(),
            q_min: overlap.q_min, q_max: overlap.q_max, points, diagnostics,
        },
        overlap,
    })
}

/// 扣背景：I_sub = I_s − α·I_b。
/// σ² = σ_s² + (α σ_b)² + (σ_α · I_b)² —— 误差含归一化与背景缩放不确定度。
pub fn subtract(
    sample: &NormalizedArtifact,
    scaled: &ScaledBufferArtifact,
    buffer_id: &str,
    buffer_version: u32,
) -> SubtractedArtifact {
    let mut points = Vec::new();
    let mut n_negative = 0usize;
    let mut diagnostics = Vec::new();
    for bp in &scaled.points {
        let sp = match sample.points.binary_search_by(|p| p.q.partial_cmp(&bp.q).unwrap()) {
            Ok(k) => &sample.points[k],
            Err(_) => continue,
        };
        let i_sub = sp.i - bp.i_buf;
        let scale_term = scaled.alpha_sigma * (bp.i_buf / scaled.alpha.max(1e-300));
        let sigma = (sp.sigma * sp.sigma
            + bp.sigma_buf * bp.sigma_buf
            + scale_term * scale_term).sqrt();
        let mut flags = Vec::new();
        if i_sub < 0.0 {
            n_negative += 1;
            flags.push("negative_after_subtraction".into());
        }
        if i_sub - sigma < 0.0 { flags.push("negative_within_1sigma".into()); }
        if bp.interpolated { flags.push("buffer_interpolated".into()); }
        points.push(SubPoint {
            id: format!("sub@{}#v{}@{:.6}", buffer_id, buffer_version, bp.q),
            q: bp.q, i_sub, sigma_sub: sigma,
            i_sample: sp.i, i_buffer_scaled: bp.i_buf,
            refs_sample: sp.refs.clone(), refs_buffer: bp.refs.clone(),
            interpolated_buffer: bp.interpolated, flags,
        });
    }
    if n_negative > 0 {
        diagnostics.push(format!("重叠区内 {n_negative} 个点扣背景后为负：保留点与诊断，不截断"));
    }
    SubtractedArtifact { points, alpha: scaled.alpha, alpha_sigma: scaled.alpha_sigma,
        n_negative, diagnostics }
}

/// 背景缩放候选（同一重叠区可给多个 α，审阅者按残差裁定）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScaleCandidate {
    pub id: String,
    pub method: String,
    pub alpha: f64,
    pub alpha_sigma: f64,
    pub chi2: f64,
    pub residual_rms: f64,
    pub q_lo: f64,
    pub q_hi: f64,
    pub n_points: usize,
    pub note: String,
}

fn interp_pair(sample: &NormalizedArtifact, buffer: &NormalizedArtifact,
    q_lo: f64, q_hi: f64) -> Vec<(f64, f64, f64, f64, f64)> {
    let bq: Vec<f64> = buffer.points.iter().map(|p| p.q).collect();
    let bi: Vec<f64> = buffer.points.iter().map(|p| p.i).collect();
    let bs: Vec<f64> = buffer.points.iter().map(|p| p.sigma).collect();
    let mut out = Vec::new();
    for sp in &sample.points {
        if sp.q < q_lo || sp.q > q_hi { continue; }
        if let Some((bv, interp, ra, rb)) = linear_interp(&bq, &bi, sp.q) {
            let bsig = if ra == rb { bs[ra] } else {
                let t = (sp.q - bq[ra]) / (bq[rb] - bq[ra]);
                let s1 = bs[ra] * (1.0 - t);
                let s2 = bs[rb] * t;
                (s1 * s1 + s2 * s2).sqrt()
            };
            out.push((sp.q, sp.i, sp.sigma, bv, bsig));
            let _ = interp;
        }
    }
    out
}

/// 在给定窗口内加权最小二乘 α（过原点），返回 α、σ_α、χ²、残差 RMS。
pub fn fit_alpha(
    sample: &NormalizedArtifact,
    buffer: &NormalizedArtifact,
    q_lo: f64,
    q_hi: f64,
) -> Option<ScaleCandidate> {
    let pairs = interp_pair(sample, buffer, q_lo, q_hi);
    if pairs.len() < 3 { return None; }
    let mut swxx = 0.0;
    let mut swxy = 0.0;
    for (_, is, ss, ib, sb) in &pairs {
        let w = 1.0 / (ss * ss + sb * sb).max(1e-30);
        swxx += w * ib * ib;
        swxy += w * ib * is;
    }
    if swxx <= 0.0 { return None; }
    let alpha = swxy / swxx;
    let alpha_sigma = 1.0 / swxx.sqrt();
    let mut chi2 = 0.0;
    let mut sq = 0.0;
    for (_, is, ss, ib, sb) in &pairs {
        let r = is - alpha * ib;
        let var = ss * ss + (alpha * sb) * (alpha * sb) + (alpha_sigma * ib) * (alpha_sigma * ib);
        chi2 += r * r / var.max(1e-30);
        sq += r * r;
    }
    let n = pairs.len();
    Some(ScaleCandidate {
        id: format!("alpha@{:.4}-{:.4}", q_lo, q_hi),
        method: format!("wls_over_q_{:.4}_{:.4}", q_lo, q_hi),
        alpha, alpha_sigma, chi2,
        residual_rms: (sq / n as f64).sqrt(),
        q_lo, q_hi, n_points: n, note: String::new(),
    })
}

/// 生成多个 α 候选：高 q 尾窗口（Porod 区惯例）与全重叠区。
pub fn alpha_candidates(
    sample: &NormalizedArtifact,
    buffer: &NormalizedArtifact,
    ov: &OverlapReport,
) -> Vec<ScaleCandidate> {
    let mut out = Vec::new();
    let span = ov.q_max - ov.q_min;
    let tail_lo = ov.q_min + 0.55 * span;
    if let Some(mut c) = fit_alpha(sample, buffer, tail_lo, ov.q_max) {
        c.id = format!("alpha_tail@{:.4}-{:.4}", tail_lo, ov.q_max);
        c.note = "高 q 尾窗口缩放".into();
        out.push(c);
    }
    if let Some(mut c) = fit_alpha(sample, buffer, ov.q_min, ov.q_max) {
        c.id = format!("alpha_full@{:.4}-{:.4}", ov.q_min, ov.q_max);
        c.note = "全重叠区缩放".into();
        out.push(c);
    }
    if let Some(mut c) = fit_alpha(sample, buffer, ov.q_min, ov.q_min + 0.35 * span) {
        c.id = format!("alpha_low@{:.4}-{:.4}", ov.q_min, ov.q_min + 0.35 * span);
        c.note = "低 q 窗口缩放（易受样品信号污染）".into();
        out.push(c);
    }
    // 标注残差不一致
    if out.len() >= 2 {
        let alphas: Vec<f64> = out.iter().map(|c| c.alpha).collect();
        let spread = alphas.iter().cloned().fold(f64::MIN, f64::max)
            - alphas.iter().cloned().fold(f64::MAX, f64::min);
        let sigs: Vec<f64> = out.iter().map(|c| c.alpha_sigma).collect();
        if spread > 2.0 * sigs.iter().cloned().fold(0.0, f64::max) {
            for c in out.iter_mut() {
                c.note.push_str("；候选 α 超出 2σ 一致性，需要人工裁定");
            }
        }
    }
    out
}

fn wls_line(x: &[f64], y: &[f64], w: &[f64]) -> Option<(f64, f64)> {
    let n = x.len();
    if n < 3 { return None; }
    let (mut sw, mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for i in 0..n {
        sw += w[i]; sx += w[i] * x[i]; sy += w[i] * y[i];
        sxx += w[i] * x[i] * x[i]; sxy += w[i] * x[i] * y[i];
    }
    let det = sw * sxx - sx * sx;
    if det.abs() < 1e-30 { return None; }
    let b = (sw * sxy - sx * sy) / det;
    let a = (sy - b * sx) / sw;
    Some((a, b))
}

/// Guinier：ln I = ln I0 − q²Rg²/3；两个并列窗口，仅输出候选。
fn guinier_windows(sub: &SubtractedArtifact) -> Vec<WindowFit> {
    let pts: Vec<&SubPoint> = sub.points.iter()
        .filter(|p| p.i_sub > 0.0 && p.sigma_sub > 0.0).collect();
    if pts.len() < 8 { return vec![]; }
    let qmax = pts[pts.len() - 1].q;
    let bounds = [
        ("guinier_narrow", (qmax * 0.0 + pts[0].q, qmax * 0.55), "qRg 更保守窗口"),
        ("guinier_wide", (pts[0].q, qmax * 0.75), "qRg 更宽窗口"),
    ];
    let mut fits = Vec::new();
    for (id, (lo, hi), note) in bounds {
        let sel: Vec<&&SubPoint> = pts.iter().filter(|p| p.q >= lo && p.q <= hi).collect();
        if sel.len() < 4 { continue; }
        let x: Vec<f64> = sel.iter().map(|p| p.q * p.q).collect();
        let y: Vec<f64> = sel.iter().map(|p| p.i_sub.ln()).collect();
        let w: Vec<f64> = sel.iter().map(|p| {
            let rel = p.sigma_sub / p.i_sub; (rel * rel).recip()
        }).collect();
        let (a, b) = match wls_line(&x, &y, &w) { Some(v) => v, None => continue };
        let rg = (-3.0 * b).sqrt();
        let i0 = a.exp();
        let mut chi2 = 0.0;
        let (mut qv, mut fv, mut rv) = (Vec::new(), Vec::new(), Vec::new());
        for p in &sel {
            let pred = i0 * (-p.q * p.q * rg * rg / 3.0).exp();
            let r = (p.i_sub - pred) / p.sigma_sub;
            chi2 += r * r;
            qv.push(p.q); fv.push(pred); rv.push(p.i_sub - pred);
        }
        let rms = (rv.iter().map(|v| v * v).sum::<f64>() / rv.len() as f64).sqrt();
        fits.push(WindowFit {
            id: id.into(), model: "guinier".into(), q_lo: lo, q_hi: hi,
            params: vec![("Rg".into(), rg), ("I0".into(), i0)],
            chi2, residual_rms: rms, n_points: sel.len(), overlap_with: vec![],
            q_values: qv, fit_values: fv, residuals: rv, note: note.into(),
        });
    }
    fits
}

/// Porod：高 q 尾部 I = A + K/q⁴ 的线性化 I 对 q⁻⁴；两个并列窗口。
fn porod_windows(sub: &SubtractedArtifact) -> Vec<WindowFit> {
    let pts: Vec<&SubPoint> = sub.points.iter()
        .filter(|p| p.i_sub > 0.0 && p.sigma_sub > 0.0).collect();
    if pts.len() < 8 { return vec![]; }
    let n = pts.len();
    let bounds = [
        ("porod_core", pts[(n * 6) / 10].q, pts[n - 1].q, "高 q 核心窗口"),
        ("porod_extended", pts[n / 2].q, pts[n - 1].q, "向中区延伸窗口"),
    ];
    let mut fits = Vec::new();
    for (id, lo, hi), ) in std::array::IntoIter::new(bounds) {}
    fits
}
