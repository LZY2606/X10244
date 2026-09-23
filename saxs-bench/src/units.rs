//! Units and coordinate bases for SAXS quantities.
//!
//! Every domain quantity carries either a physical unit (exposure, thickness,
//! transmission factor) or a coordinate basis (the q grid). Imports keep the
//! *original representation*; only the derived layer normalises. A value that
//! cannot be converted stays in place with a diagnostic instead of being
//! dropped to make a plot work.

use serde::{Deserialize, Serialize};

/// A scalar captured exactly as written, plus any parsed SI-normalised value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    /// Original textual representation from the record.
    pub raw: String,
    /// Parsed value in the source unit.
    pub value: Option<f64>,
    /// Source unit token exactly as imported (e.g. "s", "mm", "1/cm").
    pub unit: String,
}

impl Field {
    pub fn new(raw: impl Into<String>, value: Option<f64>, unit: impl Into<String>) -> Self {
        Self { raw: raw.into(), value, unit: unit.into() }
    }
}

/// Coordinate basis of a q grid. Only reciprocal-space bases are accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QBasis {
    QInvAngstrom,
    QInvNm,
    /// Scattering vector magnitude s = q/(2*pi).
    SInvAngstrom,
    SInvNm,
}

impl QBasis {
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "q_inv_a" | "a^-1" | "1/a" | "angstrom^-1" | "å^-1" | "å-1" => {
                Some(QBasis::QInvAngstrom)
            }
            "q_inv_nm" | "nm^-1" | "1/nm" => Some(QBasis::QInvNm),
            "s_inv_a" | "s_a^-1" => Some(QBasis::SInvAngstrom),
            "s_inv_nm" | "s_nm^-1" => Some(QBasis::SInvNm),
            _ => None,
        }
    }

    /// Multiplicative factor converting a coordinate in this basis to q in Å⁻¹.
    pub fn to_q_inv_angstrom(self) -> f64 {
        match self {
            QBasis::QInvAngstrom => 1.0,
            QBasis::QInvNm => 0.1,
            QBasis::SInvAngstrom => std::f64::consts::TAU,
            QBasis::SInvNm => 0.1 * std::f64::consts::TAU,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            QBasis::QInvAngstrom => "q [Å^-1]",
            QBasis::QInvNm => "q [nm^-1]",
            QBasis::SInvAngstrom => "s [Å^-1]",
            QBasis::SInvNm => "s [nm^-1]",
        }
    }
}

/// Convert an exposure field to seconds. Returns None when not convertible;
/// the original field must then be retained in place with a diagnostic.
pub fn exposure_seconds(f: &Field) -> Option<f64> {
    let v = f.value?;
    match f.unit.trim().to_ascii_lowercase().as_str() {
        "s" | "sec" | "seconds" | "second" => Some(v),
        "ms" => Some(v / 1000.0),
        "min" | "minute" | "minutes" => Some(v * 60.0),
        _ => None,
    }
}

/// Convert a thickness field to centimetres (the beam-path convention used for
/// absolute scaling). None means unconvertible.
pub fn thickness_cm(f: &Field) -> Option<f64> {
    let v = f.value?;
    match f.unit.trim().to_ascii_lowercase().as_str() {
        "cm" => Some(v),
        "mm" => Some(v / 10.0),
        "m" => Some(v * 100.0),
        "um" | "µm" => Some(v / 10_000.0),
        "a" | "angstrom" | "ångström" | "å" => Some(v * 1e-8),
        "nm" => Some(v * 1e-7),
        _ => None,
    }
}

/// Transmission is dimensionless (0,1]. A declared factor is used as given; an
/// absorbance style token is *not* silently transformed (diagnostic instead).
pub fn transmission_factor(f: &Field) -> Option<f64> {
    let v = f.value?;
    match f.unit.trim().to_ascii_lowercase().as_str() {
        "" | "1" | "ratio" | "fraction" | "dimensionless" => Some(v),
        _ => None,
    }
}
