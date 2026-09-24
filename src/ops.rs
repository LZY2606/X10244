//! 核心运算规则（RULE_VERSION 标记版本）。
//! 纯函数、确定性结果，便于离线重放。

use crate::domain::*;

/// q 坐标转换到 1/nm。无法识别的单位返回诊断。
pub fn convert_q_to_inv_nm(q: f64, unit: &str, point_id: Option<String>) -> Result<f64, ConversionDiagnostic> {
    let u = unit.trim().to_ascii_lowercase();
    let val = match u.as_str() {
        "1/nm" | "nm^-1" | "a^-1" if u == "a^-1" => return Err(bad_unit("q", point_id, unit, "ambiguous symbol a")),
        "1/nm" | "nm^-1" => q,
        "1/a" | "a^-1" | "angstrom^-1" => q * 10.0,
        _ => return Err(bad_unit("q", point_id, unit, "unknown q unit; expected 1/nm or 1/A")),
    };
    Ok(val)
}

fn bad_unit(field: &str, point_id: Option<String>, from: &str, reason: &str) -> ConversionDiagnostic {
    ConversionDiagnostic {
        field: field.to_string(),
        point_id,
        from_unit: from.to_string(),
        to_unit: if field == "q" { "1/nm".to_string() } else { String::new() },
        reason: reason.to_string(),
        code: "UNIT_UNCONVERTIBLE".to_string(),
    }
}

/// 透射率统一为分数（fraction）。
pub fn transmission_fraction(value: f64, unit: &str) -> Option<f64> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "fraction" | "ratio" | "" => Some(value),
        "percent" | "%" => Some(value / 100.0),
        _ => None,
    }
}

/// 厚度统一为 cm（绝对散射强度惯例）。
pub fn thickness_cm(qty: &Qty) -> Result<f64, ConversionDiagnostic> {
    match qty.unit.trim().to_ascii_lowercase().as_str() {
        "cm" => Ok(qty.value),
        "mm" => Ok(qty.value / 10.0),
        "m" => Ok(qty.value * 100.0),
        _ => Err(ConversionDiagnostic {
            field: "thickness".to_string(),
            point_id: None,
            from_unit: qty.unit.clone(),
            to_unit: "cm".to_string(),
            reason: format!("cannot convert thickness {:?}", qty.raw_repr),
            code: "UNIT_UNCONVERTIBLE".to_string(),
        }),
    }
}

/// 曝光统一为秒。
pub fn exposure_seconds(qty: &Qty) -> Result<f64, ConversionDiagnostic> {
    match qty.unit.trim().to_ascii_lowercase().as_str() {
        "s" | "sec" | "seconds" => Ok(qty.value),
        "ms" => Ok(qty.value / 1000.0),
        _ => Err(ConversionDiagnostic {
            field: "exposure".to_string(),
            point_id: None,
            from_unit: qty.unit.clone(),
            to_unit: "s".to_string(),
            reason: format!("cannot convert exposure {:?}", qty.raw_repr),
            code: "UNIT_UNCONVERTIBLE".to_string(),
        }),
    }
}

/// 绝对刻度归一化：
/// I_abs = scale * I_raw / (exposure_s * transmission * thickness_cm)
/// 统计误差按相同因子传播；归一化（乘性）不确定度独立贡献。
/// 无法转换坐标/采集量的点保留在原位置（converted=false + 诊断），不丢弃。
pub fn normalize(raw: &RawCurve) -> NormCurve {
    let mut diagnostics = Vec::new();

    let exposure = match exposure_seconds(&raw.exposure) {
        Ok(v) => v,
        Err(d) => {
            diagnostics.push(d);
            f64::NAN
        }
    };
    let thickness = match &raw.thickness {
        Some(t) => match thickness_cm(t) {
            Ok(v) => Some(v),
            Err(d) => {
                diagnostics.push(d);
                None
            }
        },
        None => Some(1.0), // 缓冲液/缺少厚度时以 1 cm 占位，但保留 None 语义标记
    };
    let transmission = match transmission_fraction(raw.transmission.value, &raw.transmission.unit) {
        Some(v) => v,
        None => {
            diagnostics.push(ConversionDiagnostic {
                field: "transmission".to_string(),
                point_id: None,
                from_unit: raw.transmission.unit.clone(),
                to_unit: "fraction".to_string(),
                reason: format!("cannot read transmission {:?}", raw.transmission.raw_repr),
                code: "UNIT_UNCONVERTIBLE".to_string(),
            });
            f64::NAN
        }
    };
    let scale = raw.instrument.absolute_scale.unwrap_or(1.0);
    let denom = exposure * transmission * thickness.unwrap_or(1.0);
    let factor = scale / denom;
    let factor_expr = format!(
        "{scale} / (exposure {} * transmission {} * thickness {})",
        raw.exposure.raw_repr, raw.transmission.raw_repr,
        raw.thickness.as_ref().map(|t| t.raw_repr.clone()).unwrap_or_else(|| "1 cm (assumed)".to_string())
    );

    let geom_ok = denom.is_finite() && denom > 0.0;
    let mut points = Vec::with_capacity(raw.points.len());
    for p in &raw.points {
        let mut point_diag = None;
        let q_norm = match convert_q_to_inv_nm(p.q, &p.q_unit, Some(p.id.clone())) {
            Ok(v) => v,
            Err(d) => {
                point_diag = Some(d.clone());
                diagnostics.push(d);
                f64::NAN
            }
        };
        let converted = geom_ok && point_diag.is_none();
        let (i_abs, sig_stat, sig_norm, sig_total) = if converted {
            let i = factor * p.i;
            let ss = factor * p.sigma;
            let sn = i.abs() * raw.instrument.norm_rel_uncertainty;
            (i, ss, sn, (ss * ss + sn * sn).sqrt())
        } else {
            (f64::NAN, f64::NAN, f64::NAN, f64::NAN)
        };
        points.push(NormPoint {
            id: format!("{}#{}", raw.id, p.id),
            q: q_norm,
            q_unit: "1/nm".to_string(),
            i_abs,
            sigma_stat: sig_stat,
            sigma_norm: sig_norm,
            sigma_total: sig_total,
            source_point_id: p.id.clone(),
            masked: p.masked,
            converted,
            conversion_diagnostic: point_diag,
        });
    }

    NormCurve {
        id: format!("norm:{}", raw.id),
        source_id: raw.id.clone(),
        factor,
        factor_expr,
        q_unit: "1/nm".to_string(),
        i_unit: "cm^-1 (absolute)".to_string(),
        norm_rel_uncertainty: raw.instrument.norm_rel_uncertainty,
        diagnostics,
        points,
    }
}

fn usable(n: &NormCurve) -> Vec<&NormPoint> {
    n.points.iter().filter(|p| p.converted && !p.masked).collect()
}

fn grids_match(sample: &NormCurve, buffer: &NormCurve) -> bool {
    let s: Vec<f64> = usable(sample).iter().map(|p| p.q).collect();
    let b: Vec<f64> = usable(buffer).iter().map(|p| p.q).collect();
    if s.len() != b.len() {
        return false;
    }
    const TOL: f64 = 1e-9;
    s.iter().zip(b).all(|(x, y)| (x - y).abs() <= TOL * x.abs().max(1.0))
}

/// 在缓冲液曲线上对 q 做线性插值；只在 [q_min, q_max] 内部命中，不做任何外推。
fn interpolate_buffer<'a>(
    buffer: &'a [&'a NormPoint],
    q: f64,
) -> Option<(f64, f64, f64, Vec<String>, bool)> {
    if buffer.is_empty() {
        return None;
    }
    let qlo = buffer.first().unwrap().q;
    let qhi = buffer.last().unwrap().q;
    if q < qlo || q > qhi {
        return None;
    }
    let pos = buffer.binary_search_by(|p| p.q.partial_cmp(&q).unwrap());
    match pos {
        Ok(idx) => {
            let p = buffer[idx];
            Some((p.i_abs, p.sigma_stat, p.sigma_norm, vec![p.id.clone()], false))
        }
        Err(idx) => {
            if idx == 0 || idx >= buffer.len() {
                return None;
            }
            let lo = buffer[idx - 1];
            let hi = buffer[idx];
            let w = (q - lo.q) / (hi.q - lo.q);
            let interp = |a: f64, b: f64| a + (b - a) * w;
            Some((
                interp(lo.i_abs, hi.i_abs),
                interp(lo.sigma_stat, hi.sigma_stat),
                interp(lo.sigma_norm, hi.sigma_norm),
                vec![lo.id.clone(), hi.id.clone()],
                true,
            ))
        }
    }
}

/// 背景扣除。网格不同绝不直接相减：统一只在重叠区、通过缓冲液插值后相减。
/// 无重叠区时返回 has_overlap=false 与空点集及诊断，绝不伪造结果。
pub fn subtract(
    id: &str,
    sample: &NormCurve,
    buffer: &NormCurve,
    buffer_candidate_ids: Vec<String>,
) -> Subtraction {
    let mut diagnostics = Vec::new();
    let match_ok = grids_match(sample, buffer);

    let mut spts = usable(sample);
    let bpts0 = usable(buffer);
    spts.sort_by(|a, b| a.q.partial_cmp(&b.q).unwrap());
    let mut bpts = bpts0.clone();
    bpts.sort_by(|a, b| a.q.partial_cmp(&b.q).unwrap());

    let (slo, shi) = spts.first().map(|p| p.q).zip(spts.last().map(|p| p.q)).map(|(a, b)| (a, b));
    let (blo, bhi) = bpts.first().map(|p| p.q).zip(bpts.last().map(|p| p.q)).map(|(a, b)| (a, b));

    let overlap = match (slo, shi, blo, bhi) {
        (Some(slo), Some(shi), Some(blo), Some(bhi)) => {
            let lo = slo.max(blo);
            let hi = shi.min(bhi);
            if hi <= lo {
                None
            } else {
                Some((lo, hi))
            }
        }
        _ => None,
    };

    let has_overlap = overlap.is_some();
    if !has_overlap {
        diagnostics.push(ConversionDiagnostic {
            field: "overlap".to_string(),
            point_id: None,
            from_unit: sample.q_unit.clone(),
            to_unit: buffer.q_unit.clone(),
            reason: "sample and buffer q ranges do not overlap; subtraction refused".to_string(),
            code: "NO_OVERLAP".to_string(),
        });
    }
    if has_overlap && !match_ok {
        diagnostics.push(ConversionDiagnostic {
            field: "grid".to_string(),
            point_id: None,
            from_unit: sample.q_unit.clone(),
            to_unit: buffer.q_unit.clone(),
            reason: "q grids differ; subtracting only inside overlap via buffer interpolation".to_string(),
            code: "GRID_MISMATCH_INTERPOLATED".to_string(),
        });
    }

    // 第一遍：收集重叠区配对并拟合缩放 alpha（过原点加权最小二乘）。
    struct Pair<'a> {
        sp: &'a NormPoint,
        ib: f64,
        sb: f64,
        buf_sn: f64,
        ids: Vec<String>,
        interp: bool,
    }
    let mut pairs: Vec<Pair> = Vec::new();
    if has_overlap {
        for sp in &spts {
            if let Some((ib, sb, snb, ids, interp)) = interpolate_buffer(&bpts, sp.q) {
                pairs.push(Pair { sp, ib, sb, buf_sn: snb, ids, interp });
            }
        }
    }

    let scale = if pairs.len() >= 2 {
        let mut sw = 0.0;
        let mut swxy = 0.0;
        for pr in &pairs {
            let var = pr.sp.sigma_stat.powi(2) + pr.sb.powi(2);
            let w = if var > 0.0 { 1.0 / var } else { 1.0 };
            sw += w;
            swxy += w * pr.ib * pr.sp.i_abs;
        }
        let mut swxx = 0.0;
        for pr in &pairs {
            let var = pr.sp.sigma_stat.powi(2) + pr.sb.powi(2);
            let w = if var > 0.0 { 1.0 / var } else { 1.0 };
            swxx += w * pr.ib * pr.ib;
        }
        let alpha = if swxx > 0.0 { swxy / swxx } else { f64::NAN };
        let mut chi = 0.0;
        for pr in &pairs {
            let r = pr.sp.i_abs - alpha * pr.ib;
            let var = pr.sp.sigma_stat.powi(2) + pr.sb.powi(2);
            chi += r * r / if var > 0.0 { var } else { 1.0 };
        }
        let n = pairs.len() as f64;
        // 过原点加权回归系数标准误：sqrt(chi^2_red / sum w*x^2)
        let sigma_alpha = if swxx > 0.0 && n > 1.0 {
            ((chi / (n - 1.0)) / swxx).sqrt()
        } else {
            f64::NAN
        };
        let (lo, hi) = overlap.unwrap();
        Some(BackgroundScale {
            alpha,
            sigma_alpha,
            method: "weighted-least-squares-through-origin".to_string(),
            q_min: lo,
            q_max: hi,
            n_pairs: pairs.len(),
        })
    } else if has_overlap {
        diagnostics.push(ConversionDiagnostic {
            field: "scale".to_string(),
            point_id: None,
            from_unit: String::new(),
            to_unit: String::new(),
            reason: "fewer than 2 overlapping pairs; background scale cannot be estimated".to_string(),
            code: "SCALE_UNDETERMINED".to_string(),
        });
        None
    } else {
        None
    };

    // 第二遍：生成扣除点，误差含统计、样品归一化与背景缩放不确定度。
    let mut points = Vec::new();
    let mut negatives = 0u32;
    if let Some(sc) = &scale {
        for (ord, pr) in pairs.iter().enumerate() {
            let i_sub = pr.sp.i_abs - sc.alpha * pr.ib;
            let sigma_stat = (pr.sp.sigma_stat.powi(2) + sc.alpha.powi(2) * pr.sb.powi(2)).sqrt();
            let buf_sigma_norm = pr.buf_sn;
            let sigma_norm =
                (pr.sp.sigma_norm.powi(2) + sc.alpha.powi(2) * buf_sigma_norm.powi(2)).sqrt();
            // 背景缩放不确定度：sigma_alpha * I_buffer
            let sigma_scale = sc.sigma_alpha.abs() * pr.ib.abs();
            let sigma_total =
                (sigma_stat.powi(2) + sigma_norm.powi(2) + sigma_scale.powi(2)).sqrt();
            let negative = i_sub < 0.0;
            if negative {
                negatives += 1;
            }
            points.push(SubPoint {
                id: format!("{}#{}", id, ord),
                ordinal: ord,
                q: pr.sp.q,
                q_unit: "1/nm".to_string(),
                i_sub,
                sigma_stat,
                sigma_norm,
                sigma_scale,
                sigma_total,
                sample_point_id: pr.sp.id.clone(),
                buffer_point_ids: pr.ids.clone(),
                interpolated: pr.interp,
                in_overlap: true,
                negative,
            });
        }
    }

    if negatives > 0 {
        diagnostics.push(ConversionDiagnostic {
            field: "intensity".to_string(),
            point_id: None,
            from_unit: String::new(),
            to_unit: String::new(),
            reason: format!("{negatives} subtracted points are negative; retained with NEGATIVE flag"),
            code: "NEGATIVE_INTENSITY".to_string(),
        });
    }

    let (q_min, q_max) = overlap.map(|(a, b)| (Some(a), Some(b))).unwrap_or((None, None));
    Subtraction {
        id: id.to_string(),
        sample_norm_id: sample.id.clone(),
        buffer_norm_id: buffer.id.clone(),
        buffer_candidate_ids,
        grids_match: match_ok,
        q_min,
        q_max,
        has_overlap,
        scale,
        diagnostics,
        points,
    }
}

struct FitResult {
    slope: f64,
    intercept: f64,
    r_squared: f64,
}

/// 加权线性回归 y = a + b x，权重 1/sigma^2。
fn weighted_linfit(xs: &[f64], ys: &[f64], ws: &[f64]) -> FitResult {
    let sw: f64 = ws.iter().sum();
    let swx: f64 = ws.iter().zip(xs).map(|(w, x)| w * x).sum();
    let swy: f64 = ws.iter().zip(ys).map(|(w, y)| w * y).sum();
    let swxx: f64 = ws.iter().zip(xs).map(|(w, x)| w * x * x).sum();
    let swxy: f64 = ws.iter().zip(ws.iter().zip(xs.iter().zip(ys))).map(|(w, (_, (x, y)))| w * x * y).sum();
    let den = sw * swxx - swx * swx;
    let (slope, intercept) = if den.abs() > 0.0 {
        let b = (sw * swxy - swx * swy) / den;
        let a = (swy - b * swx) / sw;
        (b, a)
    } else {
        (f64::NAN, f64::NAN)
    };
    let ybar: f64 = ys.iter().sum::<f64>() / ys.len() as f64;
    let sst: f64 = ys.iter().map(|y| (y - ybar).powi(2)).sum();
    let sse: f64 = xs.iter().zip(ys).map(|(x, y)| {
        let pred = intercept + slope * x;
        (y - pred).powi(2)
    }).sum();
    let r_squared = if sst > 0.0 { (1.0 - sse / sst).max(0.0) } else { 0.0 };
    FitResult { slope, intercept, r_squared }
}

fn windows(n: usize) -> impl Iterator<Item = (usize, usize)> {
    (0..n).flat_map(move |i| (i + 3..=n).map(move |j| (i, j)))
}

/// 对扣背景曲线做 Guinier / Porod 区间分析。
/// 保留所有候选（含被拒绝窗口的原因），报告区间重叠，但绝不自动拼成单一模型。
pub fn analyze_intervals(id: &str, sub: &Subtraction) -> IntervalAnalysis {
    let pts: Vec<&SubPoint> = sub.points.iter().filter(|p| !p.negative).collect();
    let mut guinier = Vec::new();
    let mut porod = Vec::new();

    let qrg_lo = 0.3_f64;
    let qrg_hi = 1.3_f64;
    let r2_min = 0.85_f64;

    for (i, j) in windows(pts.len()) {
        let seg = &pts[i..j];
        if seg.is_empty() {
            continue;
        }
        let ids: Vec<String> = seg.iter().map(|p| p.sample_point_id.clone()).collect();
        let qmin = seg.first().unwrap().q;
        let qmax = seg.last().unwrap().q;
        let qmid = (qmin + qmax) / 2.0;

        // Guinier: ln I = ln I0 - q^2 Rg^2/3 （要求正强度）
        let positive = seg.iter().filter(|p| p.i_sub > 0.0).count() == seg.len();
        if positive {
            let xs: Vec<f64> = seg.iter().map(|p| p.q * p.q).collect();
            let ys: Vec<f64> = seg.iter().map(|p| p.i_sub.ln()).collect();
            let ws: Vec<f64> = seg
                .iter()
                .map(|p| {
                    let rel = p.sigma_total / p.i_sub;
                    if rel > 0.0 { 1.0 / (rel * rel) } else { 1.0 }
                })
                .collect();
            let fit = weighted_linfit(&xs, &ys, &ws);
            let rg = if fit.slope < 0.0 { (-3.0 * fit.slope).sqrt() } else { f64::NAN };
            let i0 = fit.intercept.exp();
            let qrg_min = qmin * rg;
            let qrg_max = qmax * rg;
            let mut reasons = Vec::new();
            if fit.r_squared < r2_min {
                reasons.push(format!("R2 {:.3} < {r2_min}", fit.r_squared));
            }
            if !(qrg_lo..=qrg_hi).contains(&qrg_max) {
                reasons.push(format!("qmax*Rg {qrg_max:.2} outside [{qrg_lo},{qrg_hi}]"));
            }
            if !(qrg_lo..=qrg_hi).contains(&qrg_min) {
                reasons.push(format!("qmin*Rg {qrg_min:.2} outside [{qrg_lo},{qrg_hi}]"));
            }
            let rejected = !reasons.is_empty();
            guinier.push(IntervalCandidate {
                id: format!("{id}#G{:03}", guinier.len()),
                kind: ModelKind::Guinier,
                q_min: qmin,
                q_max: qmax,
                n_points: seg.len(),
                rg: Some(rg),
                i0: Some(i0),
                power: None,
                amplitude: None,
                r_squared: fit.r_squared,
                qrg_max: Some(qrg_max),
                qrg_min: Some(qrg_min),
                rejected,
                reject_reasons: reasons,
                source_point_ids: ids,
            });
        }

        // Porod: log I = log A + power * log q （要求负幂律）
        if seg.iter().all(|p| p.i_sub > 0.0) {
            let xs: Vec<f64> = seg.iter().map(|p| p.q.ln()).collect();
            let ys: Vec<f64> = seg.iter().map(|p| p.i_sub.ln()).collect();
            let ws: Vec<f64> = seg
                .iter()
                .map(|p| {
                    let rel = p.sigma_total / p.i_sub;
                    if rel > 0.0 { 1.0 / (rel * rel) } else { 1.0 }
                })
                .collect();
            let fit = weighted_linfit(&xs, &ys, &ws);
            let mut reasons = Vec::new();
            if fit.r_squared < r2_min {
                reasons.push(format!("R2 {:.3} < {r2_min}", fit.r_squared));
            }
            if fit.slope >= -1.0 {
                reasons.push(format!("power {:.2} not < -1 (not a Porod tail)", fit.slope));
            }
            // Porod 区间位于较高 q，避免与 Guinier 窗完全同段被误报。
            if qmid < 0.45 {
                reasons.push("window below Porod q region".to_string());
            }
            let rejected = !reasons.is_empty();
            porod.push(IntervalCandidate {
                id: format!("{id}#P{:03}", porod.len()),
                kind: ModelKind::Porod,
                q_min: qmin,
                q_max: qmax,
                n_points: seg.len(),
                rg: None,
                i0: None,
                power: Some(fit.slope),
                amplitude: Some(fit.intercept.exp()),
                r_squared: fit.r_squared,
                qrg_max: None,
                qrg_min: None,
                rejected,
                reject_reasons: reasons,
                source_point_ids: seg.iter().map(|p| p.sample_point_id.clone()).collect(),
            });
        }
    }

    let accepted_g: Vec<&IntervalCandidate> = guinier.iter().filter(|c| !c.rejected).collect();
    let accepted_p: Vec<&IntervalCandidate> = porod.iter().filter(|c| !c.rejected).collect();

    let mut overlaps = Vec::new();
    let mut touch = |a: &IntervalCandidate, b: &IntervalCandidate| {
        if a.q_min < b.q_max && b.q_min < a.q_max {
            overlaps.push((a.id.clone(), b.id.clone()));
        }
    };
    for i in 0..accepted_g.len() {
        for k in (i + 1)..accepted_g.len() {
            touch(accepted_g[i], accepted_g[k]);
        }
    }
    for i in 0..accepted_p.len() {
        for k in (i + 1)..accepted_p.len() {
            touch(accepted_p[i], accepted_p[k]);
        }
    }
    for g in &accepted_g {
        for p in &accepted_p {
            touch(g, p);
        }
    }

    IntervalAnalysis {
        id: id.to_string(),
        subtraction_id: sub.id.clone(),
        guinier,
        porod,
        overlaps,
        no_single_model_joined: true,
    }
}

/// 多候选残差检验：对每条候选缓冲液的扣除结果返回归一化残差（χ²_red）。
/// 不选择唯一答案，全部保留供人工裁定。
pub fn residual_test(sample: &NormCurve, candidate_subs: &[(&str, &Subtraction)]) -> Vec<ResidualScore> {
    let _ = sample;
    candidate_subs
        .iter()
        .map(|(cid, sub)| {
            let n = sub.points.len() as f64;
            let chi: f64 = sub
                .points
                .iter()
                .map(|p| (p.i_sub / if p.sigma_total > 0.0 { p.sigma_total } else { 1.0 }).powi(2))
                .sum();
            let alpha = sub.scale.as_ref().map(|s| s.alpha).unwrap_or(f64::NAN);
            let sigma_alpha = sub.scale.as_ref().map(|s| s.sigma_alpha).unwrap_or(f64::NAN);
            ResidualScore {
                buffer_candidate_id: cid.to_string(),
                chi_squared_red: if n > 1.0 { chi / (n - 1.0) } else { f64::NAN },
                alpha,
                sigma_alpha,
                n_points: sub.points.len(),
                negatives: sub.points.iter().filter(|p| p.negative).count(),
            }
        })
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResidualScore {
    pub buffer_candidate_id: String,
    pub chi_squared_red: f64,
    pub alpha: f64,
    pub sigma_alpha: f64,
    pub n_points: usize,
    pub negatives: usize,
}
