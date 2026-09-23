//! 背景扣除规则 RULE_SUBTRACT_V1（实现细节见 rules.rs 文档）。

use crate::ids;
use crate::model::*;
use crate::rules::{grids_aligned, overlap_range, GRID_MISALIGNED, NEGATIVE_INTENSITY, NO_OVERLAP, SCALE_UNCERTAINTY};

#[derive(Debug, Clone)]
pub struct SubtractParams {
    /// 拟合缩放因子 α 的重叠子范围；None 表示使用整个重叠区。
    pub fit_range: Option<[f64; 2]>,
}

impl Default for SubtractParams {
    fn default() -> Self { SubtractParams { fit_range: None } }
}

pub struct SubtractResult {
    pub subtraction: Subtraction,
    pub diagnostics: Vec<Diagnostic>,
}

struct InterpPoint {
    sample_idx: usize,
    ib: f64,
    sb: f64,
    interpolated: bool,
    buffer_sources: Vec<String>,
}

fn lookup(buffer: &[Point], q: f64) -> Option<InterpPoint> {
    if buffer.is_empty() || q < buffer[0].q || q > buffer[buffer.len() - 1].q {
        return None;
    }
    match buffer.binary_search_by(|p| p.q.partial_cmp(&q).unwrap()) {
        Ok(i) => Some(InterpPoint { sample_idx: 0, ib: buffer[i].i, sb: buffer[i].sigma, interpolated: false, buffer_sources: vec![buffer[i].point_id.clone()] }),
        Err(i) => {
            if i == 0 || i >= buffer.len() { return None; }
            let (p0, p1) = (&buffer[i - 1], &buffer[i]);
            let t = (q - p0.q) / (p1.q - p0.q);
            Some(InterpPoint {
                sample_idx: 0,
                ib: p0.i + t * (p1.i - p0.i),
                sb: ((1.0 - t) * p0.sigma).hypot(t * p1.sigma),
                interpolated: true,
                buffer_sources: vec![p0.point_id.clone(), p1.point_id.clone()],
            })
        }
    }
}

/// 样品归一曲线 - α × 缓冲液（仅重叠区、插值）。目标版本 id 用于稳定 point_id。
pub fn subtract(
    sample: &Normalized,
    buffer: &Normalized,
    params: &SubtractParams,
    sub_node: &str,
    target_version_id: &str,
) -> SubtractResult {
    let sp = &sample.points;
    let bp = &buffer.points;
    let mut diagnostics = Vec::new();

    if !grids_aligned(sp, bp) {
        diagnostics.push(Diagnostic {
            code: GRID_MISALIGNED.to_string(),
            severity: Severity::Info,
            message: "样品与缓冲液 q 网格不同：不逐点直接相减，仅在重叠 q 范围内将缓冲液线性插值到样品网格".to_string(),
            refs: vec![ids::node_normalized(&sample.material_id), ids::node_normalized(&buffer.material_id)],
        });
    }

    let overlap = overlap_range(sp, bp);
    let overlap_arr = overlap.as_ref().map(|o| [o.qmin, o.qmax]);

    let mut mapped: Vec<InterpPoint> = Vec::new();
    for (si, p) in sp.iter().enumerate() {
        if let Some(mut m) = lookup(bp, p.q) {
            m.sample_idx = si;
            mapped.push(m);
        }
    }

    let in_fit = |q: f64| -> bool {
        if let Some([lo, hi]) = params.fit_range { q >= lo && q <= hi }
        else if let Some(o) = &overlap { q >= o.qmin && q <= o.qmax } else { false }
    };
    let fit_idx: Vec<usize> = mapped.iter().enumerate().filter(|(_, m)| in_fit(sp[m.sample_idx].q)).map(|(i, _)| i).collect();

    let mut scale_fit = None;
    if overlap.is_some() && !fit_idx.is_empty() {
        // 误差加权最小二乘，权重含 (α·σb)²，迭代三次收敛。
        let mut alpha: f64 = {
            let (mut num, mut den) = (0.0, 0.0);
            for &i in &fit_idx {
                let m = &mapped[i];
                let s = &sp[m.sample_idx];
                let w = 1.0 / s.sigma.powi(2).max(f64::MIN_POSITIVE);
                num += s.i * m.ib * w;
                den += m.ib * m.ib * w;
            }
            if den > 0.0 { num / den } else { 1.0 }
        };
        for _ in 0..3 {
            let (mut num, mut den) = (0.0, 0.0);
            for &i in &fit_idx {
                let m = &mapped[i];
                let s = &sp[m.sample_idx];
                let w = 1.0 / (s.sigma.powi(2) + (alpha * m.sb).powi(2)).max(f64::MIN_POSITIVE);
                num += s.i * m.ib * w;
                den += m.ib * m.ib * w;
            }
            if den > 0.0 { alpha = num / den; }
        }
        let (mut chi2, mut info) = (0.0, 0.0);
        for &i in &fit_idx {
            let m = &mapped[i];
            let s = &sp[m.sample_idx];
            let w = 1.0 / (s.sigma.powi(2) + (alpha * m.sb).powi(2)).max(f64::MIN_POSITIVE);
            chi2 += (s.i - alpha * m.ib).powi(2) * w;
            info += m.ib * m.ib * w;
        }
        // Fisher 信息 J^T W J = Σ w·Ib²，WLS 协方差 Var(α) = 1 / (J^T W J)。
        let scale_sigma = if info > 0.0 { 1.0 / info.sqrt() } else { 0.0 };
        let n = fit_idx.len();
        let [lo, hi] = params.fit_range
            .unwrap_or_else(|| overlap.as_ref().map(|o| [o.qmin, o.qmax]).unwrap_or([0.0, 0.0]));
        scale_fit = Some(ScaleFit {
            fit_range_q: [lo, hi], scale: alpha, scale_sigma,
            n_pairs: n, degrees_of_freedom: n.saturating_sub(1), chi2,
            method: "error_weighted_ols_overlap@1".to_string(),
        });
        let rel = scale_sigma / alpha.abs().max(f64::MIN_POSITIVE);
        if !rel.is_finite() || rel > 0.1 {
            diagnostics.push(Diagnostic {
                code: SCALE_UNCERTAINTY.to_string(),
                severity: Severity::Warning,
                message: format!("背景缩放因子 α={:.4} 的相对不确定度 {:.1}% 偏大（阈值 10%），背景缩放不确定度已并入每个点的总误差带", alpha, rel * 100.0),
                refs: vec![sub_node.to_string()],
            });
        }
    }

    let alpha = scale_fit.as_ref().map(|f| f.scale).unwrap_or(1.0);
    let alpha_sigma = scale_fit.as_ref().map(|f| f.scale_sigma).unwrap_or(0.0);

    let mut points = Vec::new();
    let mut negative_refs = Vec::new();
    let scale_rel_max = scale_fit.as_ref().map(|f| f.scale_sigma / f.scale.abs().max(f64::MIN_POSITIVE)).filter(|v| v.is_finite()).unwrap_or(0.0);
    for (k, m) in mapped.iter().enumerate() {
        let s = &sp[m.sample_idx];
        let i = s.i - alpha * m.ib;
        let sigma = (s.sigma.powi(2) + (alpha * m.sb).powi(2) + (m.ib * alpha_sigma).powi(2)).sqrt();
        if i + sigma < 0.0 { negative_refs.push((k, s.q, i + sigma)); }
        let mut sources = vec![s.point_id.clone()];
        sources.extend(m.buffer_sources.iter().cloned());
        points.push(Point { point_id: ids::point(target_version_id, k), q: s.q, i, sigma, interpolated: m.interpolated, sources });
    }

    if overlap.is_none() {
        diagnostics.push(Diagnostic {
            code: NO_OVERLAP.to_string(),
            severity: Severity::Error,
            message: "样品与缓冲液 q 网格无重叠范围：不执行任何相减，不生成扣背景点，后续区间模型无输入".to_string(),
            refs: vec![ids::node_normalized(&sample.material_id), ids::node_normalized(&buffer.material_id)],
        });
        points.clear();
        negative_refs.clear();
    }

    for (k, q, bound) in &negative_refs {
        diagnostics.push(Diagnostic {
            code: NEGATIVE_INTENSITY.to_string(),
            severity: Severity::Warning,
            message: format!("扣背景点 q={:.5} 处 I+σ={:.4e}<0：点保留（不裁剪、不取零），该点不进入 Guinier/Porod 拟合候选", q, bound),
            refs: vec![ids::point(target_version_id, *k)],
        });
    }

    let subtraction = Subtraction {
        sample_node: ids::node_normalized(&sample.material_id),
        buffer_node: ids::node_normalized(&buffer.material_id),
        sample_mat_rev: sample.material_rev,
        buffer_mat_rev: buffer.material_rev,
        overlap_range_q: overlap_arr,
        scale_fit,
        status: if overlap.is_none() { SubtractionStatus::NoOverlap } else { SubtractionStatus::Ok },
        negative_points: negative_refs.iter().map(|(k, _, _)| ids::point(target_version_id, *k)).collect(),
        scale_uncertainty_rel_max: scale_rel_max,
        points,
    };
    SubtractResult { subtraction, diagnostics }
}
