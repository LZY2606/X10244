//! Absolute normalization.
//!
//! normalized I = raw I * response_scale / (exposure * thickness * transmission)
//!
//! Uncertainty includes the normalization uncertainty: relative variances from
//! transmission and the instrument response scale are propagated (independent
//! terms). The counting sigma is divided by the same normalization factor.

use crate::ids::content_id;
use crate::model::*;
use crate::quantity::{to_base, QuantityKind};

pub struct NormFactors {
    pub factor: f64,
    pub norm_rel_var: f64,
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub transmission: f64,
    pub response_scale: f64,
}

pub fn norm_factors(bundle: &RawBundle) -> NormFactors {
    let exposure_s = to_base(&bundle.exposure, QuantityKind::Exposure).map(|c| c.value).unwrap_or(f64::NAN);
    let thickness_mm =
        to_base(&bundle.thickness, QuantityKind::Thickness).map(|c| c.value).unwrap_or(f64::NAN);
    let transmission = bundle.transmission.value;
    let response_scale = if bundle.instrument.present { bundle.instrument.scale } else { 1.0 };

    let factor = response_scale / (exposure_s * thickness_mm * transmission);

    let mut norm_rel_var = 0.0;
    norm_rel_var += bundle.transmission.rel_sigma.powi(2);
    if bundle.instrument.present {
        norm_rel_var += bundle.instrument.scale_rel_sigma.powi(2);
    }

    NormFactors { factor, norm_rel_var, exposure_s, thickness_mm, transmission, response_scale }
}

pub fn normalize(bundle: &RawBundle) -> NormalizedCurve {
    let nf = norm_factors(bundle);
    let mut diagnostics = bundle.diagnostics.clone();

    if !nf.factor.is_finite() || nf.factor <= 0.0 {
        diagnostics.push("normalization factor invalid; curve cannot be produced".to_string());
    }

    let mut points = Vec::new();
    for p in bundle.points.iter() {
        let active = p.convertible && !p.masked;
        let (q, i_raw, sigma_raw) = match (p.q, p.i, p.sigma) {
            (Some(q), Some(i), s) if active && nf.factor.is_finite() => {
                (q, i, s.map(|v| v).unwrap_or(0.0).max(0.0))
            }
            _ => {
                // Keep masked/unconvertible points visible but do not normalize them.
                if let (Some(q), Some(i)) = (p.q, p.i) {
                    points.push(NormalizedPoint {
                        id: content_id("npt", &serde_json::json!({"b": bundle.id, "p": p.id})),
                        source_point: p.id,
                        q,
                        intensity: i,
                        sigma: p.sigma.unwrap_or(0.0).max(0.0),
                        norm_rel_var: 0.0,
                        norm_factor: f64::NAN,
                        masked: true,
                    });
                }
                continue;
            }
        };

        let intensity = i_raw * nf.factor;
        // absolute sigma combines counting and normalization variance
        let counting_var = (sigma_raw * nf.factor).powi(2);
        let norm_var = intensity.powi(2) * nf.norm_rel_var;
        let sigma = (counting_var + norm_var).sqrt();

        points.push(NormalizedPoint {
            id: content_id("npt", &serde_json::json!({"b": bundle.id, "p": p.id})),
            source_point: p.id,
            q,
            intensity,
            sigma,
            norm_rel_var: nf.norm_rel_var,
            norm_factor: nf.factor,
            masked: false,
        });
    }

    let id = content_id("ncurve", &serde_json::json!({"b": bundle.id}));
    NormalizedCurve {
        id,
        source_bundle: bundle.id,
        role: bundle.role,
        points,
        q_unit: "1/A".to_string(),
        i_unit: if bundle.instrument.present { "1/cm (abs)" } else { "normalized detector units" }.to_string(),
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{IngestInput, RawRow};
    use crate::quantity::Unit;

    fn bundle(present: bool, t_sigma: f64) -> RawBundle {
        ingest(IngestInput {
            label: "s".into(),
            role: Role::Sample,
            version: "v1".into(),
            rows: vec![RawRow {
                q: "0.01".into(),
                i: "100".into(),
                sigma: "10".into(),
                q_unit: "1/A".into(),
                i_unit: "1/cm".into(),
                masked: false,
            }],
            exposure: Quantity::new(10.0, "10", Unit::Second),
            thickness: Quantity::new(1.0, "1", Unit::Mm),
            transmission: Transmission { value: 0.5, rel_sigma: t_sigma, raw: "0.5".into() },
            instrument: InstrumentResponse {
                name: "cal".into(),
                source: "cal.yaml".into(),
                scale: 1.0,
                scale_rel_sigma: 0.0,
                present,
                raw_token: "1.0".into(),
            },
            mask: MaskSource { name: "m".into(), source: "m.mask".into(), raw_token: "m".into() },
            source_text: "0.01 100 10".into(),
        })
    }

    #[test]
    fn normalizes_by_exposure_thickness_transmission() {
        let c = normalize(&bundle(false, 0.0));
        let p = &c.points[0];
        // 100 / (10 * 1 * 0.5) = 20
        assert!((p.intensity - 20.0).abs() < 1e-9);
        // 10 / 5 = 2, no normalization uncertainty
        assert!((p.sigma - 2.0).abs() < 1e-9);
    }

    #[test]
    fn propagates_transmission_uncertainty() {
        let c = normalize(&bundle(false, 0.02));
        let p = &c.points[0];
        let expected = ((2.0_f64).powi(2) + (20.0 * 0.02).powi(2)).sqrt();
        assert!((p.sigma - expected).abs() < 1e-9, "{} vs {}", p.sigma, expected);
    }
}
