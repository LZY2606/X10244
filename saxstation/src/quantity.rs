//! Quantities, units and coordinate bases.
//!
//! Import keeps the *original* textual representation and the declared unit.
//! Conversion to a canonical base happens only on the derived layer. A value
//! that cannot be converted is kept in place together with a diagnostic
//! instead of being dropped to make the plot work.

use serde::{Deserialize, Serialize};

/// Units understood by the station. `Unknown` keeps the literal token so that
/// nothing is silently discarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    /// reciprocal angstrom 1/A (canonical q base)
    InvAngstrom,
    /// reciprocal nanometre 1/nm
    InvNanometre,
    /// intensity: 1/cm
    InvCm,
    /// intensity: 1/mm
    InvMm,
    /// dimensionless / arbitrary detector units
    Count,
    /// seconds (canonical exposure base)
    Second,
    /// millimetres (canonical thickness base)
    Mm,
    /// centimetres
    Cm,
    /// dimensionless / unitless ratio (transmissions)
    One,
    /// unit not understood; raw token retained
    Unknown(String),
}

impl Unit {
    pub fn parse(token: &str) -> Unit {
        match token.trim().to_ascii_lowercase().as_str() {
            "1/a" | "a^-1" | "angstrom^-1" | "å^-1" | "a-1" | "invangstrom" => Unit::InvAngstrom,
            "1/nm" | "nm^-1" | "nm-1" | "invnanometre" | "invnanometer" => Unit::InvNanometre,
            "1/cm" | "cm^-1" | "cm-1" | "invcm" => Unit::InvCm,
            "1/mm" | "mm^-1" | "mm-1" | "invmm" => Unit::InvMm,
            "count" | "counts" | "a.u." | "au" | "arb" => Unit::Count,
            "s" | "sec" | "second" | "seconds" => Unit::Second,
            "mm" => Unit::Mm,
            "cm" => Unit::Cm,
            "" | "1" | "one" | "ratio" | "dimensionless" => Unit::One,
            other => Unit::Unknown(other.to_string()),
        }
    }
}

/// A measurement preserving the imported representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub value: f64,
    pub unit: Unit,
    /// exactly as found in the source file
    pub raw: String,
}

impl Quantity {
    pub fn new(value: f64, raw: impl Into<String>, unit: Unit) -> Self {
        Quantity { value, unit, raw: raw.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversion {
    pub value: f64,
    pub base: Unit,
}

/// Convert to a canonical base. Returns `None` and leaves a diagnostic when a
/// unit is not understood.
pub fn to_base(q: &Quantity, kind: QuantityKind) -> Result<Conversion, String> {
    use Unit::*;
    let factor = match (kind, &q.unit) {
        (QuantityKind::Q, InvAngstrom) => 1.0,
        (QuantityKind::Q, InvNanometre) => 0.1,
        (QuantityKind::Intensity, InvCm) => 1.0,
        (QuantityKind::Intensity, InvMm) => 0.1,
        (QuantityKind::Intensity, Count) => 1.0,
        (QuantityKind::Exposure, Second) => 1.0,
        (QuantityKind::Thickness, Mm) => 1.0,
        (QuantityKind::Thickness, Cm) => 10.0,
        (QuantityKind::Ratio, One) => 1.0,
        (QuantityKind::Ratio, Count) => 1.0,
        (_, Unknown(token)) => return Err(format!("unknown unit {:?}", token)),
        (kind, unit) => return Err(format!("unit {:?} not valid for {:?}", unit, kind)),
    };
    Ok(Conversion {
        value: q.value * factor,
        base: match kind {
            QuantityKind::Q => InvAngstrom,
            QuantityKind::Intensity => InvCm,
            QuantityKind::Exposure => Second,
            QuantityKind::Thickness => Mm,
            QuantityKind::Ratio => One,
        },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuantityKind {
    Q,
    Intensity,
    Exposure,
    Thickness,
    Ratio,
}
