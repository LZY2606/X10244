//! Physical quantities with units. Original textual representation is always
//! retained; only the derived layer converts to canonical base units.
//! Base units: q [A^-1], length [mm], time [s], transmission [1], I [counts],
//! normalized intensity [counts/(s*mm)].
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tagged {
    pub value: f64,
    pub unit: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QPoint {
    pub value: f64,
    /// canonical coordinate frame is always inverse Angstrom
    pub unit: String,
}

fn factor_for(unit: &str, groups: &[&[&str]], factors: &[f64]) -> Option<f64> {
    let u = unit.trim();
    groups
        .iter()
        .position(|g| g.contains(&u))
        .map(|i| factors[i])
}

/// Convert q values to canonical A^-1.
pub fn q_factor(unit: &str) -> Option<f64> {
    factor_for(
        unit,
        &[
            &["A^-1", "Å^-1", "1/A", "A-1"],
            &["nm^-1", "1/nm", "nm-1"],
        ],
        &[1.0, 0.1],
    )
}

/// Convert length to mm.
pub fn length_factor(unit: &str) -> Option<f64> {
    factor_for(
        unit,
        &[
            &["mm"],
            &["cm"],
            &["m"],
            &["um", "µm"],
            &["A", "Å"],
        ],
        &[1.0, 10.0, 1000.0, 1e-3, 1e-7],
    )
}

/// Convert time to seconds.
pub fn time_factor(unit: &str) -> Option<f64> {
    factor_for(
        unit,
        &[&["s", "sec", "seconds"], &["ms"], &["min", "minutes"], &["h", "hours"]],
        &[1.0, 1e-3, 60.0, 3600.0],
    )
}

/// All physical units carry explicit reference frames; transmission is
/// dimensionless and only plain "1" / "" is accepted.
pub fn dimensionless_factor(unit: &str) -> Option<f64> {
    match unit.trim() {
        "" | "1" | "ratio" => Some(1.0),
        "%" => Some(0.01),
        _ => None,
    }
}
