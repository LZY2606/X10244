//! Quantities with explicit units / coordinate basis.
//!
//! Imported representations are always retained verbatim (`raw`). The derived
//! layer converts to canonical SAXS units; values that cannot be converted
//! stay in place with a diagnostic instead of being dropped for plotting.

use serde::{Deserialize, Serialize};

/// Canonical bases used by every derived artifact.
pub const Q_CANON: &str = "\u{00c5}^-1";
pub const I_CANON: &str = "cm^-1";

/// A measured/scalar value carrying its original textual representation,
/// unit and coordinate basis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Val {
    /// Converted value in canonical units, or `None` when not convertible.
    pub value: Option<f64>,
    /// Unit after normalization (canonical for derived, imported for raw).
    pub unit: String,
    /// Original textual representation exactly as imported.
    pub raw: String,
    /// Coordinate / measurement basis, e.g. "q in reciprocal angstrom".
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub basis: Option<String>,
    /// Conversion/parse diagnostic, present iff `value` is unusable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub diag: Option<String>,
}

impl Val {
    pub fn parsed(raw: &str, unit: &str) -> Self {
        match raw.trim().parse::<f64>() {
            Ok(v) => Val {
                value: Some(v),
                unit: unit.to_string(),
                raw: raw.to_string(),
                basis: None,
                diag: None,
            },
            Err(_) => Val {
                value: None,
                unit: unit.to_string(),
                raw: raw.to_string(),
                basis: None,
                diag: Some(format!("unparsable scalar '{}' in unit {}", raw, unit)),
            },
        }
    }

    pub fn canonical(value: f64, unit: &str, basis: &str) -> Self {
        Val {
            value: Some(value),
            unit: unit.to_string(),
            raw: format!("{} {}", value, unit),
            basis: Some(basis.to_string()),
            diag: None,
        }
    }

    pub fn is_usable(&self) -> bool {
        match self.value {
            Some(v) => v.is_finite(),
            None => false,
        }
    }
}

/// Convert a reciprocal-space q value to Å^-1.
pub fn convert_q(v: Val) -> Val {
    let factor: Option<f64> = match v.unit.as_str() {
        "\u{00c5}^-1" | "A^-1" | "1/A" | "angstrom^-1" => Some(1.0),
        "nm^-1" | "1/nm" => Some(10.0),
        _ => None,
    };
    match (v.value, factor) {
        (Some(x), Some(f)) => Val {
            value: Some(x * f),
            unit: Q_CANON.to_string(),
            basis: Some(format!("converted from {} x {}", v.unit, f)),
            ..v
        },
        (Some(_), None) => Val {
            value: None,
            diag: Some(format!("no conversion from q unit '{}' to {}", v.unit, Q_CANON)),
            ..v
        },
        _ => v,
    }
}

/// Convert an intensity to cm^-1.
pub fn convert_i(v: Val) -> Val {
    let factor: Option<f64> = match v.unit.as_str() {
        "cm^-1" | "1/cm" => Some(1.0),
        "mm^-1" | "1/mm" => Some(0.1),
        _ => None,
    };
    match (v.value, factor) {
        (Some(x), Some(f)) => Val {
            value: Some(x * f),
            unit: I_CANON.to_string(),
            basis: Some(format!("converted from {} x {}", v.unit, f)),
            ..v
        },
        (Some(_), None) => Val {
            value: None,
            diag: Some(format!("no conversion from intensity unit '{}' to {}", v.unit, I_CANON)),
            ..v
        },
        _ => v,
    }
}

/// Convert exposure to seconds.
pub fn convert_exposure(v: Val) -> Val {
    let factor = match v.unit.as_str() {
        "s" | "sec" => Some(1.0),
        "ms" => Some(0.001),
        _ => None,
    };
    match (v.value, factor) {
        (Some(x), Some(f)) => Val {
            value: Some(x * f),
            unit: "s".to_string(),
            basis: Some(format!("converted from {} x {}", v.unit, f)),
            ..v
        },
        (Some(_), None) => Val {
            value: None,
            diag: Some(format!("unknown exposure unit '{}'", v.unit)),
            ..v
        },
        _ => v,
    }
}

/// Convert sample thickness to centimetres.
pub fn convert_thickness(v: Val) -> Val {
    let factor = match v.unit.as_str() {
        "cm" => Some(1.0),
        "mm" => Some(0.1),
        _ => None,
    };
    match (v.value, factor) {
        (Some(x), Some(f)) => Val {
            value: Some(x * f),
            unit: "cm".to_string(),
            basis: Some(format!("converted from {} x {}", v.unit, f)),
            ..v
        },
        (Some(_), None) => Val {
            value: None,
            diag: Some(format!("unknown thickness unit '{}'", v.unit)),
            ..v
        },
        _ => v,
    }
}

/// Transmission is dimensionless; anything else cannot be used.
pub fn convert_transmission(v: Val) -> Val {
    match v.unit.as_str() {
        "" | "1" | "fraction" | "dimensionless" => {
            if let Some(x) = v.value {
                if !(0.0..=1.0).contains(&x) {
                    return Val {
                        value: None,
                        diag: Some(format!("transmission {} outside [0,1]", x)),
                        ..v
                    };
                }
            }
            let mut v = v;
            v.unit = "1".to_string();
            v
        }
        other => Val {
            value: None,
            diag: Some(format!("transmission must be dimensionless, got '{}'", other)),
            ..v
        },
    }
}
