//! Domain core: quantities, curves, normalization, subtraction and region evidence.
//!
//! The rules implemented here are the invariants of the whole product:
//! * q grids that differ are never subtracted point-for-point; buffer
//!   intensity is interpolated and only over the strict overlap range.
//! * uncertainty always carries statistical + normalization + background
//!   scale terms.
//! * Guinier and Porod regions are independent evidence; nothing is ever
//!   auto-joined into a single model.

pub mod units;
pub mod curve;
pub mod normal;
pub mod subtract;
pub mod regions;
pub mod model;

pub use units::*;
pub use curve::*;
pub use normal::*;
pub use subtract::*;
pub use regions::*;

/// Domain rule version, recorded on every artifact for provenance replay.
pub const RULE_VERSION: &str = "rules-1.0.0";

/// A structured diagnostic emitted while interpreting domain data.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    /// Optional stable reference (point id suffix, candidate id, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Diagnostic {
    pub fn new(code: impl Into<String>, severity: Severity, message: impl Into<String>) -> Self {
        Diagnostic { code: code.into(), severity, message: message.into(), target: None }
    }
    pub fn warn(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Warning, message)
    }
    pub fn err(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Error, message)
    }
    pub fn info(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Info, message)
    }
    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }
}
