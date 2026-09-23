//! Quantities that carry a unit or coordinate basis.
//!
//! Importers preserve the original representation (`raw_value`/`raw_unit`).
//! Derived layers convert via `to_canonical`. Values whose unit cannot be
//! converted stay in place and carry a diagnostic; they are never silently
//! dropped to make plots work.

use serde::{Deserialize, Serialize};

/// Physical quantity with measurement value, uncertainty and unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Qty {
    pub value: f64,
    pub sigma: Option<f64>,
    pub unit: String,
    pub raw_value: f64,
    pub raw_unit: String,
    pub basis: String,
}

impl Qty {
    pub fn new(value: f64, unit: &str, basis: &str) -> Self {
        Qty {
            value,
            sigma: None,
            unit: unit.to_string(),
            raw_value: value,
            raw_unit: unit.to_string(),
            basis: basis.to_string(),
        }
    }

    pub fn raw(value: f64, unit: &str) -> Self {
        Self::new(value, unit, "as-imported")
    }

    pub fn with_sigma(mut self, sigma: Option<f64>) -> Self {
        self.sigma = sigma;
        self
    }

    /// Conversion factor `raw_unit -> canonical unit of the given kind`.
    /// `None` means the unit is unknown/unconvertible for that kind.
    pub fn factor(raw_unit: &str, kind: UnitKind) -> Option<f64> {
        let key = raw_unit.trim().to_ascii_lowercase();
        match kind {
            UnitKind::Length => match key.as_str() {
                "mm" => Some(0.1),
                "cm" => Some(1.0),
                "m" => Some(100.0),
                _ => None,
            },
            UnitKind::ScatteringVector => match key.as_str() {
                "a^-1" | "å^-1" | "angstrom^-1" | "å⁻¹" => Some(10.0),
                "nm^-1" => Some(0.1),
                "cm^-1" => Some(1.0),
                _ => None,
            },
            UnitKind::Time => match key.as_str() {
                "ms" => Some(0.001),
                "s" => Some(1.0),
                _ => None,
            },
            UnitKind::Intensity => match key.as_str() {
                "counts" | "count" => Some(1.0),
                "a.u." | "au" | "arb" | "arbitrary" => Some(1.0),
                "cm^-1" | "1/cm" | "cm-1" => Some(1.0),
                _ => None,
            },
            UnitKind::Dimensionless => match key.as_str() {
                "" | "1" | "-" | "ratio" | "dimensionless" => Some(1.0),
                _ => None,
            },
        }
    }

    pub fn canonical_unit(kind: UnitKind) -> &'static str {
        match kind {
            UnitKind::Length => "cm",
            UnitKind::ScatteringVector => "cm^-1",
            UnitKind::Time => "s",
            UnitKind::Intensity => "cm^-1",
            UnitKind::Dimensionless => "1",
        }
    }

    /// Normalize to the canonical unit of `kind`; returns `None` with the raw
    /// representation intact when the unit cannot be converted.
    pub fn to_canonical(&self, kind: UnitKind) -> Option<Self> {
        let f = Self::factor(&self.unit, kind)?;
        Some(Qty {
            value: self.raw_value * f,
            sigma: self.sigma.map(|s| s * f),
            unit: Self::canonical_unit(kind).to_string(),
            raw_value: self.raw_value,
            raw_unit: self.raw_unit.clone(),
            basis: self.basis.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitKind {
    Length,
    ScatteringVector,
    Time,
    Intensity,
    Dimensionless,
}

/// Diagnostics emitted while importing or deriving, kept in the archive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String, // "error" | "warning" | "info"
    pub message: String,
    pub point_id: Option<String>,
}
