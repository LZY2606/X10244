//! Derived layer: normalized curves, background subtraction and multi-region
//! model evidence. These structs only appear after normalization; raw inputs
//! remain in [`crate::domain`].

use crate::domain::Findings;
use serde::{Deserialize, Serialize};

/// A normalized point on a canonical nm^-1 grid, in count/s after exposure,
/// transmission, thickness and detector-response correction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormPoint {
    pub id: String,
    pub index: usize,
    pub q_nm: f64,
    pub i: f64,
    /// Absolute 1-sigma intensity uncertainty (normalization propagated).
    pub sigma: f64,
    pub masked: bool,
    /// True when detector response did not cover this q (kept, flagged, not fit).
    pub uncovered_response: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedCurve {
    pub id: String,
    pub raw_id: String,
    pub logical: String,
    pub version: String,
    pub response_id: String,
    pub mask_id: String,
    pub points: Vec<NormPoint>,
    /// Normalization factors actually used (canonical units).
    pub exposure_s: f64,
    pub transmission: f64,
    pub thickness_mm: f64,
    pub findings: Findings,
}

/// One point of a background-subtracted curve, defined only on the overlap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPoint {
    pub id: String,
    pub index: usize,
    pub q_nm: f64,
    /// I_sub = I_sample - alpha * I_buffer_interp (no clipping of negatives).
    pub i: f64,
    /// Variance of the buffer normalization/scale factor alpha.
    pub sigma: f64,
    pub negative: bool,
    /// Index of the originating sample point (identity traceability).
    pub sample_index: usize,
    /// Buffer points bracketing q (identities surfaced for provenance).
    pub buffer_lo: usize,
    pub buffer_hi: usize,
    pub interp_weight: f64,
}

/// Background scaling candidate. Several legal candidates may coexist; they are
/// never silently collapsed to one answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScaleCandidate {
    pub id: String,
    pub method: String,
    pub alpha: f64,
    pub sigma_alpha: f64,
    pub q_lo: f64,
    pub q_hi: f64,
    pub points_used: usize,
    pub chi2: f64,
    pub adopted: bool,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subtraction {
    pub id: String,
    pub sample_norm_id: String,
    pub buffer_norm_id: String,
    pub alpha: f64,
    pub sigma_alpha: f64,
    pub scale_candidates: Vec<ScaleCandidate>,
    pub overlap_lo: f64,
    pub overlap_hi: f64,
    pub points: Vec<SubPoint>,
    pub findings: Findings,
}

/// A Guinier linear fit: ln I = ln I0 - q^2 Rg^2 / 3 over a window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuinierFit {
    pub id: String,
    pub q_lo: f64,
    pub q_hi: f64,
    pub points_used: usize,
    pub rg_nm: f64,
    pub i0: f64,
    pub qrg_max: f64,
    pub residual_rms: f64,
    pub residual: Vec<(f64, f64)>,
    pub linear: bool,
    pub rejected_reason: Option<String>,
}

/// A Porod power-law fit: I = A q^{-n} over a high-q window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PorodFit {
    pub id: String,
    pub q_lo: f64,
    pub q_hi: f64,
    pub points_used: usize,
    pub exponent: f64,
    pub amplitude: f64,
    pub residual_rms: f64,
    pub residual: Vec<(f64, f64)>,
    pub linear: bool,
    pub rejected_reason: Option<String>,
}

/// Multi-candidate evidence for one subtracted curve. Guinier and Porod remain
/// separate lists: they are never auto-stitched into a single piecewise model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelAnalysis {
    pub id: String,
    pub subtraction_id: String,
    pub guinier: Vec<GuinierFit>,
    pub porod: Vec<PorodFit>,
    /// Pairs of windows (region, lo..hi) that overlap and cannot both be used.
    pub overlaps: Vec<(String, String)>,
    pub findings: Findings,
}
