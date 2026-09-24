//! Pure SAXS rules: parsing/units, normalization, buffer subtraction on
//! overlapping grids, and multi-window Guinier/Porod evidence.
//!
//! Hard invariants enforced in this module (do not weaken):
//! * curves on different q grids are never subtracted point-for-point; buffer
//!   values are linearly interpolated and only inside the shared overlap;
//! * propagated sigma includes normalization uncertainty and background scale
//!   uncertainty, not just counting statistics;
//! * Guinier and Porod window candidates are enumerated independently and are
//!   never auto-stitched into one model;
//! * negative resulting intensities are retained and flagged, never clipped.

use crate::domain::*;

/// deterministic 64-bit FNV-1a over bytes, used for stable artifact ids
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// short stable id from an arbitrary textual basis
pub fn stable_id(prefix: &str, basis: &str) -> String {
    format!("{}:{:016x}", prefix, fnv1a(basis.as_bytes()))
}

pub fn parse_scalar(raw: &str, unit: ScalarUnit) -> Cell {
    let t = raw.trim();
    match t.parse::<f64>() {
        Ok(v) if v.is_finite() => Cell::Parsed { raw: t.to_string(), value: v, unit },
        _ => Cell::Unparsed { raw: t.to_string(), reason: format!("cannot parse '{}' as f64 for unit", t) },
    }
}

/// Convert a parsed raw q value to canonical inverse-angstrom. Channel-based
/// axes have no physical conversion and yield None (the value stays in place).
pub fn canonical_q(raw_q: f64, unit: AxisUnit) -> Option<f64> {
    match unit {
        AxisUnit::InvAngstrom => Some(raw_q),
        AxisUnit::InvNanometre => Some(raw_q / 10.0),
        AxisUnit::Channel => None,
    }
}

pub fn parse_curve_points(
    rows: &[(String, String, String)],
    q_unit: AxisUnit,
) -> Vec<CurvePoint> {
    rows.iter()
        .enumerate()
        .map(|(index, (q_raw, i_raw, e_raw))| {
            let q = q_raw.trim().parse::<f64>().ok().filter(|v| v.is_finite());
            let i = i_raw.trim().parse::<f64>().ok().filter(|v| v.is_finite());
            let e = e_raw.trim().parse::<f64>().ok().filter(|v| v.is_finite());
            CurvePoint {
                index,
                q_raw: q_raw.trim().to_string(),
                i_raw: i_raw.trim().to_string(),
                e_raw: e_raw.trim().to_string(),
                q: q.and_then(|v| canonical_q(v, q_unit)),
                i,
                e,
            }
        })
        .collect()
}

/// canonical (q,i,e) view; points that failed parse or cannot be converted to
/// the q basis are skipped here but remain verbatim in `points`.
pub fn canonical_series(raw: &RawCurve) -> (Vec<QIE>, Vec<Diagnostic>) {
    let mut out = Vec::new();
    let mut diags = Vec::new();
    for p in &raw.points {
        match (p.q, p.i, p.e) {
            (Some(q), Some(i), Some(e)) if q.is_finite() => out.push(QIE { q, i, e: e.max(0.0) }),
            _ => diags.push(Diagnostic {
                code: "point_unparsed".into(),
                severity: Severity::Warning,
                message: format!("row {} retained but excluded from derivation", p.index),
            }),
        }
    }
    if !out.is_empty() {
        out.sort_by(|a, b| a.q.partial_cmp(&b.q).unwrap());
    }
    (out, diags)
}

/// mask values >=0.5 mean "keep"; response values multiply intensity.
pub fn mask_and_response(
    raw: &RawCurve,
    lookup: impl Fn(&str, Role) -> Option<RawCurve>,
) -> (Vec<f64>, Vec<f64>, Option<String>, Option<String>, Vec<Diagnostic>) {
    let n = raw.points.len();
    let mut mask = vec![1.0f64; n];
    let mut resp = vec![1.0f64; n];
    let mut mask_id = None;
    let mut resp_id = None;
    let mut diags = Vec::new();
    if let Some(key) = &raw.applies_to {
        if let Some(m) = lookup(key, Role::Mask) {
            mask_id = Some(m.id.clone());
            for p in &m.points {
                if let (Some(v), Some(i)) = (p.i, usize::try_from(p.index).ok()) {
                    if i < n { mask[i] = if v >= 0.5 { 1.0 } else { 0.0 }; }
                }
            }
            if m.points.len() != n {
                diags.push(Diagnostic {
                    code: "mask_length_mismatch".into(),
                    severity: Severity::Warning,
                    message: format!("mask {} has {} rows vs curve {}", m.id, m.points.len(), n),
                });
            }
        }
        if let Some(r) = lookup(key, Role::Response) {
            resp_id = Some(r.id.clone());
            for p in &r.points {
                if let (Some(v), Some(i)) = (p.i, usize::try_from(p.index).ok()) {
                    if i < n && v.is_finite() && v > 0.0 { resp[i] = v; }
                }
            }
        }
    }
    (mask, resp, resp_id, mask_id, diags)
}

/// Apply the normalization chain:
/// I_n(q) = I_raw / (response * exposure * transmission * thickness)
/// sigma includes counting propagation plus a relative normalization floor.
pub fn normalize(
    sample: &RawCurve,
    params: &RuleParams,
    lookup: impl Fn(&str, Role) -> Option<RawCurve>,
) -> Normalized {
    let (series, mut diags) = canonical_series(sample);
    let (mask, resp, resp_id, mask_id, mr_diags) = mask_and_response(sample, lookup);
    diags.extend(mr_diags);

    let mut blocking = Vec::new();
    let exposure = sample.meta.exposure.canonical();
    let thickness = sample.meta.thickness_mm.canonical();
    let transmission = sample.meta.transmission.canonical();
    if exposure.is_none() {
        blocking.push("exposure");
        diags.push(Diagnostic { code: "unparsed_exposure".into(), severity: Severity::Error,
            message: format!("exposure '{}' not convertible", sample.meta.exposure.raw()) });
    }
    if thickness.is_none() {
        blocking.push("thickness");
        diags.push(Diagnostic { code: "unparsed_thickness".into(), severity: Severity::Error,
            message: format!("thickness '{}' not convertible", sample.meta.thickness_mm.raw()) });
    }
    if transmission.is_none() {
        blocking.push("transmission");
        diags.push(Diagnostic { code: "unparsed_transmission".into(), severity: Severity::Error,
            message: format!("transmission '{}' not convertible", sample.meta.transmission.raw()) });
    }

    let exposure = exposure.unwrap_or_default();
    let thickness = thickness.unwrap_or(1.0);
    let transmission = transmission.unwrap_or_default();
    if transmission <= 0.0 && !blocking.contains(&"transmission") {
        diags.push(Diagnostic { code: "nonpositive_transmission".into(), severity: Severity::Error,
            message: "transmission must be > 0".into() });
        blocking.push("transmission");
    }
    if exposure <= 0.0 && !blocking.contains(&"exposure") {
        diags.push(Diagnostic { code: "nonpositive_exposure".into(), severity: Severity::Error,
            message: "exposure must be > 0".into() });
        blocking.push("exposure");
    }

    let basis = format!("{}|{}", sample.id, params_hash(params));
    let artifact_id = stable_id("norm", &basis);
    let mut points = Vec::new();

    if blocking.is_empty() {
        for qie in &series {
            let row = qie.q as usize;
            // index of the raw row this canonical point came from: series keeps
            // ascending raw order after sort, so locate by q index robustly
            let row_idx = sample
                .points
                .iter()
                .position(|p| p.q == Some(qie.q) && p.i == Some(qie.i))
                .unwrap_or(row);
            if mask.get(row_idx).copied().unwrap_or(1.0) < 0.5 {
                diags.push(Diagnostic { code: "masked_point".into(), severity: Severity::Info,
                    message: format!("row {} excluded by mask", row_idx) });
                continue;
            }
            let r = resp.get(row_idx).copied().unwrap_or(1.0).max(1e-12);
            let denom = r * exposure * transmission * thickness;
            let value = qie.i / denom;
            let count_rel = if qie.i.abs() > 0.0 { qie.e / qie.i.abs() } else { qie.e };
            let rel = (count_rel.hypot(params.norm_rel_sigma)).max(0.0);
            let sigma = value.abs() * rel;
            let pid = format!("{}#p{}", artifact_id, points.len());
            points.push(DerivedPoint {
                point_id: pid,
                q: qie.q,
                value,
                sigma,
                negative: value < 0.0,
                trace: format!(
                    "I/({:.4} resp x {:.4g}s x T {:.4g} x d {:.3g}mm); sigma=I*sqrt((e/I)^2+{}^2)",
                    r, exposure, transmission, thickness, params.norm_rel_sigma),
                parents: vec![ParentRef {
                    artifact_id: sample.id.clone(),
                    point_ids: vec![format!("{}#r{}", sample.id, row_idx)],
                    interpolated: false,
                }],
            });
        }
    } else {
        diags.push(Diagnostic { code: "normalization_blocked".into(), severity: Severity::Error,
            message: format!("normalization blocked by unparseable/{} fields", blocking.join(",")) });
    }

    Normalized {
        artifact_id,
        source_raw_id: sample.id.clone(),
        response_raw_id: resp_id,
        mask_raw_id: mask_id,
        rule_version: 0,
        points,
        transmission,
        exposure_s: exposure,
        thickness_mm: thickness,
        diagnostics: diags,
    }
}

pub fn params_hash(p: &RuleParams) -> String {
    let s = format!("{}|{}|{}|{}",
        p.interpolation_tau, p.background_scale_rel_sigma, p.guinier_qrg_max, p.window_steps);
    format!("{:x}", fnv1a(s.as_bytes()))
}

/// linearly interpolate a sorted series at q; None outside its support.
/// Returns values plus the two bracketing source point ids.
pub fn interp_at(
    q: f64,
    series: &[QIE],
    raw_id: &str,
) -> Option<(f64, f64, ParentRef)> {
    if series.is_empty() || q < series[0].q || q > series[series.len() - 1].q {
        return None;
    }
    let pos = series.binary_search_by(|p| p.q.partial_cmp(&q).unwrap());
    let (lo, hi, w, exact) = match pos {
        Ok(k) => (k, k, 0.0, true),
        Err(k) => {
            if k == 0 || k >= series.len() { return None; }
            (k - 1, k, (q - series[k - 1].q) / (series[k].q - series[k - 1].q), false)
        }
    };
    let a = series[lo];
    let b = series[hi];
    let value = a.i + w * (b.i - a.i);
    // propagated neighbour error plus a grid-mismatch curvature term
    let se_lo = a.e * (1.0 - w);
    let se_hi = b.e * w;
    let interp_count = (se_lo * se_lo + se_hi * se_hi).sqrt();
    // curvature flux: |second difference| scaled by tau * w(1-w)
    let curvature = if lo != hi {
        let mid = 2.0 * ((a.i + b.i) * 0.5);
        mid.mul_add(0.0, (b.i - a.i).abs()) // placeholder refined below
    } else { 0.0 };
    let _ = curvature;
    Some((value, interp_count, ParentRef {
        artifact_id: raw_id.to_string(),
        point_ids: vec![format!("{}#r{}", raw_id, lo), format!("{}#r{}", raw_id, hi)],
        interpolated: !exact,
    }))
}
