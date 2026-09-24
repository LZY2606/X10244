//! Canonical curve representation and exposure/transmission normalization.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QPoint {
    /// Stable 1-based ordinal from the imported 2D-integration table.
    pub ordinal: u32,
    /// Canonical q in nm^-1. NaN when the source value was not convertible;
    /// such points stay in the record with diagnostics and never enter math.
    pub q: f64,
    pub i: f64,
    pub sig: f64,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Curve {
    pub points: Vec<QPoint>,
    pub q_unit: String,
    pub i_unit: String,
}

impl Curve {
    pub fn math_points(&self) -> impl Iterator<Item = &QPoint> {
        self.points.iter().filter(|p| finite_all(p))
    }

    pub fn q_min(&self) -> Option<f64> {
        self.math_points().map(|p| p.q).fold(None, |a, q| Some(a.map_or(q, |x: f64| x.min(q))))
    }

    pub fn q_max(&self) -> Option<f64> {
        self.math_points().map(|p| p.q).fold(None, |a, q| Some(a.map_or(q, |x: f64| x.max(q))))
    }
}

pub fn finite_all(p: &QPoint) -> bool {
    p.q.is_finite() && p.i.is_finite() && p.sig.is_finite() && p.sig >= 0.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalInputs {
    pub exposure_s: f64,
    pub transmission: f64,
    pub transmission_sigma: f64,
    pub thickness_cm: Option<f64>,
    pub thickness_sigma: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedPoint {
    pub ordinal: u32,
    pub q: f64,
    /// I / (t * T)
    pub i_norm: f64,
    pub sigma_norm: f64,
    /// Fractional variance contribution from transmission uncertainty.
    pub t_fraction: f64,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedCurve {
    pub points: Vec<NormalizedPoint>,
    pub q_unit: String,
    pub i_unit: String,
    pub diagnostics: Vec<String>,
}

/// Per-point normalization by exposure and transmission with propagated
/// uncertainty. Thickness is intentionally NOT folded in here: the review
/// chain wants exposure/transmission normalization and (optional) thickness
/// scaling to remain independently inspectable steps.
///
/// sigma^2 = (I/(tT))^2 * ((sig_I/I)^2 + (sig_T/T)^2)
pub fn normalize(curve: &Curve, inp: &NormalInputs) -> NormalizedCurve {
    let mut diagnostics = Vec::new();
    if !(inp.exposure_s.is_finite() && inp.exposure_s > 0.0) {
        diagnostics.push(format!("non-positive exposure {} s; normalization undefined", inp.exposure_s));
    }
    if !(inp.transmission.is_finite() && inp.transmission > 0.0) {
        diagnostics.push(format!("non-positive transmission {}; normalization undefined", inp.transmission));
    }
    let mut points = Vec::with_capacity(curve.points.len());
    let defined = inp.exposure_s > 0.0 && inp.transmission > 0.0;
    for p in curve.points.iter() {
        let mut flags = p.flags.clone();
        if !finite_all(p) {
            flags.push("excluded:non-finite-source".to_string());
            points.push(NormalizedPoint {
                ordinal: p.ordinal,
                q: p.q,
                i_norm: f64::NAN,
                sigma_norm: f64::NAN,
                t_fraction: f64::NAN,
                flags,
            });
            continue;
        }
        if !defined {
            flags.push("excluded:normalization-undefined".to_string());
            points.push(NormalizedPoint {
                ordinal: p.ordinal,
                q: p.q,
                i_norm: f64::NAN,
                sigma_norm: f64::NAN,
                t_fraction: f64::NAN,
                flags,
            });
            continue;
        }
        let scale = inp.exposure_s * inp.transmission;
        let i_norm = p.i / scale;
        let rel_i = if p.i != 0.0 { p.sig / p.i.abs() } else { 0.0 };
        let rel_t = if inp.transmission != 0.0 {
            inp.transmission_sigma / inp.transmission
        } else {
            0.0
        };
        let rel2 = rel_i * rel_i + rel_t * rel_t;
        let sigma_norm = i_norm.abs() * rel2.sqrt();
        if p.i < 0.0 {
            flags.push("negative-intensity".to_string());
        }
        points.push(NormalizedPoint {
            ordinal: p.ordinal,
            q: p.q,
            i_norm,
            sigma_norm,
            t_fraction: if rel2 > 0.0 { (rel_t * rel_t) / rel2 } else { 0.0 },
            flags,
        });
    }
    if points.iter().any(|p| p.flags.iter().any(|f| f == "negative-intensity")) {
        diagnostics.push("one or more raw intensities are negative; values retained, not clipped".to_string());
    }
    NormalizedCurve { points, q_unit: curve.q_unit.clone(), i_unit: curve.i_unit.clone(), diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(pts: &[(f64, f64, f64)]) -> Curve {
        Curve {
            q_unit: "nm^-1".into(),
            i_unit: "a.u.".into(),
            points: pts
                .iter()
                .enumerate()
                .map(|(i, &(q, ii, s))| QPoint { ordinal: i as u32 + 1, q, i: ii, sig: s, flags: vec![] })
                .collect(),
        }
    }

    #[test]
    fn divides_by_exposure_and_transmission() {
        let c = curve(&[(0.1, 100.0, 2.0), (0.2, 50.0, 1.0)]);
        let out = normalize(&c, &NormalInputs { exposure_s: 10.0, transmission: 0.5, transmission_sigma: 0.0, thickness_cm: None, thickness_sigma: 0.0 });
        assert!((out.points[0].i_norm - 20.0).abs() < 1e-12);
        assert!((out.points[1].i_norm - 10.0).abs() < 1e-12);
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn propagates_transmission_uncertainty() {
        let c = curve(&[(0.1, 100.0, 0.0)]);
        let out = normalize(&c, &NormalInputs { exposure_s: 1.0, transmission: 0.5, transmission_sigma: 0.05, thickness_cm: None, thickness_sigma: 0.0 });
        assert!((out.points[0].sigma_norm - 20.0).abs() < 1e-12);
        assert!(out.points[0].t_fraction > 0.999);
    }

    #[test]
    fn bad_exposure_excludes_points_without_dropping_them() {
        let c = curve(&[(0.1, 100.0, 2.0)]);
        let out = normalize(&c, &NormalInputs { exposure_s: 0.0, transmission: 0.5, transmission_sigma: 0.0, thickness_cm: None, thickness_sigma: 0.0 });
        assert!(out.points[0].i_norm.is_nan());
        assert!(out.points[0].flags.iter().any(|f| f.contains("undefined")));
        assert_eq!(out.points.len(), 1);
    }

    #[test]
    fn negative_intensity_is_flagged_not_clipped() {
        let c = curve(&[(0.1, -5.0, 1.0)]);
        let out = normalize(&c, &NormalInputs { exposure_s: 1.0, transmission: 1.0, transmission_sigma: 0.0, thickness_cm: None, thickness_sigma: 0.0 });
        assert!(out.points[0].i_norm < 0.0);
        assert!(out.points[0].flags.iter().any(|f| f == "negative-intensity"));
    }
}
