//! Calculation rules (deterministic, pure).
//!
//! Invariants enforced here:
//! * different q grids are never subtracted point-wise directly;
//! * interpolation is only used inside the overlapping q range;
//! * propagated errors include normalization and buffer-scale uncertainty;
//! * Guinier and Porod are independent pieces of evidence, never stitched.

use crate::ids::point_id;
use crate::model::*;
use crate::units::{convert_to_base, unit_factor};

pub fn parse_run(input: RunImportInput) -> ParsedRun {
    let n = input.rows.len();
    let mut q = vec![f64::NAN; n];
    let mut i = vec![f64::NAN; n];
    let mut e = vec![f64::NAN; n];
    let mut usable = vec![false; n];
    let mut diags = Vec::new();

    for (idx, row) in input.rows.iter().enumerate() {
        let mut row_ok = true;
        match row.q.parse::<f64>() {
            Ok(v) => match convert_to_base(v, "q", &input.q_unit) {
                Ok(b) => q[idx] = b,
                Err(reason) => {
                    row_ok = false;
                    diags.push(Diagnostic::error("UNIT_Q", reason, Some(idx)));
                }
            },
            Err(_) => {
                row_ok = false;
                diags.push(Diagnostic::error("PARSE_Q", format!("cannot parse '{}'", row.q), Some(idx)));
            }
        }
        match row.i.parse::<f64>() {
            Ok(v) => match convert_to_base(v, "intensity", &input.i_unit) {
                Ok(b) => i[idx] = b,
                Err(reason) => {
                    row_ok = false;
                    diags.push(Diagnostic::error("UNIT_I", reason, Some(idx)));
                }
            },
            Err(_) => {
                row_ok = false;
                diags.push(Diagnostic::error("PARSE_I", format!("cannot parse '{}'", row.i), Some(idx)));
            }
        }
        match row.e.parse::<f64>() {
            Ok(v) => match convert_to_base(v, "intensity", &input.err_unit) {
                Ok(b) => e[idx] = b,
                Err(reason) => {
                    row_ok = false;
                    diags.push(Diagnostic::error("UNIT_E", reason, Some(idx)));
                }
            },
            Err(_) => {
                row_ok = false;
                diags.push(Diagnostic::error("PARSE_E", format!("cannot parse '{}'", row.e), Some(idx)));
            }
        }
        usable[idx] = row_ok;
    }

    let mut scalar = |raw: &RawScalar, dim: &str, code: &str| -> Option<f64> {
        match raw.raw.parse::<f64>() {
            Ok(v) => match convert_to_base(v, dim, &raw.unit) {
                Ok(b) => Some(b),
                Err(reason) => {
                    diags.push(Diagnostic::error(code, reason, None));
                    None
                }
            },
            Err(_) => {
                diags.push(Diagnostic::error(code, format!("cannot parse '{}'", raw.raw), None));
                None
            }
        }
    };
    let exposure_s = scalar(&input.exposure, "time", "UNIT_EXPOSURE");
    let thickness_mm = scalar(&input.thickness, "length", "UNIT_THICKNESS");
    let transmission = scalar(&input.transmission, "transmission", "UNIT_TRANSMISSION");

    ParsedRun {
        input,
        q,
        i,
        e,
        row_usable: usable,
        exposure_s,
        thickness_mm,
        transmission,
        diagnostics: diags,
    }
}

/// Effective parsed run after applying an amendment (raw rows retained in
/// `input`; only the parsed effective values are replaced and reparsed).
pub fn apply_amendment(base: &ParsedRun, amend: &RunAmendment) -> ParsedRun {
    let mut input = base.input.clone();
    for (idx, row) in &amend.rows {
        if *idx < input.rows.len() {
            input.rows[*idx] = row.clone();
        }
    }
    let mut reparsed = parse_run(input);
    reparsed.diagnostics.push(Diagnostic::info(
        "AMENDMENT",
        format!("revision {}: {}", amend.revision, amend.reason),
        None,
    ));
    reparsed
}

fn response_factor(resp: &InstrumentResponse, q: f64) -> f64 {
    if resp.points.is_empty() {
        return 1.0;
    }
    if q <= resp.points[0].q {
        return resp.points[0].factor;
    }
    let last = resp.points.len() - 1;
    if q >= resp.points[last].q {
        return resp.points[last].factor;
    }
    for w in resp.points.windows(2) {
        if q >= w[0].q && q <= w[1].q {
            let t = (q - w[0].q) / (w[1].q - w[0].q);
            return w[0].factor + t * (w[1].factor - w[0].factor);
        }
    }
    1.0
}

fn in_mask(mask: &Mask, idx: usize, q: f64) -> bool {
    if mask.index_masks.contains(&idx) {
        return true;
    }
    mask.q_ranges.iter().any(|(lo, hi)| q >= *lo && q <= *hi)
}

/// I_norm = I / (exposure * thickness * transmission * response).
/// Masked rows are retained and plotted, but excluded from fitting windows.
pub fn normalize(
    run: &ParsedRun,
    resp: Option<&InstrumentResponse>,
    mask: Option<&Mask>,
    rule_version: &str,
) -> Result<NormCurve, Vec<Diagnostic>> {
    let mut blockers = run.diagnostics.iter().filter(|d| d.severity == Severity::Error).cloned().collect::<Vec<_>>();
    let (exposure_s, thickness_mm, transmission) = match (run.exposure_s, run.thickness_mm, run.transmission) {
        (Some(a), Some(b), Some(c)) => (a, b, c),
        _ => {
            if run.exposure_s.is_none() {
                blockers.push(Diagnostic::error("NORM_EXPOSURE", "missing convertible exposure", None));
            }
            if run.thickness_mm.is_none() {
                blockers.push(Diagnostic::error("NORM_THICKNESS", "missing convertible thickness", None));
            }
            if run.transmission.is_none() {
                blockers.push(Diagnostic::error("NORM_TRANSMISSION", "missing convertible transmission", None));
            }
            return Err(blockers);
        }
    };
    if exposure_s <= 0.0 {
        blockers.push(Diagnostic::error("NORM_EXPOSURE_LE0", "exposure must be positive", None));
    }
    if thickness_mm <= 0.0 {
        blockers.push(Diagnostic::error("NORM_THICKNESS_LE0", "thickness must be positive", None));
    }
    if transmission <= 0.0 || transmission > 1.5 {
        blockers.push(Diagnostic::error("NORM_TRANSMISSION_RANGE", "transmission outside physical range", None));
    }
    if !blockers.is_empty() {
        return Err(blockers);
    }

    let mut points = Vec::with_capacity(run.q.len());
    let mut diags: Vec<Diagnostic> = run
        .diagnostics
        .iter()
        .filter(|d| d.severity != Severity::Error)
        .cloned()
        .collect();
    for (idx, &q) in run.q.iter().enumerate() {
        let usable = run.row_usable[idx];
        let raw_i = run.i[idx];
        let raw_e = run.e[idx];
        let r = resp.map(|v| response_factor(v, q)).unwrap_or(1.0);
        if resp.is_some() && (r <= 0.0) {
            diags.push(Diagnostic::warn("RESPONSE_NONPOS", "non-positive response factor, point kept", Some(idx)));
        }
        let denom = exposure_s * thickness_mm * transmission * r;
        let masked = mask.map(|m| in_mask(m, idx, q)).unwrap_or(false);
        let (i_val, sig) = if usable && denom > 0.0 {
            (
                raw_i / denom,
                (raw_e / denom).hypot(raw_i / denom * 0.01),
            )
        } else {
            (f64::NAN, f64::NAN)
        };
        if usable && !masked && i_val < 0.0 {
            diags.push(Diagnostic::warn("NORM_NEGATIVE", "normalized intensity negative, retained", Some(idx)));
        }
        let id = point_id(
            "np",
            &[&run.input.id, &run.input.source_hash, &idx.to_string()],
            q,
            idx,
        );
        points.push(NormPoint {
            id,
            index: idx,
            q,
            i: i_val,
            sigma_i: sig,
            raw_row: idx,
            masked,
            usable: usable && !masked,
        });
    }

    let revision = 1;
    let id = crate::ids::digest(&[
        "norm",
        &run.input.id,
        &run.input.source_hash,
        &run.exposure_s.unwrap_or(0.0).to_bits().to_string(),
        &run.thickness_mm.unwrap_or(0.0).to_bits().to_string(),
        &run.transmission.unwrap_or(0.0).to_bits().to_string(),
        resp.map(|v| v.id.as_str()).unwrap_or("none"),
        mask.map(|v| v.id.as_str()).unwrap_or("none"),
        rule_version,
    ]);
    Ok(NormCurve {
        id,
        run_id: run.input.id.clone(),
        revision,
        q_basis: "A^-1".to_string(),
        intensity_unit: "counts/(s*mm) T-corrected".to_string(),
        meta: NormalizationMeta {
            exposure_s,
            thickness_mm,
            transmission,
            response_id: resp.map(|v| v.id.clone()),
            mask_id: mask.map(|v| v.id.clone()),
            rule_version: rule_version.to_string(),
        },
        points,
        diagnostics: diags,
    })
}

fn usable_points(c: &NormCurve) -> Vec<&NormPoint> {
    c.points.iter().filter(|p| p.usable && p.i.is_finite()).collect()
}

fn grids_coincide(a: &[f64], b: &[f64]) -> bool {
    crate::units::grids_coincide(a, b, 1e-9)
}

fn interpolate(xs: &[f64], ys: &[f64], x: f64) -> (f64, f64, usize, bool) {
    // returns (y, dy_dx slot unused, index-of-left, exact)
    if x <= xs[0] {
        return (ys[0], 0.0, 0, (x - xs[0]).abs() < 1e-12);
    }
    let n = xs.len();
    if x >= xs[n - 1] {
        return (ys[n - 1], 0.0, n - 2, (x - xs[n - 1]).abs() < 1e-12);
    }
    let mut lo = 0usize;
    let mut hi = n - 1;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if xs[mid] <= x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let exact = (x - xs[lo]).abs() < 1e-12 || (x - xs[hi]).abs() < 1e-12;
    let t = (x - xs[lo]) / (xs[hi] - xs[lo]);
    let y = ys[lo] + t * (ys[hi] - ys[lo]);
    (y, t, lo, exact)
}

/// Weighted least-squares scale of buffer to sample over a window:
/// alpha = sum(I_s I_b / var) / sum(I_b^2 / var), var = sig_s^2+sig_b^2.
fn fit_alpha_window(
    sp: &[&NormPoint],
    bq: &[f64],
    bi: &[f64],
    be: &[f64],
    lo: f64,
    hi: f64,
) -> Option<(f64, f64, f64, usize)> {
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    let mut n = 0usize;
    for p in sp {
        if p.q < lo || p.q > hi || p.q < bq[0] || p.q > bq[bq.len() - 1] {
            continue;
        }
        let (ib, _, _, _) = interpolate(bq, bi, p.q);
        let (eb, _, _, _) = interpolate(bq, be, p.q);
        if ib <= 0.0 {
            continue;
        }
        let var = p.sigma_i.powi(2) + eb.powi(2);
        if !(var > 0.0) {
            continue;
        }
        num += p.i * ib / var;
        den += ib * ib / var;
        n += 1;
    }
    if n < 2 || den <= 0.0 {
        return None;
    }
    let alpha = num / den;
    let mut chi2 = 0.0;
    let mut sq = 0.0;
    for p in sp {
        if p.q < lo || p.q > hi || p.q < bq[0] || p.q > bq[bq.len() - 1] {
            continue;
        }
        let (ib, _, _, _) = interpolate(bq, bi, p.q);
        let (eb, _, _, _) = interpolate(bq, be, p.q);
        if ib <= 0.0 {
            continue;
        }
        let var = p.sigma_i.powi(2) + eb.powi(2);
        if !(var > 0.0) {
            continue;
        }
        let r = p.i - alpha * ib;
        chi2 += r * r / var;
        sq += r * r;
    }
    let sigma_alpha = (1.0 / den).sqrt();
    let dof = (n.max(2) - 1) as f64;
    Some((alpha, sigma_alpha, chi2 / dof, (sq / n as f64).sqrt()))
}

pub struct SubtractParams<'a> {
    pub sample: &'a NormCurve,
    pub buffer: &'a NormCurve,
    pub artifact_id: &'a str,
    pub windows: &'a [(f64, f64)],
    pub manual_alpha: Option<f64>,
    pub rule_version: &'a str,
    pub created_seq: u64,
}

pub struct SubtractOutcome {
    pub version: Option<SubVersion>,
    pub diagnostics: Vec<Diagnostic>,
    pub blocked: bool,
}

/// Background subtraction. Different grids are never subtracted directly:
/// if they do not coincide the buffer is linearly interpolated, and only
/// inside the shared overlap. No overlap => blocked, no points emitted.
pub fn subtract(p: SubtractParams<'_>) -> SubtractOutcome {
    let sample = usable_points(p.sample);
    let buffer = usable_points(p.buffer);
    let mut diags = Vec::new();
    if sample.is_empty() || buffer.is_empty() {
        diags.push(Diagnostic::error("SUB_NO_USABLE", "no usable points on one side", None));
        return SubtractOutcome { version: None, diagnostics: diags, blocked: true };
    }
    let sq: Vec<f64> = sample.iter().map(|x| x.q).collect();
    let bq: Vec<f64> = buffer.iter().map(|x| x.q).collect();
    let bi: Vec<f64> = buffer.iter().map(|x| x.i).collect();
    let be: Vec<f64> = buffer.iter().map(|x| x.sigma_i).collect();
    let direct = grids_coincide(
        &p.sample.points.iter().map(|x| x.q).collect::<Vec<_>>(),
        &p.buffer.points.iter().map(|x| x.q).collect::<Vec<_>>(),
    );

    let lo = sq[0].max(bq[0]);
    let hi = sq[sq.len() - 1].min(bq[bq.len() - 1]);
    if hi <= lo {
        diags.push(Diagnostic::error(
            "SUB_NO_OVERLAP",
            format!("no overlapping q range: sample [{:.5},{:.5}] buffer [{:.5},{:.5}]", sq[0], sq[sq.len()-1], bq[0], bq[bq.len()-1]),
            None,
        ));
        return SubtractOutcome { version: None, diagnostics: diags, blocked: true };
    }
    if !direct {
        diags.push(Diagnostic::info(
            "SUB_INTERPOLATED",
            "q grids differ; buffer interpolated, subtraction restricted to overlap",
            None,
        ));
    }

    let mut candidates: Vec<ScaleCandidate> = Vec::new();
    if let Some(alpha) = p.manual_alpha {
        candidates.push(ScaleCandidate {
            id: crate::ids::digest(&["scale", "manual", p.artifact_id, &alpha.to_bits().to_string(), &p.created_seq.to_string()]),
            method: "manual".to_string(),
            alpha,
            sigma_alpha: 0.0,
            fit_window: None,
            n_points: 0,
            chi2: f64::NAN,
            rms_residual: f64::NAN,
            uncertain: true,
        });
    }
    for (k, w) in p.windows.iter().enumerate() {
        if let Some((alpha, sigma, chi2, rms)) = fit_alpha_window(&sample, &bq, &bi, &be, w.0, w.1) {
            let uncertain = (sigma / alpha).abs() > 0.05;
            if uncertain {
                diags.push(Diagnostic::warn(
                    "SCALE_UNCERTAIN",
                    format!("window {} alpha={:.4}+-{:.4} (>5% relative)", k + 1, alpha, sigma),
                    None,
                ));
            }
            candidates.push(ScaleCandidate {
                id: crate::ids::digest(&[
                    "scale", p.artifact_id, &k.to_string(), &format!("{:.10}", alpha), &p.created_seq.to_string(),
                ]),
                method: format!("weighted-ls window {}", k + 1),
                alpha,
                sigma_alpha: sigma,
                fit_window: Some(*w),
                n_points: sample
                    .iter()
                    .filter(|x| x.q >= w.0 && x.q <= w.1 && x.q >= bq[0] && x.q <= bq[bq.len()-1])
                    .count(),
                chi2,
                rms_residual: rms,
                uncertain,
            });
        } else {
            diags.push(Diagnostic::warn(
                "SCALE_WINDOW_EMPTY",
                format!("window {} has fewer than 2 overlapping points", k + 1),
                None,
            ));
        }
    }
    if candidates.is_empty() {
        diags.push(Diagnostic::error("SUB_NO_SCALE", "no usable scale candidate", None));
        return SubtractOutcome { version: None, diagnostics: diags, blocked: true };
    }
    let chosen = candidates
        .iter()
        .filter(|c| c.sigma_alpha > 0.0)
        .min_by(|a, b| a.chi2.partial_cmp(&b.chi2).unwrap())
        .or_else(|| candidates.first())
        .unwrap();
    let alpha = chosen.alpha;
    let sigma_alpha = chosen.sigma_alpha;

    let mut points = Vec::new();
    let mut flags = Vec::new();
    let mut negatives = 0usize;
    for sp in &sample {
        if sp.q < lo || sp.q > hi {
            continue;
        }
        let (ib, _, left, exact) = interpolate(&bq, &bi, sp.q);
        let (eb, _, _, _) = interpolate(&bq, &be, sp.q);
        let interp = !direct && !exact;
        let i_sub = sp.i - alpha * ib;
        let var_s = sp.sigma_i.powi(2);
        let var_b_scaled = alpha.powi(2) * eb.powi(2) + ib.powi(2) * sigma_alpha.powi(2);
        let sigma = (var_s + var_b_scaled).sqrt();
        let residual = i_sub;
        let negative = i_sub < -2.0 * sigma;
        if negative {
            negatives += 1;
            diags.push(Diagnostic::warn(
                "SUB_NEGATIVE",
                format!("I(q)-a*Ib(q) below -2sigma at q={:.5}", sp.q),
                Some(sp.index),
            ));
        }
        let bp_id = if exact {
            buffer
                .iter()
                .find(|x| (x.q - sp.q).abs() < 1e-12)
                .map(|x| x.id.clone())
                .unwrap_or_else(|| buffer[left.min(buffer.len()-1)].id.clone())
        } else {
            format!("interp#{}#{}", buffer[left].id, buffer[(left+1).min(buffer.len()-1)].id)
        };
        let id = point_id(
            "sub",
            &[p.artifact_id, &sp.id, &bp_id, &format!("{:.10}", alpha)],
            sp.q,
            sp.index,
        );
        points.push(SubPoint {
            id,
            index: sp.index,
            q: sp.q,
            i: i_sub,
            sigma_i: sigma,
            sample_point_id: sp.id.clone(),
            buffer_point_id: bp_id,
            interpolated: interp,
            residual,
            negative,
        });
    }
    if negatives > 0 {
        flags.push(format!("negative_points:{}", negatives));
    }
    if !direct {
        flags.push("grid_interpolated".to_string());
    } else {
        flags.push("grid_coincident".to_string());
    }
    if candidates.iter().any(|c| c.uncertain) {
        flags.push("scale_uncertain".to_string());
    }
    if candidates.len() > 1 {
        flags.push("multi_candidate".to_string());
    }

    let version_id = crate::ids::digest(&[
        "subv",
        p.artifact_id,
        &p.sample.id,
        &p.buffer.id,
        &format!("{:.12}", alpha),
        &p.windows.len().to_string(),
        p.rule_version,
        &p.created_seq.to_string(),
    ]);
    Ok_version(SubVersion {
        id: version_id,
        artifact_id: p.artifact_id.to_string(),
        sample_run: p.sample.run_id.clone(),
        buffer_run: p.buffer.run_id.clone(),
        sample_norm: p.sample.id.clone(),
        buffer_norm: p.buffer.id.clone(),
        q_basis: "A^-1".to_string(),
        direct_grids: direct,
        overlap: Some((lo, hi)),
        points,
        candidates,
        windows: p.windows.to_vec(),
        diagnostics: diags.clone(),
        flags,
        rule_version: p.rule_version.to_string(),
        created_seq: p.created_seq,
    })
}

// small wrapper kept explicit for readability
fn Ok_version(v: SubVersion) -> SubtractOutcome {
    SubtractOutcome { version: Some(v), diagnostics: v.diagnostics.clone(), blocked: false }
}

/// Weighted linear regression y = a + b*x. Returns (a, sig_a, b, sig_b, chi2, n).
pub fn wls(x: &[f64], y: &[f64], sy: &[f64]) -> Option<(f64, f64, f64, f64, f64, usize)> {
    let n = x.len();
    if n < 2 {
        return None;
    }
    let mut sw = 0.0f64;
    let mut sx = 0.0f64;
    let mut syy = 0.0f64;
    let mut sxx = 0.0f64;
    let mut sxy = 0.0f64;
    for i in 0..n {
        let w = 1.0 / (sy[i] * sy[i]);
        sw += w;
        sx += w * x[i];
        syy += w * y[i];
        sxx += w * x[i] * x[i];
        sxy += w * x[i] * y[i];
    }
    let delta = sw * sxx - sx * sx;
    if !(delta.abs() > 0.0) {
        return None;
    }
    let b = (sw * sxy - sx * syy) / delta;
    let a = (syy - b * sx) / sw;
    let var_a = sxx / delta;
    let var_b = sw / delta;
    let mut chi2 = 0.0;
    for i in 0..n {
        let r = y[i] - (a + b * x[i]);
        chi2 += r * r / (sy[i] * sy[i]);
    }
    Some((a, var_a.sqrt(), b, var_b.sqrt(), chi2, n))
}

pub struct FitParams<'a> {
    pub sub: &'a SubVersion,
    pub kind: &'a str, // "guinier" | "porod"
    pub q_min: f64,
    pub q_max: f64,
    pub qrg_limit: f64,
    pub rule_version: &'a str,
    pub created_seq: u64,
}

/// Fit an independent interval. Guinier: ln I vs q^2. Porod: I*q^4 vs q^-4.
/// These are separate evidence records; the project layer never stitches them.
pub fn fit_region(p: FitParams<'_>) -> Result<RegionFit, Diagnostic> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut sy = Vec::new();
    let mut pts = Vec::new();
    for sp in &p.sub.points {
        if sp.q < p.q_min || sp.q > p.q_max {
            continue;
        }
        if !(sp.i.is_finite()) || sp.i <= 0.0 {
            continue;
        }
        pts.push(sp);
        match p.kind {
            "guinier" => {
                xs.push(sp.q * sp.q);
                ys.push(sp.i.ln());
                sy.push((sp.sigma_i / sp.i).abs().max(1e-9));
            }
            "porod" => {
                xs.push(1.0 / sp.q.powi(4));
                ys.push(sp.i * sp.q.powi(4));
                sy.push((sp.sigma_i * sp.q.powi(4)).abs().max(1e-9));
            }
            other => {
                return Err(Diagnostic::error("FIT_KIND", format!("unknown region '{}'", other), None));
            }
        }
    }
    let reg = match wls(&xs, &ys, &sy) {
        Some(v) => v,
        None => {
            return Err(Diagnostic::error(
                "FIT_INSUFFICIENT",
                format!("{} interval has < 2 positive finite points", p.kind),
                None,
            ));
        }
    };
    let (intercept, sig_icpt, slope, sig_slope, chi2, n) = reg;
    let mut flags = Vec::new();
    let mut diags = Vec::new();
    let (rg, rg_sigma, i0, i0_sigma, qrg_min, qrg_max) = if p.kind == "guinier" {
        if slope >= 0.0 {
            flags.push("non_physical_slope".to_string());
            diags.push(Diagnostic::warn("GUINIER_POS_SLOPE", "ln I vs q^2 slope >= 0", None));
        }
        let rg = (-3.0 * slope).sqrt();
        let rg_sigma = (3.0 / (4.0 * -slope)).sqrt() * sig_slope;
        let i0 = intercept.exp();
        let i0_sigma = i0 * sig_icpt;
        let qmin = pts.first().map(|x| x.q).unwrap_or(0.0);
        let qmax = pts.last().map(|x| x.q).unwrap_or(0.0);
        let qrg_min = qmin * rg;
        let qrg_max = qmax * rg;
        if qrg_max > p.qrg_limit {
            flags.push("qrg_limit_exceeded".to_string());
            diags.push(Diagnostic::warn(
                "GUINIER_QRG",
                format!("q_max*Rg={:.3} exceeds limit {:.3}", qrg_max, p.qrg_limit),
                None,
            ));
        }
        (Some(rg), Some(rg_sigma), Some(i0), Some(i0_sigma), Some(qrg_min), Some(qrg_max))
    } else {
        (None, None, None, None, None, None)
    };

    let id = crate::ids::digest(&[
        "fit",
        p.kind,
        &p.sub.id,
        &format!("{:.10}", p.q_min),
        &format!("{:.10}", p.q_max),
        p.rule_version,
        &p.created_seq.to_string(),
    ]);
    Ok(RegionFit {
        id,
        artifact_id: format!("fit:{}:{}", p.kind, p.sub.artifact_id),
        sub_version: p.sub.id.clone(),
        kind: p.kind.to_string(),
        q_min: p.q_min,
        q_max: p.q_max,
        n_points: n,
        slope,
        slope_sigma: sig_slope,
        intercept,
        intercept_sigma: sig_icpt,
        chi2,
        rg,
        rg_sigma,
        i0,
        i0_sigma,
        qrg_min,
        qrg_max,
        overlap_with: Vec::new(),
        flags,
        diagnostics: diags,
        rule_version: p.rule_version.to_string(),
        created_seq: p.created_seq,
    })
}
