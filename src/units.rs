use crate::model::Diagnostic;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub raw_text: String,
    pub unit: String,
    pub value: Option<f64>,
    pub basis: String,
}

impl Quantity {
    pub fn raw(text: impl Into<String>, unit: impl Into<String>) -> Self {
        let raw_text = text.into();
        let value = raw_text.trim().parse::<f64>().ok();
        Quantity {
            raw_text,
            unit: unit.into(),
            value,
            basis: "as-imported".to_string(),
        }
    }

    pub fn numeric(text: impl Into<String>, unit: impl Into<String>, value: f64, basis: &str) -> Self {
        Quantity {
            raw_text: text.into(),
            unit: unit.into(),
            value: Some(value),
            basis: basis.to_string(),
        }
    }
}

pub fn parse_q_nm(q: &Quantity, diags: &mut Vec<Diagnostic>) -> Option<f64> {
    match q.unit.as_str() {
        "nm^-1" | "1/nm" | "nm-1" => q.value,
        "A^-1" | "1/A" | "angstrom^-1" => q.value.map(|v| v * 10.0),
        _ => {
            diags.push(Diagnostic {
                code: "Q_UNIT_UNKNOWN".into(),
                message: format!("q unit '{}' has no conversion to nm^-1; value retained in place", q.unit),
                severity: "error".into(),
                at: q.raw_text.clone(),
            });
            None
        }
    }
}

pub fn exposure_seconds(q: &Quantity, diags: &mut Vec<Diagnostic>) -> Option<f64> {
    match q.unit.as_str() {
        "s" | "sec" | "seconds" => q.value,
        "min" | "minutes" => q.value.map(|v| v * 60.0),
        "ms" => q.value.map(|v| v / 1000.0),
        _ => {
            diags.push(Diagnostic {
                code: "EXPOSURE_UNIT_UNKNOWN".into(),
                message: format!("exposure unit '{}' cannot be converted to s", q.unit),
                severity: "error".into(),
                at: q.raw_text.clone(),
            });
            None
        }
    }
}

pub fn thickness_mm(q: &Quantity, diags: &mut Vec<Diagnostic>) -> Option<f64> {
    match q.unit.as_str() {
        "mm" => q.value,
        "cm" => q.value.map(|v| v * 10.0),
        _ => {
            diags.push(Diagnostic {
                code: "THICKNESS_UNCONVERTIBLE".into(),
                message: format!(
                    "thickness '{} {}' left at source: no conversion to mm registered",
                    q.raw_text, q.unit
                ),
                severity: "error".into(),
                at: q.raw_text.clone(),
            });
            None
        }
    }
}

pub fn transmission(q: &Quantity, diags: &mut Vec<Diagnostic>) -> Option<f64> {
    let v = q.value?;
    if v <= 0.0 || v > 1.0 {
        diags.push(Diagnostic {
            code: "TRANSMISSION_OUT_OF_RANGE".into(),
            message: format!("transmission {v} outside (0,1]; retained, normalization rejected"),
            severity: "error".into(),
            at: q.raw_text.clone(),
        });
        return None;
    }
    Some(v)
}
