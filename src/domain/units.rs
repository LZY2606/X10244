//! Units and coordinate bases.
//!
//! Import layer keeps the original textual representation; only the derivation
//! layer converts to canonical units. Unconvertible values stay where they are
//! with a diagnostic instead of being dropped.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    pub raw: String,
    pub declared_unit: Option<String>,
    pub canonical: Option<f64>,
    pub canonical_unit: String,
    pub diagnostic: Option<String>,
}

impl Quantity {
    pub fn unconvertible(raw: impl Into<String>, canonical_unit: &str, why: &str) -> Self {
        Quantity {
            raw: raw.into(),
            declared_unit: None,
            canonical: None,
            canonical_unit: canonical_unit.to_string(),
            diagnostic: Some(why.to_string()),
        }
    }
}

fn norm_unit(u: &str) -> String {
    u.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .replace("angstrom", "A")
        .replace("Angstrom", "A")
        .replace("Å", "A")
        .replace("inverse", "")
        .replace("sec", "s")
        .replace("second", "s")
}

/// Parse a scalar that may embed its unit, e.g. "0.10 cm", "1.0mm", "0.91".
/// `convert` maps a normalized unit string to (factor, canonical_unit).
pub fn parse_scalar(
    raw: &str,
    default_unit: &str,
    convert: impl Fn(&str) -> Option<(f64, String)>,
) -> Quantity {
    let text = raw.trim();
    let (num_txt, unit_txt): (&str, Option<&str>) = match text
        .find(|c: char| c.is_alphabetic() || c == 'Å')
    {
        Some(pos) => (&text[..pos], Some(text[pos..].trim())),
        None => (text, None),
    };
    let declared = unit_txt.unwrap_or(default_unit).to_string();
    let num: Option<f64> = num_txt.trim().parse().ok();
    match (num, convert(&norm_unit(&declared))) {
        (Some(n), Some((factor, canonical_unit))) => Quantity {
            raw: text.to_string(),
            declared_unit: Some(declared),
            canonical: Some(n * factor),
            canonical_unit,
            diagnostic: None,
        },
        (Some(_), None) => Quantity::unconvertible(
            text,
            "",
            &format!("declared unit '{}' has no conversion table entry", declared),
        ),
        (None, _) => {
            Quantity::unconvertible(text, "", "numeric part could not be parsed")
        }
    }
}

/// q coordinate basis. Canonical: 1/A.
pub fn parse_q(raw_value: &str, declared_unit: &str) -> Quantity {
    parse_scalar(
        &if raw_value.chars().any(|c| c.is_alphabetic()) {
            raw_value.to_string()
        } else {
            format!("{} {}", raw_value, declared_unit)
        },
        "1/A",
        |u| match u {
            "1/A" | "A^-1" | "A-1" => Some((1.0, "1/A".to_string())),
            "1/nm" | "nm^-1" | "nm-1" => Some((0.1, "1/A".to_string())),
            _ => None,
        },
    )
}

/// Exposure time. Canonical: s.
pub fn parse_exposure(raw: &str) -> Quantity {
    parse_scalar(raw, "s", |u| match u {
        "s" => Some((1.0, "s".to_string())),
        "ms" => Some((1e-3, "s".to_string())),
        "min" => Some((60.0, "s".to_string())),
        _ => None,
    })
}

/// Sample/cell thickness. Canonical: cm.
pub fn parse_thickness(raw: &str) -> Quantity {
    parse_scalar(raw, "cm", |u| match u {
        "cm" => Some((1.0, "cm".to_string())),
        "mm" => Some((0.1, "cm".to_string())),
        "m" => Some((100.0, "cm".to_string())),
        _ => None,
    })
}

/// Transmission: dimensionless fraction (or percent). No canonical unit.
pub fn parse_transmission(raw: &str) -> Quantity {
    parse_scalar(raw, "fraction", |u| match u {
        "fraction" | "" => Some((1.0, "fraction".to_string())),
        "percent" | "%" => Some((0.01, "fraction".to_string())),
        _ => None,
    })
}

/// Intensity axis. Canonical representation is the imported one: without an
/// absolute calibration constant intensities are *not* transformed; we only
/// record the declared unit so every derived value carries its basis.
pub fn parse_intensity(raw_value: &str, declared_unit: &str) -> Quantity {
    parse_scalar(
        &format!("{} {}", raw_value.trim(), declared_unit),
        declared_unit,
        |u| Some((1.0, u.to_string())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q_units_convert_to_per_angstrom() {
        let a = parse_q("0.15", "nm^-1");
        assert!((a.canonical.unwrap() - 0.015).abs() < 1e-12);
        let b = parse_q("0.15 1/A", "1/A");
        assert!((b.canonical.unwrap() - 0.15).abs() < 1e-12);
    }

    #[test]
    fn unconvertible_stays_in_place() {
        let t = parse_thickness("unknown (calipers broken)");
        assert!(t.canonical.is_none());
        assert!(t.diagnostic.is_some());
        assert!(t.raw.contains("calipers"));
    }

    #[test]
    fn transmission_percent() {
        let t = parse_transmission("85 %");
        assert!((t.canonical.unwrap() - 0.85).abs() < 1e-10);
    }
}
