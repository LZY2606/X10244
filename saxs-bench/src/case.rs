//! Offline domain case pack: "散射曲线台 — SAXS 归一化、背景扣除与多区间模型证据".
//!
//! This is SAXS-specific material (not a generic table). It provides a
//! deterministic synthetic 1D azimuthal-integration result with transmission,
//! exposure, thickness, buffer candidates, instrument response and mask
//! provenance, and a replay that actually passes through: q-grid misalignment,
//! no-overlap, background-scale uncertainty, negative intensity, interval
//! overlap, buffer version switching and multi-candidate residuals.

use crate::events::{Event, EventKind};
use crate::id::stable_id;
use crate::model::*;
use crate::units::{Field, QBasis};

/// Stable profile ids (so re-import reproduces identity).
pub const SAMPLE: &str = "prof_sample_A";
pub const BUF_V1: &str = "prof_buffer_v1";
pub const BUF_V2: &str = "prof_buffer_v2";
pub const BUF_NOOVERLAP: &str = "prof_buffer_nooverlap";

fn point(index: usize, q: f64, i: f64, e: f64, masked: bool, reason: Option<&str>) -> RawPoint {
    RawPoint {
        index,
        q_raw: format!("{q:.6}"),
        i_raw: format!("{i:.6}"),
        e_raw: format!("{e:.6}"),
        q: Some(q),
        i: Some(i),
        e: Some(e),
        masked,
        mask_reason: reason.map(|s| s.to_string()),
    }
}

fn qgrid(n: usize, q0: f64, q1: f64) -> Vec<f64> {
    (0..n).map(|k| q0 + (q1 - q0) * k as f64 / (n - 1) as f64).collect()
}

/// Sample = Guinier + weak power-law, on a 0.01..0.40 Å⁻¹ grid. Near the high-q
/// tail a few points dip slightly negative after subtraction (kept, flagged).
pub fn sample_profile(rel_noise: f64) -> RawProfile {
    let qs = qgrid(60, 0.01, 0.40);
    let mut pts = Vec::new();
    for (k, q) in qs.iter().enumerate() {
        let rg = 28.0;
        let guinier = 120.0 * (-q * q * rg * rg / 3.0).exp();
        let power = 0.35 * q.powf(-3.3);
        let base = guinier + power;
        let n = rel_noise * (1.0 + 0.05 * (k as f64));
        // A retained dip near the tail: after background subtraction this
        // yields a negative point that must be kept and flagged, not clipped.
        let dip = if k == 50 { 0.55 } else { 1.0 };
        let i = base * dip * (1.0 + n);
        let e = (base.abs().sqrt().max(0.05)) * (1.0 + 0.02 * k as f64);
        let (masked, reason) = if k == 0 {
            (true, Some("beamstop: 中心束阻挡"))
        } else if k == 59 {
            (true, Some("detector_edge: 探测器边缘"))
        } else {
            (false, None)
        };
        pts.push(point(k, *q, i, e, masked, reason));
    }
    RawProfile {
        id: SAMPLE.into(),
        role: Role::Sample,
        label: "样品 A".into(),
        version: "import".into(),
        q_basis_raw: "1/A".into(),
        q_basis: Some(QBasis::QInvAngstrom),
        intensity_unit: "counts/s (detector channel 1)".into(),
        exposure: Field::new("300", Some(300.0), "s"),
        thickness: Field::new("1.5", Some(1.5), "mm"),
        transmission: Field::new("0.42", Some(0.42), "ratio"),
        transmission_rel_uncertainty: 0.03,
        instrument_response: InstrumentResponse {
            source: "beamline calibration run 2026-09-10".into(),
            reference: "resp/cal_20260910.json".into(),
            applied: true,
        },
        mask_source: MaskSource {
            source: "azimuthal integrator mask + beamstop".into(),
            reference: "mask/runA.mask".into(),
            masked_count: 2,
        },
        points: pts,
        diagnostics: vec![],
    }
}

/// Buffer v1, deliberately on a *different* (offset/denser) q grid → triggers
/// misaligned-grid overlap-only interpolation. Shape close to sample power law.
pub fn buffer_v1(scale: f64) -> RawProfile {
    let qs = qgrid(70, 0.012, 0.395);
    let mut pts = Vec::new();
    for (k, q) in qs.iter().enumerate() {
        let base = scale * 0.35 * q.powf(-3.3) * (1.0 + 0.4 * (-q * 30.0).exp());
        let n = 0.004 * (((k * 37) % 7) as f64 - 3.0);
        let i = base * (1.0 + n);
        let e = base.abs().sqrt().max(0.04) * 1.1;
        pts.push(point(k, *q, i, e, false, None));
    }
    RawProfile {
        id: BUF_V1.into(),
        role: Role::Buffer,
        label: "缓冲液 v1".into(),
        version: "v1".into(),
        q_basis_raw: "1/A".into(),
        q_basis: Some(QBasis::QInvAngstrom),
        intensity_unit: "counts/s (detector channel 1)".into(),
        exposure: Field::new("300", Some(300.0), "s"),
        thickness: Field::new("1.5", Some(1.5), "mm"),
        transmission: Field::new("0.50", Some(0.50), "ratio"),
        transmission_rel_uncertainty: 0.03,
        instrument_response: InstrumentResponse {
            source: "beamline calibration run 2026-09-10".into(),
            reference: "resp/cal_20260910.json".into(),
            applied: true,
        },
        mask_source: MaskSource {
            source: "azimuthal integrator mask".into(),
            reference: "mask/buffer_v1.mask".into(),
            masked_count: 0,
        },
        points: pts,
        diagnostics: vec![],
    }
}

/// Buffer v2: revised buffer record (version switch). Slightly different shape
/// and a distinct q grid; yields a competing residual branch.
pub fn buffer_v2(scale: f64) -> RawProfile {
    let qs = qgrid(64, 0.015, 0.42);
    let mut pts = Vec::new();
    for (k, q) in qs.iter().enumerate() {
        let base = scale * 0.30 * q.powf(-3.05) * (1.0 + 0.25 * (-q * 22.0).exp());
        let n = 0.005 * (((k * 53) % 5) as f64 - 2.0);
        let i = base * (1.0 + n);
        let e = base.abs().sqrt().max(0.04) * 1.15;
        pts.push(point(k, *q, i, e, false, None));
    }
    RawProfile {
        id: BUF_V2.into(),
        role: Role::Buffer,
        label: "缓冲液 v2".into(),
        version: "v2".into(),
        q_basis_raw: "1/A".into(),
        q_basis: Some(QBasis::QInvAngstrom),
        intensity_unit: "counts/s (detector channel 1)".into(),
        exposure: Field::new("320", Some(320.0), "s"),
        thickness: Field::new("1.5", Some(1.5), "mm"),
        transmission: Field::new("0.48", Some(0.48), "ratio"),
        transmission_rel_uncertainty: 0.04,
        instrument_response: InstrumentResponse {
            source: "beamline calibration run 2026-09-11 (re-calibrated)".into(),
            reference: "resp/cal_20260911.json".into(),
            applied: true,
        },
        mask_source: MaskSource {
            source: "azimuthal integrator mask".into(),
            reference: "mask/buffer_v2.mask".into(),
            masked_count: 0,
        },
        points: pts,
        diagnostics: vec![],
    }
}

/// Buffer whose q range does not intersect the sample at all → no-overlap.
pub fn buffer_no_overlap() -> RawProfile {
    let qs = qgrid(40, 0.80, 1.60);
    let mut pts = Vec::new();
    for (k, q) in qs.iter().enumerate() {
        let base = 0.10 * q.powf(-3.4);
        pts.push(point(k, *q, base, base.sqrt().max(0.02), false, None));
    }
    RawProfile {
        id: BUF_NOOVERLAP.into(),
        role: Role::Buffer,
        label: "缓冲液(高q，无重叠)".into(),
        version: "vx-nooverlap".into(),
        q_basis_raw: "1/A".into(),
        q_basis: Some(QBasis::QInvAngstrom),
        intensity_unit: "counts/s".into(),
        exposure: Field::new("300", Some(300.0), "s"),
        thickness: Field::new("1.5", Some(1.5), "mm"),
        transmission: Field::new("0.49", Some(0.49), "ratio"),
        transmission_rel_uncertainty: 0.03,
        instrument_response: InstrumentResponse {
            source: "beamline calibration run 2026-09-10".into(),
            reference: "resp/cal_20260910.json".into(),
            applied: true,
        },
        mask_source: MaskSource {
            source: "azimuthal integrator mask".into(),
            reference: "mask/nooverlap.mask".into(),
            masked_count: 0,
        },
        points: pts,
        diagnostics: vec![],
    }
}

/// Profile ids in import order (preserves order across export/import).
pub fn base_event_kinds(seed: i64) -> Vec<EventKind> {
    let s = if seed % 2 == 0 { 0.012 } else { -0.014 };
    vec![
        EventKind::ProjectCreated { name: "散射曲线台 · 离线案例".into() },
        EventKind::ProfileAdded { profile: sample_profile(s) },
        EventKind::ProfileAdded { profile: buffer_v1(0.92) },
        EventKind::ProfileAdded { profile: buffer_v2(0.95) },
        EventKind::ProfileAdded { profile: buffer_no_overlap() },
        EventKind::BufferCandidate { sample_id: SAMPLE.into(), buffer_id: BUF_V1.into() },
        EventKind::BufferCandidate { sample_id: SAMPLE.into(), buffer_id: BUF_V2.into() },
        EventKind::BufferCandidate { sample_id: SAMPLE.into(), buffer_id: BUF_NOOVERLAP.into() },
        // Default wide fit windows for v1/v2; no-overlap is structurally blocked.
        EventKind::FitWindow { sample_id: SAMPLE.into(), buffer_id: BUF_V1.into(), fit_lo: 0.02, fit_hi: 0.35 },
        EventKind::FitWindow { sample_id: SAMPLE.into(), buffer_id: BUF_V2.into(), fit_lo: 0.02, fit_hi: 0.35 },
        EventKind::FitWindow { sample_id: SAMPLE.into(), buffer_id: BUF_NOOVERLAP.into(), fit_lo: 0.02, fit_hi: 0.35 },
    ]
}

/// Build fully-id-stamped event envelopes deterministically (used by tests and
/// the replay endpoint). `start_ms` + seq drive timestamps; ids derive from
/// content so they are stable across re-import.
pub fn build_events(kinds: Vec<EventKind>, start_ms: i64) -> Vec<Event> {
    let mut out = Vec::new();
    for (i, kind) in kinds.into_iter().enumerate() {
        let parent = i as i64; // seq is i+1; parent is previous seq (0 = root)
        let at_ms = start_ms + i as i64;
        let kind_json = serde_json::to_string(&kind).unwrap();
        let id = stable_id("ev", &[&parent.to_string(), &kind_json, &at_ms.to_string()]);
        out.push(Event { seq: i as i64 + 1, id, parent_seq: parent, at_ms, kind });
    }
    out
}
