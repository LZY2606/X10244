//! Core SAXS rules. These functions are pure and deterministic; the derived
//! layer is recomputed from raw records + a rules version.
//!
//! Fixed semantics (do not weaken):
//! * q grids that differ are never subtracted pointwise; interpolation applies
//!   only inside the overlapping q range; points outside overlap are retained
//!   but carry no subtraction.
//! * uncertainty includes both normalisation (transmission) and fitted
//!   background-scale variance.
//! * Guinier and Porod intervals are separate candidates and are never
//!   automatically stitched into a single model.
//! * negative subtracted intensities are kept and flagged, never clipped.

use crate::id::stable_id;
use crate::model::*;
use crate::units::{exposure_seconds, thickness_cm, transmission_factor};
use serde_json::json;

/// Version of the rule set. Embedded in every derived id; bumping it changes
/// derived identities and the graph reports nodes as needing recompute.
pub const RULES_VERSION: &str = "saxs-rules-1.0.0";

fn j_overlap(lo: f64, hi: f64, n: usize, any_grid: bool) -> Judgment {
    let ev = json!({"overlap_lo": lo, "overlap_hi": hi, "overlap_points": n, "grids_differ": any_grid});
    if n == 0 {
        Judgment::blocked("grid.no_overlap", "q 网格无重叠区：不做相减", ev)
    } else if any_grid {
        Judgment::warn(
            "grid.misaligned",
            "q 网格错位：未直接相减，缓冲液在重叠区内线性插值",
            ev,
        )
    } else {
        Judgment::ok("grid.overlap_only", "仅在重叠 q 范围内插值并相减", ev)
    }
}

fn grids_differ(a: &[f64], b: &[f64], tol: f64) -> bool {
    if a.len() != b.len() {
        return true;
    }
    a.iter().zip(b).any(|(x, y)| (x - y).abs() > tol)
}

/// Overlap half-open interval of two sorted, strictly increasing q arrays.
/// Returns (lo, hi) or None when the ranges do not intersect.
pub fn overlap_range(qa: &[f64], qb: &[f64]) -> Option<(f64, f64)> {
    if qa.is_empty() || qb.is_empty() {
        return None;
    }
    let lo = qa[0].max(qb[0]);
    let hi = qa[qa.len() - 1].min(qb[qb.len() - 1]);
    if lo <= hi {
        Some((lo, hi))
    } else {
        None
    }
}

/// Linear interpolation of the buffer (q,y) onto a sample q. Only valid inside
/// the closed overlap; returns None outside. `lo_i/hi_i` are bracketing indices.
pub fn interp_within(
    q: f64,
    qb: &[f64],
    yb: &[f64],
    ov_lo: f64,
    ov_hi: f64,
) -> Option<(f64, usize, usize)> {
    if q < ov_lo - 1e-12 || q > ov_hi + 1e-12 {
        return None;
    }
    // Find first buffer q >= q.
    let mut hi = match qb.binary_search_by(|v| v.partial_cmp(&q).unwrap()) {
        Ok(i) => i,
        Err(i) => i,
    };
    if hi == 0 {
        hi = 1;
    }
    if hi >= qb.len() {
        hi = qb.len() - 1;
    }
    let lo_i = hi - 1;
    let (q0, q1) = (qb[lo_i], qb[hi]);
    let t = if q1 > q0 { (q - q0) / (q1 - q0) } else { 0.0 };
    let y = yb[lo_i] + t * (yb[hi] - yb[lo_i]);
    Some((y, lo_i, hi))
}

/// Normalise one raw profile: I_norm = I / (T · t_exp) on the canonical q grid
/// (Å⁻¹). Masked/unparseable/unconvertible values are retained on the raw
/// record and reported via judgments; only usable points produce NormPoints.
pub fn normalize(p: &RawProfile) -> NormCurve {
        let mut judgments = Vec::new();
        let Some(basis) = p.q_basis else {
            judgments.push(Judgment::blocked(
                "import.q_basis_unknown",
                "q 坐标基准无法识别：保留原始表示，不进入派生层",
                json!({"q_basis_raw": p.q_basis_raw, "profile": p.id}),
            ));
            return NormCurve {
                id: stable_id("norm", &[RULES_VERSION, &p.id]),
                source_profile: p.id.clone(),
                role: p.role,
                label: p.label.clone(),
                version: p.version.clone(),
                points: vec![],
                norm_rel_sigma: None,
                judgments,
            };
        };

        let trans = transmission_factor(&p.transmission);
        let expo = exposure_seconds(&p.exposure);
        let thickness = thickness_cm(&p.thickness);

        let t = trans.unwrap_or(f64::NAN);
        let e = expo.unwrap_or(f64::NAN);
        if trans.is_none() {
            judgments.push(Judgment::blocked(
                "norm.transmission_unconvertible",
                "透射率无法解释为无量纲因子：保留原值，不归一化",
                json!({"profile": p.id, "raw": p.transmission.raw, "unit": p.transmission.unit}),
            ));
        }
        if expo.is_none() {
            judgments.push(Judgment::blocked(
                "norm.exposure_unconvertible",
                "曝光时间无法换算为秒：保留原值，不归一化",
                json!({"profile": p.id, "raw": p.exposure.raw, "unit": p.exposure.unit}),
            ));
        }
        if thickness.is_none() {
            // Thickness only needed for absolute scaling; keep working in
            // transmission·exposure normalised units but flag it.
            judgments.push(Judgment::warn(
                "norm.thickness_unconvertible",
                "样品厚度无法换算为 cm：保留原值，绝对标度不可用",
                json!({"profile": p.id, "raw": p.thickness.raw, "unit": p.thickness.unit}),
            ));
        }
        if let Some(v) = trans {
            if !(v > 0.0 && v <= 1.0 + 1e-9) {
                judgments.push(Judgment::warn(
                    "norm.transmission_out_of_range",
                    "透射率超出 (0,1]，结果保留但需复核",
                    json!({"profile": p.id, "transmission": v}),
                ));
            }
        }

        let factor = t * e;
        let usable = trans.is_some() && expo.is_some() && factor.is_finite() && factor > 0.0;
        if !usable {
            judgments.push(Judgment::blocked(
                "norm.factor_invalid",
                "归一化因子 T·曝光 无效：不产生派生点",
                json!({"profile": p.id, "transmission": trans, "exposure_s": expo}),
            ));
        }

        let factor_b = basis.to_q_inv_angstrom();
        let mut points = Vec::new();
        for pt in &p.points {
            if pt.masked {
                continue;
            }
            let (Some(q0), Some(i0), Some(e0)) = (pt.q, pt.i, pt.e) else {
                judgments.push(Judgment::warn(
                    "import.point_unparseable",
                    "点存在无法解析的 q/I/误差：保留原始文本，跳过派生",
                    json!({"profile": p.id, "index": pt.index, "q": pt.q_raw, "i": pt.i_raw, "e": pt.e_raw}),
                ));
                continue;
            };
            if !usable {
                break;
            }
            let q = q0 * factor_b;
            let i = i0 / factor;
            let sigma = (e0 / factor).abs();
            points.push(NormPoint {
                id: stable_id("npt", &[&p.id, &pt.index.to_string()]),
                q,
                i,
                sigma_stat: sigma,
                source_profile: p.id.clone(),
                source_index: pt.index,
                q_source_basis: q0,
            });
        }

        NormCurve {
            id: stable_id("norm", &[RULES_VERSION, &p.id]),
            source_profile: p.id.clone(),
            role: p.role,
            label: p.label.clone(),
            version: p.version.clone(),
            points,
            norm_rel_sigma: Some(p.transmission_rel_uncertainty),
            judgments,
        }
}

/// Weighted least squares estimate of buffer scale k minimising
/// Σ w (I_s - k I_b)^2 over the fit window inside the overlap. Weights use the
/// buffer's statistical variance. Returns (k, sigma_k, rms, chi2, n).
pub fn fit_scale(
    sample: &NormCurve,
    buffer: &NormCurve,
    fit_lo: f64,
    fit_hi: f64,
) -> Option<(f64, f64, f64, f64, usize)> {
    let sq: Vec<f64> = sample.points.iter().map(|p| p.q).collect();
    let bq: Vec<f64> = buffer.points.iter().map(|p| p.q).collect();
    let bi: Vec<f64> = buffer.points.iter().map(|p| p.i).collect();
    let (ov_lo, ov_hi) = overlap_range(&sq, &bq)?;
    let lo = fit_lo.max(ov_lo);
    let hi = fit_hi.min(ov_hi);
    let mut swxx = 0.0;
    let mut swxy = 0.0;
    let mut n = 0usize;
    for sp in &sample.points {
        if sp.q < lo - 1e-12 || sp.q > hi + 1e-12 {
            continue;
        }
        let (yb, _, _) = interp_within(sp.q, &bq, &bi, ov_lo, ov_hi)?;
        let var = sp.sigma_stat.powi(2) + 0.0_f64.max(yb * 1e-3).powi(2);
        let w = if var > 0.0 { 1.0 / var } else { 1.0 };
        swxx += w * yb * yb;
        swxy += w * yb * sp.i;
        n += 1;
    }
    if n < 2 || swxx <= 0.0 {
        return None;
    }
    let k = swxy / swxx;
    // Variance of k from the weighted regression (1/Σ w x²), scaled by reduced
    // chi² to absorb model mismatch (background scaling uncertainty).
    let (mut chi2, mut rss) = (0.0f64, 0.0f64);
    for sp in &sample.points {
        if sp.q < lo - 1e-12 || sp.q > hi + 1e-12 {
            continue;
        }
        let (yb, _, _) = interp_within(sp.q, &bq, &bi, ov_lo, ov_hi)?;
        let r = sp.i - k * yb;
        let var = sp.sigma_stat.powi(2) + 0.0_f64.max(yb * 1e-3).powi(2);
        chi2 += r * r / if var > 0.0 { var } else { 1.0 };
        rss += r * r;
    }
    let dof = (n as f64 - 1.0).max(1.0);
    let red = (chi2 / dof).max(1e-30);
    let var_k = red / swxx;
    let sigma_k = var_k.sqrt();
    let rms = (rss / n as f64).sqrt();
    Some((k, sigma_k, rms, chi2, n))
}

/// Build one background-subtraction branch for a (sample, buffer) pair and fit
/// window. Always returns a branch; blocking conditions set `blocked` and leave
/// the chain un-subtracted while retaining inputs and judgments.
pub fn subtract_branch(
    sample: &NormCurve,
    buffer: &NormCurve,
    fit_lo: f64,
    fit_hi: f64,
) -> SubBranch {
    let branch_key = format!(
        "{}|{}|{}|{}",
        sample.id,
        buffer.id,
        fmt_q(fit_lo),
        fmt_q(fit_hi)
    );
    let id = stable_id("br", &[RULES_VERSION, &branch_key]);

    let sq: Vec<f64> = sample.points.iter().map(|p| p.q).collect();
    let bq: Vec<f64> = buffer.points.iter().map(|p| p.q).collect();
    let bi: Vec<f64> = buffer.points.iter().map(|p| p.i).collect();
    let be: Vec<f64> = buffer.points.iter().map(|p| p.sigma_stat).collect();

    let mut judgments = Vec::new();
    let mut blocked = false;

    let differ = grids_differ(&sq, &bq, 1e-9);

    let overlap = overlap_range(&sq, &bq);
    let (mut overlap_points, mut ov_lo, mut ov_hi) = (0usize, f64::NAN, f64::NAN);
    if let Some((lo, hi)) = overlap {
        ov_lo = lo;
        ov_hi = hi;
        overlap_points = sample
            .points
            .iter()
            .filter(|p| p.q >= lo - 1e-12 && p.q <= hi + 1e-12)
            .count();
    }
    match overlap {
        None => {
            judgments.push(j_overlap(f64::NAN, f64::NAN, 0, differ));
            blocked = true;
        }
        Some((lo, hi)) => judgments.push(j_overlap(lo, hi, overlap_points, differ)),
    }

    let fit = if blocked {
        None
    } else {
        fit_scale(sample, buffer, fit_lo, fit_hi).or_else(|| {
            judgments.push(Judgment::blocked(
                "bg.fit_underdetermined",
                "重叠区内拟合点不足，无法估计背景缩放",
                json!({"fit_lo": fit_lo, "fit_hi": fit_hi}),
            ));
            blocked = true;
            None
        })
    };

    let (scale, scale_sigma, rms, chi2, _fit_n) = match fit {
        Some(t) => (Some(t.0), Some(t.1), Some(t.2), Some(t.3), t.4),
        None => (None, None, None, None, 0usize),
    };
    if let Some(sk) = scale_sigma {
        if let Some(k) = scale {
            let rel = sk / k.abs().max(1e-30);
            if !rel.is_finite() || rel > 0.25 {
                judgments.push(Judgment::warn(
                    "bg.scale_uncertain",
                    "背景缩放相对不确定度过大：保留但标记不确定状态",
                    json!({"scale": k, "scale_sigma": sk, "rel": rel}),
                ));
            } else {
                judgments.push(Judgment::ok(
                    "bg.scale_uncertain",
                    "背景缩放不确定度已计入总误差",
                    json!({"scale": k, "scale_sigma": sk, "rel": rel}),
                ));
            }
        }
    }

    let s_rel = sample.norm_rel_sigma.unwrap_or(0.0);
    let b_rel = buffer.norm_rel_sigma.unwrap_or(0.0);

    let mut points = Vec::new();
    let mut neg_count = 0usize;
    for (si, sp) in sample.points.iter().enumerate() {
        let pid = stable_id("spt", &[&id, &si.to_string()]);
        if blocked {
            points.push(SubPoint {
                id: pid,
                q: sp.q,
                sample_i: sp.i,
                buffer_i: None,
                in_overlap: false,
                i_sub: None,
                sigma_total: None,
                source_sample_norm: sample.id.clone(),
                source_buffer_norm: buffer.id.clone(),
                source_sample_index: sp.source_index,
                source_buffer_lo: None,
                source_buffer_hi: None,
            });
            continue;
        }
        let interp = interp_within(sp.q, &bq, &bi, ov_lo, ov_hi);
        let interp_e = interp_within(sp.q, &bq, &be, ov_lo, ov_hi);
        match interp {
            None => points.push(SubPoint {
                id: pid,
                q: sp.q,
                sample_i: sp.i,
                buffer_i: None,
                in_overlap: false,
                i_sub: None,
                sigma_total: None,
                source_sample_norm: sample.id.clone(),
                source_buffer_norm: buffer.id.clone(),
                source_sample_index: sp.source_index,
                source_buffer_lo: None,
                source_buffer_hi: None,
            }),
            Some((yb, lo_i, hi_i)) => {
                let k = scale.unwrap();
                let sk = scale_sigma.unwrap();
                let i_sub = sp.i - k * yb;
                if i_sub < 0.0 {
                    neg_count += 1;
                }
                // σ_sample,stat² ; buffer stat propagated through scale k
                let var_s_stat = sp.sigma_stat.powi(2);
                let var_b_stat = (k * interp_e.map(|x| x.0).unwrap_or(0.0)).powi(2);
                // normalisation variance: (k yb b_rel)² + (I_s s_rel)²
                let var_b_norm = (k * yb * b_rel).powi(2);
                let var_s_norm = (sp.i * s_rel).powi(2);
                // background scaling uncertainty: (sk yb)²
                let var_scale = (sk * yb).powi(2);
                let var = var_s_stat
                    + var_b_stat
                    + var_b_norm
                    + var_s_norm
                    + var_scale;
                points.push(SubPoint {
                    id: pid,
                    q: sp.q,
                    sample_i: sp.i,
                    buffer_i: Some(yb),
                    in_overlap: true,
                    i_sub: Some(i_sub),
                    sigma_total: Some(var.sqrt()),
                    source_sample_norm: sample.id.clone(),
                    source_buffer_norm: buffer.id.clone(),
                    source_sample_index: sp.source_index,
                    source_buffer_lo: Some(lo_i),
                    source_buffer_hi: Some(hi_i),
                });
            }
        }
    }

    if neg_count > 0 {
        judgments.push(Judgment::warn(
            "sub.negative_intensity",
            "存在负的扣除强度：保留并标记，不做截断",
            json!({"negative_points": neg_count}),
        ));
    }

    let intervals = if blocked {
        Vec::new()
    } else {
        interval_candidates(&id, &points)
    };

    if !intervals.is_empty() {
        let g = intervals.iter().filter(|c| c.kind == IntervalKind::Guinier).count();
        let p = intervals.iter().filter(|c| c.kind == IntervalKind::Porod).count();
        judgments.push(Judgment::ok(
            "model.intervals_separate",
            "Guinier 与 Porod 作为独立区间候选，不自动拼成单一模型",
            json!({"guinier_candidates": g, "porod_candidates": p}),
        ));
    }

    SubBranch {
        id,
        sample_norm: sample.id.clone(),
        buffer_norm: buffer.id.clone(),
        buffer_version: buffer.version.clone(),
        buffer_label: buffer.label.clone(),
        fit_lo,
        fit_hi,
        scale,
        scale_sigma,
        overlap_lo: if blocked { None } else { Some(ov_lo) },
        overlap_hi: if blocked { None } else { Some(ov_hi) },
        overlap_points,
        rms,
        chi2,
        points,
        intervals,
        judgments,
        blocked,
    }
}

fn fmt_q(v: f64) -> String {
    // Deterministic, compact canonical formatting for stable ids.
    if v == 0.0 {
        "0".to_string()
    } else {
        format!("{v:.6e}")
    }
}

/// Weighted linear regression returning (slope, intercept, chi2, rms).
fn linfit(xs: &[f64], ys: &[f64], ws: &[f64]) -> (f64, f64, f64, f64) {
    let mut sw = 0.0;
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    for i in 0..xs.len() {
        let (w, x, y) = (ws[i], xs[i], ys[i]);
        sw += w;
        sx += w * x;
        sy += w * y;
        sxx += w * x * x;
        sxy += w * x * y;
    }
    let det = sw * sxx - sx * sx;
    let slope = (sw * sxy - sx * sy) / det;
    let intercept = (sy - slope * sx) / sw;
    let (mut chi2, mut rss) = (0.0f64, 0.0f64);
    for i in 0..xs.len() {
        let r = ys[i] - (intercept + slope * xs[i]);
        chi2 += ws[i] * r * r;
        rss += r * r;
    }
    (slope, intercept, chi2, (rss / xs.len() as f64).sqrt())
}

/// Generate independent Guinier and Porod interval candidates from subtracted
/// points. Both families are returned together but are never stitched.
pub fn interval_candidates(branch_id: &str, pts: &[SubPoint]) -> Vec<IntervalCandidate> {
    let usable: Vec<&SubPoint> = pts
        .iter()
        .filter(|p| p.in_overlap)
        .filter_map(|p| p.i_sub.and_then(|v| if v > 0.0 { Some(p) } else { None }))
        .collect();
    let mut out = Vec::new();
    if usable.len() < 4 {
        return out;
    }

    // ---------- Guinier: ln I = ln I0 - (Rg²/3) q² ----------
    let xs: Vec<f64> = usable.iter().map(|p| p.q * p.q).collect();
    let ys: Vec<f64> = usable.iter().map(|p| p.i_sub.unwrap().ln()).collect();
    let ws: Vec<f64> = usable
        .iter()
        .map(|p| {
            let s = p.sigma_total.unwrap_or(1.0).max(1e-12);
            let v = p.i_sub.unwrap();
            (v / s).powi(2)
        })
        .collect();
    let n = usable.len();
    for len in [4usize, 5, 6, 8].into_iter() {
        if len > n {
            continue;
        }
        for start in 0..=(n - len) {
            let window = &usable[start..start + len];
            let qmax = window.last().unwrap().q;
            let x = &xs[start..start + len];
            let y = &ys[start..start + len];
            let w = &ws[start..start + len];
            let (slope, _ic, chi2, rms) = linfit(x, y, w);
            if slope >= 0.0 {
                continue; // Rg² must be positive
            }
            let rg2 = -3.0 * slope;
            let rg = rg2.sqrt();
            let qrg = qmax * rg;
            let (valid, reason) = if !(0.5..=1.3).contains(&qrg) {
                (false, format!("q_max Rg={qrg:.2} 超出 Guinier 经验窗 [0.5,1.3]"))
            } else {
                (true, "qRg 在 Guinier 合法窗内".to_string())
            };
            out.push(IntervalCandidate {
                id: stable_id("iv", &[branch_id, "guinier", &start.to_string(), &len.to_string()]),
                kind: IntervalKind::Guinier,
                q_lo: window.first().unwrap().q,
                q_hi: qmax,
                n_points: len,
                chi2,
                rms,
                valid,
                reason,
                params: serde_json::json!({"Rg": rg, "qRg_max": qrg}),
            });
        }
    }

    // ---------- Porod: log-log slope (high-q tail) ----------
    let xlog: Vec<f64> = usable.iter().map(|p| p.q.ln()).collect();
    let ylog: Vec<f64> = usable.iter().map(|p| p.i_sub.unwrap().ln()).collect();
    for len in [4usize, 5, 6].into_iter() {
        if len > n {
            continue;
        }
        // Porod candidates live in the upper half of the available q range.
        let first_start = (n / 2).saturating_sub(1);
        for start in first_start..=(n - len) {
            let window = &usable[start..start + len];
            let x = &xlog[start..start + len];
            let y = &ylog[start..start + len];
            let w = &ws[start..start + len];
            let (slope, _ic, chi2, rms) = linfit(x, y, w);
            let (valid, reason) = if !(-4.5..=-3.0).contains(&slope) {
                (false, format!("Porod 斜率 {slope:.2} 超出 [-4.5,-3.0]"))
            } else {
                (true, "高 q 尾斜率落在 Porod 区间".to_string())
            };
            out.push(IntervalCandidate {
                id: stable_id("iv", &[branch_id, "porod", &start.to_string(), &len.to_string()]),
                kind: IntervalKind::Porod,
                q_lo: window.first().unwrap().q,
                q_hi: window.last().unwrap().q,
                n_points: len,
                chi2,
                rms,
                valid,
                reason,
                params: serde_json::json!({"slope": slope}),
            });
        }
    }

    // Flag overlapping *same-family* intervals as competing evidence rather
    // than merging. Cross-family overlap is expected and intentionally fine.
    mark_overlaps(&mut out);
    out
}

fn mark_overlaps(cs: &mut [IntervalCandidate]) {
    let k = IntervalKind::Guinier;
    for i in 0..cs.len() {
        for j in (i + 1)..cs.len() {
            if cs[i].kind != cs[j].kind || cs[i].kind != k && cs[j].kind != k {
                // compare families separately
            }
            if cs[i].kind == cs[j].kind
                && cs[i].q_lo < cs[j].q_hi
                && cs[j].q_lo < cs[i].q_hi
            {
                if cs[i].valid {
                    cs[i].reason = format!("{}；与同类区间重叠(多候选证据)", cs[i].reason);
                }
            }
        }
    }
}
