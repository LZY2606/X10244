//! Core domain vocabulary for the SAXS subtraction bench.
//!
//! The storage model keeps the raw representation (`text` + `unit`) of every
//! imported quantity. The derived layer is where normalization to a canonical
//! coordinate/unit basis happens. Values that cannot be converted stay in place
//! and carry a diagnostic instead of being dropped.

use serde::{Deserialize, Serialize};

/// A measured quantity exactly as imported, plus the canonical interpretation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    /// Original textual representation from the source record.
    pub text: String,
    /// Declared unit from the source record (kept verbatim).
    pub unit: String,
    /// Parsed numeric value in the declared unit, if parseable.
    pub value: Option<f64>,
    /// Canonical value after unit conversion, if convertible.
    pub canonical: Option<f64>,
    /// Canonical unit this domain uses internally, when applicable.
    pub canonical_unit: Option<String>,
    /// Diagnostics explaining why conversion was impossible (value retained).
    pub diagnostic: Option<String>,
}

impl Quantity {
    pub fn raw(text: impl Into<String>, unit: impl Into<String>) -> Self {
        let (text, unit) = (text.into(), unit.into());
        let value = text.trim().parse::<f64>().ok();
        let mut q = Quantity {
            text,
            unit,
            value,
            canonical: None,
            canonical_unit: None,
            diagnostic: None,
        };
        q.recompute();
        q
    }

    pub fn with_diag(mut self, msg: impl Into<String>) -> Self {
        self.diagnostic = Some(msg.into());
        self
    }

    pub fn recompute(&mut self) {
        let Some(v) = self.value else {
            self.diagnostic = Some(format!("unparseable numeric value: {:?}", self.text));
            return;
        };
        match self.unit.as_str() {
            // canonical q basis is nm^-1
            "nm^-1" | "1/nm" => {
                self.canonical = Some(v);
                self.canonical_unit = Some("nm^-1".to_string());
            }
            "A^-1" | "angstrom^-1" => {
                self.canonical = Some(v * 10.0);
                self.canonical_unit = Some("nm^-1".to_string());
            }
            "cm^-1" | "1/cm" => {
                self.canonical = Some(v * 1.0e-7);
                self.canonical_unit = Some("nm^-1".to_string());
            }
            // canonical transmission is dimensionless fraction
            "" | "frac" | "fraction" | "dimensionless" => {
                self.canonical = Some(v);
                self.canonical_unit = Some("dimensionless".to_string());
            }
            "percent" | "%" => {
                self.canonical = Some(v / 100.0);
                self.canonical_unit = Some("dimensionless".to_string());
            }
            // canonical exposure is seconds
            "s" | "sec" | "seconds" => {
                self.canonical = Some(v);
                self.canonical_unit = Some("s".to_string());
            }
            "ms" => {
                self.canonical = Some(v / 1000.0);
                self.canonical_unit = Some("s".to_string());
            }
            // canonical thickness is mm
            "mm" => {
                self.canonical = Some(v);
                self.canonical_unit = Some("mm".to_string());
            }
            "cm" => {
                self.canonical = Some(v * 10.0);
                self.canonical_unit = Some("mm".to_string());
            }
            "um" => {
                self.canonical = Some(v / 1000.0);
                self.canonical_unit = Some("mm".to_string());
            }
            // canonical intensity is count/s
            "count/s" | "cps" => {
                self.canonical = Some(v);
                self.canonical_unit = Some("count/s".to_string());
            }
            "count" | "counts" => {
                self.canonical = Some(v);
                self.canonical_unit = Some("count".to_string());
            }
            other => {
                self.diagnostic =
                    Some(format!("unknown unit {other:?}; value kept in original representation"));
            }
        }
    }
}

/// One row of an azimuthally integrated 2D image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawPoint {
    pub q: Quantity,
    pub i: Quantity,
    pub sigma: Quantity,
    /// Whether this point is masked out (e.g. beamstop/shadow).
    pub masked: bool,
    /// Free-form mask reason preserved from the mask source.
    pub mask_reason: Option<String>,
}

/// An imported 1D SAXS curve with the experimental metadata it was measured with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCurve {
    pub id: String,
    /// Logical role/name, e.g. "sample" or "buffer". Versioned revisions keep
    /// the same logical name but carry a new `version`.
    pub logical: String,
    pub version: String,
    pub kind: CurveKind,
    pub points: Vec<RawPoint>,
    pub exposure: Quantity,
    pub transmission: Quantity,
    pub thickness: Quantity,
    /// Human readable provenance describing the file / mask / instrument.
    pub source: SourceInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CurveKind {
    Sample,
    Buffer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SourceInfo {
    pub integration_file: String,
    pub mask_file: String,
    pub instrument_config: String,
    pub imported_at: String,
    pub note: String,
}

/// q-dependent detector response calibration. Kept as its own traceable source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentResponse {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Sorted by q_nm (canonical nm^-1). Multiplier applied to I during norm.
    pub q_nm: Vec<f64>,
    pub gain: Vec<f64>,
    pub source: SourceInfo,
}

/// Mask provenance. Per-point masking additionally lives on `RawPoint`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskSource {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub source: SourceInfo,
}

/// Everything needed to reproduce a normalization/subtraction lineage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ImportBundle {
    pub curves: Vec<RawCurve>,
    pub responses: Vec<InstrumentResponse>,
    pub masks: Vec<MaskSource>,
}

/// Status of a derived object. Multiple flags can coexist; nothing is erased to
/// force a single "clean" answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Findings {
    /// True if every required input converted and the operation fully succeeded.
    pub ok: bool,
    pub codes: Vec<String>,
    pub messages: Vec<String>,
}

impl Findings {
    pub fn ok() -> Self {
        Findings { ok: true, codes: vec![], messages: vec![] }
    }
    pub fn fail(mut self, code: &str, msg: impl Into<String>) -> Self {
        self.ok = false;
        self.codes.push(code.to_string());
        self.messages.push(msg.into());
        self
    }
    pub fn warn(mut self, code: &str, msg: impl Into<String>) -> Self {
        self.codes.push(code.to_string());
        self.messages.push(msg.into());
        self
    }
    pub fn has(&self, code: &str) -> bool {
        self.codes.iter().any(|c| c == code)
    }
}
