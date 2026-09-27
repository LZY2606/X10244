//! Raw records imported from 2D integration: original q/I/error plus
//! acquisition metadata, buffer candidates, instrument response and masks.
//!
//! The raw representation is preserved verbatim (`raw_text`, original units);
//! normalization never mutates a record.

use super::units::PhysVal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Sample,
    Buffer,
    Other,
}

/// Provenance of one raw input fragment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRef {
    pub kind: String,
    /// e.g. file name, instrument export id
    pub name: String,
    /// sha256-ish checksum of the fragment if known
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskSpec {
    pub source: SourceRef,
    /// q intervals kept (in the record's original q unit), [lo, hi]
    pub keep_q: Vec<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseSpec {
    pub source: SourceRef,
    /// Multiplicative detector response sampled at grid_q; applied I /= r(q).
    pub grid_q: Vec<f64>,
    pub values: Vec<f64>,
    pub q_unit: String,
}

/// A buffer measurement that may be subtracted from a sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BufferCandidate {
    pub record_id: String,
    pub label: String,
    /// Optional a-priori concentration ratio c_buffer/c_sample.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawPoint {
    pub q: PhysVal,
    pub i: PhysVal,
    pub sigma: PhysVal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRecord {
    /// Logical record identity (stable across revisions of the same record).
    pub name: String,
    pub version: u32,
    pub kind: RecordKind,
    pub points: Vec<RawPoint>,
    pub exposure: PhysVal,
    pub thickness: PhysVal,
    pub transmission: PhysVal,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub buffer_candidates: Vec<BufferCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<ResponseSpec>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub masks: Vec<MaskSpec>,
    pub sources: Vec<SourceRef>,
    /// Original imported text, preserved unchanged.
    pub raw_text: String,
    /// Optional free-form note (e.g. revision reason).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl RawRecord {
    /// Stable version id derived from the full preserved content.
    pub fn version_id(&self) -> super::id::Sid {
        super::id::hid_with("raw-record-v1", self)
    }
}
