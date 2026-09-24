//! Quantities with units. Original textual representation is always retained;
//! canonical conversion happens only in the derivation layer. Values that
//! cannot be converted stay at the origin together with a diagnostic.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawField {
    /// Original token exactly as imported, e.g. "1.00(2)", "0.42", "85%"
    pub text: String,
    /// Original unit token exactly as imported
    pub unit: String,
    /// Parsed numeric value (in the original unit), if the token was numeric
    pub parsed: Option<f64>,
    /// Value converted to the canonical unit, if conversion succeeded
    pub value: Option<f64>,
    /// Canonical unit label, if conversion succeeded
    pub canonical_unit: Option<String>,
    pub convertible: bool,
    pub diagnostic: Option<String>,
}

impl RawField {
    pub fn unconvertible(text: impl Into<String>, unit: impl Into<String>, why: impl Into<String>) -> Self {
        let text = text.into();
        let parsed = parse_scalar(&text);
        RawField {
            text,
            unit: unit.into(),
            parsed,
            value: None,
            canonical_unit: None,
            convertible: false,
            diagnostic: Some(why.into()),
        }
    }
}

/// Parse a plain numeric token, accepting uncertainty notation "1.234(5)"
/// and symmetric uncertainty notation "1.234+-0.005".
pub fn parse_scalar(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some((num, _unc)) = s.split_once('(') {
        return num.trim().parse::<f64>().ok();
    }
    if let Some((num, _unc)) = s.split_once("+-") {
        return num.trim().parse::<f64>().ok();
    }
    s.parse::<f64>().ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Length,
    ReciprocalLength,
    Time,
    Transmission,
}

/// (multiplicative factor to canonical unit, canonical unit label)
pub fn conversion(unit: &str, dim: Dimension) -> Option<(f64, &'static str)> {
    let u = unit.trim().trim_start_matches(['1', '/']).trim();
    let u_raw = unit.trim();
    match dim {
        Dimension::Length => match u.to_ascii_lowercase().as_str() {
            "nm" => Some((1.0, "nm")),
            "a" | "ang" | "angstrom" | "å" => Some((0.1, "nm")),
            "mm" => Some((1.0e6, "nm")),
            "cm" => Some((1.0e7, "nm")),
            "m" => Some((1.0e9, "nm")),
            _ => None,
        },
        Dimension::ReciprocalLength => match u_raw {
            "nm^-1" | "1/nm" | "nm-1" => Some((1.0, "nm^-1")),
            "A^-1" | "1/A" | "angstrom^-1" | "Å^-1" => Some((10.0, "nm^-1")),
            _ => None,
        },
        Dimension::Time => match u.to_ascii_lowercase().as_str() {
            "s" | "sec" => Some((1.0, "s")),
            "ms" => Some((1.0e-3, "s")),
            "min" => Some((60.0, "s")),
            _ => None,
        },
        Dimension::Transmission => match u.to_ascii_lowercase().as_str() {
            "" | "1" | "fraction" | "a.u." | "t" => Some((1.0, "fraction")),
            "%" | "percent" => Some((0.01, "fraction")),
            _ => None,
        },
    }
}

/// Import a scalar field: parse, convert to canonical unit or retain raw with a diagnostic.
pub fn import_field(text: &str, unit: &str, dim: Dimension) -> RawField {
    let canonical = conversion(unit, dim);
    match (parse_scalar(text), canonical) {
        (Some(v), Some((f, cu))) => RawField {
            text: text.trim().to_string(),
            unit: unit.trim().to_string(),
            parsed: Some(v),
            value: Some(v * f),
            canonical_unit: Some(cu.to_string()),
            convertible: true,
            diagnostic: None,
        },
        (Some(_), None) => RawField::unconvertible(
            text.trim(),
            unit.trim(),
            format!("unit '{}' has no registered conversion for dimension {:?}", unit.trim(), dim),
        ),
        (None, _) => RawField::unconvertible(
            text.trim(),
            unit.trim(),
            format!("'{}' is not a plain numeric scalar", text.trim()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_length_units() {
        let f = import_field("1.2", "mm", Dimension::Length);
        assert!(f.convertible);
        assert!((f.value.unwrap() - 1.2e6).abs() < 1e-6);
        assert_eq!(f.canonical_unit.as_deref(), Some("nm"));
    }

    #[test]
    fn converts_percent_transmission_and_uncertainty_notation() {
        let f = import_field("85", "%", Dimension::Transmission);
        assert!((f.value.unwrap() - 0.85).abs() < 1e-12);
        let g = import_field("0.420(3)", "mm", Dimension::Length);
        assert!((g.value.unwrap() - 4.2e5).abs() < 1e-3);
    }

    #[test]
    fn unknown_unit_stays_raw_with_diagnostic() {
        let f = import_field("1.00", "thou", Dimension::Length);
        assert!(!f.convertible);
        assert_eq!(f.value, None);
        assert!(f.diagnostic.is_some());
        assert_eq!(f.text, "1.00");
    }
}
