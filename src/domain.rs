//! 领域核心：量值保真、归一化、q 网格重叠插值扣背景、Guinier/Porod 多区间证据。
//!
//! 本模块不依赖数据库与 Web，所有规则均可纯函数测试。

use serde::{Deserialize, Serialize};

/// 带原始表示与单位基准的量值。导入时保留原表示，派生层才做规范化。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub raw: String,
    pub unit: String,
    pub value: Option<f64>,
    pub diagnostic: Option<String>,
}

impl Quantity {
    pub fn parsed(raw: impl Into<String>, unit: impl Into<String>, value: f64) -> Self {
        Self { raw: raw.into(), unit: unit.into(), value: Some(value), diagnostic: None }
    }

    pub fn unconvertible(raw: impl Into<String>, unit: impl Into<String>, diagnostic: String) -> Self {
        Self { raw: raw.into(), unit: unit.into(), value: None, diagnostic: Some(diagnostic) }
    }
}

/// 区间模型种类。Guinier 与 Porod 永远是独立候选，不自动拼成单一模型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntervalKind {
    Guinier,
    Porod,
}

/// 一个区间候选（含拟合参数、证据与残差）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub kind: IntervalKind,
    pub id: String,
    pub q_lo: f64,
    pub q_hi: f64,
    pub point_indices: Vec<usize>,
    /// Guinier: (ln I0, -Rg^2/3)；Porod: (ln K, n)，ln I = n ln q + ln K。
    pub intercept: f64,
    pub slope: f64,
    pub rg: Option<f64>,
    pub i0: Option<f64>,
    pub porod_n: Option<f64>,
    pub chi2: f64,
    pub r_squared: f64,
    pub residuals: Vec<Residual>,
    /// Guinier 判据 q_max*Rg <= 1.3 不满足时标记（仍是合法候选，只是带告警）。
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Residual {
    pub point_index: usize,
    pub q: f64,
    pub observed: f64,
    pub predicted: f64,
    pub residual: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntervalReport {
    pub candidates: Vec<IntervalCandidate>,
    /// 两两区间重叠标记（区间 id 对 + 重叠 q 范围）。
    pub overlaps: Vec<(String, String, f64, f64)>,
    pub diagnostics: Vec<String>,
}

impl IntervalReport {
    pub fn candidates_of(&self, kind: IntervalKind) -> Vec<&IntervalCandidate> {
        self.candidates.iter().filter(|c| c.kind == kind).collect()
    }
}

/// 从扣背景曲线枚举 Guinier 与 Porod 区间候选。
///
/// - Guinier：低 q 滑窗（至少 5 点），ln I 对 q^2 线性拟合；
/// - Porod：高 q 滑窗，ln I 对 ln q 线性拟合；
/// - 保留所有候选；重叠单独标记；Guinier/Porod 不自动拼接。
pub fn interval_candidates(
    sub: &SubResult,
    min_points: usize,
    // 窗长候选列表（点数），多个窗长会产生多个候选。
    window_sizes: &[usize],
) -> IntervalReport {
    let mut diags = Vec::new();
    let pts: Vec<&SubPoint> = sub
        .points
        .iter()
        .filter(|p| p.i.is_finite() && p.i > 0.0 && p.di > 0.0)
        .collect();
    let min_points = min_points.max(5);
    if pts.len() < min_points {
        diags.push("扣背景曲线正强度点不足，无法枚举区间候选".into());
        return IntervalReport { candidates: Vec::new(), overlaps: Vec::new(), diagnostics: diags };
    }

    let mut candidates = Vec::new();
    let mut seq = 0usize;
    for &w in window_sizes {
        let w = w.max(min_points);
        if w > pts.len() {
            continue;
        }
        // Guinier：从最低 q 开始的若干起点。
        let starts = (pts.len() - w + 1).min(3);
        for s in 0..starts {
            let window = &pts[s..s + w];
            if let Some(c) = fit_guinier(window, seq) {
                seq += 1;
                candidates.push(c);
            }
        }
        // Porod：从最高 q 结束的若干起点。
        let starts = (pts.len() - w + 1).min(3);
        for s in 0..starts {
            let begin = pts.len() - w - s;
            let window = &pts[begin..begin + w];
            if let Some(c) = fit_porod(window, seq) {
                seq += 1;
                candidates.push(c);
            }
        }
    }

    // 去重（同类、同 q 范围只保留一个）。
    candidates.sort_by(|a, b| a.kind.partial_cmp(&b.kind).unwrap_or(std::cmp::Ordering::Equal).then(a.q_lo.partial_cmp(&b.q_lo).unwrap()).then(a.q_hi.partial_cmp(&b.q_hi).unwrap()));
    candidates.dedup_by(|a, b| a.kind == b.kind && (a.q_lo - b.q_lo).abs() < 1e-12 && (a.q_hi - b.q_hi).abs() < 1e-12);

    let mut overlaps = Vec::new();
    for i in 0..candidates.len() {
        for j in (i + 1)..candidates.len() {
            let (a, b) = (&candidates[i], &candidates[j]);
            let lo = a.q_lo.max(b.q_lo);
            let hi = a.q_hi.min(b.q_hi);
            if hi > lo {
                overlaps.push((a.id.clone(), b.id.clone(), lo, hi));
            }
        }
    }
    if !overlaps.is_empty() {
        diags.push(format!("存在 {} 组区间重叠：候选全部保留，不自动拼接或裁剪", overlaps.len()));
    }
    diags.push(format!(
        "共 {} 个 Guinier 候选、{} 个 Porod 候选；不自动拼成单一模型，由审阅者裁定",
        candidates.iter().filter(|c| c.kind == IntervalKind::Guinier).count(),
        candidates.iter().filter(|c| c.kind == IntervalKind::Porod).count()
    ));

    IntervalReport { candidates, overlaps, diagnostics: diags }
}

fn weighted_linfit(
    xs: &[f64],
    ys: &[f64],
    ws: &[f64],
) -> Option<(f64, f64, f64)> {
    let n = xs.len();
    if n < 2 {
        return None;
    }
    let mut sw = 0.0;
    let mut swx = 0.0;
    let mut swy = 0.0;
    let mut swxx = 0.0;
    let mut swxy = 0.0;
    for i in 0..n {
        let w = ws[i];
        sw += w;
        swx += w * xs[i];
        swy += w * ys[i];
        swxx += w * xs[i] * xs[i];
        swxy += w * xs[i] * ys[i];
    }
    let det = sw * swxx - swx * swx;
    if det.abs() < 1e-30 {
        return None;
    }
    let intercept = (swy * swxx - swx * swxy) / det;
    let slope = (sw * swxy - swx * swy) / det;
    // R^2（在 y 空间，无权重版本足够作证据量）。
    let ymean = swy / sw;
    let mut sst = 0.0;
    let mut sse = 0.0;
    for i in 0..n {
        let pred = intercept + slope * xs[i];
        sst += (ys[i] - ymean).powi(2);
        sse += (ys[i] - pred).powi(2);
    }
    let r2 = if sst > 0.0 { 1.0 - sse / sst } else { 0.0 };
    Some((intercept, slope, r2))
}

fn fit_guinier(window: &[&SubPoint], seq: usize) -> Option<IntervalCandidate> {
    let xs: Vec<f64> = window.iter().map(|p| p.q.powi(2)).collect();
    let ys: Vec<f64> = window.iter().map(|p| p.i.ln()).collect();
    let ws: Vec<f64> = window.iter().map(|p| 1.0 / (p.di / p.i).powi(2).max(1e-12)).collect();
    let (intercept, slope, r2) = weighted_linfit(&xs, &ys, &ws)?;
    if slope >= 0.0 {
        return None;
    }
    let rg = (-3.0 * slope).sqrt();
    let i0 = intercept.exp();
    let q_lo = window.first().unwrap().q;
    let q_hi = window.last().unwrap().q;
    let mut warnings = Vec::new();
    if q_hi * rg > 1.3 {
        warnings.push(format!("q_max*Rg={:.2} > 1.3，超出常用 Guinier 判据", q_hi * rg));
    }
    let (chi2, residuals) = lin_residuals(window, |q| intercept + slope * q * q, |p| p.di / p.i);
    Some(IntervalCandidate {
        kind: IntervalKind::Guinier,
        id: format!("guinier-{:02}", seq),
        q_lo,
        q_hi,
        point_indices: window.iter().map(|p| p.sample_index).collect(),
        intercept,
        slope,
        rg: Some(rg),
        i0: Some(i0),
        porod_n: None,
        chi2,
        r_squared: r2,
        residuals,
        warnings,
    })
}

fn fit_porod(window: &[&SubPoint], seq: usize) -> Option<IntervalCandidate> {
    let xs: Vec<f64> = window.iter().map(|p| p.q.ln()).collect();
    let ys: Vec<f64> = window.iter().map(|p| p.i.ln()).collect();
    let ws: Vec<f64> = window.iter().map(|p| 1.0 / (p.di / p.i).powi(2).max(1e-12)).collect();
    let (intercept, slope, r2) = weighted_linfit(&xs, &ys, &ws)?;
    if slope >= 0.0 {
        return None;
    }
    let q_lo = window.first().unwrap().q;
    let q_hi = window.last().unwrap().q;
    let mut warnings = Vec::new();
    if slope > -3.0 {
        warnings.push(format!("幂指数 n={:.2} 缓于 q^-3，Porod q^-4 证据弱", slope));
    }
    let (chi2, residuals) = lin_residuals(window, |q| intercept + slope * q.ln(), |p| p.di / p.i);
    Some(IntervalCandidate {
        kind: IntervalKind::Porod,
        id: format!("porod-{:02}", seq),
        q_lo,
        q_hi,
        point_indices: window.iter().map(|p| p.sample_index).collect(),
        intercept,
        slope,
        rg: None,
        i0: None,
        porod_n: Some(slope),
        chi2,
        r_squared: r2,
        residuals,
        warnings,
    })
}

fn lin_residuals(
    window: &[&SubPoint],
    pred_ln: impl Fn(f64) -> f64,
    rel_sigma: impl Fn(&SubPoint) -> f64,
) -> (f64, Vec<Residual>) {
    let mut chi2 = 0.0;
    let residuals = window
        .iter()
        .map(|p| {
            let ln_pred = pred_ln(p.q);
            let predicted = ln_pred.exp();
            let residual = p.i - predicted;
            let sigma = p.di.max(rel_sigma(p) * p.i);
            chi2 += (residual * residual) / sigma.powi(2);
            Residual {
                point_index: p.sample_index,
                q: p.q,
                observed: p.i,
                predicted,
                residual,
                sigma,
            }
        })
        .collect();
    (chi2 / window.len() as f64, residuals)
}

/// 单个原始数据点。原始 q/I/误差全部保留。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub index: usize,
    pub q_raw: String,
    pub i_raw: String,
    pub di_raw: String,
    /// 规范化到内部基准（q: 1/A；其余量在归一化时按单位换算）后的 q。
    pub q: Option<f64>,
    pub i: Option<f64>,
    pub di: Option<f64>,
    /// 该点是否被遮罩排除；None 表示遮罩未覆盖。
    pub masked: Option<bool>,
    /// 遮罩命中来源，例如 `mask:mask-v1#37`。
    pub mask_ref: Option<String>,
    pub diagnostics: Vec<String>,
}

/// 原始曲线材料（样品或缓冲液候选）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurveMaterial {
    pub kind: CurveKind,
    pub version: String,
    pub points: Vec<CurvePoint>,
    pub exposure: Quantity,
    pub thickness: Quantity,
    pub transmission: Quantity,
    pub q_unit: String,
    pub i_unit: String,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveKind {
    Sample,
    Buffer,
}

/// 仪器响应来源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub version: String,
    pub source: String,
    /// 每点相对响应（与曲线同长时逐点应用；短于曲线时按 q 最近邻查找）。
    pub factors: Vec<f64>,
    pub q_grid: Vec<f64>,
}

/// 遮罩来源：显式列出被排除的原始点索引。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskSource {
    pub version: String,
    pub source: String,
    pub excluded: Vec<usize>,
}

/// 规范化后的曲线点（内部统一基准）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormPoint {
    pub index: usize,
    pub q: f64,
    pub i: f64,
    pub di: f64,
    pub masked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormCurve {
    pub points: Vec<NormPoint>,
    pub exposure: f64,
    pub thickness: f64,
    pub transmission: f64,
    /// 归一化常量 = exposure * thickness * transmission。
    pub norm_constant: f64,
    pub diagnostics: Vec<String>,
}

impl NormCurve {
    /// 未遮罩点。
    pub fn active(&self) -> impl Iterator<Item = &NormPoint> {
        self.points.iter().filter(|p| !p.masked)
    }

    pub fn sorted(&self) -> Vec<&NormPoint> {
        let mut v: Vec<_> = self.active().collect();
        v.sort_by(|a, b| a.q.partial_cmp(&b.q).unwrap());
        v
    }
}

/// 支持的 q 单位换算到 1/angstrom 的因子；失败时返回诊断而非猜测。
pub fn q_unit_to_inv_a(unit: &str) -> Result<f64, String> {
    match unit.trim().to_ascii_lowercase().replace(' ', "").as_str() {
        "1/a" | "a^-1" | "å^-1" | "1/angstrom" | "angstrom^-1" => Ok(1.0),
        "1/nm" | "nm^-1" => Ok(0.1),
        "1/cm" | "cm^-1" => Ok(1.0e-8),
        other => Err(format!("无法把 q 单位 '{}' 转换到 1/A", other)),
    }
}

/// 曝光时间单位 -> s。
pub fn time_to_seconds(unit: &str) -> Result<f64, String> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "s" | "sec" | "seconds" => Ok(1.0),
        "ms" => Ok(1e-3),
        "min" => Ok(60.0),
        other => Err(format!("无法把曝光单位 '{}' 转换到 s", other)),
    }
}

/// 厚度单位 -> cm。
pub fn thickness_to_cm(unit: &str) -> Result<f64, String> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "cm" => Ok(1.0),
        "mm" => Ok(0.1),
        "m" => Ok(100.0),
        "um" | "µm" => Ok(1e-4),
        other => Err(format!("无法把厚度单位 '{}' 转换到 cm", other)),
    }
}

/// 透射率无单位；记录 `t=`、`%`、0-1 三种表示。返回 (0..1 区间值, 诊断)。
pub fn parse_transmission(q: &Quantity) -> (Option<f64>, Vec<String>) {
    let mut diags = Vec::new();
    let v = match q.value {
        Some(v) => v,
        None => return (None, vec![q.diagnostic.clone().unwrap_or_else(|| "透射率缺失".into())]),
    };
    let normalized = if q.unit.trim().eq_ignore_ascii_case("%") {
        v / 100.0
    } else {
        v
    };
    if !(0.0..=1.0).contains(&normalized) {
        diags.push(format!("透射率 {} 超出 [0,1]，原始值保留在原处不参与规范化", q.raw));
        return (None, diags);
    }
    if normalized == 0.0 {
        diags.push("透射率为 0，无法做透射归一化".into());
        return (None, diags);
    }
    (Some(normalized), diags)
}

/// 两条 q 网格的重叠开区间（严格位于两端范围之内才算可插值）。
pub fn overlap_range(a: &NormCurve, b: &NormCurve) -> Option<(f64, f64)> {
    let qa: Vec<f64> = a.sorted().into_iter().map(|p| p.q).collect();
    let qb: Vec<f64> = b.sorted().into_iter().map(|p| p.q).collect();
    if qa.len() < 2 || qb.len() < 2 {
        return None;
    }
    let lo = qa.first().unwrap().max(*qb.first().unwrap());
    let hi = qa.last().unwrap().min(*qb.last().unwrap());
    // 端点必须严格落在缓冲液网格内部，插值才有定义。
    if hi <= lo {
        return None;
    }
    Some((lo, hi))
}

/// 在有序点上做线性插值；q 严格位于网格内部才返回。
pub fn interp_sorted(grid: &[&NormPoint], q: f64) -> Option<(f64, f64)> {
    if grid.len() < 2 {
        return None;
    }
    let first = grid.first().unwrap().q;
    let last = grid.last().unwrap().q;
    if !(q > first && q < last) {
        return None;
    }
    let pos = grid.binary_search_by(|p| {
        p.q.partial_cmp(&q).unwrap_or(std::cmp::Ordering::Equal)
    });
    let hi_idx = match pos {
        Ok(i) => i,
        Err(i) => i,
    };
    if hi_idx == 0 || hi_idx >= grid.len() {
        return None;
    }
    let lo = grid[hi_idx - 1];
    let hi = grid[hi_idx];
    let t = (q - lo.q) / (hi.q - lo.q);
    let val = lo.i + t * (hi.i - lo.i);
    // 插值误差按两支撑点误差线性插值（保守、无交叉项假设）。
    let err = lo.di + t * (hi.di - lo.di);
    Some((val, err))
}

/// 把原始曲线材料规范化：单位换算、逐点仪器响应、遮罩标注。
/// 无法转换的点/量保留原始表示并附诊断，不丢弃。
pub fn normalize(
    mat: &CurveMaterial,
    instrument: Option<&InstrumentResponse>,
    mask: Option<&MaskSource>,
) -> NormCurve {
    let mut diags = Vec::new();
    let q_factor = match q_unit_to_inv_a(&mat.q_unit) {
        Ok(f) => f,
        Err(d) => {
            diags.push(d);
            f64::NAN
        }
    };
    let exposure = match (mat.exposure.value, time_to_seconds(&mat.exposure.unit)) {
        (Some(v), Ok(f)) => v * f,
        _ => {
            diags.push(format!("曝光时间无法换算: {} {}", mat.exposure.raw, mat.exposure.unit));
            f64::NAN
        }
    };
    let thickness = match (mat.thickness.value, thickness_to_cm(&mat.thickness.unit)) {
        (Some(v), Ok(f)) => v * f,
        _ => {
            diags.push(format!("厚度无法换算: {} {}", mat.thickness.raw, mat.thickness.unit));
            f64::NAN
        }
    };
    let (transmission, mut tdiag) = parse_transmission(&mat.transmission);
    diags.append(&mut tdiag);
    let transmission = transmission.unwrap_or(f64::NAN);
    let norm_constant = exposure * thickness * transmission;
    if !(norm_constant.is_finite() && norm_constant > 0.0) {
        diags.push("归一化常量（曝光×厚度×透射率）无效：原始量保留，派生点全部标记不可用".into());
    }

    let points = mat
        .points
        .iter()
        .map(|p| {
            let mut pdiag = p.diagnostics.clone();
            let q = p.q.map(|v| v * q_factor).filter(|v| v.is_finite());
            let mut i = p.i.filter(|v| v.is_finite());
            let di = p.di.filter(|v| v.is_finite());
            if q.is_none() {
                pdiag.push("q 无法规范化，点保留但不参与派生".into());
            }
            if i.is_none() {
                pdiag.push("I 无法规范化，点保留但不参与派生".into());
            }
            // 仪器响应逐点校正。
            if let (Some(qv), Some(resp)) = (q, instrument) {
                if let Some(f) = response_factor(resp, qv) {
                    if f > 0.0 {
                        i = i.map(|v| v / f);
                    } else {
                        pdiag.push(format!("仪器响应因子 {}<=0，该点不做响应校正", f));
                    }
                } else {
                    pdiag.push("q 落在仪器响应表外，该点不做响应校正".into());
                }
            }
            let excluded = mask.map(|m| m.excluded.contains(&p.index)).unwrap_or(false);
            if excluded {
                pdiag.push(format!("被遮罩 {} 排除", mask.unwrap().version));
            }
            let usable = q.is_some() && i.is_some() && di.is_some();
            if !usable && !pdiag.is_empty() {
                diags.extend(pdiag.iter().cloned());
            }
            let (i, di) = if norm_constant.is_finite() && norm_constant > 0.0 {
                (i.map(|v| v / norm_constant), di.map(|v| v / norm_constant))
            } else {
                (None, None)
            };
            NormPoint {
                index: p.index,
                q: q.unwrap_or(f64::NAN),
                i: i.unwrap_or(f64::NAN),
                di: di.unwrap_or(f64::NAN),
                masked: excluded || !usable || !(norm_constant.is_finite() && norm_constant > 0.0),
            }
        })
        .collect();

    NormCurve { points, exposure, thickness, transmission, norm_constant, diagnostics: diags }
}

fn response_factor(resp: &InstrumentResponse, q: f64) -> Option<f64> {
    if resp.factors.len() == resp.q_grid.len() && resp.q_grid.len() >= 2 {
        // q_grid 假定升序、单位 1/A。
        if q < resp.q_grid[0] || q > *resp.q_grid.last().unwrap() {
            return None;
        }
        let pos = resp.q_grid.partition_point(|x| *x < q);
        if pos == 0 {
            return Some(resp.factors[0]);
        }
        if pos >= resp.q_grid.len() {
            return Some(*resp.factors.last().unwrap());
        }
        let (q0, q1) = (resp.q_grid[pos - 1], resp.q_grid[pos]);
        let t = if q1 > q0 { (q - q0) / (q1 - q0) } else { 0.0 };
        return Some(resp.factors[pos - 1] + t * (resp.factors[pos] - resp.factors[pos - 1]));
    }
    None
}

/// 扣背景后的单点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPoint {
    /// 样品规范化点索引（与样品点稳定身份对应）。
    pub sample_index: usize,
    pub q: f64,
    pub sample_i: f64,
    pub buffer_i: f64,
    pub i: f64,
    pub di: f64,
    /// 负强度（I - scale*Ib <= 0）显式保留并标记，不截断。
    pub negative: bool,
}

/// 缩放拟合结果（在重叠区内对两条规范化曲线做加权最小二乘）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScaleFit {
    pub scale: f64,
    pub scale_sigma: f64,
    pub fit_points: usize,
    pub chi2: f64,
    /// 缩放不确定度是否超过相对阈值（背景缩放不确定度情形）。
    pub uncertain: bool,
    pub sigma_rel: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubResult {
    pub points: Vec<SubPoint>,
    pub overlap: Option<(f64, f64)>,
    pub scale: Option<ScaleFit>,
    pub diagnostics: Vec<String>,
}

/// 在重叠范围内对缓冲液插值并扣背景。
///
/// 规则：q 网格不同时不直接相减；插值只在重叠范围生效；
/// 样品/缓冲误差传播外，额外计入背景缩放不确定度 scale_sigma * Ib。
/// 无重叠区时不给数值结果，只返回诊断。
pub fn subtract_background(
    sample: &NormCurve,
    buffer: &NormCurve,
    // 归一化引入的相对不确定度（如透射率标定相对误差）。
    norm_rel_sigma: f64,
    // 判定“背景缩放不确定度”的相对阈值。
    scale_uncertainty_threshold: f64,
) -> SubResult {
    let mut diags = Vec::new();
    let overlap = overlap_range(sample, buffer);
    let overlap = match overlap {
        Some(o) => o,
        None => {
            return SubResult {
                points: Vec::new(),
                overlap: None,
                scale: None,
                diagnostics: vec![
                    "两条 q 网格没有可插值的重叠区：不直接相减，也不外推".into(),
                ],
            }
        }
    };

    let sample_pts = sample.sorted();
    let buffer_pts = buffer.sorted();

    // 只在重叠开区间内的样品点参与。
    let in_range: Vec<&NormPoint> = sample_pts
        .iter()
        .copied()
        .filter(|p| p.q > overlap.0 && p.q < overlap.1)
        .collect();

    // 加权最小二乘：scale = sum w s b / sum w b^2，w = 1/(ds^2+(rel*s)^2+(db?)^2)
    // 误差采用样品与插值缓冲误差，缩放不确定度先不进权重（避免自相关）。
    let mut num = 0.0_f64;
    let mut den = 0.0_f64;
    let mut fit_points = 0usize;
    let mut interpolated: Vec<(&NormPoint, f64, f64)> = Vec::new();
    for sp in &in_range {
        let (bi, db) = match interp_sorted(&buffer_pts, sp.q) {
            Some(v) => v,
            None => continue,
        };
        interpolated.push((sp, bi, db));
        let ds2 = sp.di.powi(2) + (norm_rel_sigma * sp.i).powi(2);
        let w = 1.0 / (ds2 + db.powi(2));
        num += w * sp.i * bi;
        den += w * bi * bi;
        fit_points += 1;
    }

    if fit_points < 2 || den <= 0.0 {
        diags.push("重叠区内可插值点不足，无法估计背景缩放".into());
        return SubResult { points: Vec::new(), overlap: Some(overlap), scale: None, diagnostics: diags };
    }

    let scale = num / den;
    // 缩放协方差近似：sigma_scale^2 = 1 / sum(w b^2)（线性 WLS）。
    let scale_sigma = den.recip().sqrt().max(0.0);

    let mut chi2 = 0.0;
    let mut points = Vec::new();
    for (sp, bi, db) in interpolated {
        let sub = sp.i - scale * bi;
        // 误差传播：样品误差 + 归一化相对误差 + 缓冲插值误差 + 背景缩放不确定度。
        let variance = sp.di.powi(2)
            + (norm_rel_sigma * sp.i).powi(2)
            + (scale * db).powi(2)
            + (scale_sigma * bi).powi(2);
        let di = variance.max(0.0).sqrt();
        chi2 += (sub * sub) / variance.max(1e-30);
        if sub <= 0.0 {
            diags.push(format!("q={:.5} 1/A 处扣背景结果为负 ({:.4e})，保留负值不截断", sp.q, sub));
        }
        points.push(SubPoint {
            sample_index: sp.index,
            q: sp.q,
            sample_i: sp.i,
            buffer_i: scale * bi,
            i: sub,
            di,
            negative: sub <= 0.0,
        });
    }

    let sigma_rel = if scale.abs() > 0.0 { scale_sigma / scale.abs() } else { f64::INFINITY };
    let uncertain = sigma_rel > scale_uncertainty_threshold;
    if uncertain {
        diags.push(format!(
            "背景缩放不确定度过大：scale={:.4} ± {:.4}（相对 {:.2}% > 阈值 {:.2}%）",
            scale,
            scale_sigma,
            sigma_rel * 100.0,
            scale_uncertainty_threshold * 100.0
        ));
    }

    SubResult {
        points,
        overlap: Some(overlap),
        scale: Some(ScaleFit {
            scale,
            scale_sigma,
            fit_points,
            chi2: chi2 / fit_points.max(1) as f64,
            uncertain,
            sigma_rel,
        }),
        diagnostics: diags,
    }
}
