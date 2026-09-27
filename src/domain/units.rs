//! Physical quantities keep their original representation and unit; the
//! derived layer converts to canonical units and records the conversion.
//!
//! Canonical bases:
//! - momentum transfer `q`: 1/A (angstrom^-1)
//! - intensity `I`: 1/cm (cm^-1) absolute scale
//! - thickness: cm
//! - exposure: s
//! - transmission: dimensionless, must satisfy 0 < T <= 1

use super::diagnostic::{Diagnostic, DiagCode, Severity};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysVal {
    pub value: f64,
    pub unit: String,
}

impl PhysVal {
    pub fn new(value: f64, unit: impl Into<String>) -> Self {
        PhysVal { value, unit: unit.into() }
    }
}

/// Result of a conversion; the original value is always retained.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversion {
    pub canonical: f64,
    pub factor: f64,
    pub diagnostic: Option<Diagnostic>,
}

pub fn convert_q(v: &PhysVal) -> Conversion {
    let (factor, canonical_unit) = match v.unit.as_str() {
        "1/A" | "A^-1" | "angstrom^-1" | "nm^-1-canon-no" => (1.0, "1/A"),
        "1/nm" | "nm^-1" => (0.1, "1/A"),
        other => {
            return Conversion {
                canonical: f64::NAN,
                factor: f64::NAN,
                diagnostic: Some(Diagnostic::new(
                    DiagCode::UnconvertibleUnit,
                    Severity::Error,
                    format!("unknown q unit '{other}', value kept at original position"),
                )),
            }
        }
    };
    let _ = canonical_unit;
    finish(v, factor)
}

pub fn convert_intensity(v: &PhysVal) -> Conversion {
    let factor = match v.unit.as_str() {
        "1/cm" | "cm^-1" => 1.0,
        "1/mm" | "mm^-1" => 0.1,
        _ => {
            return Conversion {
                canonical: f64::NAN,
                factor: f64::NAN,
                diagnostic: Some(Diagnostic::new(
                    DiagCode::UnconvertibleUnit,
                    Severity::Error,
                    format!("unknown intensity unit '{}', kept at original position", v.unit),
                )),
            }
        }
    };
    finish(v, factor)
}

pub fn convert_thickness(v: &PhysVal) -> Conversion {
    let factor = match v.unit.as_str() {
        "cm" => 1.0,
        "mm" => 0.1,
        "m" => 100.0,
        _ => {
            return Conversion {
                canonical: f64::NAN,
                factor: f64::NAN,
                diagnostic: Some(Diagnostic::new(
                    DiagCode::UnconvertibleUnit,
                    Severity::Error,
                    format!("unknown thickness unit '{}', kept at original position", v.unit),
                )),
            }
        }
    };
    finish(v, factor)
}

pub fn convert_exposure(v: &PhysVal) -> Conversion {
    let factor = match v.unit.as_str() {
        "s" | "sec" => 1.0,
        "ms" => 0.001,
        "min" => 60.0,
        _ => {
            return Conversion {
                canonical: f64::NAN,
                factor: f64::NAN,
                diagnostic: Some(Diagnostic::new(
                    DiagCode::UnconvertibleUnit,
                    Severity::Error,
                    format!("unknown exposure unit '{}', kept at original position", v.unit),
                )),
            }
        }
    };
    finish(v, factor)
}

fn finish(v: &PhysVal, factor: f64) -> Conversion {
    if !v.value.is_finite() {
        return Conversion {
            canonical: v.value,
            factor,
            diagnostic: Some(Diagnostic::new(
                DiagCode::NonFinite,
                Severity::Error,
                "non-finite value preserved",
            )),
        };
    }
    Conversion { canonical: v.value * factor, factor, diagnostic: None }
}

/// Validates transmission (dimensionless). Invalid values are preserved and
/// reported; normalization is then blocked rather than fudged.
pub fn check_transmission(v: f64) -> Option<Diagnostic> {
    if !v.is_finite() || !(0.0 < v && v <= 1.0) {
        Some(Diagnostic::new(
            DiagCode::BadTransmission,
            Severity::Error,
            format!("transmission {v} outside (0,1]; normalization blocked"),
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_known_units_and_reports_unknown() {
        assert!((convert_q(&PhysVal::new(10.0, "1/nm")).canonical - 1.0).abs() < 1e-12);
        assert!((convert_thickness(&PhysVal::new(1.0, "mm")).canonical - 0.1).abs() < 1e-12);
        assert!((convert_exposure(&PhysVal::new(1.0, "min")).canonical - 60.0).abs() < 1e-12);
        assert!(convert_q(&PhysVal::new(1.0, "furlong")).diagnostic.is_some());
        assert!(check_transmission(0.5).is_none());
        assert!(check_transmission(1.2).is_some());
    }
}
