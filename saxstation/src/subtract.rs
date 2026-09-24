//! Buffer subtraction with strict q-grid semantics.
//!
//! Rules encoded here:
//! * Different q grids are **never** subtracted point-by-point.
//! * Interpolation acts only inside the shared overlap window.
//! * Uncertainty combines normalized noise with background scale uncertainty.
//! * Negative intensities are reported (warning), never clipped away.

use crate::ids::content_id;
use crate::model::*;

#[derive(Debug, Clone)]
struct XY {
    q: f64,
    i: f64,
    sigma: f64,
    norm_rel_var: f64,
    point_id: Id,
}

fn active_sorted(c: &NormalizedCurve) -> Vec<XY> {
    let mut v: Vec<XY> = c
        .points
        .iter()
        .filter(|p| !p.masked)
        .map(|p| XY {
            q: p.q,
            i: p.intensity,
            sigma: p.sigma,
            norm_rel_var: p.norm_rel_var,
            point_id: p.id.clone(),
        })
        .collect();
    v.sort_by(|a, b| a.q.partial_cmp(&b.q).unwrap());
    v
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9_f64.max(1e-6 * a.abs().max(b.abs()))
}

/// Whether two grids are identical over their *common length*. Length mismatch
/// or shifted nodes means `false`.
pub fn grids_match(a: &[XY], b: &[XY]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(x, y)| approx(x.q, y.q))
}

/// Linear interpolation of the buffer at target q. Returns the value, its
/// propagated sigma (including the buffer's normalization variance) and the
/// bracketing point ids.
fn interpolate(buffer: &[XY], q: f64) -> Option<(f64, f64, Vec<Id>, bool)> {
    if buffer.is_empty() {
        return None;
    }
    if q < buffer[0].q || q > buffer[buffer.len() - 1].q {
        return None;
    }
    match buffer.binary_search_by(|p| p.q.partial_cmp(&q).unwrap()) {
        Ok(idx) => Some((buffer[idx].i, buffer[idx].sigma, vec![buffer[idx].point_id.clone()], false)),
        Err(idx) => {
            let hi = idx;
            let lo = idx - 1;
            let a = &buffer[lo];
            let b = &buffer[hi];
            let w = (q - a.q) / (b.q - a.q);
            let val = a.i + w * (b.i - a.i);
            // linearly interpolate sigma-weighted uncertainty conservatively
            let sig = ((1.0 - w) * a.sigma.powi(2) + w * b.sigma.powi(2)).sqrt();
            Some((val, sig, vec![a.point_id.clone(), b.point_id.clone()], true))
        }
    }
}

pub struct SubtractRequest {
    pub sample: NormalizedCurve,
    pub buffer: NormalizedCurve,
    pub buffer_version: String,
    pub scale: f64,
    pub scale_rel_sigma: f64,
}

pub fn subtract(req: &SubtractRequest) -> Subtraction {
    let mut diagnostics = Vec::new();
    let sample = active_sorted(&req.sample);
    let buffer = active_sorted(&req.buffer);

    let mut status = SubStatus::Ok;
    if sample.is_empty() || buffer.is_empty() {
        status = SubStatus::MissingInput;
        diagnostics.push("sample or buffer has no usable points".to_string());
        return Subtraction {
            id: content_id("sub", &serde_json::json!({"s": req.sample.id, "b": req.buffer.id, "a": req.scale})),
            sample_norm: req.sample.id.clone(),
            buffer_norm: req.buffer.id.clone(),
            buffer_version: req.buffer_version.clone(),
            scale: req.scale,
            scale_rel_sigma: req.scale_rel_sigma,
            overlap: None,
            points: vec![],
            status,
            diagnostics,
        };
    }

    let same_grid = grids_match(&sample, &buffer);
    if !same_grid {
        diagnostics.push("q grids differ: no direct point-to-point subtraction; interpolation used".to_string());
    }

    let bmin = buffer[0].q;
    let bmax = buffer[buffer.len() - 1].q;
    let smin = sample[0].q;
    let smax = sample[sample.len() - 1].q;
    let lo = smin.max(bmin);
    let hi = smax.min(bmax);

    if !(hi > lo) && !approx(hi, lo) {
        status = SubStatus::NoOverlap;
        diagnostics.push(format!("no overlapping q range: sample [{:.5},{:.5}] buffer [{:.5},{:.5}]", smin, smax, bmin, bmax));
        return Subtraction {
            id: content_id("sub", &serde_json::json!({"s": req.sample.id, "b": req.buffer.id, "a": req.scale})),
            sample_norm: req.sample.id.clone(),
            buffer_norm: req.buffer.id.clone(),
            buffer_version: req.buffer_version.clone(),
            scale: req.scale,
            scale_rel_sigma: req.scale_rel_sigma,
            overlap: None,
            points: vec![],
            status,
            diagnostics,
        };
    }

    if !same_grid {
        status = SubStatus::GridsMismatch;
    }

    let overlap = (lo, hi);
    let mut points = Vec::new();
    for s in &sample {
        // interpolation only inside overlap
        if s.q < lo - 1e-12 || s.q > hi + 1e-12 {
            continue;
        }
        let interp = if same_grid {
            // find matching buffer node directly (no interpolation)
            match buffer.binary_search_by(|p| p.q.partial_cmp(&s.q).unwrap()) {
                Ok(idx) => Some((buffer[idx].i, buffer[idx].sigma, vec![buffer[idx].point_id.clone()], false)),
                Err(_) => interpolate(&buffer, s.q),
            }
        } else {
            interpolate(&buffer, s.q)
        };

        let Some((bval, bsig, bpts, interpolated)) = interp else { continue };

        let buffer_scaled = req.scale * bval;
        let intensity = s.i - buffer_scaled;

        // sample normalized variance
        let s_var = s.sigma.powi(2);
        // buffer variance including its normalization variance at this q
        let b_var = (req.scale * bsig).powi(2);
        // background scale uncertainty: I_b contributes (sigma_alpha * I_b)^2
        let scale_var = (req.scale_rel_sigma * buffer_scaled).powi(2);
        let sigma = (s_var + b_var + scale_var).sqrt();

        if intensity < 0.0 {
            diagnostics.push(format!("negative subtracted intensity at q={:.5}: {:.5} (kept, not clipped)", s.q, intensity));
        }

        points.push(SubPoint {
            id: content_id(
                "subpt",
                &serde_json::json!({"sub_s": req.sample.id, "sub_b": req.buffer.id, "a": req.scale, "q": s.q, "sp": s.point_id, "bp": bpts}),
            ),
            q: s.q,
            intensity,
            sigma,
            sample_point: s.point_id.clone(),
            buffer_points: bpts,
            interpolated_buffer: interpolated,
            in_overlap: true,
        });
    }

    if points.is_empty() {
        status = SubStatus::NoOverlap;
        diagnostics.push("overlap window contains no sample points".to_string());
    }

    Subtraction {
        id: content_id("sub", &serde_json::json!({"s": req.sample.id, "b": req.buffer.id, "a": req.scale})),
        sample_norm: req.sample.id.clone(),
        buffer_norm: req.buffer.id.clone(),
        buffer_version: req.buffer_version.clone(),
        scale: req.scale,
        scale_rel_sigma: req.scale_rel_sigma,
        overlap: Some(overlap),
        points,
        status,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(id: &str, role: Role, qs: &[f64]) -> NormalizedCurve {
        NormalizedCurve {
            id: id.into(),
            source_bundle: format!("b-{}", id),
            role,
            points: qs
                .iter()
                .enumerate()
                .map(|(i, q)| NormalizedPoint {
                    id: format!("{}:p{}", id, i),
                    source_point: format!("{}:src{}", id, i),
                    q: *q,
                    intensity: 100.0,
                    sigma: 1.0,
                    norm_rel_var: 0.0,
                    norm_factor: 1.0,
                    masked: false,
                })
                .collect(),
            q_unit: "1/A".into(),
            i_unit: "1/cm".into(),
            diagnostics: vec![],
        }
    }

    #[test]
    fn identical_grids_subtract_directly() {
        let s = curve("s", Role::Sample, &[0.01, 0.02, 0.03]);
        let b = curve("b", Role::Buffer, &[0.01, 0.02, 0.03]);
        let r = subtract(&SubtractRequest {
            sample: s,
            buffer: b,
            buffer_version: "buf@v1".into(),
            scale: 1.0,
            scale_rel_sigma: 0.0,
        });
        assert_eq!(r.status, SubStatus::Ok);
        assert_eq!(r.points.len(), 3);
        assert!(r.points.iter().all(|p| !p.interpolated_buffer));
        assert!(r.points.iter().all(|p| (p.intensity - 0.0).abs() < 1e-9));
    }

    #[test]
    fn shifted_grids_interpolate_only_in_overlap() {
        let s = curve("s", Role::Sample, &[0.01, 0.02, 0.03, 0.04]);
        // buffer shifted +0.005, covering 0.015..0.035
        let b = curve("b", Role::Buffer, &[0.015, 0.025, 0.035]);
        let r = subtract(&SubtractRequest {
            sample: s,
            buffer: b,
            buffer_version: "buf@v1".into(),
            scale: 1.0,
            scale_rel_sigma: 0.0,
        });
        assert_eq!(r.status, SubStatus::GridsMismatch);
        // only sample points within 0.015..0.035
        let qs: Vec<f64> = r.points.iter().map(|p| p.q).collect();
        assert_eq!(qs, vec![0.02, 0.03]);
        assert!(r.points.iter().all(|p| p.interpolated_buffer));
        assert!(r.points.iter().all(|p| p.buffer_points.len() == 2));
    }

    #[test]
    fn no_overlap_is_flagged_and_not_subtracted() {
        let s = curve("s", Role::Sample, &[0.01, 0.02]);
        let b = curve("b", Role::Buffer, &[0.10, 0.11]);
        let r = subtract(&SubtractRequest {
            sample: s,
            buffer: b,
            buffer_version: "buf@v1".into(),
            scale: 1.0,
            scale_rel_sigma: 0.0,
        });
        assert_eq!(r.status, SubStatus::NoOverlap);
        assert!(r.points.is_empty());
        assert!(r.overlap.is_none());
    }

    #[test]
    fn background_scale_uncertainty_enters_sigma() {
        let s = curve("s", Role::Sample, &[0.01]);
        let b = curve("b", Role::Buffer, &[0.01]);
        let r0 = subtract(&SubtractRequest {
            sample: s.clone(),
            buffer: b.clone(),
            buffer_version: "buf@v1".into(),
            scale: 1.0,
            scale_rel_sigma: 0.0,
        });
        let r1 = subtract(&SubtractRequest {
            sample: s,
            buffer: b,
            buffer_version: "buf@v1".into(),
            scale: 1.0,
            scale_rel_sigma: 0.05,
        });
        // buffer value 100 => scale term = 0.05*100 = 5
        let expected = (1.0_f64 + 1.0 + 25.0).sqrt();
        assert!(r0.points[0].sigma < r1.points[0].sigma);
        assert!((r1.points[0].sigma - expected).abs() < 1e-9);
    }
}
