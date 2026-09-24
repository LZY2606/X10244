//! Units and coordinate bases.
//!
//! Every domain quantity carries a unit/coordinate base. Imports preserve the
//! raw representation verbatim; the derived layer converts to canonical units
//! here. Values that cannot be converted stay in place with diagnostics instead
//! of being silently dropped for plotting.

use serde::{Deserialize, Serialize};

/// Canonical bases used inside the derived layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Canonical {
    /// Scattering vector, canonical unit 1/angstrom.
    InvAngstrom,
    /// Intensity-like quantity, canonical unit cm^-1 (1/cm).
    CmInv,
    /// Exposure time, canonical unit seconds.
    Seconds,
    /// Thickness, canonical unit centimetres.
    Cm,
    /// Transmission / scale factor, dimensionless.
    Dimensionless,
}

/// Linear conversion `x_canonical = factor * x_native`.
#[derive(Debug, Clone)]
pub struct UnitSpec {
    pub canonical: Canonical,
    pub factor: f64,
    pub label: &'static str,
}

pub const UNITS: &[UnitSpec] = &[
    UnitSpec { canonical: Canonical::InvAngstrom, factor: 1.0, label: "1/A" },
    UnitSpec { canonical: Canonical::InvAngstrom, factor: 0.1, label: "1/nm" },
    UnitSpec { canonical: Canonical::InvAngstrom, factor: 1e-8, label: "1/cm" },
    UnitSpec { canonical: Canonical::CmInv, factor: 1.0, label: "1/cm" },
    UnitSpec { canonical: Canonical::CmInv, factor: 100.0, label: "1/mm" },
    UnitSpec { canonical: Canonical::CmInv, factor: 1.0, label: "arb. u." },
    UnitSpec { canonical: Canonical::Seconds, factor: 1.0, label: "s" },
    UnitSpec { canonical: Canonical::Seconds, factor: 1.0 / 1000.0, label: "ms" },
    UnitSpec { canonical: Canonical::Cm, factor: 1.0, label: "cm" },
    UnitSpec { canonical: Canonical::Cm, factor: 0.1, label: "mm" },
    UnitSpec { canonical: Canonical::Dimensionless, factor: 1.0, label: "" },
];

/// Result of attempting to convert an imported quantity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Conversion {
    /// Converted value plus the native representation that was preserved.
    Ok {
        canonical_value: f64,
        native_value: f64,
        native_unit: String,
        factor: f64,
    },
    /// The value could not be converted; it stays at its original location.
    Unconvertible {
        native_value: f64,
        native_unit: String,
        reason: String,
    },
}

impl Conversion {
    pub fn try_convert(value: f64, unit: &str, wanted: Canonical) -> Conversion {
        let key = unit.trim();
        for spec in UNITS {
            if spec.canonical != wanted {
                continue;
            }
            if spec.label == key || (spec.label.is_empty() && key.is_empty()) {
                return Conversion::Ok {
                    canonical_value: value * spec.factor,
                    native_value: value,
                    native_unit: key.to_string(),
                    factor: spec.factor,
                };
            }
        }
        Conversion::Unconvertible {
            native_value: value,
            native_unit: key.to_string(),
            reason: format!("unit '{key}' is not a known linear unit for the canonical basis"),
        }
    }

    pub fn canonical_value(&self) -> Option<f64> {
        match self {
            Conversion::Ok { canonical_value, .. } => Some(*canonical_value),
            Conversion::Unconvertible { .. } => None,
        }
    }
}

/// A numeric value that keeps both its raw imported representation and, when
/// available, its normalized canonical value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub raw_text: String,
    pub raw_unit: String,
    pub conversion: Conversion,
}

impl Quantity {
    pub fn parse(text: &str, unit: &str, wanted: Canonical) -> Self {
        let raw_text = text.trim().to_string();
        let raw_unit = unit.trim().to_string();
        match raw_text.parse::<f64>() {
            Ok(value) if value.is_finite() => Quantity {
                conversion: Conversion::try_convert(value, &raw_unit, wanted),
                raw_text,
                raw_unit,
            },
            _ => Quantity {
                raw_text,
                raw_unit: raw_unit.clone(),
                conversion: Conversion::Unconvertible {
                    native_value: f64::NAN,
                    native_unit: raw_unit,
                    reason: "value is not a finite number".to_string(),
                },
            },
        }
    }

    pub fn canonical(&self) -> Option<f64> {
        self.conversion.canonical_value()
    }

    pub fn is_ok(&self) -> bool {
        matches!(self.conversion, Conversion::Ok { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_q_nanometres_to_angstrom() {
        let q = Quantity::parse("0.5", "1/nm", Canonical::InvAngstrom);
        assert!((q.canonical().unwrap() - 0.05).abs() < 1e-12);
    }

    #[test]
    fn unknown_unit_is_preserved_with_diagnostic() {
        let q = Quantity::parse("0.5", "furlong", Canonical::InvAngstrom);
        assert!(q.canonical().is_none());
        assert_eq!(q.raw_text, "0.5");
        assert!(matches!(q.conversion, Conversion::Unconvertible { .. }));
    }

    #[test]
    fn non_numeric_value_stays_in_place() {
        let q = Quantity::parse("N/A", "mm", Canonical::Cm);
        assert!(q.canonical().is_none());
        assert_eq!(q.raw_text, "N/A");
    }
}
