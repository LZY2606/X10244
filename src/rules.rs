//! Core SAXS rules: normalization and background subtraction.
//!
//! Invariants defended here:
//! * curves on different q grids are never subtracted point-by-point;
//! * interpolation is applied only inside the shared overlap range;
//! * uncertainty includes normalization and background-scale contributions;
//! * negative intensities are kept and flagged, never clipped away.

use crate::domain::*;
use crate::ids;
use crate::units;

/// Relative-uncertainty threshold above which the background scale is flagged
/// as not uniquely determined.
pub const BG_SCALE_REL_LIMIT: f64 = 0.05;

struct NormParams<'a> {
    exposure: f64,
    thickness: f64,
    trans: f64,
    trans_sigma: f64,
    response_scale: f64,
    response_sigma: f64,
    _m: std::marker::PhantomData<&'a ()>,
}

fn norm_params(mat: &MaterialVersion, resp: Option<&InstrumentResponse>) -> Result<NormParams<'static>, String> {
    let needed = [
        &mat.exposure,
        &mat.thickness,
        &mat.transmission,
    ];
    if !units::all_usable(&needed) {
        let mut bad = Vec::new();
        if !mat.exposure.ok() { bad.push("曝光"); }
        if !mat.thickness.ok() { bad.push("厚度"); }
        if !mat.transmission.ok() { bad.push("透射率"); }
        return Err(format!("归一化缺少可用量：{}", bad.join("、")));
    }
    let exposure = mat.exposure.value_si.unwrap();
    let thickness = mat.thickness.value_si.unwrap();
    let trans = mat.transmission.value_si.unwrap();
    if exposure <= 0.0 || thickness <= 0.0 || trans <= 0.0 {
        return Err("曝光、厚度、透射率必须为正".into());
    }
    let trans_sigma = mat.transmission_error.value_si.unwrap_or(0.0).max(0.0);
    let (response_scale, response_sigma) = match resp {
        Some(r) => (r.scale, r.scale_uncertainty),
        None => (1.0, 0.0),
    };
    Ok(NormParams {
        exposure,
        thickness,
        trans,
        trans_sigma,
        response_scale,
        response_sigma,
        _m: std::marker::PhantomData,
    })
}

/// Normalize one material version to I / (exposure * thickness * transmission).
pub fn normalize(mat: &MaterialVersion, resp: Option<&InstrumentResponse>) -> Normalized {
    let node = ids::norm_node(&mat.material_id, mat.version);
    let mut diagnostics = Vec::new();
    let params = match norm_params(mat, resp) {
        Ok(p) => Some(p),
        Err(e) => {
            diagnostics.push(e);
            None
        }
    };

    let mut points = Vec::new();
    for p in &mat.points {
        let usable = p.q.ok() && p.intensity.ok() && p.error.ok();
        let (i_si, sigma_stat, sigma_trans, sigma_response, sigma) = match (&params, usable) {
            (Some(prm), true) => {
                let q = p.q.value_si.unwrap();
                let i = p.intensity.value_si.unwrap();
                let e = p.error.value_si.unwrap();
                let denom = prm.exposure * prm.thickness * prm.trans;
                let i_si = i / denom * prm.response_scale;
                let sigma_stat = (e / denom * prm.response_scale).max(0.0);
                let rel_trans = if prm.trans > 0.0 { prm.trans_sigma / prm.trans } else { 0.0 };
                let rel_resp = if prm.response_scale > 0.0 {
                    prm.response_sigma / prm.response_scale
                } else {
                    0.0
                };
                let sigma_trans = i_si * rel_trans;
                let sigma_response = i_si * rel_resp;
                let sigma = (sigma_stat.powi(2)
                    + sigma_trans.powi(2)
                    + sigma_response.powi(2))
                    .sqrt()
                    .max(0.0);
                (i_si, sigma_stat, sigma_trans, sigma_response, sigma)
            }
            _ => (f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN),
        };
        if !usable {
            diagnostics.push(format!("点 {} 含不可转换值，已保留但不参与派生", p.id));
        }
        points.push(NormPoint {
            id: ids::norm_point(&node, &p.id),
            q_si: p.q.value_si.unwrap_or(f64::NAN),
            i_si,
            sigma,
            sigma_stat,
            sigma_trans,
            sigma_response,
            source_point: p.id.clone(),
            masked: p.masked,
        });
    }
    if resp.is_none() {
        diagnostics.push("未关联仪器响应，采用单位响应（响应不确定度为 0）".into());
    }
    Normalized { node, source: Ref::new(mat.material_id.clone(), mat.version), points, diagnostics }
}

/// Ordered unmasked usable (q, I, sigma, source_point) sample from a curve.
fn usable_series(n: &Normalized) -> Vec<(f64, f64, f64, String)> {
    n.points
        .iter()
        .filter(|p| !p.masked && p.q_si.is_finite() && p.i_si.is_finite())
        .map(|p| (p.q_si, p.i_si, p.sigma, p.source_point.clone()))
        .collect()
}

/// Linearly interpolate a (q_sorted) series at q. Only valid inside range.
fn interp(series: &[(f64, f64, f64, String)], q: f64) -> Option<(f64, f64, String)> {
    if series.len() < 2 {
        return None;
    }
    if q < series.first().unwrap().0 || q > series.last().unwrap().0 {
        return None;
    }
    match series.binary_search_by(|(x, _, _, _)| x.partial_cmp(&q).unwrap()) {
        Ok(i) => Some((series[i].1, series[i].2, series[i].3.clone())),
        Err(i) => {
            if i == 0 || i >= series.len() {
                return None;
            }
            let (q0, i0, s0, id0) = &series[i - 1];
            let (q1, i1, s1, id1) = &series[i];
            let t = (q - q0) / (q1 - q0);
            let iv = i0 + t * (i1 - i0);
            // interpolated sigma uses the larger neighbouring sigma (conservative)
            let sv = s0.max(*s1);
            Some((iv, sv, format!("{}~{}", id0, id1)))
        }
    }
}

fn grids_equal(a: &Normalized, b: &Normalized) -> bool {
    let pa: Vec<f64> = a.points.iter().map(|p| p.q_si).collect();
    let pb: Vec<f64> = b.points.iter().map(|p| p.q_si).collect();
    pa.len() == pb.len() && pa.iter().zip(&pb).all(|(x, y)| (x - y).abs() < 1e-12)
}

/// Analytical transmission/exposure scale and its propagated uncertainty.
fn analytical_scale(sample: &MaterialVersion, buffer: &MaterialVersion) -> (f64, f64) {
    let ts = sample.transmission.value_si.unwrap_or(1.0);
    let tb = buffer.transmission.value_si.unwrap_or(1.0);
    let es = sample.exposure.value_si.unwrap_or(1.0);
    let eb = buffer.exposure.value_si.unwrap_or(1.0);
    let scale = (ts * es) / (tb * eb);
    let sts = sample.transmission_error.value_si.unwrap_or(0.0);
    let stb = buffer.transmission_error.value_si.unwrap_or(0.0);
    let rel = ((sts / ts).powi(2) + (stb / tb).powi(2)).sqrt();
    (scale, (scale * rel).abs())
}

/// Weighted least squares scale a minimizing ||sample - a*buffer||^2/sigma^2
/// over the overlap, plus the fit standard error.
fn fitted_scale(pts: &[(f64, f64, f64, f64, f64)]) -> (f64, f64, f64) {
    // tuple: (q, is, ib, sig_s, sig_b)
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for (_q, is, ib, ss, sb) in pts {
        let var = ss.powi(2) + 0.0 * sb.powi(2);
        let w = if var > 0.0 { 1.0 / var } else { 1.0 };
        num += w * ib * is;
        den += w * ib * ib;
    }
    if den <= 0.0 {
        return (f64::NAN, f64::NAN, f64::INFINITY);
    }
    let scale = num / den;
    let mut chi2 = 0.0f64;
    let mut k = 0usize;
    for (_q, is, ib, ss, sb) in pts {
        let var = ss.powi(2) + (scale * sb).powi(2);
        let var = if var > 0.0 { var } else { 1.0 };
        chi2 += (is - scale * ib).powi(2) / var;
        k += 1;
    }
    let sigma_scale = if k > 1 { (chi2 / (k as f64 - 1.0) / den).sqrt() } else { 0.0 };
    let dof = k as f64 - 1.0;
    let chi2_red = if dof > 0.0 { chi2 / dof } else { 0.0 };
    (scale, sigma_scale, chi2_red)
}

/// Subtract a normalized buffer from a normalized sample.
pub fn subtract(
    sample_mat: &MaterialVersion,
    buffer_mat: &MaterialVersion,
    sample: &Normalized,
    buffer: &Normalized,
    preselect: Option<&str>,
) -> Subtracted {
    let node = ids::sub_node(&sample_mat.material_id, sample_mat.version, &buffer_mat.material_id, buffer_mat.version);
    let mut diagnostics = Vec::new();

    let s_series = usable_series(sample);
    let b_series = usable_series(buffer);

    let s_lo = s_series.first().map(|x| x.0);
    let s_hi = s_series.last().map(|x| x.0);
    let b_lo = b_series.first().map(|x| x.0);
    let b_hi = b_series.last().map(|x| x.0);

    let overlap = match (s_lo, s_hi, b_lo, b_hi) {
        (Some(sl), Some(sh), Some(bl), Some(bh)) => {
            let lo = sl.max(bl);
            let hi = sh.min(bh);
            if hi - lo > 1e-12 { Some([lo, hi]) } else { None }
        }
        _ => None,
    };

    if overlap.is_none() {
        diagnostics.push("两条曲线没有重叠 q 区间：不进行相减，也不做范围外插值".into());
        return Subtracted {
            node,
            sample: Ref::new(sample_mat.material_id.clone(), sample_mat.version),
            buffer: Ref::new(buffer_mat.material_id.clone(), buffer_mat.version),
            overlap: None,
            interpolation_used: false,
            candidates: vec![],
            chosen_candidate: None,
            points: vec![],
            diagnostics,
        };
    }
    let [lo, hi] = overlap.unwrap();

    let same_grid = grids_equal(sample, buffer);
    let interpolation_used = !same_grid;
    if same_grid {
        diagnostics.push("q 网格一致：在重叠范围内直接相减".into());
    } else {
        diagnostics.push(
            "q 网格不同：不在不同网格上直接相减，仅在重叠范围内对缓冲液线性插值".into(),
        );
    }

    // Build working points restricted to the overlap on the SAMPLE grid.
    struct W {
        q: f64,
        is: f64,
        ib: f64,
        ss: f64,
        sb: f64,
        spt: String,
        bpt: String,
    }
    let mut work: Vec<W> = Vec::new();
    for (q, is, ss, spt) in &s_series {
        if *q < lo - 1e-12 || *q > hi + 1e-12 {
            continue;
        }
        if let Some((ib, sb, bpt)) = interp(&b_series, *q) {
            work.push(W { q: *q, is: *is, ib, ss: *ss, sb, spt: spt.clone(), bpt });
        }
    }
    if work.is_empty() {
        diagnostics.push("重叠范围内没有可对齐的样品点".into());
    }

    // Analytical candidate (transmission/exposure).
    let (a_scale, a_scale_sigma) = analytical_scale(sample_mat, buffer_mat);
    let fit_input: Vec<(f64, f64, f64, f64, f64)> =
        work.iter().map(|w| (w.q, w.is, w.ib, w.ss, w.sb)).collect();
    let (f_scale, f_scale_sigma, f_chi2) = fitted_scale(&fit_input);

    let mut candidates = Vec::new();
    let a_id = ids::candidate_id(&node, "analytical", a_scale);
    candidates.push(SubCandidate {
        id: a_id.clone(),
        method: ScaleMethod::Analytical,
        scale: a_scale,
        scale_sigma: a_scale_sigma,
        chosen: false,
    });
    if f_scale.is_finite() && work.len() >= 3 {
        candidates.push(SubCandidate {
            id: ids::candidate_id(&node, "fitted", f_scale),
            method: ScaleMethod::Fitted,
            scale: f_scale,
            scale_sigma: f_scale_sigma,
            chosen: false,
        });
    }

    // Analytical is the deterministic default; a previously chosen candidate
    // (e.g. reviewer switched to fitted) is preserved if it still exists.
    let chosen_id = match preselect {
        Some(pid) if candidates.iter().any(|c| c.id == pid) => pid.to_string(),
        _ => a_id,
    };
    for c in &mut candidates {
        if c.id == chosen_id {
            c.chosen = true;
        }
    }
    let chosen = candidates.iter().find(|c| c.id == chosen_id).unwrap().clone();

    if chosen.scale_sigma / chosen.scale.abs() > BG_SCALE_REL_LIMIT {
        diagnostics.push(format!(
            "背景缩放相对不确定度 {:.3} 超过阈值 {:.0}%，扣背景尺度不能唯一确定",
            chosen.scale_sigma / chosen.scale.abs(),
            BG_SCALE_REL_LIMIT * 100.0
        ));
    }
    if candidates.len() > 1 {
        let a_chi = work
            .iter()
            .map(|w| {
                let r = w.is - a_scale * w.ib;
                let var = w.ss.powi(2) + (a_scale * w.sb).powi(2);
                if var > 0.0 { r.powi(2) / var } else { 0.0 }
            })
            .sum::<f64>()
            / (work.len().max(2) as f64 - 1.0);
        if (a_chi - f_chi2).abs() <= 0.5 * f_chi2.max(a_chi) + 0.5 {
            diagnostics.push("多候选残差检验：解析尺度与拟合尺度残差接近，两者均为合法候选".into());
        }
    }

    let mut points = Vec::new();
    let mut any_negative = false;
    for w in &work {
        let i_sub = w.is - chosen.scale * w.ib;
        let sigma_bg_scale = (chosen.scale_sigma * w.ib).abs();
        let sigma = (w.ss.powi(2) + (chosen.scale * w.sb).powi(2) + sigma_bg_scale.powi(2)).sqrt();
        let residual = if sigma > 0.0 { i_sub / sigma } else { 0.0 };
        if i_sub < 0.0 {
            any_negative = true;
        }
        points.push(SubPoint {
            id: ids::sub_point_id(&node, &w.spt, &w.bpt),
            q_si: w.q,
            i_sub,
            sigma,
            sigma_bg_scale,
            residual,
            sample_source_point: w.spt.clone(),
            buffer_source_point: w.bpt.clone(),
            negative: i_sub < 0.0,
        });
    }
    if any_negative {
        diagnostics.push("出现负强度：保留负号并标记，未做截断或非负约束".into());
    }

    Subtracted {
        node,
        sample: Ref::new(sample_mat.material_id.clone(), sample_mat.version),
        buffer: Ref::new(buffer_mat.material_id.clone(), buffer_mat.version),
        overlap: Some([lo, hi]),
        interpolation_used,
        candidates,
        chosen_candidate: Some(chosen.id),
        points,
        diagnostics,
    }
}
