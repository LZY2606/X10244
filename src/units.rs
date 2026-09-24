//! Unit / coordinate-basis handling.
//!
//! Rule: raw representations are preserved at import; only the derived layer
//! canonicalises. Values that cannot be converted stay in place and produce a
//! diagnostic instead of being silently dropped.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Qty {
    pub value: f64,
    pub unit: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    Q,           // canonical: 1/nm
    Length,      // canonical: mm
    Time,        // canonical: s
    Dimensionless,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConvErr {
    UnknownUnit(String),
    Incompatible { unit: String, dim: Dim },
}

/// Canonicalisation factor to the canonical unit of `dim`.
fn factor(unit: &str, dim: Dim) -> Option<f64> {
    let u = unit.trim();
    match dim {
        Dim::Q => match u {
            "1/nm" | "nm^-1" | "nm-1" => Some(1.0),
            "1/angstrom" | "1/A" | "A^-1" | "angstrom^-1" => Some(10.0), // 1 A^-1 = 10 nm^-1
            _ => None,
        },
        Dim::Length => match u {
            "mm" => Some(1.0),
            "cm" => Some(10.0),
            "m" => Some(1000.0),
            "um" => Some(0.001),
            _ => None,
        },
        Dim::Time => match u {
            "s" => Some(1.0),
            "ms" => Some(0.001),
            "min" => Some(60.0),
            _ => None,
        },
        Dim::Dimensionless => match u {
            "" | "1" => Some(1.0),
            _ => None,
        },
    }
}

pub fn to_canonical(q: &Qty, dim: Dim) -> Result<f64, ConvErr> {
    match factor(&q.unit, dim) {
        Some(f) => Ok(q.value * f),
        None => {
            // Distinguish "unknown unit entirely" from "known but wrong dimension".
            let known_anywhere = [Dim::Q, Dim::Length, Dim::Time, Dim::Dimensionless]
                .iter()
                .any(|d| factor(&q.unit, *d).is_some());
            if known_anywhere {
                Err(ConvErr::Incompatible { unit: q.unit.clone(), dim })
            } else {
                Err(ConvErr::UnknownUnit(q.unit.clone()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_known_units() {
        assert_eq!(to_canonical(&Qty { value: 1.0, unit: "1/A".into() }, Dim::Q).unwrap(), 10.0);
        assert_eq!(to_canonical(&Qty { value: 0.15, unit: "cm".into() }, Dim::Length).unwrap(), 1.5);
        assert_eq!(to_canonical(&Qty { value: 0.42, unit: "1".into() }, Dim::Dimensionless).unwrap(), 0.42);
    }

    #[test]
    fn rejects_unknown_unit() {
        let e = to_canonical(&Qty { value: 3.0, unit: "marks".into() }, Dim::Length).unwrap_err();
        assert_eq!(e, ConvErr::UnknownUnit("marks".into()));
    }

    #[test]
    fn rejects_incompatible_dimension() {
        let e = to_canonical(&Qty { value: 2.0, unit: "s".into() }, Dim::Length).unwrap_err();
        assert!(matches!(e, ConvErr::Incompatible { .. }));
    }
}
