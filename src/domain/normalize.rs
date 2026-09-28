//! Detector -> absolute-ish normalization.
//!
//! I_norm(q) = I_2d(q) / (t * d * Tr)
//! Error budget (1 sigma) combines the counting error with the propagated
//! normalization uncertainty from exposure / thickness / transmission:
//!     (sigma_I / I)^2 + (sigma_t/t)^2 + (sigma_d/d)^2 + (sigma_Tr/Tr)^2
//! No absolute calibration constant is assumed; the intensity basis is kept.

use crate::domain::identity::content_id;
use crate::domain::types::{
    Diagnostic, Material, NormalizedPoint, NormalizedResult, RULE_VERSION,
};

/// Relative 1-sigma uncertainties assumed for instrumented quantities.
pub const EXPOSURE_REL_UNC: f64 = 0.0005;
pub const THICKNESS_REL_UNC: f64 = 0.01;
pub const TRANSMISSION_REL_UNC: f64 = 0.005;

pub fn normalize(mat: &Material) -> NormalizedResult {
    let mut diagnostics = Vec::new();
    let factor_parts: Vec<f64> = [
        mat.exposure.canonical,
        mat.thickness.canonical,
        mat.transmission.canonical,
    ]
    .into_iter()
    .flatten()
    .collect();

    if mat.exposure.canonical.is_none() {
        diagnostics.push(Diagnostic {
            code: "unconvertible_exposure".into(),
            severity: "error".into(),
            message: format!("exposure left as '{}': {}", mat.exposure.raw,
                mat.exposure.diagnostic.clone().unwrap_or_default()),
            point_id: None,
        });
    }
    if mat.thickness.canonical.is_none() {
        diagnostics.push(Diagnostic {
            code: "unconvertible_thickness".into(),
            severity: "error".into(),
            message: format!("thickness left as '{}': {}", mat.thickness.raw,
                mat.thickness.diagnostic.clone().unwrap_or_default()),
            point_id: None,
        });
    }
    if mat.transmission.canonical.is_none() {
        diagnostics.push(Diagnostic {
            code: "unconvertible_transmission".into(),
            severity: "error".into(),
            message: format!("transmission left as '{}': {}", mat.transmission.raw,
                mat.transmission.diagnostic.clone().unwrap_or_default()),
            point_id: None,
        });
    }

    let usable = factor_parts.len() == 3
        && factor_parts.iter().all(|v| v.is_finite() && *v > 0.0);
    let factor = if usable {
        Some(factor_parts.iter().product())
    } else {
        None
    };

    let norm_unc_rel = (EXPOSURE_REL_UNC.powi(2)
        + THICKNESS_REL_UNC.powi(2)
        + TRANSMISSION_REL_UNC.powi(2))
    .sqrt();

    let mut points = Vec::new();
    for p in &mat.points {
        let q = p.q.canonical;
        let i = p.i.canonical;
        let s = p.sigma.canonical;
        match (q, i, s, factor) {
            (Some(qv), Some(iv), Some(sv), Some(c)) => {
                let inorm = iv / c;
                let sigma_count = sv / c;
                let sigma_norm = inorm.abs() * norm_unc_rel;
                let sigma = (sigma_count.powi(2) + sigma_norm.powi(2)).sqrt();
                points.push(NormalizedPoint {
                    point_id: p.point_id.clone(),
                    source_point_id: p.point_id.clone(),
                    q: qv,
                    q_unit: p.q.canonical_unit.clone(),
                    i: inorm,
                    i_unit: p.i.canonical_unit.clone(),
                    sigma,
                    norm_uncertainty: sigma_norm,
                });
            }
            _ => {
                diagnostics.push(Diagnostic {
                    code: "point_not_normalized".into(),
                    severity: "warning".into(),
                    message: "point retained in raw material but excluded from normalization"
                        .into(),
                    point_id: Some(p.point_id.clone()),
                });
            }
        }
    }

    let mut result = NormalizedResult {
        id: String::new(),
        material_id: mat.id.clone(),
        rule_version: RULE_VERSION.into(),
        norm_factor: factor,
        points,
        diagnostics,
    };
    result.id = content_id("normalized", &serde_json::to_value(&result).unwrap());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::units::{parse_exposure, parse_intensity, parse_q, parse_thickness, parse_transmission};
    use crate::domain::types::{MaterialKind, RawPoint};

    fn sample_mat(thickness_raw: &str) -> Material {
        Material {
            id: "m".into(),
            logical_id: "l".into(),
            version: "v0".into(),
            kind: MaterialKind::Sample,
            points: vec![RawPoint {
                point_id: "m:v0:p3".into(),
                q: parse_q("0.05", "1/A"),
                i: parse_intensity("0.001", "cm^-1"),
                sigma: parse_intensity("0.00002", "cm^-1"),
            }],
            exposure: parse_exposure("1 s"),
            thickness: parse_thickness(thickness_raw),
            transmission: parse_transmission("0.85"),
            scale_uncertainty: None,
            instrument_source: "instrument_v1.json".into(),
            mask_source: "mask_v1.json".into(),
            source_files: vec!["x.csv".into()],
            note: None,
        }
    }

    #[test]
    fn normalization_divides_by_geometry() {
        let r = normalize(&sample_mat("0.1 cm"));
        assert!(r.norm_factor.is_some());
        let c = 1.0 * 0.1 * 0.85;
        assert!((r.points[0].i - 0.001 / c).abs() < 1e-15);
        assert!(r.points[0].norm_uncertainty > 0.0);
        assert!(r.points[0].sigma > 0.00002 / c);
    }

    #[test]
    fn unconvertible_thickness_blocks_normalization_not_import() {
        let r = normalize(&sample_mat("unknown"));
        assert!(r.norm_factor.is_none());
        assert!(r.points.is_empty());
        assert!(r
            .diagnostics
            .iter()
            .any(|d| d.code == "unconvertible_thickness"));
    }
}
