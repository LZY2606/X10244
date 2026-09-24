//! Offline domain case pack: "Scattering curve bench — SAXS normalization,
//! buffer subtraction and multi-interval model evidence".
//!
//! Deterministic synthetic BSA-like curves (no network, same bytes in tests
//! and in the UI). The pack covers, as replayable situations:
//! grid mismatch, no-overlap region, background scale uncertainty,
//! negative intensity, Guinier/Porod interval overlap, buffer version switch
//! and multi-candidate residual testing.

use serde_json::json;

use crate::engine::ImportRequest;
use crate::model::*;

fn guinier_porod(q: f64, i0: f64, rg: f64, c: f64, alpha: f64) -> f64 {
    let g = i0 * (-(q * q) * rg * rg / 3.0).exp();
    let p = c * q.powf(-alpha);
    g + p
}

/// Deterministic pseudo-noise (LCG), seeded for byte-stable fixtures.
fn noise(seed: u64, i: usize) -> f64 {
    let mut x = seed.wrapping_add((i as u64).wrapping_mul(0x9E3779B97F4A7C15));
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58476D1CE4E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D049BB133111EB);
    x ^= x >> 31;
    (x as f64 / u64::MAX as f64) - 0.5
}

fn rows(
    qs: &[f64],
    intensity: impl Fn(usize, f64) -> f64,
    err_frac: f64,
    seed: u64,
) -> Vec<[String; 3]> {
    qs.iter()
        .enumerate()
        .map(|(i, q)| {
            let n = noise(seed, i) * err_frac * 0.4;
            let v = intensity(i, *q) * (1.0 + n);
            // raw representation: q in A^-1 (original instrument unit), I as text
            let qa = q / 10.0;
            [
                format!("{:.5}", qa),
                format!("{v:.4e}"),
                format!("{:.4e}", (err_frac * v).abs().max(1e-6)),
            ]
        })
        .collect()
}

fn response() -> InstrumentResponse {
    InstrumentResponse {
        source: "cases/bsa_like/instrument_response.json".to_string(),
        description: "Pilatus-style flat-field response, synthetic deterministic".to_string(),
        gain: vec![
            PointQ { q: 0.0, g: 0.985 },
            PointQ { q: 1.0, g: 1.000 },
            PointQ { q: 3.0, g: 1.012 },
            PointQ { q: 5.0, g: 0.995 },
        ],
    }
}

fn mask(ranges: Vec<[f64; 2]>, pixels: i64) -> Mask {
    Mask {
        source: "cases/bsa_like/mask.json".to_string(),
        description: "beamstop + inter-module gaps".to_string(),
        excluded_ranges: ranges,
        pixels_masked: Some(pixels),
    }
}

fn base_meta(
    kind: CurveKind,
    label: &str,
    name: &str,
    rows: Vec<[String; 3]>,
    buffer_of: Option<&str>,
) -> ImportRequest {
    ImportRequest {
        kind,
        label: label.to_string(),
        path: format!("cases/bsa_like/{name}.dat"),
        format_tag: "SAXS-1D v3 (I(q) integrated from 2D)".to_string(),
        q_unit: "A^-1".to_string(),
        i_unit: "counts".to_string(),
        rows,
        exposure: ("1800".to_string(), "s".to_string()),
        thickness: ("1.20".to_string(), "mm".to_string()),
        transmission: ("0.420(3)".to_string(), "fraction".to_string()),
        transmission_sigma: Some(("0.003".to_string(), "fraction".to_string())),
        buffer_candidate_of: buffer_of.map(String::from),
        instrument_response: Some(response()),
        mask: Some(mask(vec![[0.0, 0.06]], 128_440)),
    }
}

/// Normalized-shape solvent profile: nearly constant with weak Porod tail.
/// In raw counts it gets the same exposure/transmission/thickness factors,
/// so normalized sample and buffer share this exact background (k -> 1).
fn buffer_profile(q: f64) -> f64 {
    1.0 + 0.25 * q.powf(-0.6) + 0.004 * q.powf(-3.2)
}

/// Sample grid: 0.05..2.5 nm^-1 (0.05 step, 50 pts)
pub fn sample_grid() -> Vec<f64> {
    (1..=50).map(|i| 0.05 * i as f64).collect()
}
/// Buffer grid version 1: same grid as sample
pub fn buffer_grid_v1() -> Vec<f64> {
    sample_grid()
}
/// Buffer grid version 2: offset grid (q-grid mismatch)
pub fn buffer_grid_v2() -> Vec<f64> {
    (1..=50).map(|i| 0.047 + 0.05 * i as f64).collect()
}

pub fn sample_request() -> ImportRequest {
    let qs = sample_grid();
    let r = rows(
        &qs,
        |_, q| guinier_porod(q, 520.0, 2.0, 0.05, 3.8) + buffer_profile(q),
        0.04,
        11,
    );
    base_meta(CurveKind::Sample, "BSA 5 mg/mL", "sample_bsa", r, None)
}

pub fn buffer_request_v1() -> ImportRequest {
    let qs = buffer_grid_v1();
    let r = rows(
        &qs,
        |_, q| buffer_profile(q),
        0.06,
        23,
    );
    base_meta(CurveKind::Buffer, "PBS buffer v1", "buffer_pbs_v1", r, Some("BSA 5 mg/mL"))
}

/// v2 buffer: shifted grid, slightly rescaled background -> scale uncertainty.
pub fn buffer_request_v2() -> ImportRequest {
    let qs = buffer_grid_v2();
    let r = rows(
        &qs,
        |_, q| buffer_profile(q) * 1.08 + 0.15,
        0.14,
        47,
    );
    let mut m = base_meta(CurveKind::Buffer, "PBS buffer v2", "buffer_pbs_v2", r, Some("BSA 5 mg/mL"));
    m.mask = Some(mask(vec![[0.0, 0.06]], 128_512));
    m
}

/// A buffer whose q range starts beyond the sample range end => no overlap.
pub fn buffer_request_no_overlap() -> ImportRequest {
    let qs: Vec<f64> = (1..=20).map(|i| 2.7 + 0.05 * i as f64).collect();
    let r = rows(&qs, |_, q| 0.4 * q.powf(-3.4), 0.05, 91);
    let mut m = base_meta(CurveKind::Buffer, "PBS buffer far-q", "buffer_no_overlap", r, Some("BSA 5 mg/mL"));
    m.mask = Some(mask(vec![], 0));
    m
}

/// Sample variant whose high-q subtraction lands negative at several points.
pub fn sample_request_negative() -> ImportRequest {
    let qs = sample_grid();
    let r = rows(
        &qs,
        |i, q| {
            let mut v = guinier_porod(q, 520.0, 3.1, 0.02, 3.8) + buffer_profile(q);
            if i >= 42 {
                v -= 3.0; // pushes the last high-q points below the buffer
            }
            v
        },
        0.05,
        131,
    );
    base_meta(CurveKind::Sample, "BSA 5 mg/mL (high-q dip)", "sample_bsa_dip", r, None)
}

/// A raw record with an unconvertible thickness: stays raw with a diagnostic,
/// normalization is blocked (never silently dropped to make a plot).
pub fn sample_request_bad_unit() -> ImportRequest {
    let mut m = sample_request();
    m.label = "BSA bad thickness unit".to_string();
    m.path = "cases/bsa_like/sample_bad_unit.dat".to_string();
    m.thickness = ("1.20".to_string(), "thou".to_string());
    m
}

pub struct CasePack {
    pub requests: Vec<ImportRequest>,
    pub description: String,
}

pub fn pack() -> CasePack {
    CasePack {
        requests: vec![
            sample_request(),
            buffer_request_v1(),
            buffer_request_v2(),
            buffer_request_no_overlap(),
            sample_request_negative(),
            sample_request_bad_unit(),
        ],
        description: "BSA-like SAXS bench: retained raw q/I/error, exposure, thickness, \
transmission, buffer candidates, instrument response and mask sources."
            .to_string(),
    }
}

/// The short (<90 s) guided tour steps surfaced in the UI.
pub fn tour_script() -> serde_json::Value {
    json!([
        {"step": 1, "title": "Retained sources",
         "body": "Import the case pack; raw q/I/error, exposure, thickness, transmission, buffer candidates, response and mask are all present."},
        {"step": 2, "title": "Normalize both branches",
         "body": "I/(t T d) with transmission sigma propagated into e_norm."},
        {"step": 3, "title": "Subtract v1 buffer",
         "body": "Identical grids: clean subtraction, scale uncertainty in errors."},
        {"step": 4, "title": "Switch to v2 buffer",
         "body": "q-grid mismatch: interpolation only inside overlap; guarded result."},
        {"step": 5, "title": "No-overlap and negatives",
         "body": "Far-q buffer has no overlap; dip sample yields retained negative points."},
        {"step": 6, "title": "Multi-interval evidence",
         "body": "Guinier and Porod candidates with overlap flags; multi-candidate retained."},
        {"step": 7, "title": "Review and time travel",
         "body": "Click a derived point to trace both sources; revisit any old event."},
    ])
}
