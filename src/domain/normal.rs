//! Normalization of an integrated curve to absolute-like scale.
//!
//! ```text
//! I_n(q) = I_raw(q) / K,   K = t_exp * T * d
//! ```
//!
//! Propagated variance (independent inputs):
//! ```text
//! sigma_n^2 = (sigma_raw / K)^2
//!           + I_n^2 * [(sigma_t/t)^2 + (sigma_T/T)^2 + (sigma_d/d)^2]
//! ```
//! The second term is the *normalization uncertainty* and is kept separate in
//! `PointErr` so the background step can include it explicitly.

use super::*;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NormalParams {
    pub exposure_s: f64,
    pub sigma_exposure_s: Option<f64>,
    pub transmission: f64,
    pub sigma_transmission: Option<f64>,
    pub thickness_cm: f64,
    pub sigma_thickness_cm: Option<f64>,
    /// Beamstop / fitted lower-q cutoff. Points below are stored but masked.
    pub q_min_fit: f64,
}

impl NormalParams {
    pub fn factor(&self) -> f64 {
        self.exposure_s * self.transmission * self.thickness_cm
    }

    /// Relative normalization variance `(sigma_K/K)^2`.
    pub fn rel_variance(&self) -> f64 {
        let term = |v: f64, s: Option<f64>| s.map(|sv| (sv / v).powi(2)).unwrap_or(0.0);
        term(self.exposure_s, self.sigma_exposure_s)
            + term(self.transmission, self.sigma_transmission)
            + term(self.thickness_cm, self.sigma_thickness_cm)
    }
}

/// Per-point variance split, so every downstream number can show where its
/// uncertainty came from.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PointErr {
    /// Statistical term from the detector counts.
    pub stat: f64,
    /// Relative normalization variance times I^2 (absolute variance).
    pub norm: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NormalPoint {
    pub i: usize,
    pub q: f64,
    pub intensity: f64,
    pub stat: f64,
    pub norm_var: f64,
    /// Total sigma = sqrt(stat^2 + norm_var).
    pub sigma: f64,
    pub masked: bool,
    /// Negative normalized intensities are kept, never clipped.
    pub negative: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NormalResult {
    pub rule_version: String,
    pub factor: f64,
    pub rel_norm_variance: f64,
    pub output_unit: String,
    pub points: Vec<NormalPoint>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn normalize(raw: &Curve, params: &NormalParams) -> NormalResult {
    let mut diagnostics = raw.validate_grid();
    diagnostics.extend(validate_norm_inputs(
        params.exposure_s,
        params.thickness_cm,
        params.transmission,
    ));

    let factor = params.factor();
    let rel_var = params.rel_variance();
    let usable = factor.is_finite() && factor > 0.0;

    let mut points = Vec::with_capacity(raw.points.len());
    let mut neg_count = 0usize;
    for p in &raw.points {
        let (intensity, stat, norm_var, sigma) = if usable {
            let intensity = p.intensity / factor;
            let stat = p.sigma / factor;
            let norm_var = intensity.powi(2) * rel_var;
            let sigma = (stat.powi(2) + norm_var).sqrt();
            (intensity, stat, norm_var, sigma)
        } else {
            // Keep the point in place with a diagnostic instead of dropping it;
            // it cannot be plotted as a derived number.
            (f64::NAN, f64::NAN, f64::NAN, f64::NAN)
        };
        let negative = intensity.is_finite() && intensity < 0.0;
        if negative {
            neg_count += 1;
        }
        if p.masked || (p.q.is_finite() && p.q < params.q_min_fit) {
            diagnostics.push(
                Diagnostic::info("FIT_MASKED", "point outside usable q range; retained but masked")
                    .with_target(p.i.to_string()),
            );
        }
        points.push(NormalPoint {
            i: p.i,
            q: p.q,
            intensity,
            stat,
            norm_var,
            sigma,
            masked: p.masked || p.q < params.q_min_fit,
            negative,
        });
    }

    if neg_count > 0 {
        diagnostics.push(Diagnostic::warn(
            "NEGATIVE_INTENSITY",
            format!("{neg_count} normalized points are negative; retained without clipping"),
        ));
    }
    if !usable {
        diagnostics.push(Diagnostic::err(
            "NORMALIZATION_FACTOR_INVALID",
            "normalization factor is not a positive finite number; derivation failed",
        ));
    }

    NormalResult {
        rule_version: RULE_VERSION.to_string(),
        factor,
        rel_norm_variance: rel_var,
        output_unit: match raw.i_unit {
            IUnit::Arbitrary => "normalized arb. units".to_string(),
            IUnit::PerCm => "1/cm".to_string(),
        },
        points,
        diagnostics,
    }
}
