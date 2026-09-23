use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QUnit {
    AngstromInv,
    NmInv,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IUnit {
    CmInv,
    Arbitrary,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub subject: Option<String>,
}

impl Diagnostic {
    pub fn new(code: &str, message: impl Into<String>, subject: Option<String>) -> Self {
        Diagnostic { code: code.to_string(), message: message.into(), subject }
    }
}

impl QUnit {
    pub fn parse(s: &str) -> Option<QUnit> {
        match s.trim() {
            "A^-1" | "1/A" | "angstrom^-1" | "Angstrom^-1" => Some(QUnit::AngstromInv),
            "nm^-1" | "1/nm" => Some(QUnit::NmInv),
            _ => None,
        }
    }
    pub fn factor_to_angstrom(&self) -> f64 {
        match self {
            QUnit::AngstromInv => 1.0,
            QUnit::NmInv => 0.1,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            QUnit::AngstromInv => "A^-1",
            QUnit::NmInv => "nm^-1",
        }
    }
}

impl IUnit {
    pub fn parse(s: &str) -> Option<IUnit> {
        match s.trim() {
            "cm^-1" | "1/cm" => Some(IUnit::CmInv),
            "arb" | "arbitrary" | "counts" => Some(IUnit::Arbitrary),
            _ => None,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            IUnit::CmInv => "cm^-1",
            IUnit::Arbitrary => "arb",
        }
    }
}

/// A scalar domain quantity that always carries its unit and uncertainty.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
    pub sigma: f64,
}
