//! Units, coordinate bases and values that remember their raw representation.
//!
//! Imported values keep the exact original textual form. Derived layers work
//! only on the normalized (canonical) representation. A value that cannot be
//! converted stays in place (`converted = None`) and carries a diagnostic
//! instead of being dropped for plotting.

use super::Diagnostic;

/// Scattering-vector coordinate basis. Canonical: `1/angstrom`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum QUnit {
    PerAngstrom,
    PerNanometer,
}

impl QUnit {
    pub fn canonical(&self) -> &'static str {
        "1/A"
    }
    pub fn label(&self) -> &'static str {
        match self {
            QUnit::PerAngstrom => "1/A",
            QUnit::PerNanometer => "1/nm",
        }
    }
    /// Multiplicative factor turning a number in this unit into 1/angstrom.
    pub fn to_per_angstrom(&self) -> f64 {
        match self {
            QUnit::PerAngstrom => 1.0,
            QUnit::PerNanometer => 0.1,
        }
    }
    pub fn parse(label: &str) -> Option<QUnit> {
        let s = label.trim().to_ascii_lowercase().replace(' ', "");
        match s.as_str() {
            "1/a" | "a^-1" | "å^-1" | "angstrom^-1" | "a-1" => Some(QUnit::PerAngstrom),
            "1/nm" | "nm^-1" | "nm-1" => Some(QUnit::PerNanometer),
            _ => None,
        }
    }
}

/// Intensity basis. Canonical absolute scale: `1/cm` (macroscopic cross section).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IUnit {
    /// Arbitrary detector counts per second (raw integrated image).
    Arbitrary,
    /// Absolute scale cm^-1 (water calibration already applied).
    PerCm,
}

impl IUnit {
    pub fn label(&self) -> &'static str {
        match self {
            IUnit::Arbitrary => "arb. units",
            IUnit::PerCm => "1/cm",
        }
    }
    pub fn parse(label: &str) -> Option<IUnit> {
        let s = label.trim().to_ascii_lowercase();
        if s.contains("arb") {
            Some(IUnit::Arbitrary)
        } else if s.contains("/cm") || s.contains("cm^-1") || s.contains("cm-1") {
            Some(IUnit::PerCm)
        } else {
            None
        }
    }
}

/// Length units used for sample thickness. Canonical: centimetre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LengthUnit {
    Mm,
    Cm,
}

impl LengthUnit {
    pub fn to_cm(&self) -> f64 {
        match self {
            LengthUnit::Mm => 0.1,
            LengthUnit::Cm => 1.0,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            LengthUnit::Mm => "mm",
            LengthUnit::Cm => "cm",
        }
    }
    pub fn parse(label: &str) -> Option<LengthUnit> {
        match label.trim().to_ascii_lowercase().as_str() {
            "mm" => Some(LengthUnit::Mm),
            "cm" => Some(LengthUnit::Cm),
            _ => None,
        }
    }
}

/// A scalar with its original textual representation preserved.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Scalar {
    pub raw: String,
    pub raw_unit: Option<String>,
    pub value: f64,
}

impl Scalar {
    pub fn new(raw: impl Into<String>, value: f64, raw_unit: Option<String>) -> Self {
        Scalar { raw: raw.into(), raw_unit, value }
    }
    pub fn from_f64(value: f64) -> Self {
        Scalar { raw: format_value(value), raw_unit: None, value }
    }
}

/// A parsed data cell: the original text is always retained.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawCell {
    pub raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
}

impl RawCell {
    pub fn parse(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let trimmed = raw.trim();
        let value = trimmed.parse::<f64>().ok().filter(|v| v.is_finite());
        RawCell { raw, value }
    }
    pub fn from_f64(v: f64) -> Self {
        RawCell { raw: format_value(v), value: Some(v) }
    }
}

pub fn format_value(v: f64) -> String {
    if v == 0.0 {
        "0".to_string()
    } else if v.abs() >= 1e5 || v.abs() < 1e-4 {
        format!("{:.6e}", v)
    } else {
        let s = format!("{:.6}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Exposure time in seconds (always positive for normalized intensity).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExposureSec(pub f64);

/// Transmission fraction, dimensionless, 0 < T <= 1.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transmission(pub f64);

/// Validate the normalization scalar inputs, returning diagnostics rather than
/// silently inventing values.
pub fn validate_norm_inputs(
    exposure_s: f64,
    thickness_cm: f64,
    transmission: f64,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    if !(exposure_s.is_finite() && exposure_s > 0.0) {
        diags.push(Diagnostic::err(
            "EXPOSURE_INVALID",
            "exposure time must be finite and strictly positive",
        ));
    }
    if !(thickness_cm.is_finite() && thickness_cm > 0.0) {
        diags.push(Diagnostic::err(
            "THICKNESS_INVALID",
            "sample thickness must be finite and strictly positive",
        ));
    }
    if !(transmission.is_finite() && transmission > 0.0 && transmission <= 1.0 + 1e-9) {
        diags.push(Diagnostic::err(
            "TRANSMISSION_INVALID",
            "transmission must satisfy 0 < T <= 1",
        ));
    }
    diags
}
