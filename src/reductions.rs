use crate::domain::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const RULE_VERSION: &str = "saxs-rules-2026-09-24";

pub fn stable_hash(value: &impl Serialize) -> String {
    let json = serde_json::to_string(value).unwrap_or_default();
    let mut hash_1: u64 = 0xcbf29ce484222325;
    let mut hash_2: u64 = 0x100000001b3;
    for byte in json.as_bytes() {
        hash_1 ^= u64::from(*byte);
        hash_1 = hash_1.wrapping_mul(0x100000001b3);
        hash_2 = hash_2.wrapping_add(u64::from(*byte));
        hash_2 ^= hash_2.rotate_left(13);
    }
    format!("h{:016x}{:016x}", hash_1, hash_2)
}

fn diagnostic(code: &str, severity: Severity, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        severity,
        message: message.to_string(),
    }
}

fn finite_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn curve_factor(curve: &Curve, response: &InstrumentResponse) -> f64 {
    curve.exposure.value * curve.thickness.value * curve.transmission.value * response.scale_factor
}

fn raw_point_version(curve_id: &str, point: &RawPoint) -> String {
    stable_hash(&(
        curve_id,
        &point.id,
        point.q,
        point.intensity,
        point.sigma,
        point.masked,
    ))
}

pub fn curve_version_id(curve: &Curve) -> String {
    stable_hash(&(
        &curve.id,
        curve
            .points
            .iter()
            .map(|p| raw_point_version(&curve.id, p))
            .collect::<Vec<_>>(),
        curve.exposure.value,
        curve.thickness.value,
        curve.transmission.value,
        &curve.instrument_response_id,
        curve.instrument_response.value,
        &curve.mask_source,
    ))
}

pub fn response_version_id(response: &InstrumentResponse) -> String {
    stable_hash(response)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReductionRequest {
    pub key: String,
    pub sample_id: String,
    pub buffer_id: String,
    pub label: String,
}

fn normalize_curve(
    curve: &Curve,
    response: &InstrumentResponse,
) -> (Vec<NormalizedPoint>, Vec<Diagnostic>) {
    let mut out = Vec::new();
    let mut diags = Vec::new();
    let factor = curve_factor(curve, response);
    if !finite_positive(factor) {
        diags.push(diagnostic(
            "normalization-factor-invalid",
            Severity::Error,
            "Exposure, thickness, transmission and response must define a finite positive normalization factor.",
        ));
    }
    for point in curve.points.iter().filter(|p| !p.masked) {
        let status = if point.intensity < 0.0 || point.sigma <= 0.0 || !finite_positive(factor) {
            PointStatus::Invalid
        } else if point.intensity == 0.0 {
            PointStatus::Warning
        } else {
            PointStatus::Ok
        };
        if point.intensity < 0.0 {
            diags.push(diagnostic(
                "negative-raw-intensity",
                Severity::Warning,
                "Negative raw intensity is retained and flagged rather than clipped.",
            ));
        }
        let normalized = point.intensity / factor;
        let statistical = point.sigma / factor;
        let normalization = normalized * response.relative_uncertainty;
        let sigma = (statistical.powi(2) + normalization.powi(2)).sqrt();
        out.push(NormalizedPoint {
            id: format!("{}:n:{}", curve.id, point.id),
            q: point.q,
            intensity: normalized,
            sigma,
            status,
            source: ProvenanceRef {
                entity_id: curve.id.clone(),
                version_id: curve_version_id(curve),
                point_id: Some(point.id.clone()),
            },
        });
    }
    (out, diags)
}

fn interpolate_background(
    buffer: &[NormalizedPoint],
    q: f64,
) -> Option<(f64, f64, String, String, String)> {
    if buffer.is_empty() {
        return None;
    }
    if q < buffer.first()?.q || q > buffer.last()?.q {
        return None;
    }
    match buffer.binary_search_by(|p| p.q.partial_cmp(&q).unwrap_or(std::cmp::Ordering::Equal)) {
        Ok(index) => Some((
            buffer[index].intensity,
            buffer[index].sigma,
            "exact-grid".to_string(),
            buffer[index].id.clone(),
            buffer[index].source.version_id.clone(),
        )),
        Err(index) => {
            let right = buffer.get(index)?;
            let left = buffer.get(index.checked_sub(1)?)?;
            let span = right.q - left.q;
            if span <= 0.0 {
                return None;
            }
            let weight_right = (q - left.q) / span;
            let weight_left = (right.q - q) / span;
            let value = left.intensity * weight_left + right.intensity * weight_right;
            let sigma =
                ((left.sigma * weight_left).powi(2) + (right.sigma * weight_right).powi(2)).sqrt();
            let interpolation_id = format!("interp[{}/{},{}]", left.id, right.id, q);
            Some((
                value,
                sigma,
                "linear-overlap".to_string(),
                interpolation_id,
                right.source.version_id.clone(),
            ))
        }
    }
}

fn weighted_scale(
    sample: &[NormalizedPoint],
    background: &[BackgroundInterpolatedPoint],
) -> (f64, f64, Vec<f64>, bool) {
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    let mut exact = true;
    let mut sample_sigmas = Vec::new();
    for (s, b) in sample.iter().zip(background) {
        if s.status == PointStatus::Invalid || !s.sigma.is_finite() || s.sigma <= 0.0 {
            continue;
        }
        let weight = 1.0 / s.sigma.powi(2);
        numerator += s.intensity * b.intensity * weight;
        denominator += b.intensity.powi(2) * weight;
        sample_sigmas.push(s.sigma);
        exact &= b.interpolation == "exact-grid";
    }
    if denominator > 0.0 && denominator.is_finite() {
        let alpha = numerator / denominator;
        let statistical = (1.0 / denominator).sqrt();
        let normalization = alpha.abs() * 0.006_f64.max(0.0);
        (
            alpha,
            (statistical.powi(2) + normalization.powi(2)).sqrt(),
            sample_sigmas,
            exact,
        )
    } else {
        (f64::NAN, f64::NAN, sample_sigmas, exact)
    }
}

fn ols(xs: &[f64], ys: &[f64], weights: &[f64]) -> Option<(f64, f64)> {
    let count = xs.len() as f64;
    if count < 2.0 {
        return None;
    }
    let mut sum_w = 0.0;
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xx = 0.0;
    let mut sum_xy = 0.0;
    for ((x, y), w) in xs.iter().zip(ys).zip(weights) {
        sum_w += w;
        sum_x += w * x;
        sum_y += w * y;
        sum_xx += w * x * x;
        sum_xy += w * x * y;
    }
    let determinant = sum_w * sum_xx - sum_x * sum_x;
    if determinant.abs() <= f64::MIN_POSITIVE {
        return None;
    }
    let intercept = (sum_y * sum_xx - sum_x * sum_xy) / determinant;
    let slope = (sum_w * sum_xy - sum_x * sum_y) / determinant;
    Some((slope, intercept))
}

fn fit_interval(points: &[&SubtractedPoint], model: &str, id: String) -> IntervalCandidate {
    let usable: Vec<&SubtractedPoint> = points.iter().copied().filter(|p| p.intensity > 0.0 && p.sigma > 0.0).collect();
    let q_min = points.first().map(|p| p.q).unwrap_or(f64::NAN);
    let q_max = points.last().map(|p| p.q).unwrap_or(f64::NAN);
    let mut candidate = IntervalCandidate {
        id,
        model: model.to_string(),
        q_min,
        q_max,
        q_rg: None,
        i0: None,
        slope: None,
        chi_squared: f64::NAN,
        rms_residual: f64::NAN,
        max_abs_residual: f64::NAN,
        point_count: usable.len(),
        accepted: None,
        diagnostic: None,
    };
    if usable.len() < 3 {
        candidate.diagnostic = Some(diagnostic(
            "interval-insufficient-positive-points",
            Severity::Error,
            "At least three positive subtracted points are required for this model candidate.",
        ));
        return candidate;
    }
    let xs: Vec<f64> = if model == "guinier" {
        usable.iter().map(|p| p.q.powi(2)).collect()
    } else {
        usable.iter().map(|p| p.q.ln()).collect()
    };
    let ys: Vec<f64> = usable.iter().map(|p| p.intensity.ln()).collect();
    let weights: Vec<f64> = usable.iter().map(|p| 1.0 / p.sigma.powi(2)).collect();
    let Some((slope, intercept)) = ols(&xs, &ys, &weights) else {
        candidate.diagnostic = Some(diagnostic(
            "interval-fit-singular",
            Severity::Error,
            "The q interval is numerically singular.",
        ));
        return candidate;
    };
    let predictions: Vec<f64> = if model == "guinier" {
        usable
            .iter()
            .map(|p| (intercept + slope * p.q.powi(2)).exp())
            .collect()
    } else {
        usable
            .iter()
            .map(|p| (intercept + slope * p.q.ln()).exp())
            .collect()
    };
    let mut chi_squared = 0.0;
    let mut squared = 0.0;
    let mut max_abs: f64 = 0.0;
    for (point, prediction) in usable.iter().zip(predictions) {
        let residual = point.intensity - prediction;
        chi_squared += residual.powi(2) / point.sigma.powi(2);
        squared += residual.powi(2);
        max_abs = max_abs.max(residual.abs());
    }
    candidate.slope = Some(slope);
    candidate.i0 = Some(intercept.exp());
    candidate.chi_squared = chi_squared;
    candidate.rms_residual = (squared / usable.len() as f64).sqrt();
    candidate.max_abs_residual = max_abs;
    if model == "guinier" {
        if slope < 0.0 {
            candidate.q_rg = Some((-3.0 / slope).sqrt());
        } else {
            candidate.diagnostic = Some(diagnostic(
                "guinier-positive-slope",
                Severity::Error,
                "Guinier slope is non-negative; Rg is not reported.",
            ));
        }
    }
    candidate
}

fn interval_candidates(points: &[SubtractedPoint]) -> Vec<IntervalCandidate> {
    let usable: Vec<&SubtractedPoint> = points
        .iter()
        .filter(|p| p.status != PointStatus::Invalid && p.intensity > 0.0)
        .collect();
    if usable.len() < 6 {
        return Vec::new();
    }
    let low_end = ((usable.len() as f64) * 0.62).ceil() as usize;
    let high_start = ((usable.len() as f64) * 0.38).floor() as usize;
    let mut low = fit_interval(&usable[..low_end], "guinier", "guinier-low-q".to_string());
    let mut high = fit_interval(&usable[high_start..], "porod", "porod-high-q".to_string());
    if low.q_max > high.q_min {
        let message = "Guinier and Porod candidates overlap; they are retained separately and are not automatically joined into one model.";
        low.diagnostic.get_or_insert_with(|| {
            diagnostic("model-interval-overlap", Severity::Warning, message)
        });
        high.diagnostic = Some(diagnostic(
            "model-interval-overlap",
            Severity::Warning,
            message,
        ));
    }
    vec![low, high]
}

pub fn reduce(bundle: &CaseBundle, request: &ReductionRequest) -> ReductionRun {
    let mut diagnostics = Vec::new();
    let sample = match bundle.samples.get(&request.sample_id) {
        Some(sample) => &sample.curve,
        None => {
            return invalid_run(
                request,
                "sample-missing",
                "Requested sample is absent from the immutable case bundle.",
            )
        }
    };
    let buffer = match bundle.buffers.get(&request.buffer_id) {
        Some(buffer) => &buffer.curve,
        None => {
            return invalid_run(
                request,
                "buffer-missing",
                "Requested buffer candidate version is absent.",
            )
        }
    };
    let Some(response) = bundle.responses.get(&sample.instrument_response_id) else {
        return invalid_run(
            request,
            "response-missing",
            "Instrument response source is absent.",
        );
    };

    let (sample_norm, sample_diags) = normalize_curve(sample, response);
    let (buffer_norm, buffer_diags) = normalize_curve(buffer, response);
    diagnostics.extend(sample_diags);
    diagnostics.extend(buffer_diags);

    let mut background = Vec::new();
    let mut used_exact_grid = true;
    for point in &sample_norm {
        if let Some((intensity, sigma, interpolation, point_id, buffer_version)) =
            interpolate_background(&buffer_norm, point.q)
        {
            used_exact_grid &= interpolation == "exact-grid";
            background.push(BackgroundInterpolatedPoint {
                id: format!("{}:b-at-{}", buffer.id, point.id),
                q: point.q,
                intensity,
                sigma,
                interpolation,
                source: ProvenanceRef {
                    entity_id: buffer.id.clone(),
                    version_id: buffer_version,
                    point_id: Some(point_id),
                },
            });
        }
    }

    if background.is_empty() {
        diagnostics.push(diagnostic(
            "no-overlapping-q-range",
            Severity::Error,
            "The q grids have no overlap; no subtraction is performed.",
        ));
    } else if !used_exact_grid {
        diagnostics.push(diagnostic(
            "q-grid-interpolation",
            Severity::Warning,
            "Different q grids are never directly subtracted; linear interpolation is used only inside the overlap.",
        ));
    }

    let (scale, scale_sigma, _, _) = weighted_scale(&sample_norm[..background.len()], &background);
    if !scale.is_finite() || !scale_sigma.is_finite() || scale <= 0.0 {
        diagnostics.push(diagnostic(
            "background-scale-undetermined",
            Severity::Error,
            "A finite positive background scale could not be estimated.",
        ));
    } else if scale_sigma / scale.abs() > 0.10 {
        diagnostics.push(diagnostic(
            "background-scale-uncertain",
            Severity::Warning,
            "Relative background scale uncertainty exceeds 10%; the uncertainty band includes scale uncertainty.",
        ));
    }

    let mut subtracted = Vec::new();
    for (sample_point, background_point) in sample_norm.iter().zip(background.iter()) {
        let intensity = if scale.is_finite() {
            sample_point.intensity - scale * background_point.intensity
        } else {
            f64::NAN
        };
        let sigma = if scale.is_finite() {
            let sample_term = sample_point.sigma.powi(2);
            let background_term = scale.powi(2) * background_point.sigma.powi(2);
            let scale_term = background_point.intensity.powi(2) * scale_sigma.powi(2);
            (sample_term + background_term + scale_term).sqrt()
        } else {
            f64::NAN
        };
        let status = if !scale.is_finite() || sample_point.status == PointStatus::Invalid {
            PointStatus::Invalid
        } else if intensity < 0.0 {
            PointStatus::Warning
        } else {
            PointStatus::Ok
        };
        if intensity < 0.0 {
            diagnostics.push(diagnostic(
                "negative-subtracted-intensity",
                Severity::Warning,
                "Negative subtracted intensity is retained with its uncertainty instead of being clipped.",
            ));
        }
        let stable_id = format!(
            "derived:{}:{}:{}",
            sample.id,
            buffer.id,
            sample_point
                .source
                .point_id
                .as_deref()
                .unwrap_or(&sample_point.id)
        );
        subtracted.push(SubtractedPoint {
            id: stable_id.clone(),
            id_stable: stable_id,
            q: sample_point.q,
            sample_intensity: sample_point.intensity,
            background_intensity: background_point.intensity,
            scale,
            intensity,
            sigma,
            status,
            sample_source: sample_point.source.clone(),
            background_source: background_point.source.clone(),
        });
    }

    let intervals = if scale.is_finite() {
        interval_candidates(&subtracted)
    } else {
        Vec::new()
    };
    let sample_version = curve_version_id(sample);
    let buffer_version = curve_version_id(buffer);
    let response_version = response_version_id(response);
    let overlap = (
        background.first().map(|p| p.q),
        background.last().map(|p| p.q),
    );
    let mut hashes = BTreeMap::new();
    hashes.insert(format!("sample:{}", sample.id), sample_version.clone());
    hashes.insert(format!("buffer:{}", buffer.id), buffer_version.clone());
    hashes.insert(format!("response:{}", response.id), response_version);
    hashes.insert("rules".to_string(), RULE_VERSION.to_string());
    let version_id = stable_hash(&(&request.key, &hashes));
    let status = if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        RunStatus::Invalid
    } else if diagnostics.iter().any(|d| d.severity == Severity::Warning) {
        RunStatus::Warning
    } else {
        RunStatus::Valid
    };

    ReductionRun {
        id: version_id,
        key: request.key.clone(),
        sample_id: sample.id.clone(),
        buffer_id: buffer.id.clone(),
        status,
        normalized_sample: sample_norm,
        interpolated_background: background,
        subtracted,
        scale,
        scale_sigma,
        scale_method: "weighted-overlap-least-squares".to_string(),
        overlap_q_min: overlap.0,
        overlap_q_max: overlap.1,
        intervals,
        diagnostics,
        input_hashes: hashes,
        rule_version: RULE_VERSION.to_string(),
    }
}

fn invalid_run(request: &ReductionRequest, code: &str, message: &str) -> ReductionRun {
    ReductionRun {
        id: stable_hash(&(&request.key, code, RULE_VERSION)),
        key: request.key.clone(),
        sample_id: request.sample_id.clone(),
        buffer_id: request.buffer_id.clone(),
        status: RunStatus::Invalid,
        normalized_sample: Vec::new(),
        interpolated_background: Vec::new(),
        subtracted: Vec::new(),
        scale: f64::NAN,
        scale_sigma: f64::NAN,
        scale_method: "weighted-overlap-least-squares".to_string(),
        overlap_q_min: None,
        overlap_q_max: None,
        intervals: Vec::new(),
        diagnostics: vec![diagnostic(code, Severity::Error, message)],
        input_hashes: BTreeMap::new(),
        rule_version: RULE_VERSION.to_string(),
    }
}
