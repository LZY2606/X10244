use crate::model::*;
use serde_json::json;

/// Absolute normalization of a sample curve.
/// I_norm = I / (exposure * thickness * transmission).
/// Uncertainty includes counting error AND transmission (normalization) error.
pub fn normalize(src: &SourceRecord) -> (Curve, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let raw = src.curve.as_ref().expect("normalized input has curve");
    let meta = src.meta.as_ref().expect("normalized input has meta");
    let denom = meta.exposure_s * meta.thickness_mm * meta.transmission;
    if denom <= 0.0 {
        diags.push(Diagnostic {
            code: "invalid-normalization-factor".into(),
            message: format!("exposure*thickness*transmission = {}", denom),
            subject: Some(src.id.clone()),
        });
    }
    let rel_t = if meta.transmission != 0.0 {
        meta.transmission_sigma / meta.transmission
    } else {
        0.0
    };
    let points = raw
        .points
        .iter()
        .map(|p| {
            let i = p.i / denom;
            // sigma/I from counting + transmission normalization uncertainty
            let rel_i = if p.i != 0.0 { p.sigma / p.i.abs() } else { 0.0 };
            let rel = (rel_i * rel_i + rel_t * rel_t).sqrt();
            Point {
                q: p.q,
                i,
                sigma: i.abs() * rel,
            }
        })
        .collect();
    let curve = Curve {
        q_unit: raw.q_unit.clone(),
        i_unit: "1/cm".into(),
        points,
    };
    (curve, diags)
}

/// Overlap window [max(lo), min(hi)] in shared q units.
fn overlap_range(a: &Curve, b: &Curve) -> Option<(f64, f64)> {
    let lo = a.q_min().max(b.q_min());
    let hi = a.q_max().min(b.q_max());
    if hi > lo + 1e-12 {
        Some((lo, hi))
    } else {
        None
    }
}

/// Piecewise-linear interpolation of src onto target q (both must share q unit).
fn interpolate_at(src: &Curve, q: f64) -> Point {
    let p = &src.points;
    if q <= p.first().unwrap().q {
        return p.first().unwrap().clone();
    }
    if q >= p.last().unwrap().q {
        return p.last().unwrap().clone();
    }
    for w in p.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if q >= a.q && q <= b.q {
            let f = (q - a.q) / (b.q - a.q);
            return Point {
                q,
                i: a.i + f * (b.i - a.i),
                sigma: (a.sigma * a.sigma + f * f * (b.sigma * b.sigma - a.sigma * a.sigma)).sqrt(),
            };
        }
    }
    unreachable!()
}

pub struct SubtractParams {
    pub scale: f64,       // buffer scale factor (typically fitted)
    pub scale_sigma: f64, // background scale uncertainty
}

pub struct SubtractOutcome {
    pub curve: Option<Curve>,
    pub status: NodeStatus,
    pub diagnostics: Vec<Diagnostic>,
    pub mode: String, // "direct" | "interpolated-overlap" | "no-overlap"
    pub overlap: Option<(f64, f64)>,
}

/// Background subtraction with the hard grid rule:
/// different q grids are NEVER subtracted pointwise; the buffer is interpolated
/// onto the sample grid and ONLY inside the shared overlap range.
pub fn subtract(sample: &Curve, buffer: &Curve, p: &SubtractParams, sample_id: &str) -> SubtractOutcome {
    let mut diags = Vec::new();
    if sample.q_unit != buffer.q_unit {
        return SubtractOutcome {
            curve: None,
            status: NodeStatus::Failed("incompatible-q-units".into()),
            diagnostics: vec![Diagnostic {
                code: "incompatible-q-units".into(),
                message: format!("sample {} vs buffer {}", sample.q_unit, buffer.q_unit),
                subject: Some(sample_id.into()),
            }],
            mode: "no-overlap".into(),
            overlap: None,
        };
    }
    let same = sample.same_grid(buffer);
    let overlap = overlap_range(sample, buffer);
    if !same && overlap.is_none() {
        return SubtractOutcome {
            curve: None,
            status: NodeStatus::Failed("no-overlap".into()),
            diagnostics: vec![Diagnostic {
                code: "no-overlap".into(),
                message: "sample and buffer q ranges do not overlap; subtraction refused".into(),
                subject: Some(sample_id.into()),
            }],
            mode: "no-overlap".into(),
            overlap: None,
        };
    }
    let (mode, points): (String, Vec<Point>) = if same {
        ("direct".into(), direct(sample, buffer, p))
    } else {
        diags.push(Diagnostic {
            code: "q-grid-mismatch".into(),
            message: "q grids differ; subtraction restricted to overlap via interpolation".into(),
            subject: Some(sample_id.into()),
        });
        ("interpolated-overlap".into(), interpolated(sample, buffer, p, overlap.unwrap()))
    };

    let negatives = points.iter().filter(|p| p.i < 0.0).count();
    if negatives > 0 {
        diags.push(Diagnostic {
            code: "negative-intensity".into(),
            message: format!("{} subtracted points are negative; retained, not clipped", negatives),
            subject: Some(sample_id.into()),
        });
    }
    let curve = Curve {
        q_unit: sample.q_unit.clone(),
        i_unit: sample.i_unit.clone(),
        points,
    };
    let status = NodeStatus::Valid;
    SubtractOutcome {
        curve: Some(curve),
        status,
        diagnostics: diags,
        mode,
        overlap,
    }
}

fn direct(sample: &Curve, buffer: &Curve, p: &SubtractParams) -> Vec<Point> {
    sample
        .points
        .iter()
        .zip(buffer.points.iter())
        .map(|(s, b)| {
            let ib = p.scale * b.i;
            let sub = s.i - ib;
            // counting + buffer counting (scaled) + background scale uncertainty
            let var = s.sigma * s.sigma
                + (p.scale * b.sigma).powi(2)
                + (p.scale_sigma * b.i).powi(2);
            Point {
                q: s.q,
                i: sub,
                sigma: var.sqrt(),
            }
        })
        .collect()
}

fn interpolated(
    sample: &Curve,
    buffer: &Curve,
    p: &SubtractParams,
    (lo, hi): (f64, f64),
) -> Vec<Point> {
    let mut out = Vec::new();
    for s in sample.points.iter() {
        if s.q < lo - 1e-12 || s.q > hi + 1e-12 {
            continue; // interpolation only inside overlap
        }
        let b = interpolate_at(buffer, s.q);
        let ib = p.scale * b.i;
        let var = s.sigma * s.sigma
            + (p.scale * b.sigma).powi(2)
            + (p.scale_sigma * b.i).powi(2);
        out.push(Point {
            q: s.q,
            i: s.i - ib,
            sigma: var.sqrt(),
        });
    }
    out
}

/// Residual of a subtraction result against a flat model over the analysed
/// window. Kept per buffer candidate so several candidates can coexist.
pub fn residual_stats(curve: &Curve, q_lo: f64, q_hi: f64) -> serde_json::Value {
    let vals: Vec<f64> = curve
        .points
        .iter()
        .filter(|p| p.q >= q_lo && p.q <= q_hi)
        .map(|p| p.i)
        .collect();
    let n = vals.len().max(1) as f64;
    let mean = vals.iter().sum::<f64>() / n;
    let chi = curve
        .points
        .iter()
        .filter(|p| p.q >= q_lo && p.q <= q_hi)
        .map(|p| {
            let s = if p.sigma > 0.0 { p.sigma } else { 1.0 };
            ((p.i - mean) / s).powi(2)
        })
        .sum::<f64>()
        / n;
    json!({"n": n as u64, "mean": mean, "chi2_red": chi, "q_lo": q_lo, "q_hi": q_hi})
}

/// Sliding-window Guinier candidates (ln I vs q^2).
/// Overlapping windows are retained as separate candidates; the system never
/// merges Guinier and Porod evidence into one model automatically.
pub fn guinier_candidates(node_id: &str, curve: &Curve, window: usize, step: usize) -> Vec<IntervalCandidate> {
    let pts: Vec<&Point> = curve.points.iter().filter(|p| p.i > 0.0).collect();
    let mut out = Vec::new();
    if pts.len() < window {
        return out;
    }
    let mut k = 0usize;
    let mut start = 0usize;
    while start + window <= pts.len() {
        let win = &pts[start..start + window];
        let xs: Vec<f64> = win.iter().map(|p| p.q * p.q).collect();
        let ys: Vec<f64> = win.iter().map(|p| p.i.ln()).collect();
        if let Some((slope, intercept, r2)) = linfit(&xs, &ys) {
            if slope < 0.0 && r2 > 0.8 {
                let rg = (-3.0 * slope).sqrt();
                let qlo = win.first().unwrap().q;
                let qhi = win.last().unwrap().q;
                out.push(IntervalCandidate {
                    id: format!("{}#guinier-{}", node_id, k),
                    q_lo: qlo,
                    q_hi: qhi,
                    fit: json!({"slope": slope, "intercept": intercept, "r2": r2,
                                "rg": rg, "qrg_max": qhi * rg}),
                    accepted: None,
                });
            }
        }
        start += step;
        k += 1;
    }
    // keep even overlapping intervals; annotate overlap pairs upstream
    out
}

/// Sliding-window Porod candidates (ln I vs q) over the high-q tail.
pub fn porod_candidates(node_id: &str, curve: &Curve, window: usize, step: usize) -> Vec<IntervalCandidate> {
    let pts: Vec<&Point> = curve
        .points
        .iter()
        .filter(|p| p.i > 0.0)
        .collect();
    let mut out = Vec::new();
    if pts.len() < window {
        return out;
    }
    let mut start = 0usize;
    let mut k = 0usize;
    while start + window <= pts.len() {
        let win = &pts[start..start + window];
        let xs: Vec<f64> = win.iter().map(|p| p.q.ln()).collect();
        let ys: Vec<f64> = win.iter().map(|p| p.i.ln()).collect();
        if let Some((slope, intercept, r2)) = linfit(&xs, &ys) {
            let qlo = win.first().unwrap().q;
            let qhi = win.last().unwrap().q;
            // Porod-like if slope in roughly [-6,-2]
            if slope < -1.5 && r2 > 0.6 {
                out.push(IntervalCandidate {
                    id: format!("{}#porod-{}", node_id, k),
                    q_lo: qlo,
                    q_hi: qhi,
                    fit: json!({"slope": slope, "intercept": intercept, "r2": r2}),
                    accepted: None,
                });
            }
        }
        start += step;
        k += 1;
    }
    out
}

/// Annotate overlapping interval pairs ("interval-overlap" evidence retained).
pub fn overlapping_pairs(a: &[IntervalCandidate], b: &[IntervalCandidate]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for x in a {
        for y in b {
            if x.q_lo < y.q_hi && y.q_lo < x.q_hi {
                pairs.push((x.id.clone(), y.id.clone()));
            }
        }
    }
    pairs
}

fn linfit(xs: &[f64], ys: &[f64]) -> Option<(f64, f64, f64)> {
    let n = xs.len() as f64;
    let mx = xs.iter().sum::<f64>() / n;
    let my = ys.iter().sum::<f64>() / n;
    let mut cov = 0.0;
    let mut vx = 0.0;
    let mut vy = 0.0;
    for (x, y) in xs.iter().zip(ys) {
        cov += (x - mx) * (y - my);
        vx += (x - mx).powi(2);
        vy += (y - my).powi(2);
    }
    if vx <= 0.0 || vy <= 0.0 {
        return None;
    }
    let slope = cov / vx;
    let intercept = my - slope * mx;
    let r = cov / (vx * vy).sqrt();
    Some((slope, intercept, r * r))
}
