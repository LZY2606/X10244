//! 领域计算规则。规则版本随输出写入，重放与“哪版规则造成差异”反查均依赖它。
//!
//! 关键语义：
//! * RULE_SUBTRACT_V1：q 网格不同时不直接相减；只在重叠 q 范围内把缓冲液
//!   线性插值到样品网格后再相减；无重叠则不产生任何点。
//! * RULE_SUBTRACT_V1：每个点的总误差包含归一化相对误差与背景缩放不确定度。
//! * RULE_MODEL_V1：Guinier 与 Porod 区间各自独立给出全部合法候选，
//!   绝不自动拼成单一模型；无法唯一决定时保留全部候选。

use crate::model::*;
use crate::ids;
use crate::units::{self, Quant, CANON_Q, CANON_INTENSITY};

pub const RULE_NORMALIZE: &str = "rules/normalize@1";
pub const RULE_SUBTRACT: &str = "rules/subtract@1";
pub const RULE_MODEL: &str = "rules/model@1";

pub const GRID_MISALIGNED: &str = "grid_misaligned";
pub const NO_OVERLAP: &str = "no_overlap";
pub const SCALE_UNCERTAINTY: &str = "scale_uncertainty";
pub const NEGATIVE_INTENSITY: &str = "negative_intensity";
pub const REGION_OVERLAP: &str = "region_overlap";
pub const BUFFER_SWITCH: &str = "buffer_version_switch";
pub const MULTI_CANDIDATE: &str = "multi_candidate_residual";
pub const UNCONVERTIBLE: &str = "unconvertible_value";
pub const MASKED_POINT: &str = "masked_point";

/// q 网格是否“相同”：点数一致且所有坐标在相对容差内相等。
pub fn grids_aligned(a: &[Point], b: &[Point]) -> bool {
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    a.iter().zip(b).all(|(x, y)| q_equal(x.q, y.q))
}

fn q_equal(x: f64, y: f64) -> bool {
    let tol = 1e-9 * x.abs().max(y.abs()).max(1e-12);
    (x - y).abs() <= tol
}

#[derive(Debug, Clone)]
pub struct Overlap {
    pub qmin: f64,
    pub qmax: f64,
}

pub fn overlap_range(a: &[Point], b: &[Point]) -> Option<Overlap> {
    let amin = a.iter().map(|p| p.q).fold(f64::INFINITY, f64::min);
    let amax = a.iter().map(|p| p.q).fold(f64::NEG_INFINITY, f64::max);
    let bmin = b.iter().map(|p| p.q).fold(f64::INFINITY, f64::min);
    let bmax = b.iter().map(|p| p.q).fold(f64::NEG_INFINITY, f64::max);
    let lo = amin.max(bmin);
    let hi = amax.min(bmax);
    if lo < hi && (hi - lo) > 1e-12 { Some(Overlap { qmin: lo, qmax: hi }) } else { None }
}

/// 把 y=f(x) 线性插值到 xq。超出网格返回 None（不做外推）。
fn interp(xs: &[f64], ys: &[f64], xq: f64) -> Option<(f64, f64, usize, usize)> {
    if xs.is_empty() { return None; }
    if xq < xs[0] || xq > xs[xs.len() - 1] { return None; }
    let idx = match xs.binary_search_by(|v| v.partial_cmp(&xq).unwrap()) {
        Ok(i) => return Some((ys[i], 0.0, i, i)),
        Err(i) => i,
    };
    if idx == 0 || idx >= xs.len() { return None; }
    let (x0, x1) = (xs[idx - 1], xs[idx]);
    let (y0, y1) = (ys[idx - 1], ys[idx]);
    let t = (xq - x0) / (x1 - x0);
    Some((y0 + t * (y1 - y0), t, idx - 1, idx))
}

fn qq(row: &CurveRow) -> Result<f64, (String, String)> {
    let q = Quant::raw(&row.q_text, &row.q_unit, CANON_Q);
    match q.value {
        Some(v) if v.is_finite() && v > 0.0 => Ok(v),
        _ => Err((q.raw_text.clone(), q.diagnostic.unwrap_or_else(|| "q 非正或非有限".to_string()))),
    }
}

fn ii(row: &CurveRow, _i_unit: &str) -> Result<(f64, f64), (String, String)> {
    let canonical = if row.i_unit == "cm^-1" || row.i_unit == "1/cm" { "cm^-1" } else { CANON_INTENSITY };
    let iv = Quant::raw(&row.i_text, &row.i_unit, canonical);
    let ev = Quant::raw(&row.err_text, &row.err_unit, canonical);
    match (iv.value, ev.value) {
        (Some(i), Some(e)) if i.is_finite() && e.is_finite() && e >= 0.0 => Ok((i, e)),
        _ => Err((iv.raw_text.clone(), iv.diagnostic.or(ev.diagnostic).unwrap_or_else(|| "强度/误差无效".to_string()))),
    }
}

/// 解析遮罩的逐字区间描述。支持 "q<0.01 A^-1"、"q>0.3"、"0.1..0.12"、"beamstop"（无作用点，仅记录）。
fn mask_hit(ranges: &[String], q: f64) -> bool {
    for r in ranges {
        let s = r.trim();
        let num = |key: &str| -> Option<f64> {
            let rest = s.strip_prefix(key)?;
            let rest = rest.trim().trim_start_matches('=').trim();
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | 'e' | 'E' | '+')).collect();
            num.replace(',', ".").parse().ok()
        };
        if let Some(v) = num("q<") { if q < v { return true; } continue; }
        if let Some(v) = num("q>") { if q > v { return true; } continue; }
        if let Some(v) = num("q<=") { if q <= v { return true; } continue; }
        if let Some(v) = num("q>=") { if q >= v { return true; } continue; }
        if s.contains("..") {
            let parts: Vec<&str> = s.split("..").collect();
            if parts.len() == 2 {
                if let (Ok(lo), Ok(hi)) = (parts[0].trim().parse::<f64>(), parts[1].trim().split_whitespace().next().unwrap_or("").parse::<f64>()) {
                    if q >= lo && q <= hi { return true; }
                }
            }
        }
    }
    false
}

pub struct NormalizeResult {
    pub normalized: Normalized,
    pub diagnostics: Vec<Diagnostic>,
    pub q_unit: String,
    pub i_unit: String,
}

/// 原始材料 -> 规范化曲线：单位转换、遮罩、曝光/厚度/透射归一。
pub fn normalize(mat: &Material) -> NormalizeResult {
    let i_unit = mat.rows.first().map(|r| if r.i_unit.trim().is_empty() { CANON_INTENSITY.to_string() } else if r.i_unit == "cm^-1" || r.i_unit == "1/cm" { "cm^-1".to_string() } else { r.i_unit.clone() }).unwrap_or_else(|| CANON_INTENSITY.to_string());
    let exposure = mat.exposure.as_ref().and_then(|q| q.value).filter(|v| *v > 0.0);
    let thickness = mat.thickness.as_ref().and_then(|q| q.value).filter(|v| *v > 0.0);
    let transmission = mat.transmission.as_ref().and_then(|q| q.value).filter(|v| *v > 0.0);

    let mut diagnostics = Vec::new();
    let mut parsed: Vec<(usize, f64, f64, f64)> = Vec::new();
    let mut row_diag: Vec<(usize, String)> = Vec::new();
    for (i, row) in mat.rows.iter().enumerate() {
        let qres = qq(row);
        let ires = ii(row, &i_unit);
        match (qres, ires) {
            (Ok(q), Ok((iv, ev))) => parsed.push((i, q, iv, ev)),
            (Err((qt, qm)), ires) => {
                let msg = match &ires { Ok(_) => qm, Err((_, im)) => format!("{}; {}", qm, im) };
                row_diag.push((i, msg.clone()));
                diagnostics.push(Diagnostic { code: UNCONVERTIBLE.to_string(), severity: Severity::Warning, message: format!("材料 {} 行 {} 值 '{}' 无法规范化，保留在原始记录中：{}", mat.id, i, qt, msg), refs: vec![ids::material_version(&mat.id, mat.revision)] });
            }
        }
    }

    let mask_ranges: Vec<String> = mat.mask.as_ref().map(|m| m.ranges.clone()).unwrap_or_default();
    let mut masked_rows = Vec::new();
    let mut kept: Vec<(usize, f64, f64, f64)> = Vec::new();
    for (ri, q, iv, ev) in parsed {
        if mask_hit(&mask_ranges, q) {
            masked_rows.push(ri);
            diagnostics.push(Diagnostic { code: MASKED_POINT.to_string(), severity: Severity::Info, message: format!("材料 {} 原始行 {}（q≈{} {}）被遮罩来源 {} 剔除", mat.id, ri, row_q_text(mat, ri), q_unit_label(), mat.mask.as_ref().map(|m| m.file.as_str()).unwrap_or("?")), refs: vec![ids::material_version(&mat.id, mat.revision)] });
        } else {
            kept.push((ri, q, iv, ev));
        }
    }
    kept.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    kept.dedup_by(|a, b| q_equal(a.1, b.1));

    let norm = |v: f64| {
        let mut n = v;
        if let Some(e) = exposure { n /= e; }
        if let Some(t) = transmission { n /= t; }
        if let Some(d) = thickness { n /= d; }
        n
    };

    let vid = ids::material_version(&mat.id, mat.revision);
    let points: Vec<Point> = kept.iter().enumerate().map(|(k, (ri, q, iv, ev))| {
        let i = norm(*iv);
        // 相对误差在乘性归一化下保持不变。
        let rel = if iv.abs() > 0.0 { ev / iv.abs() } else { 0.0 };
        let sigma = i.abs() * rel;
        Point { point_id: ids::point(&vid, k), q: *q, i, sigma, interpolated: false, sources: vec![format!("{}#row{}", vid, ri)] }
    }).collect();

    if !diagnostics.iter().any(|d| d.code == UNCONVERTIBLE) {
        // 顶层标量的转换诊断
        for (name, q) in [("曝光", mat.exposure.as_ref()), ("厚度", mat.thickness.as_ref()), ("透射率", mat.transmission.as_ref())].iter().filter_map(|(n, q)| q.map(|q| (n, q))) {
            if !q.ok() {
                let msg = q.diagnostic.clone().unwrap_or_default();
                diagnostics.push(Diagnostic { code: UNCONVERTIBLE.to_string(), severity: Severity::Warning, message: format!("材料 {} {}无法规范化：{}（原始表示 '{} {}' 已保留）", mat.id, name, msg, q.raw_text, q.raw_unit), refs: vec![vid.clone()] });
            }
        }
    }

    let normalized = Normalized {
        material_id: mat.id.clone(),
        material_rev: mat.revision,
        q_unit: CANON_Q.to_string(),
        i_unit: i_unit.clone(),
        exposure_s: exposure,
        thickness_cm: thickness,
        transmission,
        instrument_ref: mat.instrument_ref.clone(),
        mask_applied: mat.mask.clone(),
        masked_rows,
        row_diagnostics: row_diag,
        points,
    };
    NormalizeResult { normalized, diagnostics, q_unit: CANON_Q.to_string(), i_unit }
}

fn row_q_text(mat: &Material, i: usize) -> String {
    mat.rows.get(i).map(|r| format!("{} {}", r.q_text, r.q_unit)).unwrap_or_default()
}
fn q_unit_label() -> String { CANON_Q.to_string() }


