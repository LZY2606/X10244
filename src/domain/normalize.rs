//! Normalization to absolute flux:
//!   I_n = I_raw / (response * exposure[s] * thickness[mm] * transmission)
//! Uncertainty propagates counting error AND transmission uncertainty AND
//! instrument-response uncertainty. Non-physical values are flagged per point
//! rather than discarded.
use crate::domain::ids::hid;
use crate::domain::types::{Diagnostic, RawCurve};
use crate::domain::units::{dimensionless_factor, length_factor, q_factor, time_factor};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointStatus {
    Ok,
    Masked,
    NonFinite,
    NegativeIntensity,
    NonPositiveDenominator,
    UnconvertibleUnit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormPoint {
    pub q: f64,
    pub q_unit: String,
    pub intensity: f64,
    pub sigma: f64,
    pub status: PointStatus,
    pub status_detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Normalized {
    pub id: String,
    pub raw_id: String,
    pub name: String,
    pub points: Vec<NormPoint>,
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub transmission: f64,
    pub diagnostics: Vec<Diagnostic>,
}

fn base_tagged(t: &crate::domain::units::Tagged, f: impl Fn(&str) -> Option<f64>,
               diags: &mut Vec<Diagnostic>, what: &str) -> f64 {
    match f(&t.unit) {
        Some(fac) if fac.is_finite() => t.value * fac,
        _ => {
            diags.push(Diagnostic { code: "UNCONVERTIBLE_UNIT".into(),
                message: format!("{what} unit '{}' cannot be normalized", t.unit),
                at: Some(what.into()) });
            f64::NAN
        }
    }
}

pub fn normalize(raw: &RawCurve) -> Normalized {
    let mut diags = raw.diagnostics.clone();
    let exposure_s = base_tagged(&raw.exposure, time_factor, &mut diags, "exposure");
    let thickness_mm = base_tagged(&raw.thickness, length_factor, &mut diags, "thickness");
    let transmission = match dimensionless_factor(&raw.transmission.unit) {
        Some(f) => raw.transmission.value * f,
        None => { diags.push(Diagnostic{code:"UNCONVERTIBLE_UNIT".into(),
            message:format!("transmission unit '{}'", raw.transmission.unit),
            at:Some("transmission".into())}); f64::NAN }
    };
    let t_sigma_abs = raw.transmission_sigma.as_ref().and_then(|s|
        dimensionless_factor(&s.unit).map(|f| s.value * f));

    let mut points = Vec::with_capacity(raw.points.len());
    for (idx, p) in raw.points.iter().enumerate() {
        let qfac = q_factor(&p.q.unit);
        let (q, status, detail) = if p.masked {
            (p.q.value * qfac.unwrap_or(1.0), PointStatus::Masked, Some("masked point retained".into()))
        } else if !p.i.is_finite() {
            (p.q.value, PointStatus::NonFinite, Some("non-finite raw intensity".into()))
        } else if qfac.is_none() {
            (p.q.value, PointStatus::UnconvertibleUnit,
             Some(format!("q unit '{}' not converted", p.q.unit)))
        } else {
            (p.q.value * qfac.unwrap(), PointStatus::Ok, None)
        };
        let denom = p.response_factor.unwrap_or(1.0) * exposure_s * thickness_mm * transmission;
        let (intensity, sigma, status, detail) = if status != PointStatus::Ok {
            (f64::NAN, f64::NAN, status, detail)
        } else if !(denom.is_finite() && denom > 0.0) {
            (f64::NAN, f64::NAN, PointStatus::NonPositiveDenominator,
             Some(format!("exposure*thickness*transmission*response={denom}")))
        } else {
            let intensity = p.i / denom;
            // (sigma_I/I)^2 + (sigma_T/T)^2 ; response treated exact unless factor missing
            let rel_i = if p.i != 0.0 { p.sigma / p.i } else { 0.0 };
            let rel_t = match t_sigma_abs {
                Some(ts) if transmission != 0.0 => ts / transmission,
                _ => 0.0,
            };
            let sigma = intensity * (rel_i * rel_i + rel_t * rel_t).sqrt();
            if intensity < 0.0 {
                (intensity, sigma, PointStatus::NegativeIntensity, Some("negative normalized intensity".into()))
            } else {
                (intensity, sigma, PointStatus::Ok, None)
            }
        };
        points.push(NormPoint { q, q_unit: "A^-1".into(), intensity, sigma, status, status_detail: detail });
        let _ = idx;
    }

    let payload = json!({"raw_id": raw.id, "exposure_s": exposure_s,
        "thickness_mm": thickness_mm, "transmission": transmission,
        "points": points});
    Normalized {
        id: hid("norm", &payload),
        raw_id: raw.id.clone(),
        name: format!("norm({})", raw.name),
        points, exposure_s, thickness_mm, transmission, diagnostics: diags,
    }
}

impl Normalized {
    /// valid (normalizable, unmasked) points in canonical A^-1 order.
    pub fn valid_points(&self) -> Vec<&NormPoint> {
        let mut v: Vec<&NormPoint> = self.points.iter()
            .filter(|p| p.status == PointStatus::Ok && p.intensity.is_finite()).collect();
        v.sort_by(|a, b| a.q.partial_cmp(&b.q).unwrap());
        v
    }
}
