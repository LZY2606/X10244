//! Normalization of a raw record to an absolute-scale curve.
//!
//! I_norm(q) = I_raw(q) / (exposure * thickness * transmission) [+ response]
//! sigma_norm = sigma_raw / (exposure * thickness * transmission)
//!
//! Relative uncertainties of exposure/thickness/transmission are folded into
//! the propagated sigma; the raw record itself is never modified.

use super::curve::{Curve, Point};
use super::diagnostic::{Diagnostic, DiagCode, Diags, Severity};
use super::id::Sid;
use super::record::RawRecord;
use super::units::{check_transmission, convert_exposure, convert_intensity, convert_q, convert_thickness};

pub const RULE_VERSION: &str = "normalize.v1";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NormalizeInput {
    /// version id of the raw record used
    pub record_version_id: String,
    pub record_name: String,
    pub record_version: u32,
    /// relative uncertainties folded into the propagated error
    pub rel_exposure: f64,
    pub rel_thickness: f64,
    pub rel_transmission: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NormalizeOutput {
    pub curve: Curve,
    pub input: NormalizeInput,
    pub diagnostics: Diags,
    /// q indices of the raw record excluded by masks (raw points retained).
    pub masked_indices: Vec<usize>,
}

pub fn normalize(record: &RawRecord) -> NormalizeOutput {
    let mut diags: Diags = Vec::new();
    let mut blocked = false;

    let exp = convert_exposure(&record.exposure);
    if let Some(d) = &exp.diagnostic {
        diags.push(d.clone());
        blocked = true;
    }
    let th = convert_thickness(&record.thickness);
    if let Some(d) = &th.diagnostic {
        diags.push(d.clone());
        blocked = true;
    }
    if let Some(d) = check_transmission(record.transmission.value) {
        diags.push(d);
        blocked = true;
    }
    if exp.canonical <= 0.0 || th.canonical <= 0.0 {
        diags.push(Diagnostic::new(
            DiagCode::NonFinite,
            Severity::Error,
            "exposure/thickness must be positive",
        ));
        blocked = true;
    }

    let mut points: Vec<Point> = Vec::new();
    let mut masked_indices: Vec<usize> = Vec::new();
    if !blocked {
        let denom = exp.canonical * th.canonical * record.transmission.value;
        // Response correction (multiplicative), interpolated onto the record grid.
        let resp: Option<(Vec<f64>, Vec<f64>)> = record.response.as_ref().map(|r| {
            let xs: Vec<f64> = r
                .grid_q
                .iter()
                .map(|q| convert_q(&super::units::PhysVal::new(*q, r.q_unit.clone())).canonical)
                .collect();
            (xs, r.values.clone())
        });
        for (idx, rp) in record.points.iter().enumerate() {
            let qc = convert_q(&rp.q);
            if let Some(d) = qc.diagnostic {
                diags.push(d.at(format!("point[{idx}].q")));
                continue;
            }
            let ic = convert_intensity(&rp.i);
            if let Some(d) = ic.diagnostic {
                diags.push(d.at(format!("point[{idx}].i")));
                continue;
            }
            let sc = convert_intensity(&rp.sigma);
            if let Some(d) = sc.diagnostic {
                diags.push(d.at(format!("point[{idx}].sigma")));
                continue;
            }
            if masked(record, qc.canonical) {
                diags.push(
                    Diagnostic::new(DiagCode::MaskedPoint, Severity::Info, "excluded by mask")
                        .at(format!("point[{idx}]")),
                );
                masked_indices.push(idx);
                continue;
            }
            let mut i = ic.canonical / denom;
            let mut s = sc.canonical / denom;
            if let Some((xs, ys)) = &resp {
                if let Some((r, _)) = super::curve::interp_at(xs, ys, qc.canonical) {
                    if r > 0.0 {
                        i /= r;
                        s /= r;
                    }
                }
            }
            let rel2 = 0.0005f64.powi(2) + 0.001f64.powi(2) + 0.002f64.powi(2);
            let sigma = (s * s + i * i * rel2).sqrt();
            points.push(Point { q: qc.canonical, i, sigma });
        }
    }

    let input = NormalizeInput {
        record_version_id: record.version_id().to_string(),
        record_name: record.name.clone(),
        record_version: record.version,
        rel_exposure: 0.0005,
        rel_thickness: 0.001,
        rel_transmission: 0.002,
    };
    let curve_id = Sid(format!(
        "saxs1:norm:{}",
        super::id::hid_with("norm", &(&input, &points)).as_str()[6..].to_string()
    ));
    NormalizeOutput { curve: Curve::new(curve_id, points), input, diagnostics: diags, masked_indices }
}

fn masked(record: &RawRecord, q_canonical: f64) -> bool {
    for m in &record.masks {
        let mut inside_any = false;
        for [lo, hi] in &m.keep_q {
            let lo_c = convert_q(&super::units::PhysVal::new(*lo, record.points[0].q.unit.clone())).canonical;
            let hi_c = convert_q(&super::units::PhysVal::new(*hi, record.points[0].q.unit.clone())).canonical;
            if q_canonical >= lo_c - 1e-12 && q_canonical <= hi_c + 1e-12 {
                inside_any = true;
                break;
            }
        }
        if !inside_any {
            return true;
        }
    }
    false
}
