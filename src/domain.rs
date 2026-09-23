//! SAXS domain rules: normalization, background subtraction, interval models.
//!
//! Invariants enforced here:
//! - q grids that differ are never subtracted directly; interpolation only
//!   happens inside the overlap range.
//! - Errors include normalization and background-scale uncertainty.
//! - Guinier and Porod intervals are never auto-merged into a single model;
//!   every legal candidate is kept when no unique decision is possible.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
    #[serde(default)]
    pub sigma: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PointStatus {
    Ok,
    NegativeIntensity,
    ExcludedNoOverlap,
    UnconvertibleUnit,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRef {
    pub curve: String,
    pub point: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurvePoint {
    pub id: String,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub status: PointStatus,
    #[serde(default)]
    pub sources: Vec<SourceRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Curve {
    pub id: String,
    pub kind: String,
    pub q_unit: String,
    pub i_unit: String,
    pub points: Vec<CurvePoint>,
    #[serde(default)]
    pub meta: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub detail: Value,
}

impl Diagnostic {
    pub fn new(code: &str, message: &str, detail: Value) -> Self {
        Diagnostic { code: code.to_string(), message: message.to_string(), detail }
    }
}

/// Multiplicative factor converting a q value of `unit` into 1/angstrom.
pub fn q_unit_factor(unit: &str) -> Option<f64> {
    match unit {
        "1/angstrom" | "1/A" | "A^-1" => Some(1.0),
        "1/nm" | "nm^-1" => Some(0.1),
        _ => None,
    }
}

/// Normalization: I_norm = I / (exposure * transmission * thickness).
/// Relative uncertainties of every normalization factor enter the error.
pub fn normalize(
    sample: &Curve,
    exposure: &Quantity,
    transmission: &Quantity,
    thickness: &Quantity,
) -> (Curve, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let factor = exposure.value * transmission.value * thickness.value;
    let rel2 = (exposure.sigma / exposure.value).powi(2)
        + (transmission.sigma / transmission.value).powi(2)
        + (thickness.sigma / thickness.value).powi(2);
    diags.push(Diagnostic::new(
        "NormalizationUncertainty",
        "error budget includes exposure/transmission/thickness uncertainty",
        json!({
            "exposure": exposure, "transmission": transmission, "thickness": thickness,
            "relative_variance": rel2,
        }),
    ));
    // Quantities whose unit cannot be converted stay in raw meta with a diagnostic.
    if let Some(obj) = sample.meta.as_object() {
        for (k, v) in obj {
            if let Some(unit) = v.get("unit").and_then(|u| u.as_str()) {
                if k.contains("distance") && q_unit_factor(unit).is_none() && unit != "mm" && unit != "m" {
                    diags.push(Diagnostic::new(
                        "UnitUnconvertible",
                        "value kept at raw position with diagnostic; not dropped for plotting",
                        json!({ "field": k, "raw": v }),
                    ));
                }
            }
        }
    }
    let points = sample
        .points
        .iter()
        .map(|p| CurvePoint {
            id: format!("norm:{}", p.id),
            q: p.q,
            i: p.i / factor,
            sigma: (p.i / factor).abs() * ((p.sigma / p.i).powi(2) + rel2).sqrt(),
            status: p.status.clone(),
            sources: vec![SourceRef { curve: sample.id.clone(), point: p.id.clone() }],
        })
        .collect();
    (
        Curve {
            id: format!("norm:{}", sample.id),
            kind: "normalized".into(),
            q_unit: sample.q_unit.clone(),
            i_unit: format!("{}/(s*mm)", sample.i_unit),
            points,
            meta: json!({ "normalization_factor": factor }),
        },
        diags,
    )
}

pub struct SubtractOutcome {
    pub curve: Option<Curve>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Background subtraction with scale factor s +/- sigma_s.
/// Never subtracts misaligned grids directly; interpolates the buffer onto the
/// sample grid only inside the overlap range. Points outside the overlap are
/// kept with status ExcludedNoOverlap. Error budget:
///   sigma^2 = sigma_sample^2 + (s*sigma_buffer)^2 + (I_buffer*sigma_s)^2
pub fn subtract(sample: &Curve, buffer: &Curve, scale: &Quantity) -> SubtractOutcome {
    let mut diags = Vec::new();
    diags.push(Diagnostic::new(
        "BackgroundScaleUncertainty",
        "background scale uncertainty contributes (I_buffer * sigma_scale)^2 to the error",
        json!({ "scale": scale }),
    ));

    let factor = match q_unit_factor(&buffer.q_unit) {
        Some(f) => f,
        None => {
            diags.push(Diagnostic::new(
                "UnitUnconvertible",
                "buffer q unit cannot be converted; subtraction refused, raw kept",
                json!({ "buffer_q_unit": buffer.q_unit }),
            ));
            return SubtractOutcome { curve: None, diagnostics: diags };
        }
    };
    let sample_factor = q_unit_factor(&sample.q_unit).unwrap_or(1.0);
    if (factor - sample_factor).abs() > 1e-12 {
        diags.push(Diagnostic::new(
            "UnitConverted",
            "buffer q grid converted to sample q unit in the derived layer; raw kept as imported",
            json!({ "from": buffer.q_unit, "to": sample.q_unit }),
        ));
    }
    let buf_q: Vec<f64> = buffer.points.iter().map(|p| p.q * factor / sample_factor).collect();

    let s_min = sample.points.iter().map(|p| p.q).fold(f64::INFINITY, f64::min);
    let s_max = sample.points.iter().map(|p| p.q).fold(f64::NEG_INFINITY, f64::max);
    let b_min = buf_q.iter().cloned().fold(f64::INFINITY, f64::min);
    let b_max = buf_q.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let lo = s_min.max(b_min);
    let hi = s_max.min(b_max);
    if lo >= hi {
        diags.push(Diagnostic::new(
            "NoOverlap",
            "sample and buffer q ranges do not overlap; subtraction refused",
            json!({ "sample_range": [s_min, s_max], "buffer_range": [b_min, b_max] }),
        ));
        return SubtractOutcome { curve: None, diagnostics: diags };
    }

    let s_step = median_step(&sample.points.iter().map(|p| p.q).collect::<Vec<_>>());
    let b_step = median_step(&buf_q);
    if (s_step - b_step).abs() > 1e-9 {
        diags.push(Diagnostic::new(
            "GridMisaligned",
            "q grids differ; buffer interpolated onto sample grid, never subtracted directly",
            json!({ "sample_step": s_step, "buffer_step": b_step }),
        ));
    }
    diags.push(Diagnostic::new(
        "InterpolatedRange",
        "interpolation only valid inside the overlap range",
        json!({ "overlap": [lo, hi] }),
    ));

    let mut points = Vec::new();
    let mut negative: Vec<String> = Vec::new();
    let mut excluded: Vec<String> = Vec::new();
    for p in &sample.points {
        if p.q < lo || p.q > hi {
            excluded.push(p.id.clone());
            points.push(CurvePoint {
                id: format!("sub:{}", p.id),
                q: p.q,
                i: f64::NAN,
                sigma: f64::NAN,
                status: PointStatus::ExcludedNoOverlap,
                sources: vec![SourceRef { curve: sample.id.clone(), point: p.id.clone() }],
            });
            continue;
        }
        let (bi, bsigma, srcs) = interpolate(buffer, &buf_q, p.q);
        let i = p.i - scale.value * bi;
        let var = p.sigma.powi(2)
            + (scale.value * bsigma).powi(2)
            + (bi * scale.sigma).powi(2);
        let mut status = PointStatus::Ok;
        if i < 0.0 {
            status = PointStatus::NegativeIntensity;
            negative.push(p.id.clone());
        }
        let mut sources = vec![SourceRef { curve: sample.id.clone(), point: p.id.clone() }];
        sources.extend(srcs);
        points.push(CurvePoint {
            id: format!("sub:{}", p.id),
            q: p.q,
            i,
            sigma: var.sqrt(),
            status,
            sources,
        });
    }
    if !negative.is_empty() {
        diags.push(Diagnostic::new(
            "NegativeIntensity",
            "negative intensities kept and flagged, not clamped",
            json!({ "points": negative }),
        ));
    }
    if !excluded.is_empty() {
        diags.push(Diagnostic::new(
            "ExcludedNoOverlap",
            "points outside the overlap range excluded from subtraction",
            json!({ "points": excluded }),
        ));
    }
    (
        SubtractOutcome {
            curve: Some(Curve {
                id: format!("sub:{}", sample.id),
                kind: "subtracted".into(),
                q_unit: sample.q_unit.clone(),
                i_unit: sample.i_unit.clone(),
                points,
                meta: json!({ "buffer": buffer.id, "scale": scale, "overlap": [lo, hi] }),
            }),
            diagnostics: diags,
        }
    )
}

fn median_step(qs: &[f64]) -> f64 {
    if qs.len() < 2 {
        return 0.0;
    }
    let mut steps: Vec<f64> = qs.windows(2).map(|w| w[1] - w[0]).collect();
    steps.sort_by(|a, b| a.partial_cmp(b).unwrap());
    steps[steps.len() / 2]
}

/// Linear interpolation of the buffer curve at q; returns (I, sigma, source refs
/// of the two bracketing buffer points).
fn interpolate(buffer: &Curve, buf_q: &[f64], q: f64) -> (f64, f64, Vec<SourceRef>) {
    let pts = &buffer.points;
    let mut j = 0;
    while j + 1 < pts.len() && buf_q[j + 1] < q {
        j += 1;
    }
    let k = (j + 1).min(pts.len() - 1);
    let (q0, q1) = (buf_q[j], buf_q[k]);
    let t = if (q1 - q0).abs() < 1e-15 { 0.0 } else { (q - q0) / (q1 - q0) };
    let i = pts[j].i + t * (pts[k].i - pts[j].i);
    let s = pts[j].sigma + t * (pts[k].sigma - pts[j].sigma);
    (
        i,
        s,
        vec![
            SourceRef { curve: buffer.id.clone(), point: pts[j].id.clone() },
            SourceRef { curve: buffer.id.clone(), point: pts[k].id.clone() },
        ],
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalSpec {
    pub id: String,
    pub kind: String, // "guinier" | "porod"
    pub q_min: f64,
    pub q_max: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CandidateStatus {
    Pending,
    Chosen,
    Unchosen,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub id: String,
    pub kind: String,
    pub q_min: f64,
    pub q_max: f64,
    pub params: Value,
    pub residuals: Vec<Value>,
    pub status: CandidateStatus,
    pub overlaps: Vec<String>,
}

fn weighted_linfit(xs: &[f64], ys: &[f64], ws: &[f64]) -> (f64, f64) {
    let sw: f64 = ws.iter().sum();
    let sx: f64 = xs.iter().zip(ws).map(|(x, w)| x * w).sum();
    let sy: f64 = ys.iter().zip(ws).map(|(y, w)| y * w).sum();
    let sxx: f64 = xs.iter().zip(ws).map(|(x, w)| x * x * w).sum();
    let sxy: f64 = xs.iter().zip(ws).map(|(x, w)| x * w).zip(ys).map(|(a, y)| a * y).sum();
    let d = sw * sxx - sx * sx;
    let slope = (sw * sxy - sx * sy) / d;
    let intercept = (sy - slope * sx) / sw;
    (slope, intercept)
}

/// Fit Guinier/Porod interval candidates. Overlapping intervals of different
/// kinds are flagged but never merged into a single model; all candidates are
/// returned so no legal alternative is swallowed.
pub fn fit_intervals(curve: &Curve, specs: &[IntervalSpec]) -> (Vec<IntervalCandidate>, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let mut cands = Vec::new();
    for spec in specs {
        let sel: Vec<&CurvePoint> = curve
            .points
            .iter()
            .filter(|p| p.q >= spec.q_min && p.q <= spec.q_max && p.i.is_finite() && p.i > 0.0)
            .collect();
        if sel.len() < 3 {
            diags.push(Diagnostic::new(
                "IntervalTooSparse",
                "fewer than 3 usable points in interval; candidate not fitted",
                json!({ "interval": spec.id }),
            ));
            continue;
        }
        let (xs, ys, ws): (Vec<f64>, Vec<f64>, Vec<f64>) = match spec.kind.as_str() {
            "guinier" => (
                sel.iter().map(|p| p.q * p.q).collect(),
                sel.iter().map(|p| p.i.ln()).collect(),
                sel.iter().map(|p| (p.i / p.sigma).powi(2)).collect(),
            ),
            _ => (
                sel.iter().map(|p| p.q.ln()).collect(),
                sel.iter().map(|p| p.i.ln()).collect(),
                sel.iter().map(|p| (p.i / p.sigma).powi(2)).collect(),
            ),
        };
        let (slope, intercept) = weighted_linfit(&xs, &ys, &ws);
        let params = if spec.kind == "guinier" {
            json!({ "Rg_A": (-3.0 * slope).max(0.0).sqrt(), "I0": intercept.exp(), "slope": slope })
        } else {
            json!({ "porod_exponent": -slope, "I0_proxy": intercept.exp(), "slope": slope })
        };
        let residuals: Vec<Value> = sel
            .iter()
            .zip(&xs)
            .zip(&ys)
            .map(|((p, x), y)| {
                let fit = slope * x + intercept;
                json!({ "point": p.id, "q": p.q, "residual": y - fit })
            })
            .collect();
        cands.push(IntervalCandidate {
            id: spec.id.clone(),
            kind: spec.kind.clone(),
            q_min: spec.q_min,
            q_max: spec.q_max,
            params,
            residuals,
            status: CandidateStatus::Pending,
            overlaps: Vec::new(),
        });
    }
    // Flag overlaps between different model kinds; never merge them.
    let mut overlap_pairs = Vec::new();
    for a in 0..cands.len() {
        for b in (a + 1)..cands.len() {
            let (ca, cb) = (cands[a].clone(), cands[b].clone());
            if ca.kind != cb.kind && ca.q_min < cb.q_max && cb.q_min < ca.q_max {
                overlap_pairs.push(json!([ca.id, cb.id]));
                cands[a].overlaps.push(cb.id.clone());
                cands[b].overlaps.push(ca.id.clone());
            }
        }
    }
    if !overlap_pairs.is_empty() {
        diags.push(Diagnostic::new(
            "IntervalOverlap",
            "Guinier and Porod intervals overlap; kept as separate candidates, not merged into a single model",
            json!({ "pairs": overlap_pairs }),
        ));
    }
    if cands.iter().filter(|c| c.status == CandidateStatus::Pending).count() > 1 {
        diags.push(Diagnostic::new(
            "MultiCandidateResiduals",
            "multiple legal candidates retained with per-point residuals; no unique answer forced",
            json!({ "candidates": cands.iter().map(|c| c.id.clone()).collect::<Vec<_>>() }),
        ));
    }
    (cands, diags)
}
