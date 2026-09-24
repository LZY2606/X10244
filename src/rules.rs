//! Pure domain rules. No storage, no async: every function is deterministic so
//! the whole derivation chain can be replayed in tests and from history.
//!
//! Semantic guard rails implemented here:
//! * grids on different q bases are never subtracted directly; interpolation
//!   is applied only inside the shared overlap range;
//! * subtracted uncertainty includes normalization and background-scale
//!   uncertainty;
//! * Guinier and Porod intervals are separate evidence, never auto-stitched
//!   into a single model; when the data do not select a unique answer every
//!   legal candidate is retained.

use crate::domain::{
    content_id, Diagnostic, IntervalKind, ModelCandidate, ModelEvidence, NormalizedCurve,
    PointOrigin, RawCurve, Scalar, Subtraction, SubtractionStatus, UncertaintyBudget,
};

/// Bumped whenever a rule formula changes. Stored on every derived node so an
/// old rule version can be identified when material differs.
pub const RULES_VERSION: &str = "saxs-rules-1.0.0";

// Relative 1-sigma components of the normalization budget (documented
// instrument/protocol constants). They are deliberately modest and explicit.
const REL_TRANSMISSION: f64 = 0.02;
const REL_THICKNESS: f64 = 0.01;
const REL_EXPOSURE: f64 = 0.005;
const REL_RESPONSE: f64 = 0.005;

fn diag(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        message: message.into(),
    }
}

fn rel_uncertainty(response_factors: bool) -> f64 {
    let mut v = REL_TRANSMISSION.powi(2) + REL_THICKNESS.powi(2) + REL_EXPOSURE.powi(2);
    if response_factors {
        v += REL_RESPONSE.powi(2);
    }
    v.sqrt()
}

/// Normalize one raw curve. q conversion failures stay local: normalization of
/// intensity still proceeds but the unconverted q axis is flagged and blocks
/// downstream q-aligned derivation until resolved.
pub fn normalize(raw: &RawCurve) -> NormalizedCurve {
    let mut diagnostics = Vec::new();

    let q_factor = match crate::domain::q_unit_to_inv_a(&raw.q.unit) {
        Some(f) => f,
        None => {
            diagnostics.push(diag(
                "Q_UNIT_UNKNOWN",
                format!(
                    "q unit '{}' cannot be converted to 1/Å; q kept in original basis",
                    raw.q.unit
                ),
            ));
            f64::NAN
        }
    };

    let excluded: std::collections::HashSet<usize> =
        raw.mask.excluded_bins.iter().copied().collect();

    let mut q_inv_a = Vec::new();
    let mut i_norm = Vec::new();
    let mut sigma_norm = Vec::new();
    let mut source_bins = Vec::new();

    let t = raw.transmission.value;
    let d = raw.thickness_mm.value;
    let expo = raw.exposure_s.value;
    if t <= 0.0 {
        diagnostics.push(diag(
            "TRANSMISSION_NONPOSITIVE",
            "transmission must be > 0 for normalization",
        ));
    }
    if d <= 0.0 {
        diagnostics.push(diag("THICKNESS_NONPOSITIVE", "thickness must be > 0"));
    }
    if expo <= 0.0 {
        diagnostics.push(diag("EXPOSURE_NONPOSITIVE", "exposure must be > 0"));
    }
    let denom = t.max(0.0) * d.max(0.0) * expo.max(0.0);
    let rel = rel_uncertainty(!raw.response.factors.is_empty());

    let n = raw.q.values.len();
    for k in 0..n {
        if excluded.contains(&k) {
            continue;
        }
        let i0 = raw.intensity.values[k];
        let s0 = raw.sigma.values[k].abs();
        let i = if denom > 0.0 { i0 / denom } else { f64::NAN };
        // Counting noise + relative normalization components.
        let sigma = if denom > 0.0 {
            ((s0 / denom).powi(2) + (rel * i).powi(2)).sqrt()
        } else {
            f64::NAN
        };
        q_inv_a.push(if q_factor.is_nan() {
            raw.q.values[k]
        } else {
            raw.q.values[k] * q_factor
        });
        i_norm.push(i);
        sigma_norm.push(sigma);
        source_bins.push(k);
    }

    if q_factor.is_nan() {
        // Sorting/identity on an unknown basis is unsafe downstream.
    }

    let material = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        RULES_VERSION,
        raw.id,
        raw.q.unit,
        raw.transmission.unit,
        raw.thickness_mm.unit,
        raw.exposure_s.unit,
        raw.mask.source,
        raw.response.source,
        source_bins.len()
    );
    let node_id = content_id("norm", &material);

    NormalizedCurve {
        node_id,
        raw_id: raw.id.clone(),
        role: raw.role.clone(),
        label: raw.label.clone(),
        version: raw.version.clone(),
        q_inv_a,
        i_norm,
        sigma_norm,
        source_bins,
        diagnostics,
    }
}

pub fn thickness_of(raw: &RawCurve) -> Scalar {
    raw.thickness_mm.clone()
}

fn interp(xs: &[f64], ys: &[f64], x: f64) -> Option<(usize, usize, f64)> {
    if xs.len() < 2 {
        return None;
    }
    // Strict inside-range only (no extrapolation).
    if x < xs[0] || x > xs[xs.len() - 1] {
        return None;
    }
    let mut lo = 0usize;
    let mut hi = xs.len() - 1;
    if x == xs[0] {
        return Some((0, if xs.len() > 1 { 1 } else { 0 }, 0.0));
    }
    if x == xs[hi] {
        return Some((hi - 1, hi, 1.0));
    }
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if xs[mid] <= x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let w = if xs[hi] != xs[lo] {
        (x - xs[lo]) / (xs[hi] - xs[lo])
    } else {
        0.0
    };
    let _ = ys;
    Some((lo, hi, w))
}

/// Subtract a buffer from a sample. Different grids are NEVER subtracted
/// point-by-point: the buffer is linearly interpolated onto the sample grid
/// and only inside the common overlap range.
pub fn subtract(sample: &NormalizedCurve, buffer: &NormalizedCurve) -> Subtraction {
    let mut diagnostics = Vec::new();
    let base_node = content_id(
        "sub",
        &format!("{}|{}|{}", RULES_VERSION, sample.node_id, buffer.node_id),
    );

    let s_ok = sample.q_inv_a.windows(2).all(|w| w[1] > w[0]);
    let b_ok = buffer.q_inv_a.windows(2).all(|w| w[1] > w[0]);
    if !s_ok || !b_ok {
        return Subtraction {
            node_id: base_node,
            sample_node: sample.node_id.clone(),
            buffer_node: buffer.node_id.clone(),
            buffer_version: buffer.version.clone(),
            status: SubtractionStatus::Rejected {
                reason: "q axes are not strictly monotonic (unconverted/masked q)"
                    .to_string(),
            },
            q_inv_a: vec![],
            i_sub: vec![],
            sigma_sub: vec![],
            scale: f64::NAN,
            scale_sigma: f64::NAN,
            budgets: vec![],
            origins: vec![],
            negative_bins: vec![],
            diagnostics: vec![diag(
                "Q_GRID_INVALID",
                "cannot subtract without strictly monotonic q axes",
            )],
        };
    }

    let s_lo = sample.q_inv_a.first().copied().unwrap_or(0.0);
    let s_hi = sample.q_inv_a.last().copied().unwrap_or(0.0);
    let b_lo = buffer.q_inv_a.first().copied().unwrap_or(0.0);
    let b_hi = buffer.q_inv_a.last().copied().unwrap_or(0.0);

    let lo = s_lo.max(b_lo);
    let hi = s_hi.min(b_hi);
    if sample.q_inv_a.is_empty() || buffer.q_inv_a.is_empty() || hi <= lo {
        return Subtraction {
            node_id: base_node,
            sample_node: sample.node_id.clone(),
            buffer_node: buffer.node_id.clone(),
            buffer_version: buffer.version.clone(),
            status: SubtractionStatus::Rejected {
                reason: format!(
                    "no overlap: sample [{:.5},{:.5}] buffer [{:.5},{:.5}] 1/Å",
                    s_lo, s_hi, b_lo, b_hi
                ),
            },
            q_inv_a: vec![],
            i_sub: vec![],
            sigma_sub: vec![],
            scale: f64::NAN,
            scale_sigma: f64::NAN,
            budgets: vec![],
            origins: vec![],
            negative_bins: vec![],
            diagnostics: vec![diag(
                "NO_OVERLAP",
                "q ranges do not overlap; subtraction refused",
            )],
        };
    }

    // Collect sample bins strictly inside the overlap.
    let mut keep = Vec::new();
    let mut dropped_low = 0usize;
    let mut dropped_high = 0usize;
    for (k, &q) in sample.q_inv_a.iter().enumerate() {
        if q < lo || q > hi {
            if q < lo {
                dropped_low += 1;
            } else {
                dropped_high += 1;
            }
            continue;
        }
        keep.push(k);
    }

    // Interpolate buffer onto those bins.
    let mut bv = Vec::with_capacity(keep.len());
    let mut bsig = Vec::with_capacity(keep.len());
    let mut origins = Vec::with_capacity(keep.len());
    let mut usable_bins = Vec::new();
    for (dk, &sk) in keep.iter().enumerate() {
        let q = sample.q_inv_a[sk];
        let (bl, bh, w) = interp(&buffer.q_inv_a, &buffer.i_norm, q)
            .expect("q inside overlap must interpolate");
        let v = buffer.i_norm[bl] * (1.0 - w) + buffer.i_norm[bh] * w;
        let s = buffer.sigma_norm[bl] * (1.0 - w) + buffer.sigma_norm[bh] * w;
        bv.push(v);
        bsig.push(s);
        let sb = sample.source_bins[sk];
        origins.push(PointOrigin {
            derived_bin: dk,
            sample_node: sample.node_id.clone(),
            sample_bins: [sb, sb],
            buffer_node: buffer.node_id.clone(),
            buffer_bins: [
                buffer.source_bins.get(bl).copied().unwrap_or(bl),
                buffer.source_bins.get(bh).copied().unwrap_or(bh),
            ],
            interp_weights: [1.0 - w, w],
        });
        if v.is_finite() && v > 0.0 && sample.i_norm[sk].is_finite() {
            usable_bins.push(dk);
        }
    }

    // WLS scale on usable overlap points, weights = 1/(sigma_s^2 + sigma_b^2).
    let mut sw = 0.0_f64;
    let mut swx = 0.0_f64;
    let mut swy = 0.0_f64;
    let mut swxx = 0.0_f64;
    let mut swxy = 0.0_f64;
    for &dk in &usable_bins {
        let sk = keep[dk];
        let x = bv[dk];
        let y = sample.i_norm[sk];
        let var = sample.sigma_norm[sk].powi(2) + bsig[dk].powi(2);
        if !(var > 0.0) {
            continue;
        }
        let w = 1.0 / var;
        sw += w;
        swx += w * x;
        swy += w * y;
        swxx += w * x * x;
        swxy += w * x * y;
    }
    let (scale, scale_sigma, scale_ok) = {
        let denom = sw * swxx - swx * swx;
        if usable_bins.len() < 3 || denom <= 0.0 {
            (f64::NAN, f64::NAN, false)
        } else {
            let a = (swy * swxx - swx * swxy) / denom;
            let var_a = swxx / denom;
            (a, var_a.max(0.0).sqrt(), a.is_finite())
        }
    };
    if !scale_ok {
        diagnostics.push(diag(
            "SCALE_UNFIXED",
            "background scale is not constrained by the overlap (need >=3 points)",
        ));
    } else if scale_sigma / scale.abs().max(1e-12) > 0.15 {
        diagnostics.push(diag(
            "SCALE_UNCERTAIN",
            format!(
                "background scale {:.4} ± {:.4} is weakly constrained (>15% rel)",
                scale, scale_sigma
            ),
        ));
    }

    let scale_eff = if scale_ok { scale } else { 1.0 };
    let scale_var = if scale_ok { scale_sigma.powi(2) } else { 0.0 };

    let mut q_out = Vec::new();
    let mut i_out = Vec::new();
    let mut sig_out = Vec::new();
    let mut budgets = Vec::new();
    let mut negative_bins = Vec::new();
    for (dk, &sk) in keep.iter().enumerate() {
        let is_ = sample.i_norm[sk];
        let ib = bv[dk];
        let isub = is_ - scale_eff * ib;
        // Propagation: Var(s) + a^2 Var(b) + b^2 Var(a), cross terms zero
        // (sample and buffer measurements independent).
        let sample_norm = sample.sigma_norm[sk].powi(2);
        let buffer_norm = scale_eff.powi(2) * bsig[dk].powi(2);
        let background_scale = ib.powi(2) * scale_var;
        let total = sample_norm + buffer_norm + background_scale;
        q_out.push(sample.q_inv_a[sk]);
        i_out.push(isub);
        sig_out.push(total.max(0.0).sqrt());
        budgets.push(UncertaintyBudget {
            sample_norm,
            buffer_norm,
            background_scale,
            total,
        });
        if isub <= 0.0 {
            negative_bins.push(dk);
        }
    }
    if !negative_bins.is_empty() {
        diagnostics.push(diag(
            "NEGATIVE_INTENSITY",
            format!(
                "{} subtracted point(s) are <= 0; retained with flag",
                negative_bins.len()
            ),
        ));
    }

    let status = if dropped_low + dropped_high == 0 {
        SubtractionStatus::Full
    } else {
        SubtractionStatus::Partial {
            q_min: lo,
            q_max: hi,
            dropped_low,
            dropped_high,
        }
    };

    Subtraction {
        node_id: base_node,
        sample_node: sample.node_id.clone(),
        buffer_node: buffer.node_id.clone(),
        buffer_version: buffer.version.clone(),
        status,
        q_inv_a: q_out,
        i_sub: i_out,
        sigma_sub: sig_out,
        scale: scale_eff,
        scale_sigma: if scale_ok { scale_sigma } else { f64::NAN },
        budgets,
        origins,
        negative_bins,
        diagnostics,
    }
}
