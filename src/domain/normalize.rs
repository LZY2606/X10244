use super::types::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormalizedBuffer {
    pub id: String,
    pub label: String,
    pub buffer_version: String,
    pub curve: Curve,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormalizedOutput {
    pub rule_version: String,
    pub sample: Curve,
    pub buffers: Vec<NormalizedBuffer>,
    pub scale_factor: f64,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormalizeParams {
    pub norm_rel_uncertainty: f64,
}

impl Default for NormalizeParams {
    fn default() -> Self {
        NormalizeParams {
            norm_rel_uncertainty: 0.01,
        }
    }
}

pub fn normalize(
    exposure: &ExposureRecord,
    node_id: &str,
    params: &NormalizeParams,
) -> NormalizedOutput {
    let mut diagnostics = Vec::new();
    let thickness = exposure.sample_thickness.value;
    let transmission = exposure.transmission.value;
    let exposure_time = exposure.exposure_time.value;

    let scale = match (thickness, transmission, exposure_time) {
        (Some(d), Some(t), Some(te)) if d > 0.0 && t > 0.0 && te > 0.0 => 1.0 / (d * t * te),
        _ => f64::NAN,
    };
    if !scale.is_finite() || scale <= 0.0 {
        diagnostics.push(Diagnostic::new(
            D_UNIT_UNCONVERTIBLE,
            "error",
            "厚度/透射率/曝光时间缺失、不可转换或非正，无法归一化；原始值保留在导入记录中",
        ));
    }
    diagnostics.extend(
        [
            &exposure.sample_thickness.diagnostic,
            &exposure.transmission.diagnostic,
            &exposure.exposure_time.diagnostic,
        ]
        .into_iter()
        .flatten()
        .cloned(),
    );
    diagnostics.extend(exposure.diagnostics.iter().cloned());

    if params.norm_rel_uncertainty > 0.0 {
        diagnostics.push(
            Diagnostic::new(
                D_NORMALIZATION_UNCERTAINTY,
                "info",
                format!(
                    "归一化相对不确定度 {:.1}% 已计入每个点的误差带",
                    params.norm_rel_uncertainty * 100.0
                ),
            )
            .context(serde_json::json!({
                "relative_uncertainty": params.norm_rel_uncertainty,
            })),
        );
    }

    let mut sample = Curve::new(
        format!("{node_id}:sample"),
        format!("{} (归一化)", exposure.sample_curve.label),
        "nm^-1",
        "cm^-1",
    );
    for p in &exposure.sample_curve.points {
        let sigma = (p.sigma * scale).hypot(p.i * scale * params.norm_rel_uncertainty);
        sample.push(p.q, p.i * scale, sigma);
    }

    let mut buffers = Vec::new();
    for b in &exposure.buffer_candidates {
        let mut curve = Curve::new(
            format!("{node_id}:buf:{}", b.id),
            format!("{} (归一化)", b.label),
            "nm^-1",
            "cm^-1",
        );
        for p in &b.curve.points {
            let sigma = (p.sigma * scale).hypot(p.i * scale * params.norm_rel_uncertainty);
            curve.push(p.q, p.i * scale, sigma);
        }
        buffers.push(NormalizedBuffer {
            id: b.id.clone(),
            label: b.label.clone(),
            buffer_version: b.buffer_version.clone(),
            curve,
        });
    }

    NormalizedOutput {
        rule_version: RULE_VERSION_NORMALIZE.to_string(),
        sample,
        buffers,
        scale_factor: scale,
        diagnostics,
    }
}
