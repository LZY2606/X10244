//! Unit / coordinate-basis handling.
//! Import keeps the raw representation; conversion happens only in the
//! derived layer. Values that cannot be converted stay in place and get a
//! diagnostic instead of being dropped for plotting.

use crate::model::{Diagnostic, Quantity};

fn factor(from: &str, to: &str) -> Option<f64> {
    if from == to {
        return Some(1.0);
    }
    match (from, to) {
        ("ms", "s") => Some(1e-3),
        ("s", "ms") => Some(1e3),
        ("cm", "mm") => Some(10.0),
        ("mm", "cm") => Some(0.1),
        ("m", "mm") => Some(1e3),
        ("mm", "m") => Some(1e-3),
        ("1/nm", "1/angstrom") => Some(0.1),
        ("1/angstrom", "1/nm") => Some(10.0),
        ("dimensionless", "dimensionless") => Some(1.0),
        _ => None,
    }
}

pub fn can_convert(from: &str, to: &str) -> bool {
    factor(from, to).is_some()
}

/// Convert a quantity to a target unit. On failure the quantity is returned
/// unchanged together with a diagnostic; nothing is silently dropped.
pub fn convert(q: &Quantity, to: &str) -> Result<Quantity, (Quantity, Diagnostic)> {
    match factor(&q.unit, to) {
        Some(f) => Ok(Quantity {
            value: q.value * f,
            unit: to.to_string(),
            sigma: q.sigma.map(|s| s * f.abs()),
            raw: q.raw.clone(),
        }),
        None => Err((
            q.clone(),
            Diagnostic {
                code: "unit_unconvertible".into(),
                message: format!(
                    "cannot convert '{}' to '{}'; value left at original position",
                    q.unit, to
                ),
                affected: vec![],
            },
        )),
    }
}

/// Parse a raw scalar with unit, e.g. "2500 ms" or "0.152 cm".
pub fn parse_raw(raw: &str) -> Option<Quantity> {
    let mut parts = raw.split_whitespace();
    let value: f64 = parts.next()?.parse().ok()?;
    let unit = parts.next().unwrap_or("dimensionless").to_string();
    Some(Quantity { value, unit, sigma: None, raw: Some(raw.to_string()) })
}
