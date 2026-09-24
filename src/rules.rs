use crate::domain::*;

/// q 单位转换因子 -> 1/angstrom。无法转换时返回 None（值留在原位置并附诊断）。
pub fn q_unit_factor(unit: &str) -> Option<f64> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "1/a" | "a^-1" | "1/angstrom" | "angstrom^-1" | "å^-1" | "1/å" => Some(1.0),
        "1/nm" | "nm^-1" => Some(10.0),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct NormParams {
    /// 归一化（透射率/厚度/曝光）的相对不确定度。
    pub scale_uncertainty: f64,
}

/// 归一化：I_norm = I / (T * d * t)。误差包含计数误差与归一化缩放不确定度。
pub fn normalize(raw: &RawCurve, p: &NormParams) -> (Vec<NormPoint>, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let factor = raw.transmission * raw.thickness_cm * raw.exposure_s;
    if !(factor.is_finite() && factor > 0.0) {
        diags.push(Diagnostic::new(
            "norm_factor_invalid",
            format!("transmission*thickness*exposure={factor} 不是正数，无法归一化"),
        ));
        let points = raw
            .points
            .iter()
            .enumerate()
            .map(|(idx, pt)| NormPoint {
                q: pt.q,
                i: f64::NAN,
                sigma: f64::NAN,
                convertible: q_unit_factor(&pt.q_unit).is_some(),
                source_index: idx,
            })
            .collect();
        return (points, diags);
    }
    let mut points = Vec::with_capacity(raw.points.len());
    for (idx, pt) in raw.points.iter().enumerate() {
        match q_unit_factor(&pt.q_unit) {
            Some(f) => {
                let i_norm = pt.i / factor;
                let count_part = pt.sigma / factor;
                let norm_part = p.scale_uncertainty * i_norm;
                let sigma = (count_part * count_part + norm_part * norm_part).sqrt();
                points.push(NormPoint { q: pt.q * f, i: i_norm, sigma, convertible: true, source_index: idx });
            }
            None => {
                diags.push(Diagnostic::at(
                    "unit_unconvertible",
                    format!("q 单位 '{}' 无法转换到 1/angstrom，原值保留在原位置", pt.q_unit),
                    idx,
                ));
                points.push(NormPoint { q: pt.q, i: f64::NAN, sigma: f64::NAN, convertible: false, source_index: idx });
            }
        }
    }
    (points, diags)
}

#[derive(Debug, Clone)]
pub struct SubParams {
    pub scale: f64,
    pub scale_sigma: f64,
}

fn grids_equal(a: &[NormPoint], b: &[NormPoint]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(x, y)| (x.q - y.q).abs() < 1e-9)
}

/// 线性插值（仅在目标 q 落在缓冲液 q 范围内时使用）。
fn interp(q: f64, xs: &[NormPoint]) -> Option<(NormPoint, Option<usize>)> {
    if xs.len() < 2 {
        return None;
    }
    if q < xs[0].q || q > xs[xs.len() - 1].q {
        return None;
    }
    let mut lo = 0usize;
    let mut hi = xs.len() - 1;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if xs[mid].q <= q {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let (a, b) = (&xs[lo], &xs[hi]);
    let f = if (b.q - a.q).abs() < 1e-15 { 0.0 } else { (q - a.q) / (b.q - a.q) };
    let p = NormPoint {
        q,
        i: a.i + f * (b.i - a.i),
        sigma: ((1.0 - f) * a.sigma.powi(2) + f * b.sigma.powi(2)).sqrt(),
        convertible: true,
        source_index: a.source_index,
    };
    Some((p, Some(lo)))
}

pub struct SubOutcome {
    pub points: Vec<SubPoint>,
    pub diagnostics: Vec<Diagnostic>,
    pub candidates: Vec<ScaleCandidate>,
    /// direct | interpolated | none
    pub mode: String,
    pub overlap: Option<(f64, f64)>,
}

fn residual_rms(points: &[SubPoint]) -> f64 {
    let tail_n = (points.len() / 5).max(3).min(points.len());
    let tail = &points[points.len() - tail_n..];
    let s: f64 = tail.iter().map(|p| (p.i / p.sigma.max(1e-300)).powi(2)).sum();
    (s / tail_n as f64).sqrt()
}

/// 背景扣除。
///
/// 语义：
/// - q 网格相同才可直接相减；不同则只在重叠 q 范围内插值缓冲液；
/// - 无重叠区时不出结果（不产生伪曲线）；
/// - 误差 = sqrt(样品误差² + (scale*缓冲误差)² + (scale_sigma*缓冲强度)²)；
/// - 负强度保留并附诊断，不裁剪；
/// - 多个缩放候选的残差全部保留，残差接近时标记为无法唯一决定。
pub fn subtract(sample: &[NormPoint], buffer: &[NormPoint], p: &SubParams) -> SubOutcome {
    let s: Vec<&NormPoint> = sample.iter().filter(|x| x.convertible).collect();
    let b: Vec<NormPoint> = buffer.iter().filter(|x| x.convertible).cloned().collect();
    let mut diagnostics = Vec::new();

    if s.is_empty() || b.is_empty() {
        diagnostics.push(Diagnostic::new("no_overlap", "样品或缓冲液没有可比较的点"));
        return SubOutcome { points: vec![], diagnostics, candidates: vec![], mode: "none".into(), overlap: None };
    }

    let direct = grids_equal(&s.iter().map(|x| (*x).clone()).collect::<Vec<_>>(), &b);
    let (lo, hi) = (s[0].q.max(b[0].q), s[s.len() - 1].q.min(b[b.len() - 1].q));
    let overlap = if lo <= hi + 1e-12 { Some((lo, hi)) } else { None };

    if overlap.is_none() {
        diagnostics.push(Diagnostic::new(
            "no_overlap",
            format!(
                "样品 q∈[{:.5},{:.5}] 与缓冲液 q∈[{:.5},{:.5}] 无重叠，不进行相减",
                s[0].q, s[s.len() - 1].q, b[0].q, b[b.len() - 1].q
            ),
        ));
        return SubOutcome { points: vec![], diagnostics, candidates: vec![], mode: "none".into(), overlap: None };
    }
    if !direct {
        diagnostics.push(Diagnostic::new(
            "q_grid_mismatch",
            "样品与缓冲液 q 网格不同：不直接相减，仅在重叠 q 范围内对缓冲液线性插值",
        ));
    }

    let mut points = Vec::new();
    for (si, sp) in s.iter().enumerate() {
        if sp.q < lo - 1e-12 || sp.q > hi + 1e-12 {
            continue;
        }
        let bp = if direct {
            Some((b[si].clone(), Some(si)))
        } else {
            interp(sp.q, &b)
        };
        if let Some((bp, bi)) = bp {
            let i_sub = sp.i - p.scale * bp.i;
            if i_sub < 0.0 {
                diagnostics.push(Diagnostic::at(
                    "negative_intensity",
                    format!("q={:.5} 处扣除后强度为 {:.4e}，保留负值（不裁剪、不置零）", sp.q, i_sub),
                    points.len(),
                ));
            }
            let sigma2 = sp.sigma.powi(2)
                + (p.scale * bp.sigma).powi(2)
                + (p.scale_sigma * bp.i).powi(2);
            points.push(SubPoint {
                q: sp.q,
                i: i_sub,
                sigma: sigma2.sqrt(),
                sample_index: sp.source_index,
                buffer_index: bi.map(|_| bp.source_index),
            });
        }
    }

    let candidates = build_candidates(&s, &b, direct, lo, hi, p);
    if let Some(ambig) = candidates
        .iter()
        .filter(|c| !c.scale.is_nan())
        .map(|c| c.residual)
        .fold(None, |acc: Option<(f64, f64)>, r| {
            Some(match acc {
                None => (r, r),
                Some((mn, mx)) => (mn.min(r), mx.max(r)),
            })
        })
    {
        let (mn, mx) = ambig;
        if mn > 0.0 && mx / mn < 1.10 {
            diagnostics.push(Diagnostic::new(
                "ambiguous_candidates",
                "不同背景缩放候选的高 q 残差相差不足 10%：无法唯一决定，全部合法候选已保留",
            ));
        }
    }

    SubOutcome {
        points,
        diagnostics,
        candidates,
        mode: if direct { "direct".into() } else { "interpolated".into() },
        overlap,
    }
}

fn build_candidates(
    s: &[&NormPoint],
    b: &[NormPoint],
    direct: bool,
    lo: f64,
    hi: f64,
    p: &SubParams,
) -> Vec<ScaleCandidate> {
    let scales = if p.scale_sigma > 0.0 {
        vec![p.scale - p.scale_sigma, p.scale, p.scale + p.scale_sigma]
    } else {
        vec![p.scale]
    };
    let mut out = Vec::new();
    for scale in scales {
        let mut pts = Vec::new();
        for (si, sp) in s.iter().enumerate() {
            if sp.q < lo - 1e-12 || sp.q > hi + 1e-12 {
                continue;
            }
            let bp = if direct {
                Some((b[si].clone(), Some(si)))
            } else {
                interp(sp.q, b)
            };
            if let Some((bp, _)) = bp {
                pts.push(SubPoint {
                    q: sp.q,
                    i: sp.i - scale * bp.i,
                    sigma: (sp.sigma.powi(2) + (scale * bp.sigma).powi(2) + (p.scale_sigma * bp.i).powi(2)).sqrt(),
                    sample_index: sp.source_index,
                    buffer_index: None,
                });
            }
        }
        out.push(ScaleCandidate { scale, residual: residual_rms(&pts), adopted: (scale - p.scale).abs() < f64::EPSILON });
    }
    out
}

/// 最小二乘直线拟合，返回 (截距, 斜率, 残差 RMS)。
fn linfit(xs: &[f64], ys: &[f64]) -> Option<(f64, f64, f64)> {
    let n = xs.len() as f64;
    if n < 2.0 {
        return None;
    }
    let mx: f64 = xs.iter().sum::<f64>() / n;
    let my: f64 = ys.iter().sum::<f64>() / n;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    for (x, y) in xs.iter().zip(ys.iter()) {
        sxx += (x - mx).powi(2);
        sxy += (x - mx) * (y - my);
    }
    if sxx.abs() < 1e-20 {
        return None;
    }
    let slope = sxy / sxx;
    let intercept = my - slope * mx;
    let rms = (ys
        .iter()
        .zip(xs.iter())
        .map(|(y, x)| (y - (intercept + slope * x)).powi(2))
        .sum::<f64>()
        / n)
        .sqrt();
    Some((intercept, slope, rms))
}

/// 区间拟合：Guinier 与 Porod 各自独立；区间重叠时两者都保留并附诊断，
/// 永远不自动拼成单一模型。
pub fn fit_intervals(points: &[SubPoint], specs: &[IntervalSpec]) -> (Vec<IntervalFit>, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    for (i, a) in specs.iter().enumerate() {
        for b in &specs[i + 1..] {
            if a.q_min < b.q_max && b.q_min < a.q_max {
                diags.push(Diagnostic::new(
                    "interval_overlap",
                    format!(
                        "{} 区间 [{:.4},{:.4}] 与 {} 区间 [{:.4},{:.4}] 重叠：两个拟合均独立保留，不合并为单一模型",
                        a.kind, a.q_min, a.q_max, b.kind, b.q_min, b.q_max
                    ),
                ));
            }
        }
    }
    diags.push(Diagnostic::new(
        "intervals_not_merged",
        "Guinier 与 Porod 区间为并列证据，不自动拼接成单一模型",
    ));

    let mut fits = Vec::new();
    for spec in specs {
        let sel: Vec<&SubPoint> =
            points.iter().filter(|p| p.q >= spec.q_min - 1e-12 && p.q <= spec.q_max + 1e-12).collect();
        if sel.len() < 3 {
            diags.push(Diagnostic::new(
                "interval_underdetermined",
                format!("{} 区间 [{:.4},{:.4}] 内只有 {} 个点，无法唯一决定拟合", spec.kind, spec.q_min, spec.q_max, sel.len()),
            ));
            continue;
        }
        match spec.kind.as_str() {
            "guinier" => {
                let pos: Vec<&SubPoint> = sel.iter().cloned().filter(|p| p.i > 0.0).collect();
                let xs: Vec<f64> = pos.iter().map(|p| p.q * p.q).collect();
                let ys: Vec<f64> = pos.iter().map(|p| p.i.ln()).collect();
                if let Some((ln_i0, slope, res)) = linfit(&xs, &ys) {
                    if slope >= 0.0 {
                        diags.push(Diagnostic::new(
                            "guinier_nonphysical",
                            format!("Guinier 拟合斜率 {:.3e} >= 0，Rg 不物理；拟合保留但标记", slope),
                        ));
                    }
                    let rg = (-3.0 * slope).sqrt();
                    fits.push(IntervalFit {
                        kind: "guinier".into(),
                        q_min: spec.q_min,
                        q_max: spec.q_max,
                        params: vec![("ln_I0".into(), ln_i0), ("I0".into(), ln_i0.exp()), ("Rg_A".into(), rg)],
                        residual: res,
                        n_points: pos.len(),
                    });
                }
            }
            "porod" => {
                let pos: Vec<&SubPoint> = sel.iter().cloned().filter(|p| p.i > 0.0).collect();
                let xs: Vec<f64> = pos.iter().map(|p| p.q.ln()).collect();
                let ys: Vec<f64> = pos.iter().map(|p| p.i.ln()).collect();
                if let Some((intercept, slope, res)) = linfit(&xs, &ys) {
                    fits.push(IntervalFit {
                        kind: "porod".into(),
                        q_min: spec.q_min,
                        q_max: spec.q_max,
                        params: vec![("ln_amplitude".into(), intercept), ("exponent".into(), slope)],
                        residual: res,
                        n_points: pos.len(),
                    });
                }
            }
            other => {
                diags.push(Diagnostic::new("unknown_interval_kind", format!("未知区间类型 {other}")));
            }
        }
    }
    (fits, diags)
}
