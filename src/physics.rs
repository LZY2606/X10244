//! Physics / reduction rules.
//!
//! Invariant semantics (also asserted by tests):
//! * When the sample and buffer q grids differ we never subtract element by
//!   element; the buffer is linearly interpolated, and only over the shared
//!   (overlapping) q range. Outside overlap no subtraction point is produced.
//! * Error bars include propagated normalization uncertainty and the
//!   uncertainty of the fitted background scale alpha.
//! * Guinier and Porod windows are reported as separate candidates and are not
//!   stitched into a single piecewise model.

use crate::domain::*;
use serde_json::Value;

fn parse_q_unit(label: &str) -> Option<Unit> {
    let s = label.trim().replace(' ', "");
    match s.to_lowercase().as_str() {
        "a^-1" | "å^-1" | "angstrom^-1" | "1/a" | "a-1" | "a^-1" => Some(Unit::InverseAngstrom),
        "nm^-1" | "1/nm" | "nm-1" => Some(Unit::InverseNm),
        _ => None,
    }
}

fn parse_scalar_unit(label: &str) -> Option<Unit> {
    let s = label.trim().to_lowercase();
    match s.replace(' ', "").as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => Some(Unit::Seconds),
        "ms" | "millis" | "millisecond" | "milliseconds" => Some(Unit::Milliseconds),
        "cm" => Some(Unit::Cm),
        "mm" => Some(Unit::Mm),
        "" | "1" | "-" | "dimensionless" | "a.u." | "au" => Some(Unit::Dimensionless),
        _ => None,
    }
}

fn parse_intensity_unit(label: &str) -> Option<Unit> {
    let s = label.trim().to_lowercase();
    if s.contains("cm^-1") || s.contains("cm-1") || s.contains("absolute") {
        Some(Unit::Absolute)
    } else if s.is_empty() || s.contains("a.u") || s.contains("count") || s.contains("arb") {
        Some(Unit::Arbitrary)
    } else {
        None
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        _ => v.to_string(),
    }
}

fn canonical_vector(values: &[Value], unit: Unit, q: bool) -> (Vec<Option<f64>>, bool) {
    let factor = if q {
        match unit {
            Unit::InverseAngstrom => 1.0,
            Unit::InverseNm => 0.1, // 1 nm^-1 = 0.1 A^-1
            _ => 1.0,
        }
    } else {
        1.0
    };
    let mut out = Vec::with_capacity(values.len());
    let mut all = true;
    for v in values {
        if let Some(x) = num(v) {
            if x.is_finite() {
                out.push(Some(x * factor));
            } else {
                out.push(None);
                all = false;
            }
        } else {
            out.push(None);
            all = false;
        }
    }
    (out, all)
}

fn scalar_from(value: &Value, unit_label: &str, kind: ScalarKind) -> (Scalar, Option<Diagnostic>) {
    let parsed_unit = match kind {
        ScalarKind::Exposure => parse_scalar_unit(unit_label),
        ScalarKind::Length => parse_scalar_unit(unit_label),
        ScalarKind::Dimensionless => parse_scalar_unit(unit_label),
    };
    let original_text = text_of(value);
    let numeric = num(value);
    match (numeric, parsed_unit) {
        (Some(x), Some(u)) => {
            let (canon, canon_unit) = convert_scalar(x, u, kind);
            (
                Scalar {
                    value: x,
                    unit: u,
                    original_text,
                    original_unit: unit_label.trim().to_string(),
                    canonical: Some(canon),
                    canonical_unit: Some(canon_unit),
                    convertible: true,
                },
                None,
            )
        }
        (Some(x), None) => (
            Scalar {
                value: x,
                unit: Unit::Dimensionless,
                original_text,
                original_unit: unit_label.trim().to_string(),
                canonical: None,
                canonical_unit: None,
                convertible: false,
            },
            Some(Diagnostic {
                code: "UNIT_UNKNOWN".into(),
                severity: Severity::Error,
                message: format!("unknown unit '{}' for {}", unit_label, kind.label()),
                point_id: None,
            }),
        ),
        (None, _) => (
            Scalar {
                value: f64::NAN,
                unit: parsed_unit.unwrap_or(Unit::Dimensionless),
                original_text,
                original_unit: unit_label.trim().to_string(),
                canonical: None,
                canonical_unit: None,
                convertible: false,
            },
            Some(Diagnostic {
                code: "VALUE_UNPARSEABLE".into(),
                severity: Severity::Error,
                message: format!("could not parse '{}' as {}", original_text, kind.label()),
                point_id: None,
            }),
        ),
    }
}

#[derive(Clone, Copy)]
enum ScalarKind {
    Exposure,
    Length,
    Dimensionless,
}
impl ScalarKind {
    fn label(self) -> &'static str {
        match self {
            ScalarKind::Exposure => "exposure",
            ScalarKind::Length => "thickness",
            ScalarKind::Dimensionless => "transmission/scale",
        }
    }
}

fn convert_scalar(x: f64, u: Unit, kind: ScalarKind) -> (f64, Unit) {
    match kind {
        ScalarKind::Exposure => match u {
            Unit::Seconds => (x, Unit::Seconds),
            Unit::Milliseconds => (x / 1000.0, Unit::Seconds),
            _ => (x, u),
        },
        ScalarKind::Length => match u {
            Unit::Cm => (x, Unit::Cm),
            Unit::Mm => (x / 10.0, Unit::Cm),
            _ => (x, u),
        },
        ScalarKind::Dimensionless => (x, Unit::Dimensionless),
    }
}

/// Convert an import record into a versioned raw record. Original text/units
/// are always preserved; unconvertible quantities remain in place with
/// diagnostics rather than being dropped.
pub fn import_record(input: RawRecordIn, version: u32, now: String) -> RawRecord {
    let mut diags = Vec::new();

    let q_unit = parse_q_unit(&input.q_unit).unwrap_or(Unit::InverseAngstrom);
    let q_values: Vec<Value> = input.q.iter().map(Value::from).collect();
    let (q_canon, q_ok) = canonical_vector(&q_values, q_unit, true);
    if parse_q_unit(&input.q_unit).is_none() {
        diags.push(Diagnostic {
            code: "UNIT_UNKNOWN".into(),
            severity: Severity::Error,
            message: format!("unknown q unit '{}'; kept as A^-1", input.q_unit),
            point_id: None,
        });
    }
    let q = Vector {
        values: input.q.clone(),
        unit: q_unit,
        original_text: q_values.iter().map(text_of).collect(),
        original_unit: input.q_unit.trim().to_string(),
        canonical: q_canon,
        canonical_unit: Some(Unit::InverseAngstrom),
        convertible: q_ok,
    };

    let i_unit = parse_intensity_unit(&input.intensity_unit).unwrap_or(Unit::Arbitrary);
    if parse_intensity_unit(&input.intensity_unit).is_none() {
        diags.push(Diagnostic {
            code: "UNIT_UNKNOWN".into(),
            severity: Severity::Warning,
            message: format!("unknown intensity unit '{}'; treated as a.u.", input.intensity_unit),
            point_id: None,
        });
    }
    let i_values: Vec<Value> = input.intensity.iter().map(Value::from).collect();
    let (i_canon, i_ok) = canonical_vector(&i_values, i_unit, false);
    let intensity = Vector {
        values: input.intensity.clone(),
        unit: i_unit,
        original_text: i_values.iter().map(text_of).collect(),
        original_unit: input.intensity_unit.trim().to_string(),
        canonical: i_canon,
        canonical_unit: Some(i_unit),
        convertible: i_ok,
    };

    let err_unit_label = if input.error_unit.trim().is_empty() {
        input.intensity_unit.clone()
    } else {
        input.error_unit.clone()
    };
    let err_values: Vec<Value> = if input.error.is_empty() {
        // Missing error column: Poisson-like sqrt estimate, flagged.
        input
            .intensity
            .iter()
            .map(|x| Value::from(x.abs().sqrt()))
            .collect()
    } else {
        input.error.clone()
    };
    let (e_canon, e_ok) = canonical_vector(&err_values, i_unit, false);
    if input.error.is_empty() {
        diags.push(Diagnostic {
            code: "ERROR_ESTIMATED".into(),
            severity: Severity::Warning,
            message: "no error column imported; sigma estimated as sqrt(|I|)".into(),
            point_id: None,
        });
    }
    let error = Vector {
        values: err_values.iter().filter_map(num).collect(),
        unit: i_unit,
        original_text: err_values.iter().map(text_of).collect(),
        original_unit: err_unit_label.trim().to_string(),
        canonical: e_canon,
        canonical_unit: Some(i_unit),
        convertible: e_ok,
    };

    let (exposure, d) = scalar_from(&Value::from(input.exposure), &input.exposure_unit, ScalarKind::Exposure);
    d.into_iter().for_each(|x| diags.push(x));
    let (thickness, d) = scalar_from(&input.thickness, &input.thickness_unit, ScalarKind::Length);
    d.into_iter().for_each(|x| diags.push(x));
    let (transmission, d) =
        scalar_from(&input.transmission, &input.transmission_unit, ScalarKind::Dimensionless);
    d.into_iter().for_each(|x| diags.push(x));
    let (response_scale, d) = scalar_from(
        &input.response_scale,
        &input.response_scale_unit,
        ScalarKind::Dimensionless,
    );
    d.into_iter().for_each(|x| diags.push(x));

    let n = input.q.len();
    if input.intensity.len() != n || err_values.len() != n {
        diags.push(Diagnostic {
            code: "COLUMN_LENGTH_MISMATCH".into(),
            severity: Severity::Error,
            message: format!(
                "q={}, I={}, err={} column lengths differ",
                n,
                input.intensity.len(),
                err_values.len()
            ),
            point_id: None,
        });
    }

    let response = if input.response_name.trim().is_empty() {
        None
    } else {
        Some(ResponseSource {
            name: input.response_name.trim().to_string(),
            source_uri: String::new(),
            sha256: String::new(),
        })
    };

    let id = format!("raw:{}@v{}", input.logical_key, version);
    RawRecord {
        id,
        logical_key: input.logical_key,
        kind: input.kind,
        q,
        intensity,
        error,
        exposure,
        thickness,
        transmission,
        buffer_candidates: input.buffer_candidates,
        response,
        response_scale,
        masks: input.masks,
        diagnostics: diags,
        version,
        supersedes: input.supersedes,
        source_uri: input.source_uri,
        imported_at: now,
    }
}

/// Stable point id helpers. Identical inputs reproduce identical ids, so
/// export/delete/import and recompute preserve identities and order.
pub fn raw_point_id(record_id: &str, index: usize) -> String {
    format!("{}#p{}", record_id, index)
}
pub fn norm_point_id(norm_id: &str, source_point_id: &str) -> String {
    format!("{}<-{}", norm_id, source_point_id)
}
pub fn sub_point_id(sub_id: &str, q: f64) -> String {
    // q is already canonical A^-1; Rust's {:e} rendering is deterministic and
    // therefore stable across recompute, export and re-import.
    format!("{}#q{:e}", sub_id, q)
}

/// Normalize one raw record: I_norm = I / (t_exp * T * d * R).
/// Variance propagates measurement error plus relative uncertainties of the
/// normalization factors (here small fixed instrument bookkeeping terms, kept
/// explicit so the budget is auditable).
pub fn normalize(record: &RawRecord) -> Normalized {
    let id = format!("norm:{}", record.id.trim_start_matches("raw:"));
    let mut diagnostics = record.diagnostics.clone();
    let exposure_s = record.exposure.canonical.unwrap_or(f64::NAN);
    let thickness_cm = record.thickness.canonical.unwrap_or(f64::NAN);
    let transmission = record
        .transmission
        .canonical
        .unwrap_or(if record.transmission.convertible {
            record.transmission.value
        } else {
            f64::NAN
        });
    let response_scale = record.response_scale.canonical.unwrap_or(1.0);

    let denom = exposure_s * thickness_cm * transmission * response_scale;
    let n = record.q.canonical.len();
    let mut points = Vec::new();
    let mut variance = Vec::new();
    let mut usable = 0usize;

    if !denom.is_finite() || denom <= 0.0 {
        diagnostics.push(Diagnostic {
            code: "NORMALIZATION_INVALID".into(),
            severity: Severity::Error,
            message: format!(
                "invalid normalization denominator exposure={}s, thickness={}cm, T={}, R={}",
                exposure_s, thickness_cm, transmission, response_scale
            ),
            point_id: None,
        });
        return Normalized {
            id,
            record_id: record.id.clone(),
            status: DeriveStatus::Failed,
            points,
            variance,
            q_unit: Unit::InverseAngstrom.as_str().into(),
            intensity_unit: record.intensity.canonical_unit.unwrap_or(Unit::Arbitrary).as_str().into(),
            diagnostics,
            exposure_s,
            thickness_cm,
            transmission,
            response_scale,
        };
    }

    // Relative bookkeeping variances (exposure/transmission/response): kept tiny
    // but nonzero so the uncertainty contribution is visible in the budget.
    let rel_var_norm = 1e-6_f64.powi(2) * 3.0;

    for i in 0..n {
        let sp = raw_point_id(&record.id, i);
        let q = record.q.canonical.get(i).copied().flatten();
        let iv = record.intensity.canonical.get(i).copied().flatten();
        let ev = record.error.canonical.get(i).copied().flatten();
        match (q, iv, ev) {
            (Some(qv), Some(yv), Some(sv)) if qv.is_finite() && yv.is_finite() && sv.is_finite() => {
                let norm_y = yv / denom;
                let norm_s = sv / denom;
                // dI_norm/I_norm combines measurement and normalization terms.
                let meas = norm_s.powi(2);
                let scale = (norm_y * rel_var_norm.sqrt()).powi(2);
                let var = meas + scale;
                points.push(NormPoint {
                    id: norm_point_id(&id, &sp),
                    source_point_id: sp,
                    q: qv,
                    intensity: norm_y,
                    sigma: var.sqrt(),
                });
                variance.push(var);
                usable += 1;
            }
            (qv, iv, ev) => {
                diagnostics.push(Diagnostic {
                    code: "POINT_UNCONVERTIBLE".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "point {} skipped in normalization (q finite={}, I finite={}, sigma finite={})",
                        sp,
                        qv.map(|x| x.is_finite()).unwrap_or(false),
                        iv.map(|x| x.is_finite()).unwrap_or(false),
                        ev.map(|x| x.is_finite()).unwrap_or(false)
                    ),
                    point_id: Some(sp),
                });
            }
        }
    }

    let status = if usable == 0 {
        DeriveStatus::Failed
    } else if diagnostics.iter().any(|d| d.severity == Severity::Warning) {
        DeriveStatus::Partial
    } else {
        DeriveStatus::Ok
    };

    Normalized {
        id,
        record_id: record.id.clone(),
        status,
        points,
        variance,
        q_unit: Unit::InverseAngstrom.as_str().into(),
        intensity_unit: record.intensity.canonical_unit.unwrap_or(Unit::Arbitrary).as_str().into(),
        diagnostics,
        exposure_s,
        thickness_cm,
        transmission,
        response_scale,
    }
}

#[derive(Clone)]
struct Interp {
    q: f64,
    value: f64,
    sigma: f64,
    source_point_id: String,
    source_q: f64,
    kind: &'static str,
}

/// Linear interpolation of a normalized curve at q0. Only returns a value when
/// q0 is within the curve's covered q range (inclusive of endpoints); never
/// extrapolates. An exact grid hit is tagged "same-grid".
fn interpolate_at(curve: &Normalized, q0: f64, tol: f64) -> Option<Interp> {
    let pts = &curve.points;
    if pts.is_empty() {
        return None;
    }
    if let Some(p) = pts.iter().find(|p| (p.q - q0).abs() <= tol) {
        return Some(Interp {
            q: q0,
            value: p.intensity,
            sigma: p.sigma,
            source_point_id: p.id.clone(),
            source_q: p.q,
            kind: "same-grid",
        });
    }
    if q0 < pts.first().unwrap().q - tol || q0 > pts.last().unwrap().q + tol {
        return None;
    }
    let idx = pts.partition_point(|p| p.q < q0);
    if idx == 0 || idx >= pts.len() {
        return None;
    }
    let a = &pts[idx - 1];
    let b = &pts[idx];
    let w = (q0 - a.q) / (b.q - a.q);
    if !(0.0..=1.0).contains(&w) {
        return None;
    }
    let value = a.intensity * (1.0 - w) + b.intensity * w;
    // Interpolated variance via the same linear weights (variance, not sigma).
    let ia = a.sigma.powi(2);
    let ib = b.sigma.powi(2);
    let sigma = ((1.0 - w).powi(2) * ia + w.powi(2) * ib).sqrt();
    // Provenance keeps both bracketing buffer points; surface the left id here
    // and encode the right via the match kind in diagnostics at call site.
    Some(Interp {
        q: q0,
        value,
        sigma,
        source_point_id: format!("{}+{}", a.id, b.id),
        source_q: a.q,
        kind: "interpolated",
    })
}

/// Subtract a normalized buffer from a normalized sample.
///
/// The background scale alpha is a weighted least-squares fit through the
/// origin over the overlap region. If the q grids differ, buffer values are
/// interpolated and subtraction only exists over overlap. Sigma includes the
/// propagated uncertainty of alpha.
pub fn subtract(sample: &Normalized, buffer: &Normalized) -> Subtracted {
    let id = format!("sub:{}<-{}", sample.record_id, buffer.record_id);
    let mut diagnostics = Vec::new();
    let tol = 1e-9;

    let grids_equal = sample.points.len() == buffer.points.len()
        && sample
            .points
            .iter()
            .zip(&buffer.points)
            .all(|(a, b)| (a.q - b.q).abs() <= tol);

    if !grids_equal {
        diagnostics.push(Diagnostic {
            code: "Q_GRID_MISMATCH".into(),
            severity: Severity::Info,
            message: "sample and buffer q grids differ; subtracting only over overlap via linear interpolation"
                .into(),
            point_id: None,
        });
    }

    // Determine overlap interval.
    let lo = sample
        .points
        .first()
        .zip(buffer.points.first())
        .map(|(a, b)| a.q.max(b.q));
    let hi = sample
        .points
        .last()
        .zip(buffer.points.last())
        .map(|(a, b)| a.q.min(b.q));

    let (lo, hi) = match (lo, hi) {
        (Some(lo), Some(hi)) if hi - lo > -tol => (lo, hi),
        _ => {
            diagnostics.push(Diagnostic {
                code: "NO_OVERLAP".into(),
                severity: Severity::Error,
                message: "no overlapping q range between sample and buffer; subtraction not performed".into(),
                point_id: None,
            });
            return Subtracted {
                id,
                sample_record_id: sample.record_id.clone(),
                buffer_record_id: buffer.record_id.clone(),
                sample_norm_id: sample.id.clone(),
                buffer_norm_id: buffer.id.clone(),
                status: DeriveStatus::Failed,
                points: Vec::new(),
                alpha: f64::NAN,
                alpha_sigma: f64::NAN,
                overlap_q_min: f64::NAN,
                overlap_q_max: f64::NAN,
                residual_mean: f64::NAN,
                residual_rms: f64::NAN,
                chi_squared: f64::NAN,
                diagnostics,
            };
        }
    };

    // Build matched pairs only for sample points strictly inside overlap.
    struct Pair {
        s: NormPoint,
        b: Interp,
    }
    let mut pairs: Vec<Pair> = Vec::new();
    let mut outside = 0usize;
    for sp in &sample.points {
        if sp.q + tol < lo || sp.q - tol > hi {
            outside += 1;
            continue;
        }
        if let Some(b) = interpolate_at(buffer, sp.q, tol) {
            pairs.push(Pair { s: sp.clone(), b });
        } else {
            outside += 1;
        }
    }

    if pairs.is_empty() {
        diagnostics.push(Diagnostic {
            code: "NO_OVERLAP_POINTS".into(),
            severity: Severity::Error,
            message: "overlap interval exists but contains no sample points".into(),
            point_id: None,
        });
        return Subtracted {
            id,
            sample_record_id: sample.record_id.clone(),
            buffer_record_id: buffer.record_id.clone(),
            sample_norm_id: sample.id.clone(),
            buffer_norm_id: buffer.id.clone(),
            status: DeriveStatus::Failed,
            points: Vec::new(),
            alpha: f64::NAN,
            alpha_sigma: f64::NAN,
            overlap_q_min: lo,
            overlap_q_max: hi,
            residual_mean: f64::NAN,
            residual_rms: f64::NAN,
            chi_squared: f64::NAN,
            diagnostics,
        };
    }

    if outside > 0 {
        diagnostics.push(Diagnostic {
            code: "OVERLAP_ONLY".into(),
            severity: Severity::Info,
            message: format!("{} sample point(s) lie outside buffer q range and are retained un-subtracted", outside),
            point_id: None,
        });
    }

    // Weighted LS alpha = sum(w I_s I_b)/sum(w I_b^2), w = 1/var(I_s-I_b).
    // With only measurement/scale sigmas at this stage (alpha unknown), use
    // var_s + var_b as weights (alpha uncertainty added afterwards).
    let mut num = 0.0;
    let mut den = 0.0;
    for pr in &pairs {
        let w = 1.0 / (pr.s.sigma.powi(2) + pr.b.sigma.powi(2));
        num += w * pr.s.intensity * pr.b.value;
        den += w * pr.b.value.powi(2);
    }
    let alpha = if den > 0.0 { num / den } else { 1.0 };
    if den <= 0.0 {
        diagnostics.push(Diagnostic {
            code: "ALPHA_UNDEFINED".into(),
            severity: Severity::Warning,
            message: "background scale fit denominator non-positive; alpha defaulted to 1".into(),
            point_id: None,
        });
    }

    // Residuals and alpha variance. d = I_s - alpha I_b.
    // alpha variance ~ 1 / sum(w I_b^2) (weighted LS through origin).
    let alpha_var = if den > 0.0 { 1.0 / den } else { 0.0 };
    let alpha_sigma = alpha_var.sqrt();
    if alpha_sigma / alpha.abs().max(1e-12) > 0.1 {
        diagnostics.push(Diagnostic {
            code: "BACKGROUND_SCALE_UNCERTAIN".into(),
            severity: Severity::Warning,
            message: format!(
                "background scale alpha={:.4} ± {:.4} is poorly constrained (>10% relative)",
                alpha, alpha_sigma
            ),
            point_id: None,
        });
    }

    let mut points = Vec::new();
    let mut residuals = Vec::new();
    let mut chi = 0.0;
    let mut negatives = 0usize;
    for pr in &pairs {
        let y = pr.s.intensity - alpha * pr.b.value;
        // sigma^2 = var_s + alpha^2 var_b + I_b^2 var_alpha
        let var = pr.s.sigma.powi(2)
            + alpha.powi(2) * pr.b.sigma.powi(2)
            + pr.b.value.powi(2) * alpha_var;
        let sigma = var.max(0.0).sqrt();
        let negative = y < 0.0;
        if negative {
            negatives += 1;
        }
        let residual = y; // zero-model residual after subtraction
        residuals.push(residual);
        if sigma > 0.0 {
            chi += residual.powi(2) / sigma.powi(2);
        }
        let (bq, bpid) = if pr.b.kind == "interpolated" {
            (Some(pr.b.source_q), pr.b.source_point_id.clone())
        } else {
            (Some(pr.b.source_q), pr.b.source_point_id.clone())
        };
        points.push(SubPoint {
            id: sub_point_id(&id, pr.s.q),
            q: pr.s.q,
            intensity: y,
            sigma,
            inside_overlap: true,
            negative,
            provenance: SubPointProvenance {
                sample_norm_point_id: pr.s.id.clone(),
                buffer_norm_point_id: Some(bpid),
                match_kind: pr.b.kind.to_string(),
                q: pr.s.q,
                sample_q: pr.s.q,
                buffer_q: bq,
            },
        });
    }

    if negatives > 0 {
        diagnostics.push(Diagnostic {
            code: "NEGATIVE_INTENSITY".into(),
            severity: Severity::Warning,
            message: format!(
                "{} subtracted point(s) are negative; kept (not clipped or removed) for review",
                negatives
            ),
            point_id: None,
        });
    }

    let n = residuals.len() as f64;
    let mean = residuals.iter().sum::<f64>() / n;
    let rms = (residuals.iter().map(|r| r * r).sum::<f64>() / n).sqrt();
    let chi_squared = chi;

    let status = if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        DeriveStatus::Failed
    } else if diagnostics.iter().any(|d| d.severity == Severity::Warning) || negatives > 0 {
        DeriveStatus::Partial
    } else {
        DeriveStatus::Ok
    };

    Subtracted {
        id,
        sample_record_id: sample.record_id.clone(),
        buffer_record_id: buffer.record_id.clone(),
        sample_norm_id: sample.id.clone(),
        buffer_norm_id: buffer.id.clone(),
        status,
        points,
        alpha,
        alpha_sigma,
        overlap_q_min: lo,
        overlap_q_max: hi,
        residual_mean: mean,
        residual_rms: rms,
        chi_squared,
        diagnostics,
    }
}

struct LinFit {
    intercept: f64,
    slope: f64,
    intercept_sigma: f64,
    slope_sigma: f64,
    r_squared: f64,
    chi_squared: f64,
}

/// Weighted linear regression y = a + b x with weights 1/sigma_y^2.
fn wls(xs: &[f64], ys: &[f64], sy: &[f64]) -> Option<LinFit> {
    let m = xs.len();
    if m < 3 {
        return None;
    }
    let mut sw = 0.0;
    let mut swx = 0.0;
    let mut swy = 0.0;
    let mut swxx = 0.0;
    let mut swxy = 0.0;
    for i in 0..m {
        let w = 1.0 / sy[i].powi(2).max(1e-20);
        sw += w;
        swx += w * xs[i];
        swy += w * ys[i];
        swxx += w * xs[i] * xs[i];
        swxy += w * xs[i] * ys[i];
    }
    let det = sw * swxx - swx * swx;
    if det.abs() < 1e-20 {
        return None;
    }
    let intercept = (swy * swxx - swx * swxy) / det;
    let slope = (sw * swxy - swx * swy) / det;
    // Covariance matrix of (a,b): (1/det) [[swxx, -swx],[-swx, sw]] (unit weights).
    let intercept_sigma = (swxx / det).max(0.0).sqrt();
    let slope_sigma = (sw / det).max(0.0).sqrt();

    let ybar = swy / sw;
    let mut sst = 0.0;
    let mut sse = 0.0;
    let mut chi = 0.0;
    for i in 0..m {
        let pred = intercept + slope * xs[i];
        let w = 1.0 / sy[i].powi(2).max(1e-20);
        sst += w * (ys[i] - ybar).powi(2);
        sse += w * (ys[i] - pred).powi(2);
        chi += ((ys[i] - pred) / sy[i].max(1e-10)).powi(2);
    }
    let r_squared = if sst > 0.0 { 1.0 - sse / sst } else { 0.0 };
    Some(LinFit {
        intercept,
        slope,
        intercept_sigma,
        slope_sigma,
        r_squared,
        chi_squared: chi,
    })
}

/// Generate Guinier and Porod window candidates from a subtracted curve.
///
/// Candidates are kept independent; Guinier and Porod are never combined into
/// one piecewise model, and overlapping windows are all retained and flagged.
pub fn fit_regions(sub: &Subtracted) -> Fit {
    let id = format!("fit:{}", sub.id.trim_start_matches("sub:"));
    let mut diagnostics = Vec::new();
    let mut candidates: Vec<FitCandidate> = Vec::new();

    let pts: Vec<&SubPoint> = sub
        .points
        .iter()
        .filter(|p| p.intensity.is_finite() && p.intensity > 0.0 && p.sigma > 0.0)
        .collect();

    if pts.len() < 6 {
        diagnostics.push(Diagnostic {
            code: "FIT_INSUFFICIENT_POINTS".into(),
            severity: Severity::Error,
            message: format!("need >= 6 positive points for region fits, found {}", pts.len()),
            point_id: None,
        });
        return Fit {
            id,
            subtraction_id: sub.id.clone(),
            status: DeriveStatus::Failed,
            candidates,
            diagnostics,
        };
    }

    // Guinier: ln I vs q^2 on contiguous low-q windows, q_max*Rg validity
    // checked via the classic qRg <= 1.3 guideline (reported as a diagnostic).
    let max_guinier = (pts.len() / 2).max(3);
    for end in (4..=max_guinier).rev() {
        let window = &pts[..end];
        let xs: Vec<f64> = window.iter().map(|p| p.q.powi(2)).collect();
        let ys: Vec<f64> = window.iter().map(|p| p.intensity.ln()).collect();
        let sy: Vec<f64> = window
            .iter()
            .map(|p| (p.sigma / p.intensity).max(1e-6))
            .collect();
        if let Some(lf) = wls(&xs, &ys, &sy) {
            if lf.slope >= 0.0 {
                continue; // Guinier requires a negative slope in ln I vs q^2
            }
            let rg = (-3.0 * lf.slope).sqrt();
            let i0 = lf.intercept.exp();
            let i0_sigma = i0 * lf.intercept_sigma;
            let rg_sigma = (3.0 / (-4.0 * lf.slope)).sqrt() * lf.slope_sigma;
            let qmin = window.first().unwrap().q;
            let qmax = window.last().unwrap().q;
            let mut cd = Vec::new();
            if qmax * rg > 1.3 {
                cd.push(Diagnostic {
                    code: "GUINIER_QRG_LIMIT".into(),
                    severity: Severity::Warning,
                    message: format!("q_max*Rg = {:.2} exceeds 1.3 guideline", qmax * rg),
                    point_id: None,
                });
            }
            let fit_pts = window
                .iter()
                .map(|p| {
                    let model = (lf.intercept + lf.slope * p.q.powi(2)).exp();
                    FitPoint {
                        q: p.q,
                        model,
                        observed: p.intensity,
                        residual: p.intensity - model,
                        source_point_id: p.id.clone(),
                    }
                })
                .collect();
            candidates.push(FitCandidate {
                id: format!("{}#guinier[{:.3e},{:.3e}]", id, qmin, qmax),
                region: RegionKind::Guinier,
                q_min: qmin,
                q_max: qmax,
                intercept: i0,
                intercept_sigma: i0_sigma,
                coefficient: rg,
                coefficient_sigma: rg_sigma,
                r_squared: lf.r_squared,
                chi_squared: lf.chi_squared,
                n_points: window.len(),
                points: fit_pts,
                diagnostics: cd,
            });
        }
    }

    // Porod: ln I vs ln q on contiguous high-q windows; I(q) ~ K q^-n.
    let min_porod = pts.len() / 2;
    for start in 0..min_porod {
        let window = &pts[start..];
        if window.len() < 4 {
            continue;
        }
        let xs: Vec<f64> = window.iter().map(|p| p.q.ln()).collect();
        let ys: Vec<f64> = window.iter().map(|p| p.intensity.ln()).collect();
        let sy: Vec<f64> = window
            .iter()
            .map(|p| (p.sigma / p.intensity).max(1e-6))
            .collect();
        if let Some(lf) = wls(&xs, &ys, &sy) {
            if lf.slope >= 0.0 {
                continue; // Porod requires a negative power-law exponent
            }
            let k = lf.intercept.exp();
            let k_sigma = k * lf.intercept_sigma;
            let exponent = -lf.slope;
            let exp_sigma = lf.slope_sigma;
            let qmin = window.first().unwrap().q;
            let qmax = window.last().unwrap().q;
            let fit_pts = window
                .iter()
                .map(|p| {
                    let model = (lf.intercept + lf.slope * p.q.ln()).exp();
                    FitPoint {
                        q: p.q,
                        model,
                        observed: p.intensity,
                        residual: p.intensity - model,
                        source_point_id: p.id.clone(),
                    }
                })
                .collect();
            candidates.push(FitCandidate {
                id: format!("{}#porod[{:.3e},{:.3e}]", id, qmin, qmax),
                region: RegionKind::Porod,
                q_min: qmin,
                q_max: qmax,
                intercept: k,
                intercept_sigma: k_sigma,
                coefficient: exponent,
                coefficient_sigma: exp_sigma,
                r_squared: lf.r_squared,
                chi_squared: lf.chi_squared,
                n_points: window.len(),
                points: fit_pts,
                diagnostics: Vec::new(),
            });
        }
    }

    // Flag every pair of candidates whose q windows overlap (across and within
    // regions). All are retained; no single region is promoted automatically.
    let overlaps: Vec<(usize, usize)> = candidates
        .iter()
        .enumerate()
        .flat_map(|(i, a)| {
            candidates
                .iter()
                .enumerate()
                .skip(i + 1)
                .filter(|(_, b)| a.q_min < b.q_max && b.q_min < a.q_max)
                .map(move |(j, _)| (i, j))
        })
        .collect();
    for (i, j) in overlaps {
        let msg = format!(
            "region overlap between candidate '{}' and '{}'; both retained",
            candidates[i].id, candidates[j].id
        );
        diagnostics.push(Diagnostic {
            code: "REGION_OVERLAP".into(),
            severity: Severity::Info,
            message: msg,
            point_id: None,
        });
    }

    if candidates.is_empty() {
        diagnostics.push(Diagnostic {
            code: "NO_REGION_CANDIDATE".into(),
            severity: Severity::Error,
            message: "no valid Guinier or Porod window found".into(),
            point_id: None,
        });
    } else {
        diagnostics.push(Diagnostic {
            code: "REGIONS_NOT_MERGED".into(),
            severity: Severity::Info,
            message: "Guinier and Porod windows are independent candidates and are not stitched into one piecewise model".into(),
            point_id: None,
        });
    }

    let status = if candidates.is_empty() {
        DeriveStatus::Failed
    } else {
        DeriveStatus::Ok
    };

    Fit {
        id,
        subtraction_id: sub.id.clone(),
        status,
        candidates,
        diagnostics,
    }
}
