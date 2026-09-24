//! Source-layer types. The source layer keeps the *original representation*
//! verbatim (`original`); only the derived layer converts to canonical units.
use crate::domain::units::{q_factor, length_factor, time_factor, dimensionless_factor, QPoint, Tagged};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Sample,
    Buffer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    /// locator inside the original fragment, e.g. "points[3].q"
    pub at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub source: String,
    /// multiplicative detector response factors (I_true = I_meas / factor)
    pub factors: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskSource {
    pub source: String,
    pub masked: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawPoint {
    pub q: QPoint,
    pub i: f64,
    pub i_unit: String,
    pub sigma: f64,
    pub response_factor: Option<f64>,
    pub masked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCurve {
    /// content-addressed identity derived from the verbatim input fragment
    pub id: String,
    pub lineage: String,
    pub name: String,
    pub role: Role,
    pub points: Vec<RawPoint>,
    pub exposure: Tagged,
    pub thickness: Tagged,
    pub transmission: Tagged,
    pub transmission_sigma: Option<Tagged>,
    pub response: Option<InstrumentResponse>,
    pub mask: Option<MaskSource>,
    /// unconvertible / malformed values are kept in place (see `original`)
    /// and diagnosed here instead of being silently dropped for plotting
    pub diagnostics: Vec<Diagnostic>,
    pub revises: Option<String>,
    pub original: Value,
}

fn tagged(v: &Value, key: &str, diags: &mut Vec<Diagnostic>, at: &str,
         conv: impl Fn(&str) -> Option<f64>) -> Option<Tagged> {
    let node = v.get(key)?;
    match node {
        Value::Object(o) => {
            let num = o.get("value")?.as_f64()?;
            let unit = o.get("unit").and_then(|u| u.as_str()).unwrap_or("").to_string();
            match conv(&unit) {
                Some(_) => Some(Tagged { value: num, unit }),
                None => {
                    diags.push(Diagnostic {
                        code: "UNKNOWN_UNIT".into(),
                        message: format!("{key}={num} '{unit}' has no known conversion; value kept in place"),
                        at: Some(at.to_string()),
                    });
                    Some(Tagged { value: num, unit })
                }
            }
        }
        _ => {
            diags.push(Diagnostic { code: "BAD_TAGGED".into(),
                message: format!("{key} must be {{value,unit}}"), at: Some(at.into()) });
            None
        }
    }
}

/// Parse one import fragment. Parsing never destroys data: unconvertible
/// values remain in `original` and surface as diagnostics.
pub fn parse_raw_curve(input: Value, lineage: String, revises: Option<String>) -> RawCurve {
    let mut diags = Vec::new();
    let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let role = match input.get("role").and_then(|v| v.as_str()) {
        Some("buffer") => Role::Buffer,
        _ => Role::Sample,
    };
    let exposure = tagged(&input, "exposure", &mut diags, "exposure", time_factor)
        .unwrap_or(Tagged { value: f64::NAN, unit: String::new() });
    let thickness = tagged(&input, "thickness", &mut diags, "thickness", length_factor)
        .unwrap_or(Tagged { value: f64::NAN, unit: String::new() });
    let transmission = tagged(&input, "transmission", &mut diags, "transmission", dimensionless_factor)
        .unwrap_or(Tagged { value: f64::NAN, unit: String::new() });
    let transmission_sigma = input.get("transmission_sigma").map(|_|
        tagged(&input, "transmission_sigma", &mut diags, "transmission_sigma", dimensionless_factor)
            .unwrap_or(Tagged { value: f64::NAN, unit: String::new() }));

    let response = input.get("instrument_response").and_then(|r| r.as_object()).map(|o| {
        let source = o.get("source").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
        let factors = o.get("factors").and_then(|v| v.as_array()).map(|a| {
            a.iter().enumerate().filter_map(|(i, x)| match x.as_f64() {
                Some(f) => Some(f),
                None => { diags.push(Diagnostic{code:"BAD_RESPONSE".into(),
                    message:"non-numeric response factor skipped".into(),
                    at:Some(format!("instrument_response.factors[{i}]"))}); None }
            }).collect()
        }).unwrap_or_default();
        InstrumentResponse { source, factors }
    });
    let mask = input.get("mask").and_then(|r| r.as_object()).map(|o| {
        let source = o.get("source").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
        let masked = o.get("masked").and_then(|v| v.as_array()).map(|a|
            a.iter().map(|x| x.as_bool().unwrap_or(false)).collect()).unwrap_or_default();
        MaskSource { source, masked }
    });

    let mut points = Vec::new();
    if let Some(arr) = input.get("points").and_then(|v| v.as_array()) {
        for (i, p) in arr.iter().enumerate() {
            let qv = p.get("q").and_then(|q| q.get("value")).and_then(|v| v.as_f64());
            let qu = p.get("q").and_then(|q| q.get("unit")).and_then(|v| v.as_str()).unwrap_or("A^-1");
            if qv.is_none() {
                diags.push(Diagnostic{code:"BAD_Q".into(),
                    message:"point without numeric q retained in original".into(),
                    at:Some(format!("points[{i}].q"))});
            }
            if q_factor(qu).is_none() {
                diags.push(Diagnostic{code:"UNKNOWN_Q_UNIT".into(),
                    message:format!("q unit '{qu}' cannot be converted to A^-1; point kept in place"),
                    at:Some(format!("points[{i}].q.unit"))});
            }
            let iv = p.get("i").and_then(|v| v.as_f64()).unwrap_or(f64::NAN);
            let iu = p.get("i_unit").and_then(|v| v.as_str()).unwrap_or("counts").to_string();
            let sigma = p.get("sigma").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if !iv.is_finite() {
                diags.push(Diagnostic{code:"BAD_I".into(),
                    message:"non-finite intensity retained in original".into(),
                    at:Some(format!("points[{i}].i"))});
            }
            let response_factor = response.as_ref().and_then(|r| r.factors.get(i)).copied();
            if let Some(f) = response_factor {
                if !(f.is_finite() && f > 0.0) {
                    diags.push(Diagnostic{code:"BAD_RESPONSE".into(),
                        message:"response factor must be finite and positive".into(),
                        at:Some(format!("points[{i}]"))});
                }
            }
            let masked = mask.as_ref().and_then(|m| m.masked.get(i)).copied().unwrap_or(false);
            if let Some(q) = qv {
                points.push(RawPoint {
                    q: QPoint { value: q, unit: qu.to_string() },
                    i: iv, i_unit: iu, sigma, response_factor, masked,
                });
            }
        }
    }

    let mut c = RawCurve {
        id: String::new(), lineage, name, role, points,
        exposure, thickness, transmission, transmission_sigma,
        response, mask, diagnostics: diags, revises,
        original: input.clone(),
    };
    c.id = crate::domain::ids::hid("raw", &crate::domain::ids::canonical(&input).into());
    c
}
