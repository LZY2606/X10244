//! Buffer background subtraction.
//!
//! Hard semantic rules (do not violate):
//! * different q grids are NEVER subtracted point-to-point; the buffer is
//!   linearly interpolated onto sample q positions;
//! * interpolation acts only inside the common q range; points of the sample
//!   outside the overlap are reported but never subtracted;
//! * when the overlap is empty no subtraction curve is produced (state
//!   `no_overlap`);
//! * the output sigma combines normalization uncertainty of both branches AND
//!   background-scale uncertainty;
//! * multiple legal scale choices are kept as independent candidates; a
//!   decision never overwrites the others.
use crate::domain::ids::hid;
use crate::domain::normalize::Normalized;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaleMode {
    /// scale given explicitly with an uncertainty
    Given,
    /// alpha fitted by weighted least squares over the overlap
    Fitted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScaleFit {
    pub alpha: f64,
    pub sigma_alpha: f64,
    pub n_points: usize,
    /// weighted residual mean (chi2 / dof) of the fit itself
    pub reduced_chi2: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubPointStatus {
    Ok,
    OutsideOverlap,
    Negative,
    NonFinite,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPoint {
    pub q: f64,
    pub q_unit: String,
    pub sample: f64,
    pub buffer: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub status: SubPointStatus,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    Completed,
    NoOverlap,
    Degenerate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResidualSummary {
    pub n: usize,
    pub mean: f64,
    pub std: f64,
    pub reduced_chi2: f64,
    pub n_negative: usize,
    pub n_runs: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubCandidate {
    pub id: String,
    pub buffer_id: String,
    pub buffer_name: String,
    pub mode: ScaleMode,
    pub scale: f64,
    pub scale_sigma: f64,
    pub fit: Option<ScaleFit>,
    pub q_overlap: Option<[f64; 2]>,
    pub state: CandidateState,
    pub points: Vec<SubPoint>,
    pub residual: Option<ResidualSummary>,
    /// stable per-point identity of an OK result:
    /// pt:<sub_id>:<candidate_id>:<index>
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subtraction {
    pub id: String,
    pub sample_norm_id: String,
    pub sample_name: String,
    pub candidates: Vec<SubCandidate>,
    /// candidate id currently accepted (pointer; switching never deletes)
    pub accepted: Option<String>,
}

fn linear_interp(qs: &[f64], vs: &[f64], q: f64) -> Option<(f64, f64, f64, f64, f64)> {
    if qs.len() < 2 {
        return None;
    }
    if q < qs[0] || q > qs[qs.len() - 1] {
        return None;
    }
    let pos = qs.partition_point(|x| *x < q);
    if pos == 0 {
        return Some((vs[0], qs[0], vs[0], qs[0], vs[0]));
    }
    let i1 = pos.min(qs.len() - 1);
    let i0 = i1 - 1;
    if (qs[i1] - qs[i0]).abs() < f64::MIN_POSITIVE {
        return Some((vs[i0], qs[i0], vs[i0], qs[i0], vs[i0]));
    }
    let w = (q - qs[i0]) / (qs[i1] - qs[i0]);
    let v = vs[i0] * (1.0 - w) + vs[i1] * w;
    Some((v, qs[i0], vs[i0], qs[i1], vs[i1]))
}

/// Build one candidate (buffer branch) against one normalized sample.
/// `given_scale = Some((alpha, sigma))`; otherwise alpha is fitted.
pub fn build_candidate(sample: &Normalized, buffer: &Normalized,
                       given_scale: Option<(f64, f64)>) -> SubCandidate {
    let sp = sample.valid_points();
    let bp_all = buffer.valid_points();
    let bqs: Vec<f64> = bp_all.iter().map(|p| p.q).collect();
    let bvs: Vec<f64> = bp_all.iter().map(|p| p.intensity).collect();
    let bss: Vec<f64> = bp_all.iter().map(|p| p.sigma).collect();

    let overlap = if bqs.len() >= 2 && !sp.is_empty() {
        let lo = bqs[0].max(sp[0].q);
        let hi = bqs[bqs.len() - 1].min(sp[sp.len() - 1].q);
        if lo <= hi { Some([lo, hi]) } else { None }
    } else { None };

    let (mode, given_alpha, given_sigma) = match given_scale {
        Some((a, s)) => (ScaleMode::Given, a, s),
        None => (ScaleMode::Fitted, f64::NAN, f64::NAN),
    };

    let base = SubCandidate {
        id: String::new(), buffer_id: buffer.raw_id.clone(),
        buffer_name: buffer.name.clone(), mode,
        scale: given_alpha, scale_sigma: given_sigma,
        fit: None, q_overlap: overlap, state: CandidateState::NoOverlap,
        points: Vec::new(), residual: None, note: None,
    };
    let Some([lo, hi]) = overlap else {
        return finish_candidate(sample, buffer, base, given_scale);
    };

    let interp_buffer = |q: f64| -> Option<(f64, f64)> {
        linear_interp(&bqs, &bvs, q).map(|(v, q0, v0, q1, v1)| {
            let (s0, s1) = {
                let i0 = bqs.partition_point(|x| *x < q0).saturating_sub(1);
                let i1 = (i0 + 1).min(bss.len() - 1);
                (bss.get(i0).copied().unwrap_or(0.0), bss.get(i1).copied().unwrap_or(0.0))
            };
            let _ = (v0, v1);
            let w = if (q1 - q0).abs() < f64::MIN_POSITIVE { 0.0 } else { (q - q0) / (q1 - q0) };
            let sig = (s0 * s0 * (1.0 - w) + s1 * s1 * w).max(0.0).sqrt();
            (v, sig)
        })
    };

    // weighted least squares alpha minimizing sum (Is - alpha Ib)^2 / (ss^2 + ... )
    // iterated once with sample+buffer variance weights.
    let mut num = 0.0_f64;
    let mut den = 0.0_f64;
    for p in &sp {
        if p.q < lo || p.q > hi { continue; }
        if let Some((ib, _sb)) = interp_buffer(p.q) {
            let w = 1.0 / (p.sigma * p.sigma + 1e-30);
            num += w * p.intensity * ib;
            den += w * ib * ib;
        }
    }

    let mut cand = base;
    if mode == ScaleMode::Fitted {
        if den <= 0.0 || !den.is_finite() {
            cand.state = CandidateState::Degenerate;
            return finish_candidate(sample, buffer, cand, given_scale);
        }
        let alpha = num / den;
        // sigma_alpha from residual variance / denominator
        let mut chi2 = 0.0_f64;
        let mut n = 0usize;
        for p in &sp {
            if p.q < lo || p.q > hi { continue; }
            if let Some((ib, _)) = interp_buffer(p.q) {
                let r = p.intensity - alpha * ib;
                chi2 += r * r / (p.sigma * p.sigma + 1e-30);
                n += 1;
            }
        }
        let dof = n.saturating_sub(1).max(1) as f64;
        let var_resid = chi2 / dof;
        let sigma_alpha = (var_resid / den).max(0.0).sqrt();
        cand.scale = alpha;
        cand.scale_sigma = sigma_alpha;
        cand.fit = Some(ScaleFit { alpha, sigma_alpha, n_points: n, reduced_chi2: chi2 / dof });
    }
    finish_candidate(sample, buffer, cand, given_scale)
}

fn finish_candidate(sample: &Normalized, buffer: &Normalized,
                    mut cand: SubCandidate, _given: Option<(f64, f64)>) -> SubCandidate {
    let sp = sample.valid_points();
    let bp_all = buffer.valid_points();
    let bqs: Vec<f64> = bp_all.iter().map(|p| p.q).collect();
    let bvs: Vec<f64> = bp_all.iter().map(|p| p.intensity).collect();
    let bss: Vec<f64> = bp_all.iter().map(|p| p.sigma).collect();

    let mut points = Vec::new();
    if cand.state == CandidateState::NoOverlap {
        cand.points = points;
        cand.id = hid("subcand", &json!({
            "sample": sample.raw_id, "buffer": buffer.raw_id,
            "mode": cand.mode, "state":"no_overlap"}));
        return cand;
    }
    let [lo, hi] = cand.q_overlap.unwrap();
    let alpha = cand.scale;
    let sa = cand.scale_sigma;

    for p in &sp {
        let q = p.q;
        if q < lo || q > hi {
            points.push(SubPoint { q, q_unit: "A^-1".into(), sample: p.intensity,
                buffer: f64::NAN, intensity: f64::NAN, sigma: f64::NAN,
                status: crate::domain::subtract::SubPointStatus::OutsideOverlap,
                detail: Some("no interpolation outside common q range".into()) });
            continue;
        }
        let Some((ib, sb)) = linear_interp(&bqs, &bvs, q).map(|(v,q0,_,q1,_)| {
            let w = if (q1-q0).abs()<f64::MIN_POSITIVE {0.0} else {(q-q0)/(q1-q0)};
            let i0 = bqs.partition_point(|x|*x<q0).saturating_sub(1);
            let i1=(i0+1).min(bss.len()-1);
            let s0=bss.get(i0).copied().unwrap_or(0.0);
            let s1=bss.get(i1).copied().unwrap_or(0.0);
            (v, (s0*s0*(1.0-w)+s1*s1*w).max(0.0).sqrt())
        }) else {
            points.push(SubPoint { q, q_unit: "A^-1".into(), sample: p.intensity,
                buffer: f64::NAN, intensity: f64::NAN, sigma: f64::NAN,
                status: SubPointStatus::OutsideOverlap,
                detail: Some("interpolation failed".into()) });
            continue;
        };
        // I_sub = Is - alpha Ib
        let intensity = p.intensity - alpha * ib;
        // sigma^2 = ss^2 + (alpha*sb)^2 + (Ib*sigma_alpha)^2
        let var = p.sigma * p.sigma
            + (alpha * sb) * (alpha * sb)
            + (ib * sa) * (ib * sa);
        let sigma = var.max(0.0).sqrt();
        let (status, detail) = if !intensity.is_finite() {
            (SubPointStatus::NonFinite, Some("non-finite result".into()))
        } else if intensity < 0.0 {
            (SubPointStatus::Negative, Some("negative background-subtracted intensity".into()))
        } else {
            (SubPointStatus::Ok, None)
        };
        points.push(SubPoint { q, q_unit: "A^-1".into(), sample: p.intensity,
            buffer: ib, intensity, sigma, status, detail });
    }
    let ok: Vec<&SubPoint> = points.iter().filter(|p| p.status == SubPointStatus::Ok).collect();
    let residual = if cand.state != CandidateState::Degenerate && !ok.is_empty() {
        let n = ok.len() as f64;
        let mean = ok.iter().map(|p| p.intensity).sum::<f64>() / n;
        let var = ok.iter().map(|p| (p.intensity - mean).powi(2)).sum::<f64>() / n;
        let chi2 = ok.iter().map(|p| (p.intensity - mean).powi(2) / (p.sigma * p.sigma + 1e-30)).sum::<f64>();
        let n_negative = points.iter().filter(|p| p.status == SubPointStatus::Negative).count();
        let mut n_runs = 1usize;
        let mut last = ok[0].intensity >= 0.0;
        for p in ok.iter().skip(1) {
            let sgn = p.intensity >= 0.0;
            if sgn != last { n_runs += 1; last = sgn; }
        }
        Some(ResidualSummary { n: ok.len(), mean, std: var.sqrt(),
            reduced_chi2: chi2 / n.max(1.0), n_negative, n_runs })
    } else { None };

    cand.points = points;
    cand.residual = residual;
    if cand.state != CandidateState::Degenerate {
        cand.state = CandidateState::Completed;
    }
    let payload = json!({"sample": sample.raw_id, "buffer": buffer.raw_id,
        "mode": cand.mode, "scale": cand.scale, "scale_sigma": cand.scale_sigma,
        "overlap": cand.q_overlap, "state": cand.state});
    cand.id = hid("subcand", &payload);
    cand
}

/// Assemble a subtraction from one sample and every legal buffer candidate.
/// Multiple buffers => multiple candidates; multi-candidate residual testing
/// is preserved rather than collapsed to a single answer.
pub fn subtraction(sample: &Normalized,
                   buffer_branches: &[(Normalized, Option<(f64, f64)>)],
                   accepted: Option<String>) -> Subtraction {
    let candidates = buffer_branches.iter()
        .map(|(b, g)| build_candidate(sample, b, *g)).collect();
    let payload = json!({"sample_norm": sample.id,
        "candidates": candidates.iter().map(|c| &c.id).collect::<Vec<_>>()});
    Subtraction {
        id: hid("sub", &payload),
        sample_norm_id: sample.id.clone(),
        sample_name: sample.name.clone(),
        candidates, accepted,
    }
}
