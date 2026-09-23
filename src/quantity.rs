//! Domain quantities.
//!
//! Every measured value keeps its original textual representation, unit and
//! coordinate basis. Normalized values are produced only on the derived layer;
//! a value that cannot be converted stays in place with diagnostics instead of
//! being dropped for the sake of plotting.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Measure {
    /// Parsed value in the canonical SI-ish target unit after conversion.
    pub value: f64,
    /// Exactly as imported.
    pub raw: String,
    /// Unit token exactly as imported.
    pub unit: String,
    /// Coordinate/reference basis, e.g. "q[1/angstrom]", "thickness[mm]".
    pub basis: String,
    pub convertible: bool,
    pub diagnostics: Vec<String>,
}

impl Measure {
    /// Parse an imported measure and convert to `target`.
    /// `basis` is the canonical basis label (e.g. "exposure[s]", "transmission[1]").
    pub fn imported(raw: &str, unit: &str, target: &str, basis: &str) -> Measure {
        let mut diagnostics = Vec::new();
        let parsed: Option<f64> = raw.trim().parse().ok();
        let factor = conversion_factor(unit.trim(), target);
        let convertible = factor.is_some();
        let value = match (parsed, factor) {
            (Some(v), Some(f)) => v * f,
            (Some(v), None) => {
                diagnostics.push(format!(
                    "unit '{unit}' cannot be converted to '{target}'; original representation retained"
                ));
                v
            }
            (None, _) => {
                diagnostics.push(format!("value '{raw}' is not numeric; kept in place"));
                f64::NAN
            }
        };
        if let (Some(_), None) = (parsed, factor) {
            diagnostics.push(format!("plotting uses unconverted value on a separate diagnostic axis"));
        }
        Measure { value, raw: raw.trim().to_string(), unit: unit.trim().to_string(), basis: basis.to_string(), convertible, diagnostics }
    }

    pub fn derived(value: f64, unit: &str, basis: &str) -> Measure {
        Measure {
            value,
            raw: crate::ids::canon(value),
            unit: unit.to_string(),
            basis: basis.to_string(),
            convertible: true,
            diagnostics: vec![],
        }
    }
}

/// Multiplicative factor from `from` to `to` for the supported unit families.
pub fn conversion_factor(from: &str, to: &str) -> Option<f64> {
    let f = from.to_ascii_lowercase().replace([' ', '−'], "");
    let t = to.to_ascii_lowercase().replace([' ', '−'], "");
    if f == t {
        return Some(1.0);
    }
    match (f.as_str(), t.as_str()) {
        // length -> mm
        ("mm", "mm") | ("cm", "cm") | ("um", "um") => Some(1.0),
        ("cm", "mm") => Some(10.0),
        ("mm", "cm") => Some(0.1),
        ("um", "mm") => Some(1e-3),
        ("mm", "um") => Some(1e3),
        ("nm", "mm") => Some(1e-6),
        ("angstrom", "mm") | ("a", "mm") | ("å", "mm") => Some(1e-7),
        ("mm", "nm") => Some(1e6),
        ("mm", "angstrom") | ("mm", "a") => Some(1e7),
        // time -> s
        ("ms", "s") => Some(1e-3),
        ("s", "ms") => Some(1e3),
        ("min", "s") => Some(60.0),
        ("s", "min") => Some(1.0 / 60.0),
        // reciprocal length -> 1/angstrom
        ("nm^-1", "angstrom^-1") | ("1/nm", "angstrom^-1") => Some(0.1),
        ("angstrom^-1", "nm^-1") | ("angstrom^-1", "1/nm") => Some(10.0),
        ("a^-1", "angstrom^-1") | ("1/a", "angstrom^-1") | ("1/angstrom", "angstrom^-1") => {
            Some(1.0)
        }
        // intensity flow
        ("counts/ms", "counts/s") => Some(1e3),
        ("counts/min", "counts/s") => Some(1.0 / 60.0),
        ("counts/s", "counts/ms") => Some(1e-3),
        ("counts", "counts") => Some(1.0),
        ("arb", "arb") | ("a.u.", "arb") | ("au", "arb") => Some(1.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_original_representation_and_diagnostic() {
        let m = Measure::imported("0.050", "cm", "mm", "thickness[mm]");
        assert!((m.value - 0.5).abs() < 1e-12);
        assert_eq!(m.raw, "0.050");
        assert!(m.convertible);
        let bad = Measure::imported("1.2", "fortnights", "s", "exposure[s]");
        assert!(!bad.convertible);
        assert!(bad.value.is_nan() == false);
        assert!(!bad.diagnostics.is_empty());
    }

    #[test]
    fn non_numeric_stays_in_place() {
        let bad = Measure::imported("n/a", "s", "s", "exposure[s]");
        assert!(bad.value.is_nan());
        assert!(!bad.convertible || !bad.diagnostics.is_empty());
    }
}
