use super::types::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubtractionParams {
    pub scale: f64,
    pub scale_rel_uncertainty: f64,
}

impl Default for SubtractionParams {
    fn default() -> Self {
        SubtractionParams {
            scale: 1.0,
            scale_rel_uncertainty: 0.02,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubtractedCurve {
    pub id: String,
    pub buffer_id: String,
    pub buffer_version: String,
    pub curve: Curve,
    pub residual_metric: f64,
    pub dropped_outside_overlap: usize,
    pub negative_count: usize,
    pub grid_mismatch: bool,
    pub overlap: Option<[f64; 2]>,
    pub failed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubtractionOutput {
    pub rule_version: String,
    pub params: SubtractionParams,
    pub candidates: Vec<SubtractedCurve>,
    pub adopted_buffer_id: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

fn grids_equal(a: &Curve, b: &Curve) -> bool {
    a.points.len() == b.points.len()
        && a
            .points
            .iter()
            .zip(&b.points)
            .all(|(x, y)| (x.q - y.q).abs() < 1e-12)
}

fn interpolate_at(q: f64, curve: &Curve) -> Option<(f64, f64)> {
    let pts = &curve.points;
    if pts.is_empty() || q < pts.first().unwrap().q - 1e-12 || q > pts.last().unwrap().q + 1e-12 {
        return None;
    }
    match pts.binary_search_by(|p| p.q.partial_cmp(&q).unwrap()) {
        Ok(idx) => Some((pts[idx].i, pts[idx].sigma)),
        Err(insert) => {
            if insert == 0 || insert >= pts.len() {
                return None;
            }
            let p0 = &pts[insert - 1];
            let p1 = &pts[insert];
            let f = (q - p0.q) / (p1.q - p0.q);
            Some((p0.i + f * (p1.i - p0.i), p0.sigma + f * (p1.sigma - p0.sigma)))
        }
    }
}

pub fn subtract(
    sample: &Curve,
    buffers: &[super::normalize::NormalizedBuffer],
    adopted_buffer_id: Option<&str>,
    params: &SubtractionParams,
    node_id: &str,
) -> SubtractionOutput {
    let mut diagnostics = vec![Diagnostic::new(
        D_SCALE_UNCERTAINTY,
        "info",
        format!(
            "背景缩放 alpha = {:.3} ± {:.1}%，缩放不确定度已计入相减误差",
            params.scale,
            params.scale_rel_uncertainty * 100.0
        ),
    )
    .context(serde_json::json!({
        "alpha": params.scale,
        "alpha_rel_uncertainty": params.scale_rel_uncertainty,
    }))];

    let mut candidates = Vec::new();
    for b in buffers {
        let cand = subtract_pair(sample, b, params, &format!("{node_id}:{}", b.id), &mut diagnostics);
        candidates.push(cand);
    }

    if candidates.len() > 1 {
        diagnostics.push(
            Diagnostic::new(
                D_MULTI_CANDIDATE,
                "info",
                "存在多个缓冲液候选，保留全部相减结果与残差指标，由审阅者裁定，不自动覆盖",
            )
            .context(serde_json::json!({
                "buffer_ids": buffers.iter().map(|b| b.id.clone()).collect::<Vec<_>>(),
            })),
        );
    }

    let adopted_buffer_id = adopted_buffer_id
        .filter(|id| candidates.iter().any(|c| &c.buffer_id == id))
        .or_else(|| candidates.first().map(|c| c.buffer_id.clone()));

    SubtractionOutput {
        rule_version: RULE_VERSION_SUBTRACT.to_string(),
        params: params.clone(),
        candidates,
        adopted_buffer_id,
        diagnostics,
    }
}

fn subtract_pair(
    sample: &Curve,
    buffer: &super::normalize::NormalizedBuffer,
    params: &SubtractionParams,
    cand_id: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> SubtractedCurve {
    let bcurve = &buffer.curve;
    let grid_mismatch = !grids_equal(sample, bcurve);
    if grid_mismatch {
        diagnostics.push(
            Diagnostic::new(
                D_QGRID_MISMATCH,
                "warning",
                format!(
                    "样品与 {} 的 q 网格不一致：不直接相减，仅在重叠范围对缓冲液线性插值",
                    buffer.label
                ),
            )
            .context(serde_json::json!({
                "buffer_id": buffer.id,
                "sample_n": sample.points.len(),
                "buffer_n": bcurve.points.len(),
            })),
        );
    }

    let q_lo = sample
        .points
        .first()
        .map(|p| p.q)
        .unwrap()
        .max(bcurve.points.first().map(|p| p.q).unwrap_or(f64::NAN));
    let q_hi = sample
        .points
        .last()
        .map(|p| p.q)
        .unwrap()
        .min(bcurve.points.last().map(|p| p.q).unwrap_or(f64::NAN));

    if q_lo > q_hi || q_lo.is_nan() || q_hi.is_nan() {
        diagnostics.push(
            Diagnostic::new(
                D_NO_OVERLAP,
                "error",
                format!(
                    "样品与 {} 的 q 范围无重叠：不生成相减曲线，保留输入片段与失败状态",
                    buffer.label
                ),
            )
            .context(serde_json::json!({
                "buffer_id": buffer.id,
            })),
        );
        return SubtractedCurve {
            id: cand_id.to_string(),
            buffer_id: buffer.id.clone(),
            buffer_version: buffer.buffer_version.clone(),
            curve: Curve::new(cand_id, format!("相减失败 ({})", buffer.label), "nm^-1", "cm^-1"),
            residual_metric: f64::NAN,
            dropped_outside_overlap: sample.points.len(),
            negative_count: 0,
            grid_mismatch,
            overlap: None,
            failed: true,
        };
    }

    let mut out = Curve::new(
        cand_id,
        format!("样品 - {}", buffer.label),
        "nm^-1",
        "cm^-1",
    );
    let mut dropped = 0usize;
    let mut negative_count = 0usize;
    let mut interp_used = false;
    for sp in &sample.points {
        let Some((bi, bs)) = interpolate_at(sp.q, bcurve) else {
            dropped += 1;
            continue;
        };
        if grid_mismatch {
            interp_used = true;
        }
        let i_sub = sp.i - params.scale * bi;
        let sigma = (sp.sigma.powi(2)
            + (params.scale * bs).powi(2)
            + (bi * params.scale * params.scale_rel_uncertainty).powi(2))
        .sqrt();
        if i_sub < 0.0 {
            negative_count += 1;
        }
        out.push(sp.q, i_sub, sigma);
    }
    if dropped > 0 {
        diagnostics.push(
            Diagnostic::new(
                D_POINT_OUTSIDE_OVERLAP,
                "warning",
                format!("{}：{dropped} 个样品点落在重叠范围之外，被排除而非外推", buffer.label),
            )
            .context(serde_json::json!({
                "buffer_id": buffer.id,
                "dropped": dropped,
                "overlap": [q_lo, q_hi],
            })),
        );
    }
    if interp_used {
        diagnostics.push(
            Diagnostic::new(
                D_INTERPOLATION_APPLIED,
                "info",
                format!("{}：已在重叠范围 [{q_lo:.4}, {q_hi:.4}] nm^-1 内线性插值缓冲液", buffer.label),
            )
            .context(serde_json::json!({
                "buffer_id": buffer.id,
                "overlap": [q_lo, q_hi],
                "interpolated_points": out.points.len(),
            })),
        );
    }
    if negative_count > 0 {
        diagnostics.push(
            Diagnostic::new(
                D_NEGATIVE_INTENSITY,
                "warning",
                format!(
                    "{}：{negative_count} 个相减点强度为负，保留并标记，不截断、不强制归零",
                    buffer.label
                ),
            )
            .context(serde_json::json!({
                "buffer_id": buffer.id,
                "negative_count": negative_count,
            })),
        );
    }

    let tail_n = (out.points.len() / 5).max(3);
    let residual_metric = out.points[out.points.len() - tail_n..]
        .iter()
        .map(|p| p.i.abs())
        .sum::<f64>()
        / tail_n as f64;

    SubtractedCurve {
        id: cand_id.to_string(),
        buffer_id: buffer.id.clone(),
        buffer_version: buffer.buffer_version.clone(),
        curve: out,
        residual_metric,
        dropped_outside_overlap: dropped,
        negative_count,
        grid_mismatch,
        overlap: Some([q_lo, q_hi]),
        failed: false,
    }
}
