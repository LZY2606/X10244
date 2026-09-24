//! Buffer subtraction on potentially mismatched q grids.
//!
//! Rules enforced here (mirrored by the machine-readable events):
//! * grids are never subtracted point-by-point unless they coincide;
//! * interpolation only acts inside the shared q overlap;
//! * no overlap at all => a failed result, not a fabricated curve;
//! * buffer scaling uncertainty propagates into every output variance;
//! * negative residuals are retained and flagged.

use serde::{Deserialize, Serialize};

use crate::domain::curves::NormalizedPoint;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubPoint {
    pub ordinal: u32,
    pub q: f64,
    pub i_sub: f64,
    pub sigma_sub: f64,
    pub interp_buffer: bool,
    pub weight_lo: f64,
    pub buffer_lo_ordinal: Option<u32>,
    pub buffer_hi_ordinal: Option<u32>,
    pub sample_ordinal: u32,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtractedCurve {
    pub points: Vec<SubPoint>,
    pub overlap_lo: Option<f64>,
    pub overlap_hi: Option<f64>,
    pub scale_k: f64,
    pub scale_sigma: f64,
    pub scale_source: String,
    pub diagnostics: Vec<String>,
    pub ok: bool,
    pub grid_relation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubConfig {
    /// "residual" => k from weighted least squares in an anchor window;
    /// "fixed" => use `k`.
    pub mode: String,
    pub k: Option<f64>,
    pub k_sigma: f64,
    pub fit_q_lo: Option<f64>,
    pub fit_q_hi: Option<f64>,
    pub thickness_cm: Option<f64>,
    pub thickness_sigma: f64,
}

impl Default for SubConfig {
    fn default() -> Self {
        SubConfig { mode: "residual".into(), k: None, k_sigma: 0.0, fit_q_lo: None, fit_q_hi: None, thickness_cm: None, thickness_sigma: 0.0 }
    }
}

fn active(p: &NormalizedPoint) -> bool {
    p.i_norm.is_finite() && p.sigma_norm.is_finite() && p.q.is_finite()
}

fn range(pts: &[NormalizedPoint]) -> Option<(f64, f64)> {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for p in pts.iter().filter(|p| active(p)) {
        lo = lo.min(p.q);
        hi = hi.max(p.q);
    }
    if lo.is_infinite() { None } else { Some((lo, hi)) }
}

/// Piecewise-linear interpolation of the buffer at q, only called within an
/// established overlap. Returns None exactly when q cannot be bracketed by
/// active buffer points (which the caller prevents via the overlap check).
fn interp(buf: &[NormalizedPoint], q: f64) -> Option<(f64, f64, f64, Option<u32>, Option<u32>)> {
    let b: Vec<&NormalizedPoint> = buf.iter().filter(|p| active(p)).collect();
    if b.is_empty() {
        return None;
    }
    if q <= b[0].q {
        return Some((b[0].i_norm, b[0].sigma_norm, if (q - b[0].q).abs() < 1e-12 { 0.0 } else { 1.0 }, Some(b[0].ordinal), b.get(1).map(|p| p.ordinal)));
    }
    for w in b.windows(2) {
        if q <= w[1].q {
            let (lo, hi) = (w[0], w[1]);
            let wlo = if hi.q == lo.q { 0.0 } else { (hi.q - q) / (hi.q - lo.q) };
            let iv = wlo * lo.i_norm + (1.0 - wlo) * hi.i_norm;
            let sv = wlo * lo.sigma_norm + (1.0 - wlo) * hi.sigma_norm;
            return Some((iv, sv, 1.0 - wlo, Some(lo.ordinal), Some(hi.ordinal)));
        }
    }
    let last = b[b.len() - 1];
    Some((last.i_norm, last.sigma_norm, 0.0, Some(last.ordinal), None))
}

/// Weighted least squares k = sum w S B / sum w B^2 over the anchor window
/// (default: widest shared window). sigma_k^2 = 1 / sum w B^2.
pub fn estimate_scale(
    sample: &[NormalizedPoint],
    buffer: &[NormalizedPoint],
    lo: Option<f64>,
    hi: Option<f64>,
) -> Option<(f64, f64, usize)> {
    let mut num = 0.0;
    let mut den = 0.0;
    let mut n = 0usize;
    for s in sample.iter().filter(|p| active(p)) {
        if let Some(l) = lo {
            if s.q < l {
                continue;
            }
        }
        if let Some(h) = hi {
            if s.q > h {
                continue;
            }
        }
        if let Some((bv, bs, _, _, _)) = interp(buffer, s.q) {
            let var = s.sigma_norm * s.sigma_norm + bs * bs;
            if !(var > 0.0) || bv == 0.0 {
                continue;
            }
            let w = 1.0 / var;
            num += w * s.i_norm * bv;
            den += w * bv * bv;
            n += 1;
        }
    }
    if den <= 0.0 || n < 2 {
        return None;
    }
    Some((num / den, (1.0 / den).sqrt(), n))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResidualSummary {
    pub candidate: String,
    pub k: f64,
    pub k_sigma: f64,
    pub chi2: f64,
    pub anchor_points: usize,
    pub mean_residual: f64,
    pub legal: bool,
}

/// Residual evidence for one buffer candidate over a common anchor window.
/// Multiple legal candidates are intentionally all returned by the caller.
pub fn residual_summary(
    candidate: &str,
    sample: &[NormalizedPoint],
    buffer: &[NormalizedPoint],
    cfg: &SubConfig,
) -> Option<ResidualSummary> {
    let (k, ks, n) = match estimate_scale(sample, buffer, cfg.fit_q_lo, cfg.fit_q_hi) {
        Some(v) => v,
        None => return None,
    };
    let mut chi2 = 0.0;
    let mut sum = 0.0;
    let mut cnt = 0usize;
    for s in sample.iter().filter(|p| active(p)) {
        if let Some(l) = cfg.fit_q_lo {
            if s.q < l {
                continue;
            }
        }
        if let Some(h) = cfg.fit_q_hi {
            if s.q > h {
                continue;
            }
        }
        if let Some((bv, bs, _, _, _)) = interp(buffer, s.q) {
            let r = s.i_norm - k * bv;
            let var = s.sigma_norm * s.sigma_norm + (k * bs) * (k * bs) + (bv * ks) * (bv * ks);
            if var > 0.0 {
                chi2 += r * r / var;
            }
            sum += r;
            cnt += 1;
        }
    }
    if cnt == 0 {
        return None;
    }
    Some(ResidualSummary {
        candidate: candidate.to_string(),
        k,
        k_sigma: ks,
        chi2: if cnt > 1 { chi2 / (cnt as f64 - 1.0) } else { chi2 },
        anchor_points: n,
        mean_residual: sum / cnt as f64,
        legal: n >= 2,
    })
}

pub fn subtract(sample: &[NormalizedPoint], buffer: &[NormalizedPoint], cfg: &SubConfig) -> SubtractedCurve {
    let mut diag = Vec::new();
    let sr = range(sample);
    let br = range(buffer);
    if sr.is_none() || br.is_none() {
        return SubtractedCurve {
            points: vec![],
            overlap_lo: None,
            overlap_hi: None,
            scale_k: f64::NAN,
            scale_sigma: f64::NAN,
            scale_source: "none".into(),
            diagnostics: vec!["no usable finite points in one of the inputs".into()],
            ok: false,
            grid_relation: "empty".into(),
        };
    }
    let (slo, shi) = sr.unwrap();
    let (blo, bhi) = br.unwrap();
    let lo = slo.max(blo);
    let hi = shi.min(bhi);
    if lo > hi {
        diag.push(format!("no overlap: sample [{slo:.6}, {shi:.6}] vs buffer [{blo:.6}, {bhi:.6}]"));
        return SubtractedCurve {
            points: vec![],
            overlap_lo: None,
            overlap_hi: None,
            scale_k: f64::NAN,
            scale_sigma: f64::NAN,
            scale_source: "none".into(),
            diagnostics: diag,
            ok: false,
            grid_relation: "disjoint".into(),
        };
    }

    // Classify grid relation using shared sample q values.
    let bqs: Vec<f64> = buffer.iter().filter(|p| active(p)).map(|p| p.q).collect();
    let coincident = sample
        .iter()
        .filter(|p| active(p) && p.q >= lo && p.q <= hi)
        .all(|p| bqs.iter().any(|q| (*q - p.q).abs() < 1e-9));
    let relation = if coincident { "coincident" } else { "interpolated-overlap" };
    if relation == "interpolated-overlap" {
        diag.push("q grids differ; buffer is linearly interpolated only inside the shared overlap".to_string());
    }

    let (k, ks, source) = if cfg.mode == "fixed" {
        match cfg.k {
            Some(v) if v.is_finite() => (v, cfg.k_sigma, "fixed"),
            _ => return SubtractedCurve {
                points: vec![],
                overlap_lo: Some(lo),
                overlap_hi: Some(hi),
                scale_k: f64::NAN,
                scale_sigma: f64::NAN,
                scale_source: "none".into(),
                diagnostics: vec!["fixed scale requested but k missing".into()],
                ok: false,
                grid_relation: relation.into(),
            },
        }
    } else {
        match estimate_scale(sample, buffer, cfg.fit_q_lo.or(Some(lo)), cfg.fit_q_hi.or(Some(hi))) {
            Some((k, ks, _)) => (k, ks.max(cfg.k_sigma), "residual-fit"),
            None => return SubtractedCurve {
                points: vec![],
                overlap_lo: Some(lo),
                overlap_hi: Some(hi),
                scale_k: f64::NAN,
                scale_sigma: f64::NAN,
                scale_source: "none".into(),
                diagnostics: vec!["scale estimation underdetermined in overlap window".into()],
                ok: false,
                grid_relation: relation.into(),
            },
        }
    };

    let mut points = Vec::new();
    let mut neg = 0usize;
    for s in sample.iter() {
        let mut flags: Vec<String> = Vec::new();
        if !active(s) {
            flags.push("excluded:source".into());
            points.push(SubPoint {
                ordinal: s.ordinal,
                q: s.q,
                i_sub: f64::NAN,
                sigma_sub: f64::NAN,
                interp_buffer: false,
                weight_lo: f64::NAN,
                buffer_lo_ordinal: None,
                buffer_hi_ordinal: None,
                sample_ordinal: s.ordinal,
                flags,
            });
            continue;
        }
        if s.q < lo || s.q > hi {
            flags.push("outside-overlap:retained-not-subtracted".into());
            points.push(SubPoint {
                ordinal: s.ordinal,
                q: s.q,
                i_sub: f64::NAN,
                sigma_sub: f64::NAN,
                interp_buffer: false,
                weight_lo: f64::NAN,
                buffer_lo_ordinal: None,
                buffer_hi_ordinal: None,
                sample_ordinal: s.ordinal,
                flags,
            });
            continue;
        }
        let (bv, bs, whi, lo_ord, hi_ord) = interp(buffer, s.q).expect("inside overlap must bracket");
        let interp_flag = !matches!(lo_ord.zip(hi_ord), Some((a, b)) if a == b)
            && bqs.iter().all(|q| (*q - s.q).abs() >= 1e-9);
        let mut val = s.i_norm - k * bv;
        let mut var = s.sigma_norm * s.sigma_norm + (k * bs) * (k * bs) + (bv * ks) * (bv * ks);
        // Optional thickness scaling kept as a separate, visible step.
        if let Some(d) = cfg.thickness_cm {
            if d > 0.0 {
                val /= d;
                let rel_d = cfg.thickness_sigma / d;
                var = var / (d * d) + (val * rel_d) * (val * rel_d);
            }
        }
        if val < 0.0 {
            flags.push("negative-residual".into());
            neg += 1;
        }
        if interp_flag {
            flags.push("buffer-interpolated".into());
        }
        points.push(SubPoint {
            ordinal: s.ordinal,
            q: s.q,
            i_sub: val,
            sigma_sub: var.sqrt(),
            interp_buffer: interp_flag,
            weight_lo: 1.0 - whi,
            buffer_lo_ordinal: lo_ord,
            buffer_hi_ordinal: hi_ord,
            sample_ordinal: s.ordinal,
            flags,
        });
    }
    if neg > 0 {
        diag.push(format!("{neg} negative residual points kept and flagged"));
    }
    SubtractedCurve {
        points,
        overlap_lo: Some(lo),
        overlap_hi: Some(hi),
        scale_k: k,
        scale_sigma: ks,
        scale_source: source.into(),
        diagnostics: diag,
        ok: true,
        grid_relation: relation.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn np(ord: u32, q: f64, i: f64, s: f64) -> NormalizedPoint {
        NormalizedPoint { ordinal: ord, q, i_norm: i, sigma_norm: s, t_fraction: 0.0, flags: vec![] }
    }

    #[test]
    fn disjoint_grids_fail_without_fabrication() {
        let s = vec![np(1, 0.1, 10.0, 0.1), np(2, 0.2, 8.0, 0.1)];
        let b = vec![np(1, 1.0, 1.0, 0.1), np(2, 2.0, 0.5, 0.1)];
        let out = subtract(&s, &b, &SubConfig { mode: "fixed".into(), k: Some(1.0), k_sigma: 0.0, ..Default::default() });
        assert!(!out.ok);
        assert_eq!(out.grid_relation, "disjoint");
        assert!(out.diagnostics[0].contains("no overlap"));
        assert!(out.points.is_empty());
    }

    #[test]
    fn mismatched_grid_interpolates_only_inside_overlap() {
        let s = vec![np(1, 0.10, 10.0, 0.1), np(2, 0.15, 9.0, 0.1), np(3, 0.30, 5.0, 0.1), np(4, 0.50, 1.0, 0.1)];
        let b = vec![np(1, 0.10, 2.0, 0.01), np(2, 0.20, 1.8, 0.01), np(3, 0.30, 1.5, 0.01)];
        let out = subtract(&s, &b, &SubConfig { mode: "fixed".into(), k: Some(1.0), k_sigma: 0.0, ..Default::default() });
        assert!(out.ok);
        assert_eq!(out.grid_relation, "interpolated-overlap");
        // q=0.15 is a midpoint interpolation to 1.9.
        let p15 = out.points.iter().find(|p| (p.q - 0.15).abs() < 1e-9).unwrap();
        assert!(p15.interp_buffer);
        assert!((p15.i_sub - (9.0 - 1.9)).abs() < 1e-9);
        // q=0.50 lies outside overlap and stays as a flagged gap.
        let p50 = out.points.iter().find(|p| (p.q - 0.50).abs() < 1e-9).unwrap();
        assert!(p50.i_sub.is_nan());
        assert!(p50.flags.iter().any(|f| f.contains("outside-overlap")));
    }

    #[test]
    fn scale_uncertainty_inflates_variance() {
        let s = vec![np(1, 0.1, 10.0, 0.0)];
        let b = vec![np(1, 0.1, 2.0, 0.0)];
        let out = subtract(&s, &b, &SubConfig { mode: "fixed".into(), k: Some(1.0), k_sigma: 0.1, ..Default::default() });
        assert!((out.points[0].sigma_sub - 0.2).abs() < 1e-12);
    }

    #[test]
    fn coincident_grids_are_direct_difference() {
        let s = vec![np(1, 0.1, 10.0, 0.1), np(2, 0.2, 8.0, 0.1)];
        let b = vec![np(1, 0.1, 4.0, 0.1), np(2, 0.2, 3.0, 0.1)];
        let out = subtract(&s, &b, &SubConfig { mode: "fixed".into(), k: Some(1.0), k_sigma: 0.0, ..Default::default() });
        assert_eq!(out.grid_relation, "coincident");
        assert!((out.points[0].i_sub - 6.0).abs() < 1e-6);
        assert!(out.scale_k.is_finite());
    }

    #[test]
    fn residual_test_keeps_multiple_legal_candidates() {
        let s = vec![np(1, 0.1, 10.0, 0.1), np(2, 0.2, 9.0, 0.1), np(3, 0.3, 8.0, 0.1)];
        let b1 = vec![np(1, 0.1, 5.0, 0.05), np(2, 0.2, 4.5, 0.05), np(3, 0.3, 4.0, 0.05)];
        let b2 = vec![np(1, 0.1, 4.9, 0.05), np(2, 0.2, 4.6, 0.05), np(3, 0.3, 4.1, 0.05)];
        let cfg = SubConfig::default();
        let r1 = residual_summary("buf-a", &s, &b1, &cfg).unwrap();
        let r2 = residual_summary("buf-b", &s, &b2, &cfg).unwrap();
        assert!(r1.legal && r2.legal);
        assert!((r1.k - 2.0).abs() < 1e-6);
    }

    #[test]
    fn negative_residuals_are_flagged_not_clipped() {
        let s = vec![np(1, 0.1, 1.0, 0.1)];
        let b = vec![np(1, 0.1, 2.0, 0.1)];
        let out = subtract(&s, &b, &SubConfig { mode: "fixed".into(), k: Some(1.0), k_sigma: 0.0, ..Default::default() });
        assert!(out.points[0].i_sub < 0.0);
        assert!(out.points[0].flags.contains(&"negative-residual".to_string()));
    }
}
