// Core SAXS rules. These functions are pure: they take immutable inputs and
// return new derived objects plus diagnostics. They never mutate sources.
//
// Guarded semantics (exercised by the counter-example pack):
//  * mismatched q grids are never subtracted point-by-point;
//  * interpolation happens only inside the shared overlap window;
//  * sigma includes both normalization uncertainty and background scale
//    uncertainty;
//  * Guinier and Porod intervals are independent candidates and are never
//    fused into a single automatic model.

use crate::domain::*;
use crate::id::{canon_f64, stable_id};

#[derive(Debug, Clone, Copy)]
pub struct NormUncertainties {
    pub transmission_rel: f64,
    pub exposure_rel: f64,
    pub thickness_rel: f64,
}

impl NormUncertainties {
    pub fn combined_rel(&self) -> f64 {
        (self.transmission_rel.powi(2) + self.exposure_rel.powi(2) + self.thickness_rel.powi(2))
            .sqrt()
    }
}

fn point_id(node_prefix: &str, node_id: &str, i: usize) -> String {
    stable_id("p", &[node_prefix, node_id, &i.to_string()])
}

/// Absolute intensity scale:
/// I_norm(q) = I_raw(q) * response_scale * exposure / (transmission * thickness)
pub fn normalize(raw: &RawCurve, response: &InstrumentResponse) -> NormalizedCurve {
    let exp = raw.exposure_s.value;
    let thick = raw.thickness_mm.value;
    let trans = raw.transmission.value;

    let mut diagnostics = raw.diagnostics.clone();
    let usable = match (exp, thick, trans) {
        (Some(e), Some(d), Some(t)) if e > 0.0 && d > 0.0 && t > 0.0 => {
            response.scale * e / (t * d)
        }
        _ => {
            diagnostics.push(
                "NORMALIZATION_INCOMPLETE: exposure/thickness/transmission missing or non-positive"
                    .to_string(),
            );
            f64::NAN
        }
    };

    let norm_rel_sigma = NormUncertainties {
        transmission_rel: rel_uncertainty(&raw.transmission),
        exposure_rel: rel_uncertainty(&raw.exposure_s),
        thickness_rel: rel_uncertainty(&raw.thickness_mm),
    }
    .combined_rel();

    let node_id = stable_id(
        "norm",
        &[
            &raw.id,
            &response.id,
            &canon_f64(response.scale),
            &canon_f64(exp.unwrap_or(f64::NAN)),
            &canon_f64(thick.unwrap_or(f64::NAN)),
            &canon_f64(trans.unwrap_or(f64::NAN)),
            ALGO_VERSION,
        ],
    );

    let mut points = Vec::with_capacity(raw.points.len());
    for p in &raw.points {
        let value = if usable.is_nan() { f64::NAN } else { p.intensity * usable };
        let sigma = if usable.is_nan() {
            f64::NAN
        } else {
            // statistical sigma propagated together with relative norm error
            let stat = p.sigma * usable;
            let norm = value.abs() * norm_rel_sigma;
            (stat.powi(2) + norm.powi(2)).sqrt()
        };
        let masked = p.masked.unwrap_or(false);
        points.push(DerivedPoint {
            id: point_id("norm", &node_id, p.i),
            i: p.i,
            q: p.q,
            value,
            sigma,
            source_a: Some(p.i),
            source_b: None,
            flags: if masked { vec!["MASKED".to_string()] } else { vec![] },
        });
    }

    NormalizedCurve {
        id: node_id,
        raw_id: raw.id.clone(),
        points,
        norm_factor: usable,
        norm_rel_sigma,
        q_unit: raw.q_unit.clone(),
        i_unit: raw.i_unit.clone(),
        diagnostics,
    }
}

// Relative uncertainty stored on a Valued is encoded as a diagnostic token
// "rel_sigma=<x>"; when absent a conservative instrument-level default is used.
fn rel_uncertainty(v: &Valued) -> f64 {
    for d in &v.diagnostics {
        if let Some(rest) = d.strip_prefix("rel_sigma=") {
            if let Ok(x) = rest.trim().parse::<f64>() {
                return x;
            }
        }
    }
    0.02
}
