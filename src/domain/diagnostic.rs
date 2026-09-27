//! Diagnostics produced while importing or deriving curves.
//!
//! A diagnostic never silently drops a value: unconvertible quantities stay
//! in their original location and are referenced by the diagnostic.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagCode {
    /// A value carried an unrecognized / incompatible unit; left in place.
    UnconvertibleUnit,
    /// Transmission outside (0, 1] (still preserved, normalization blocked).
    BadTransmission,
    /// Non-finite input value.
    NonFinite,
    /// q point excluded by a mask (raw point retained).
    MaskedPoint,
    /// Two curves do not overlap in q.
    NoOverlap,
    /// q grids differ: interpolation used, direct subtraction forbidden.
    GridMismatchInterp,
    /// Overlap exists but too few points for a fitted scale / model fit.
    InsufficientOverlap,
    /// Background scaling uncertain above the reporting threshold.
    ScaleUncertain,
    /// A background-subtracted point is negative.
    NegativeIntensity,
    /// Two model windows overlap in q.
    IntervalOverlap,
    /// More than one model candidate is competitive.
    MultiCandidate,
    /// Buffer candidate switched from a previously used version.
    BufferVersionSwitch,
    /// A parameter override was applied.
    ParameterOverride,
    /// A derivation could not run and produced no result.
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: DiagCode,
    pub severity: Severity,
    /// Human readable explanation.
    pub message: String,
    /// Optional index / key into the original data the diagnostic refers to.
    pub at: Option<String>,
}

impl Diagnostic {
    pub fn new(code: DiagCode, severity: Severity, message: impl Into<String>) -> Self {
        Diagnostic { code, severity, message: message.into(), at: None }
    }

    pub fn at(mut self, at: impl Into<String>) -> Self {
        self.at = Some(at.into());
        self
    }
}

pub type Diags = Vec<Diagnostic>;
