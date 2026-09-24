//! Quantities carry their unit and coordinate basis. Import keeps the raw
//! textual representation; the derived layer canonicalizes known units and
//! leaves unconvertible values in place with a diagnostic instead of dropping
//! them to please a chart.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Measured {
    pub value: f64,
    pub raw: String,
    pub unit: String,
    pub canonical: Option<f64>,
    pub canonical_unit: Option<String>,
    pub convertible: bool,
    pub diagnostic: Option<String>,
}

impl Measured {
    pub fn parse(value: &str, unit: &str, kind: UnitKind) -> Self {
        let v: f64 = value.trim().parse().unwrap_or(f64::NAN);
        let (canon, cu, ok, diag) = if v.is_nan() {
            (None, None, false, Some(format!("non-numeric value {value:?} kept at origin")))
        } else {
            match (kind, unit.trim()) {
                (UnitKind::Q, "A^-1") | (UnitKind::Q, "Å^-1") | (UnitKind::Q, "1/A") => {
                    (Some(v * 10.0), Some("nm^-1".to_string()), true, None)
                }
                (UnitKind::Q, "nm^-1") | (UnitKind::Q, "1/nm") => {
                    (Some(v), Some("nm^-1".to_string()), true, None)
                }
                (UnitKind::Intensity, "a.u.") | (UnitKind::Intensity, "cm^-1")
                | (UnitKind::Intensity, "1/(cm sr)") => {
                    (Some(v), Some(unit.trim().to_string()), true, None)
                }
                (UnitKind::Time, "s") => (Some(v), Some("s".to_string()), true, None),
                (UnitKind::Time, "ms") => (Some(v * 1e-3), Some("s".to_string()), true, None),
                (UnitKind::Length, "mm") => (Some(v), Some("cm".to_string()), true, None),
                (UnitKind::Length, "cm") => (Some(v), Some("cm".to_string()), true, None),
                (UnitKind::Dimensionless, "") | (UnitKind::Dimensionless, "-") => {
                    (Some(v), Some(String::new()), true, None)
                }
                _ => (
                    None,
                    None,
                    false,
                    Some(format!("unconvertible unit {:?} for {:?}; value retained", unit, kind)),
                ),
            }
        };
        Measured {
            value: v,
            raw: value.trim().to_string(),
            unit: unit.trim().to_string(),
            canonical: if cu.as_deref() == Some(unit.trim()) { Some(v) } else { canon },
            canonical_unit: cu,
            convertible: ok,
            diagnostic: diag,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitKind {
    Q,
    Intensity,
    Time,
    Length,
    Dimensionless,
}

impl std::fmt::Display for UnitKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            UnitKind::Q => "q",
            UnitKind::Intensity => "intensity",
            UnitKind::Time => "time",
            UnitKind::Length => "length",
            UnitKind::Dimensionless => "dimensionless",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_known_units_and_keeps_raw() {
        let m = Measured::parse("0.012", "A^-1", UnitKind::Q);
        assert!(m.convertible);
        assert!((m.canonical.unwrap() - 0.12).abs() < 1e-12);
        assert_eq!(m.canonical_unit.as_deref(), Some("nm^-1"));
        assert_eq!(m.raw, "0.012");
    }

    #[test]
    fn unknown_unit_is_kept_with_diagnostic() {
        let m = Measured::parse("3", "furlongs", UnitKind::Length);
        assert!(!m.convertible);
        assert!(m.canonical.is_none());
        assert!(m.diagnostic.is_some());
        assert_eq!(m.raw, "3");
    }

    #[test]
    fn non_numeric_value_is_not_dropped() {
        let m = Measured::parse("N/A", "s", UnitKind::Time);
        assert!(!m.convertible);
        assert!(m.value.is_nan());
    }
}
