//! Units and coordinate bases. Raw representations are kept verbatim; only
//! the derived layer normalizes. Values that cannot be converted stay at the
//! source with a diagnostic instead of being dropped for plotting.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit {
    /// dimension family, e.g. "q", "intensity", "time", "transmission",
    /// "length", "response"
    pub dim: String,
    /// canonical symbol exactly as imported, e.g. "nm^-1", "A^-1"
    pub symbol: String,
}

/// Convert a value expressed in `symbol` to the canonical base unit of `dim`.
/// Returns None with a reason when the unit is unknown / incompatible.
pub fn convert_to_base(value: f64, dim: &str, symbol: &str) -> Result<f64, String> {
    let factor = unit_factor(dim, symbol)?;
    Ok(value * factor)
}

pub fn base_unit(dim: &str) -> Option<&'static str> {
    Some(match dim {
        "q" => "A^-1",
        "intensity" => "rel",
        "time" => "s",
        "transmission" => "1",
        "length" => "mm",
        "response" => "1",
        _ => return None,
    })
}

/// Multiplicative factor from the given symbol to the dim base unit.
pub fn unit_factor(dim: &str, symbol: &str) -> Result<f64, String> {
    let normalized = symbol.replace('Å', "A").replace('µ', "u");
    match (dim, normalized.as_str()) {
        ("q", "A^-1") | ("q", "1/A") | ("q", "angstrom^-1") => Ok(1.0),
        ("q", "nm^-1") | ("q", "1/nm") => Ok(0.1),
        ("intensity", "rel") | ("intensity", "counts/s") | ("intensity", "") => Ok(1.0),
        ("time", "s") | ("time", "sec") | ("time", "seconds") => Ok(1.0),
        ("time", "ms") => Ok(0.001),
        ("transmission", "1") | ("transmission", "") => Ok(1.0),
        ("length", "mm") => Ok(1.0),
        ("length", "cm") => Ok(10.0),
        ("length", "m") => Ok(1000.0),
        ("response", "1") | ("response", "") => Ok(1.0),
        _ => Err(format!("no conversion for dim '{}' unit '{}'", dim, symbol)),
    }
}

/// True when two grids share the same base and points coincide within a
/// relative+absolute tolerance (direct subtraction allowed).
pub fn grids_coincide(a: &[f64], b: &[f64], tol: f64) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).all(|(x, y)| {
        let scale = x.abs().max(y.abs()).max(1e-9);
        (x - y).abs() <= tol * scale
    })
}
