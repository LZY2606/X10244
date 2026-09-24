//! Import 2D-integrated I(q) records.
//!
//! The raw q/I/sigma text, exposure, thickness, transmission, buffer
//! candidates, instrument response and mask provenance are all retained. A row
//! whose units cannot be converted stays in place with a diagnostic.

use crate::ids::content_id;
use crate::model::*;
use crate::quantity::{to_base, Quantity, QuantityKind, Unit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRow {
    pub q: String,
    pub i: String,
    pub sigma: String,
    pub q_unit: String,
    pub i_unit: String,
    #[serde(default)]
    pub masked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IngestInput {
    pub label: String,
    pub role: Role,
    pub version: String,
    pub rows: Vec<RawRow>,
    pub exposure: Quantity,
    pub thickness: Quantity,
    pub transmission: Transmission,
    pub instrument: InstrumentResponse,
    pub mask: MaskSource,
    pub source_text: String,
}

fn parse_number(raw: &str) -> Option<f64> {
    raw.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

pub fn ingest(input: IngestInput) -> RawBundle {
    let mut diagnostics = Vec::new();
    let mut points = Vec::new();

    for (index, row) in input.rows.iter().enumerate() {
        let q_unit = Unit::parse(&row.q_unit);
        let i_unit = Unit::parse(&row.i_unit);
        let q_raw = Quantity::new(parse_number(&row.q).unwrap_or(f64::NAN), row.q.clone(), q_unit.clone());
        let i_raw = Quantity::new(parse_number(&row.i).unwrap_or(f64::NAN), row.i.clone(), i_unit.clone());
        let sigma_raw =
            Quantity::new(parse_number(&row.sigma).unwrap_or(f64::NAN), row.sigma.clone(), i_unit.clone());

        let mut pdiag = Vec::new();
        let mut convertible = true;

        if q_raw.value.is_nan() {
            convertible = false;
            pdiag.push(format!("q not numeric: {:?}", row.q));
        }
        if i_raw.value.is_nan() {
            convertible = false;
            pdiag.push(format!("I not numeric: {:?}", row.i));
        }
        if sigma_raw.value.is_nan() {
            pdiag.push(format!("sigma missing/invalid: {:?}; treated as zero estimate", row.sigma));
        }

        let q = to_base(&q_raw, QuantityKind::Q).ok();
        let i = to_base(&i_raw, QuantityKind::Intensity).ok();
        let sigma = to_base(&sigma_raw, QuantityKind::Intensity).ok();
        if q.is_none() {
            convertible = false;
            pdiag.push(format!("q unit {:?} cannot be converted to 1/A; value retained", q_raw.unit));
        }
        if i.is_none() {
            convertible = false;
            pdiag.push(format!("intensity unit {:?} cannot be converted; value retained", i_raw.unit));
        }

        if !convertible {
            diagnostics.push(format!("row {}: {}", index, pdiag.join("; ")));
        } else if !pdiag.is_empty() {
            diagnostics.push(format!("row {}: {}", index, pdiag.join("; ")));
        }

        let pid_seed = serde_json::json!({
            "bundle": input.label,
            "version": input.version,
            "index": index,
            "q": row.q,
            "i": row.i,
            "sigma": row.sigma,
            "q_unit": row.q_unit,
            "i_unit": row.i_unit,
            "masked": row.masked,
        });

        points.push(Point {
            id: content_id("pt", &pid_seed),
            index,
            q_raw,
            i_raw,
            sigma_raw,
            q: q.map(|c| c.value),
            i: i.map(|c| c.value),
            sigma: sigma.map(|c| c.value),
            convertible,
            masked: row.masked,
            diagnostics: pdiag,
        });
    }

    if let Err(e) = to_base(&input.exposure, QuantityKind::Exposure) {
        diagnostics.push(format!("exposure: {}", e));
    }
    if let Err(e) = to_base(&input.thickness, QuantityKind::Thickness) {
        diagnostics.push(format!("thickness: {}", e));
    }
    if !(input.transmission.value.is_finite() && input.transmission.value > 0.0) {
        diagnostics.push("transmission invalid or non-positive".to_string());
    }

    let id = content_id(
        "bundle",
        &serde_json::json!({
            "label": input.label,
            "role": input.role,
            "version": input.version,
            "rows": input.rows,
            "exposure": input.exposure,
            "thickness": input.thickness,
            "transmission": input.transmission,
            "instrument_name": input.instrument.name,
            "instrument_scale": input.instrument.scale,
            "mask": input.mask.name,
        }),
    );

    RawBundle {
        id,
        label: input.label,
        role: input.role,
        version: input.version,
        points,
        exposure: input.exposure,
        thickness: input.thickness,
        transmission: input.transmission,
        instrument: input.instrument,
        mask: input.mask,
        source_text: input.source_text,
        diagnostics,
    }
}
