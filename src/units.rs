//! Unit parsing and conversion.
//!
//! Import stores the original representation verbatim. This module produces the
//! canonical value used by the derived layer. A value that cannot be converted
//! is returned with `value_si = None` and a diagnostic; it is never deleted to
//! satisfy a plotting routine.

use crate::domain::Qty;

/// Physical coordinate/unit family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Q,
    Intensity,
    Error,
    Exposure,
    Thickness,
    Transmission,
}

impl Family {
    fn canonical(&self) -> &'static str {
        match self {
            Family::Q => "A^-1",
            Family::Intensity => "count",
            Family::Error => "count",
            Family::Exposure => "s",
            Family::Thickness => "cm",
            Family::Transmission => "dimensionless",
        }
    }
}

fn parse_f64(raw: &str) -> Option<f64> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    let v: f64 = t.parse().ok()?;
    if v.is_finite() { Some(v) } else { None }
}

/// Multiplier from a supplied unit to the family's canonical unit.
fn factor(family: Family, unit: &str) -> Option<f64> {
    let u = unit.trim().to_lowercase();
    let u = u.replace('å', "a").replace("ång", "ang");
    Some(match family {
        Family::Q => match u.as_str() {
            "a^-1" | "1/a" | "angstrom^-1" | "angstrom-1" | "a-1" => 1.0,
            "nm^-1" | "1/nm" | "nm-1" => 0.1,
            _ => return None,
        },
        Family::Intensity | Family::Error => match u.as_str() {
            "count" | "counts" | "cnt" | "cnts" | "" => 1.0,
            "cm^-1" | "1/cm" | "cm-1" => 1.0,
            _ => return None,
        },
        Family::Exposure => match u.as_str() {
            "s" | "sec" | "seconds" | "second" => 1.0,
            "ms" => 0.001,
            _ => return None,
        },
        Family::Thickness => match u.as_str() {
            "cm" => 1.0,
            "mm" => 0.1,
            "m" => 100.0,
            _ => return None,
        },
        Family::Transmission => match u.as_str() {
            "" | "t" | "dimensionless" | "ratio" => 1.0,
            _ => return None,
        },
    })
}

/// Build a [`Qty`] from an original token and unit, converting when possible.
pub fn qty(raw: &str, unit: &str, family: Family) -> Qty {
    let canonical = family.canonical();
    let mut out = Qty::raw(raw, unit, canonical);
    match (parse_f64(raw), factor(family, unit)) {
        (Some(v), Some(f)) => out.value_si = Some(v * f),
        (None, _) => {
            out.diagnostic = Some(format!("无法解析数值：'{}'", raw));
        }
        (_, None) => {
            out.diagnostic = Some(format!("无法把单位 '{}' 转换为 {}", unit, canonical));
        }
    }
    out
}

/// True when every required scalar quantity is usable.
pub fn all_usable(qtys: &[&Qty]) -> bool {
    qtys.iter().all(|q| q.ok())
}
