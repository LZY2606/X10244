use serde::{Deserialize, Serialize};

pub const RULE_VERSION_NORMALIZE: &str = "norm-1.0";
pub const RULE_VERSION_SUBTRACT: &str = "sub-1.0";
pub const RULE_VERSION_MODEL: &str = "model-1.0";

pub const D_QGRID_MISMATCH: &str = "QGRID_MISMATCH";
pub const D_NO_OVERLAP: &str = "NO_OVERLAP";
pub const D_SCALE_UNCERTAINTY: &str = "SCALE_UNCERTAINTY";
pub const D_NEGATIVE_INTENSITY: &str = "NEGATIVE_INTENSITY";
pub const D_INTERVAL_OVERLAP: &str = "INTERVAL_OVERLAP";
pub const D_BUFFER_VERSION_SWITCH: &str = "BUFFER_VERSION_SWITCH";
pub const D_MULTI_CANDIDATE: &str = "MULTI_CANDIDATE";
pub const D_UNIT_UNCONVERTIBLE: &str = "UNIT_UNCONVERTIBLE";
pub const D_INTERPOLATION_APPLIED: &str = "INTERPOLATION_APPLIED";
pub const D_POINT_OUTSIDE_OVERLAP: &str = "POINT_OUTSIDE_OVERLAP";
pub const D_NORMALIZATION_UNCERTAINTY: &str = "NORMALIZATION_UNCERTAINTY";
pub const D_GUINIER_EXCLUDED: &str = "GUINIER_EXCLUDED_NONPOSITIVE";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    #[serde(default)]
    pub context: serde_json::Value,
}

impl Diagnostic {
    pub fn new(code: &str, severity: &str, message: impl Into<String>) -> Self {
        Diagnostic {
            code: code.to_string(),
            severity: severity.to_string(),
            message: message.into(),
            context: serde_json::Value::Null,
        }
    }

    pub fn context(mut self, context: serde_json::Value) -> Self {
        self.context = context;
        self

    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RawQuantity {
    pub raw: String,
    pub unit: Option<String>,
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<Diagnostic>,
}

pub fn parse_quantity(raw: &str, unit: Option<&str>, kind: QuantityKind) -> RawQuantity {
    let unit_norm = unit.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
    let parsed: Option<f64> = raw.trim().parse::<f64>().ok();
    let (factor, ok_unit): (f64, bool) = match (kind, unit_norm.as_deref()) {
        (QuantityKind::Thickness, Some("cm")) | (QuantityKind::Thickness, None) => (1.0, true),
        (QuantityKind::Thickness, Some("mm")) => (0.1, true),
        (QuantityKind::Transmission, None)
        | (QuantityKind::Transmission, Some("ratio"))
        | (QuantityKind::Transmission, Some("T")) => (1.0, true),
        (QuantityKind::Exposure, Some("s")) | (QuantityKind::Exposure, None) => (1.0, true),
        (QuantityKind::Exposure, Some("ms")) => (0.001, true),
        _ => (1.0, false),
    };
    let value = if ok_unit {
        parsed.map(|v| v * factor)
    } else {
        None
    };
    let diagnostic = if parsed.is_none() {
        Some(Diagnostic::new(
            D_UNIT_UNCONVERTIBLE,
            "error",
            format!("原始值 {raw:?} 无法解析为数值，保留在原始位置"),
        ))
    } else if !ok_unit {
        Some(Diagnostic::new(
            D_UNIT_UNCONVERTIBLE,
            "error",
            format!(
                "原始值 {raw:?} 的单位 {:?} 无法转换为目标单位，保留原始表示",
                unit_norm
            ),
        ))
    } else {
        None
    };
    RawQuantity {
        raw: raw.trim().to_string(),
        unit: unit_norm,
        value,
        diagnostic,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuantityKind {
    Thickness,
    Transmission,
    Exposure,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CurvePoint {
    pub id: String,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    pub id: String,
    pub label: String,
    pub q_unit: String,
    pub i_unit: String,
    pub points: Vec<CurvePoint>,
}

impl Curve {
    pub fn new(id: impl Into<String>, label: impl Into<String>, q_unit: &str, i_unit: &str) -> Self {
        Curve {
            id: id.into(),
            label: label.into(),
            q_unit: q_unit.to_string(),
            i_unit: i_unit.to_string(),
            points: Vec::new(),
        }
    }

    pub fn push(&mut self, q: f64, i: f64, sigma: f64) {
        let id = format!("{}/p{:03}", self.id, self.points.len());
        self.points.push(CurvePoint { id, q, i, sigma });
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BufferCandidate {
    pub id: String,
    pub label: String,
    pub buffer_version: String,
    pub curve: Curve,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InstrumentResponse {
    pub id: String,
    pub label: String,
    pub source: String,
    pub curve: Curve,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MaskSource {
    pub id: String,
    pub detector: String,
    pub mask_file: String,
    pub beamstop_region: String,
    pub detector_distance: RawQuantity,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExposureRecord {
    pub id: String,
    pub label: String,
    pub sample_thickness: RawQuantity,
    pub transmission: RawQuantity,
    pub exposure_time: RawQuantity,
    pub sample_curve: Curve,
    pub buffer_candidates: Vec<BufferCandidate>,
    pub instrument_response: Option<InstrumentResponse>,
    pub mask_source: Option<MaskSource>,
    pub diagnostics: Vec<Diagnostic>,
}
