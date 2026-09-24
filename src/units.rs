//! Units, coordinate bases and raw representation handling.
//!
//! Domain rule: all quantities carry a unit or coordinate basis. Import keeps
//! the original representation; the derivation layer is the only place where
//! canonical normalization happens. Values that cannot be converted stay at
//! their original location with a diagnostic instead of being dropped for
//! plotting.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitKind {
    /// Scattering vector coordinate, canonical base is 1/angstrom.
    Q,
    /// Scattering intensity, canonical base is 1/cm.
    Intensity,
    /// Intensity uncertainty, same base as intensity.
    IntensityError,
    /// Exposure time, canonical base is seconds.
    Time,
    /// Sample thickness, canonical base is cm.
    Length,
    /// Transmission (dimensionless).
    Dimensionless,
    /// Detector pixel coordinate (kept as-is, never converted).
    Pixel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitSpec {
    pub kind: UnitKind,
    /// Original unit token as written by the beamline export, e.g. "nm^-1".
    pub raw: String,
}

impl UnitSpec {
    pub fn q(raw: &str) -> Self {
        Self { kind: UnitKind::Q, raw: raw.to_string() }
    }
    pub fn intensity(raw: &str) -> Self {
        Self { kind: UnitKind::Intensity, raw: raw.to_string() }
    }
    pub fn time(raw: &str) -> Self {
        Self { kind: UnitKind::Time, raw: raw.to_string() }
    }
    pub fn length(raw: &str) -> Self {
        Self { kind: UnitKind::Length, raw: raw.to_string() }
    }
    pub fn dimensionless(raw: &str) -> Self {
        Self { kind: UnitKind::Dimensionless, raw: raw.to_string() }
    }
}

/// Multiplicative factor converting `raw` unit values into the canonical base.
/// Returns None when the unit token is unknown for this kind: the value must
/// remain in place with a diagnostic.
pub fn conversion_factor(kind: UnitKind, raw: &str) -> Option<f64> {
    let t = raw.trim().to_ascii_lowercase().replace(' ', "");
    let factor = match kind {
        UnitKind::Q => match t.as_str() {
            "a^-1" | "å^-1" | "angstrom^-1" | "angstrom^-¹" | "1/a" | "a-1" => 1.0,
            "nm^-1" | "1/nm" | "nm-1" => 10.0,
            _ => return None,
        },
        UnitKind::Intensity | UnitKind::IntensityError => match t.as_str() {
            "cm^-1" | "1/cm" | "cm-1" => 1.0,
            "mm^-1" | "1/mm" | "mm-1" => 0.1,
            _ => return None,
        },
        UnitKind::Time => match t.as_str() {
            "s" | "sec" | "second" | "seconds" => 1.0,
            "ms" => 0.001,
            _ => return None,
        },
        UnitKind::Length => match t.as_str() {
            "cm" => 1.0,
            "mm" => 0.1,
            "m" => 100.0,
            "a" | "angstrom" => 1e-8,
            _ => return None,
        },
        UnitKind::Dimensionless => match t.as_str() {
            "" | "1" | "-" | "t" => 1.0,
            _ => return None,
        },
        UnitKind::Pixel => match t.as_str() {
            "px" | "pixel" => 1.0,
            _ => return None,
        },
    };
    Some(factor)
}

pub fn canonical_label(kind: UnitKind) -> &'static str {
    match kind {
        UnitKind::Q => "1/angstrom",
        UnitKind::Intensity | UnitKind::IntensityError => "1/cm",
        UnitKind::Time => "s",
        UnitKind::Length => "cm",
        UnitKind::Dimensionless => "1",
        UnitKind::Pixel => "px",
    }
}
