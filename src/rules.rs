use crate::model::*;

pub const RULE_VERSION: &str = "saxs-rules/1.0";

/// Convert a q value to 1/Angstrom. Returns::None when the unit is unknown:
/// the value must stay where it is and get a diagnostic instead.
pub fn q_to_angstrom(q: f64, unit: &str) -> Option<f64> {
    match unit {
        "1/Angstrom" | "1/A" | "A^-1" => Some(q),
        "1/nm" | "nm^-1" => Some(q / 10.0),
        _ => None,
    }
}

/// Parse a raw CSV (q,i,sigma) keeping the raw text untouched.
pub fn parse_curve_csv(label: &str, q_unit: &str, i_unit: &str, raw: &str) -> RawCurve {
    let mut points = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("q,") {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < 3 {
            continue;
        }
        if let (Ok(q), Ok(i), Ok(s)) = (
            cols[0].trim().parse::<f64>(),
            cols[1].trim().parse::<f64>(),
            cols[2].trim().parse::<f64>(),
        ) {
            points.push(CurvePoint { q, i, sigma: s });
        }
    }
    RawCurve {
        label: label.to_string(),
        q_unit: q_unit.to_string(),
        i_unit: i_unit.to_string(),
        points,
        raw: raw.to_string(),
    }
}

/// Normalization: I_norm = I / (exposure_s * thickness_cm * transmission).
/// Uncertainty propagates the transmission relative uncertainty.
/// q is normalized to 1/Angstrom; unconvertible q values stay in place and
/// collect a diagnostic instead of being dropped.
pub fn normalize(raw: &RawCurve, exposure: &Qty, thickness: &Qty, transmission: &Qty) -> NormCurve {
    let mut diagnostics = Vec::new();
    let factor = exposure.value * thickness.value * transmission.value;
    let t_rel = transmission.unc.unwrap_or(0.0) / transmission.value;
    let mut points = Vec::new();
    for p in &raw.points {
        match q_to_angstrom(p.q, &raw.q_unit) {
            Some(q) => {
                let i = p.i / factor;
                let sigma = (p.sigma / factor).hypot(i * t_rel);
                points.push(CurvePoint { q, i, sigma });
            }
            None => diagnostics.push(Diagnostic {
                code: "unit_unconvertible_kept_in_place".into(),
                message: format!(
                    "q={} [{}] cannot be converted to 1/Angstrom; value kept in raw layer only",
                    p.q, raw.q_unit
                ),
                q: None,
            }),
        }
    }
    NormCurve {
        label: raw.label.clone(),
        q_unit: "1/Angstrom".into(),
        i_unit: "1/cm".into(),
        points,
        norm_factor: factor,
        diagnostics,
    }
}

/// Linear interpolation of the buffer curve at q. Only valid inside the
/// buffer's own q range; callers must restrict to the overlap region.
fn interp(buf: &[CurvePoint], q: f64) -> Option<(f64, f64, String, Vec<usize>)> {
    if buf.is_empty() {
        return None;
    }
    for (idx, p) in buf.iter().enumerate() {
        if (p.q - q).abs() < 1e-12 {
            return Some((p.i, p.sigma, "exact_grid".into(), vec![idx]));
        }
    }
    for w in 0..buf.len() - 1 {
        let (a, b) = (&buf[w], &buf[w + 1]);
        if a.q <= q && q <= b.q {
            let t = (q - a.q) / (b.q - a.q);
            let i = a.i + t * (b.i - a.i);
            let s = a.sigma + t * (b.sigma - a.sigma);
            return Some((i, s, "linear_interp".into(), vec![w, w + 1]));
        }
    }
    None
}

/// Background subtraction with the project invariants:
/// - grids that differ are never subtracted point-by-point directly;
/// - interpolation happens only inside the overlap of both q ranges;
/// - the error budget includes normalization and background-scale
///   uncertainty: sigma^2 = s_s^2 + (scale*s_b)^2 + (I_b*scale_unc)^2.
pub fn subtract(sample: &NormCurve, buffer: &NormCurve, scale: f64, scale_unc: f64) -> Subtraction {
    let mut diagnostics = Vec::new();
    let same_grid = sample.points.len() == buffer.points.len()
        && sample
            .points
            .iter()
            .zip(buffer.points.iter())
            .all(|(a, b)| (a.q - b.q).abs() < 1e-12);
    if !same_grid {
        diagnostics.push(Diagnostic {
            code: "grid_mismatch_no_direct_subtract".into(),
            message: "q grids differ; no direct point-wise subtraction; \
                      buffer is interpolated onto the sample grid inside the overlap only"
                .into(),
            q: None,
        });
    }
    let (s_min, s_max) = (
        sample.points.first().map(|p| p.q),
        sample.points.last().map(|p| p.q),
    );
    let (b_min, b_max) = (
        buffer.points.first().map(|p| p.q),
        buffer.points.last().map(|p| p.q),
    );
    let (s_min, s_max, b_min, b_max) = match (s_min, s_max, b_min, b_max) {
        (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
        _ => {
            return Subtraction {
                scale,
                scale_unc,
                overlap: None,
                status: "no_overlap".into(),
                points: vec![],
                diagnostics: vec![Diagnostic {
                    code: "no_overlap".into(),
                    message: "empty curve; no overlap".into(),
                    q: None,
                }],
            }
        }
    };
    let lo = s_min.max(b_min);
    let hi = s_max.min(b_max);
    if lo >= hi {
        diagnostics.push(Diagnostic {
            code: "no_overlap".into(),
            message: format!(
                "sample range [{s_min:.4},{s_max:.4}] and buffer range [{b_min:.4},{b_max:.4}] do not overlap; subtraction refused"
            ),
            q: None,
        });
        return Subtraction {
            scale,
            scale_unc,
            overlap: None,
            status: "no_overlap".into(),
            points: vec![],
            diagnostics,
        };
    }
    diagnostics.push(Diagnostic {
        code: "interpolation_overlap_only".into(),
        message: format!("interpolation restricted to overlap [{lo:.4}, {hi:.4}] 1/Angstrom"),
        q: None,
    });
    diagnostics.push(Diagnostic {
        code: "error_includes_norm_and_scale".into(),
        message: format!(
            "sigma^2 = sigma_sample^2 + (scale*sigma_buffer)^2 + (I_buffer*scale_unc)^2 with scale={scale}+-{scale_unc}"
        ),
        q: None,
    });

    let mut points = Vec::new();
    for sp in &sample.points {
        if sp.q < lo || sp.q > hi {
            points.push(SubPoint {
                q: sp.q,
                i_sub: f64::NAN,
                sigma_sub: f64::NAN,
                status: "out_of_overlap".into(),
                sample: sp.clone(),
                buffer: None,
            });
            continue;
        }
        let (bi, bs, method, src) = interp(&buffer.points, sp.q).expect("inside overlap");
        let i_sub = sp.i - scale * bi;
        let sigma_sub = (sp.sigma.powi(2) + (scale * bs).powi(2) + (bi * scale_unc).powi(2)).sqrt();
        let status = if i_sub < 0.0 {
            diagnostics.push(Diagnostic {
                code: "negative_intensity".into(),
                message: format!("I_sub({:.4}) = {:.4} < 0; point kept and flagged", sp.q, i_sub),
                q: Some(sp.q),
            });
            "negative_intensity"
        } else {
            "ok"
        };
        points.push(SubPoint {
            q: sp.q,
            i_sub,
            sigma_sub,
            status: status.into(),
            sample: sp.clone(),
            buffer: Some(BufferTrace {
                q: sp.q,
                i_interp: bi,
                sigma_interp: bs,
                method,
                src_indices: src,
            }),
        });
    }
    let status = if points.iter().any(|p| p.status == "out_of_overlap") {
        "partial"
    } else {
        "ok"
    };
    Subtraction {
        scale,
        scale_unc,
        overlap: Some((lo, hi)),
        status: status.into(),
        points,
        diagnostics,
    }
}

fn linfit(xs: &[f64], ys: &[f64]) -> (f64, f64, f64) {
    let n = xs.len() as f64;
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let d = n * sxx - sx * sx;
    let slope = (n * sxy - sx * sy) / d;
    let intercept = (sy - slope * sx) / n;
    let chi2: f64 = xs
        .iter()
        .zip(ys)
        .map(|(x, y)| {
            let r = y - (slope * x + intercept);
            r * r
        })
        .sum();
    (slope, intercept, chi2 / n.max(1.0))
}

/// Guinier candidates: ln I vs q^2 over several low-q windows. Multiple
/// candidates with comparable residuals are all kept (status "ambiguous");
/// nothing is merged with Porod ranges.
pub fn guinier_candidates(sub: &Subtraction) -> Vec<IntervalCandidate> {
    let ok: Vec<&SubPoint> = sub
        .points
        .iter()
        .filter(|p| p.status != "out_of_overlap" && p.i_sub > 0.0)
        .collect();
    let mut out = Vec::new();
    for &(frac_lo, frac_hi) in &[(0.0f64, 0.25f64), (0.05, 0.35), (0.0, 0.45)] {
        let n = ok.len();
        let (a, b) = ((n as f64 * frac_lo) as usize, (n as f64 * frac_hi) as usize);
        if b - a < 4 {
            continue;
        }
        let win = &ok[a..b];
        let xs: Vec<f64> = win.iter().map(|p| p.q * p.q).collect();
        let ys: Vec<f64> = win.iter().map(|p| p.i_sub.ln()).collect();
        let (slope, intercept, chi) = linfit(&xs, &ys);
        let rg = (-3.0 * slope).max(0.0).sqrt();
        let i0 = intercept.exp();
        let mut diag = vec![Diagnostic {
            code: "intervals_not_merged".into(),
            message: "Guinier interval stays a separate candidate; it is never auto-merged with Porod ranges".into(),
            q: None,
        }];
        if rg * win.last().unwrap().q > 1.3 {
            diag.push(Diagnostic {
                code: "guinier_qrg_limit".into(),
                message: format!("q_max*Rg = {:.2} exceeds 1.3; candidate kept but flagged", rg * win.last().unwrap().q),
                q: None,
            });
        }
        out.push(IntervalCandidate {
            kind: "guinier".into(),
            q_min: win.first().unwrap().q,
            q_max: win.last().unwrap().q,
            params: serde_json::json!({"Rg_A": rg, "I0": i0}),
            residual: chi,
            chosen: false,
            diagnostics: diag,
        });
    }
    mark_ambiguity(&mut out);
    out
}

/// Porod candidates: ln I vs ln q over several high-q windows.
pub fn porod_candidates(sub: &Subtraction) -> Vec<IntervalCandidate> {
    let ok: Vec<&SubPoint> = sub
        .points
        .iter()
        .filter(|p| p.status != "out_of_overlap" && p.i_sub > 0.0)
        .collect();
    let mut out = Vec::new();
    for &(frac_lo, frac_hi) in &[(0.55f64, 1.0f64), (0.7, 1.0)] {
        let n = ok.len();
        let (a, b) = ((n as f64 * frac_lo) as usize, (n as f64 * frac_hi) as usize);
        if b - a < 4 {
            continue;
        }
        let win = &ok[a..b];
        let xs: Vec<f64> = win.iter().map(|p| p.q.ln()).collect();
        let ys: Vec<f64> = win.iter().map(|p| p.i_sub.ln()).collect();
        let (slope, intercept, chi) = linfit(&xs, &ys);
        out.push(IntervalCandidate {
            kind: "porod".into(),
            q_min: win.first().unwrap().q,
            q_max: win.last().unwrap().q,
            params: serde_json::json!({"porod_exponent": -slope, "scale": intercept.exp()}),
            residual: chi,
            chosen: false,
            diagnostics: vec![Diagnostic {
                code: "intervals_not_merged".into(),
                message: "Porod interval stays a separate candidate; it is never auto-merged with Guinier ranges".into(),
                q: None,
            }],
        });
    }
    mark_ambiguity(&mut out);
    out
}

fn mark_ambiguity(cands: &mut [IntervalCandidate]) {
    if cands.len() < 2 {
        return;
    }
    let best = cands
        .iter()
        .map(|c| c.residual)
        .fold(f64::INFINITY, f64::min);
    let comparable = cands.iter().filter(|c| c.residual <= best * 4.0).count() > 1;
    if comparable {
        for c in cands.iter_mut() {
            c.diagnostics.push(Diagnostic {
                code: "multi_candidate_ambiguous".into(),
                message: "residuals are comparable; all legal candidates are kept for review".into(),
                q: None,
            });
        }
    }
}

/// Detect overlap between Guinier and Porod candidate ranges; ranges are
/// reported but never merged into a single model.
pub fn interval_overlap_diagnostics(
    guinier: &[IntervalCandidate],
    porod: &[IntervalCandidate],
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for g in guinier {
        for p in porod {
            if g.q_min < p.q_max && p.q_min < g.q_max {
                out.push(Diagnostic {
                    code: "interval_overlap".into(),
                    message: format!(
                        "guinier [{:.4},{:.4}] overlaps porod [{:.4},{:.4}]; both kept as separate candidates",
                        g.q_min, g.q_max, p.q_min, p.q_max
                    ),
                    q: None,
                });
            }
        }
    }
    out
}
