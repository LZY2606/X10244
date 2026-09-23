use crate::domain::*;

pub fn normalize_curve(curve: &RawCurve) -> NormalizedCurve {
    let mut diagnostics = curve.diagnostics.clone();
    let exposure = curve.exposure.value;
    let thickness = curve.thickness.value;
    let transmission = curve.transmission.value;
    let response = curve.instrument_response.value;
    let inputs = [exposure, thickness, transmission, response];
    let valid = inputs.iter().all(|value| value.is_some_and(|v| v > 0.0));
    let normalization = if valid {
        exposure.unwrap() * thickness.unwrap() * transmission.unwrap() * response.unwrap()
    } else {
        0.0
    };
    let relative_sigma = if valid {
        [
            curve.exposure.uncertainty / exposure.unwrap(),
            curve.thickness.uncertainty / thickness.unwrap(),
            curve.transmission.uncertainty / transmission.unwrap(),
            curve.instrument_response.uncertainty / response.unwrap(),
        ]
        .into_iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt()
    } else {
        f64::NAN
    };
    if !valid {
        diagnostics.push("normalization blocked because a positive input is unavailable");
    }
    let points = curve
        .points
        .iter()
        .map(|point| {
            let value = if valid && normalization.is_finite() { point.i / normalization } else { f64::NAN };
            let sigma = if valid && normalization.is_finite() {
                let relative = ((point.sigma / point.i).powi(2) + relative_sigma.powi(2)).sqrt();
                value.abs() * relative
            } else {
                f64::NAN
            };
            NormalizedPoint {
                stable_id: format!("normalized:{}:{}", curve.current_version, point.point_id),
                source_point_id: point.point_id.clone(),
                q: point.q,
                i: value,
                sigma,
                raw_curve_version: curve.current_version.clone(),
            }
        })
        .collect();
    NormalizedCurve {
        node_id: format!("normalization:{}", curve.current_version),
        raw_curve_version: curve.current_version.clone(),
        normalization,
        normalization_relative_sigma: relative_sigma,
        points,
        valid,
        diagnostics,
    }
}

pub fn subtract_background(
    sample: &NormalizedCurve,
    buffer: &NormalizedCurve,
    buffer_candidate_id: &str,
    scale_definitions: &[ScaleCandidateDefinition],
    scale_candidate_id: &str,
) -> Vec<SubtractionResult> {
    let overlap = overlap_bounds(&sample.points, &buffer.points);
    scale_definitions
        .iter()
        .map(|definition| {
            one_subtraction(
                sample,
                buffer,
                buffer_candidate_id,
                definition,
                scale_candidate_id,
                overlap,
            )
        })
        .collect()
}

fn one_subtraction(
    sample: &NormalizedCurve,
    buffer: &NormalizedCurve,
    buffer_candidate_id: &str,
    definition: &ScaleCandidateDefinition,
    selected_scale_id: &str,
    overlap: Option<(f64, f64)>,
) -> SubtractionResult {
    let node_id = format!(
        "subtraction:{}:{}:{}",
        sample.raw_curve_version, buffer.raw_curve_version, definition.id
    );
    let mut diagnostics = Vec::new();
    if !sample.valid || !buffer.valid {
        return SubtractionResult {
            node_id,
            sample_version: sample.raw_curve_version.clone(),
            buffer_version: buffer.raw_curve_version.clone(),
            buffer_candidate_id: buffer_candidate_id.to_string(),
            scale_candidate_id: definition.id.clone(),
            q_overlap_min: None,
            q_overlap_max: None,
            status: DerivationStatus::InvalidInputs,
            scale: None,
            points: Vec::new(),
            residuals: Vec::new(),
            diagnostics: vec!["normalized inputs are invalid".to_string()],
            valid: false,
        };
    }
    let Some((q_min, q_max)) = overlap else {
        diagnostics.push("no overlapping q interval; direct point subtraction is forbidden".to_string());
        return SubtractionResult {
            node_id,
            sample_version: sample.raw_curve_version.clone(),
            buffer_version: buffer.raw_curve_version.clone(),
            buffer_candidate_id: buffer_candidate_id.to_string(),
            scale_candidate_id: definition.id.clone(),
            q_overlap_min: None,
            q_overlap_max: None,
            status: DerivationStatus::BlockedNoOverlap,
            scale: None,
            points: Vec::new(),
            residuals: Vec::new(),
            diagnostics,
            valid: false,
        };
    };

    let exact_grid = same_q_grid(&sample.points, &buffer.points);
    if exact_grid {
        diagnostics.push("sample and buffer use the same q grid; each point is matched directly".to_string());
    } else {
        diagnostics.push("q grids differ; buffer values are linearly interpolated only inside the overlap".to_string());
    }

    let fit = if definition.id == "scale_weighted_fit" {
        fit_scale(sample, buffer, definition, q_min, q_max)
    } else {
        nominal_scale(definition)
    };
    diagnostics.push(format!(
        "background scale uncertainty sigma={:.6} includes declared scaling uncertainty",
        fit.scale_sigma
    ));

    let mut points = Vec::new();
    let mut residuals = Vec::new();
    for sample_point in &sample.points {
        if sample_point.q < q_min || sample_point.q > q_max {
            continue;
        }
        let (buffer_i, low, high, interpolated) = if exact_grid {
            let matched = buffer.points.iter().find(|p| approx_eq(p.q, sample_point.q));
            let Some(matched) = matched else { continue };
            (matched.i, Some(matched.source_point_id.clone()), None, false)
        } else {
            match interpolate_point(&buffer.points, sample_point.q) {
                Some((value, low_id, high_id)) => (value, Some(low_id), Some(high_id), true),
                None => continue,
            }
        };
        let predicted = fit.scale * buffer_i;
        let buffer_sigma = buffer_sigma(&buffer.points, sample_point.q).unwrap_or(0.0);
        let background_sigma = predicted
            .hypot(fit.scale * buffer_sigma)
            .hypot(fit.scale_sigma * buffer_i);
        let sigma = sample_point.sigma.hypot(background_sigma);
        let value = sample_point.i - predicted;
        let residual = ResidualPoint {
            q: sample_point.q,
            observed: sample_point.i,
            predicted_background: predicted,
            residual: value,
            sigma,
        };
        if value < 0.0 {
            diagnostics.push(format!("negative subtracted intensity retained at q={:.6}", sample_point.q));
        }
        points.push(SubtractedPoint {
            stable_id: format!("{}:{}", node_id, sample_point.source_point_id),
            q: sample_point.q,
            i: value,
            sigma,
            sample_point_id: sample_point.source_point_id.clone(),
            buffer_low_id: low,
            buffer_high_id: high,
            interpolated,
            negative: value < 0.0,
        });
        residuals.push(residual);
    }

    let valid = fit.valid && !points.is_empty();
    if points.is_empty() {
        diagnostics.push("overlap contains no usable derived points".to_string());
    }
    let mut fit_diagnostics = fit.diagnostics.clone();
    diagnostics.append(&mut fit_diagnostics);
    SubtractionResult {
        node_id,
        sample_version: sample.raw_curve_version.clone(),
        buffer_version: buffer.raw_curve_version.clone(),
        buffer_candidate_id: buffer_candidate_id.to_string(),
        scale_candidate_id: definition.id.clone(),
        q_overlap_min: Some(q_min),
        q_overlap_max: Some(q_max),
        status: DerivationStatus::Ready,
        scale: Some(ScaleFit {
            accepted: definition.id == selected_scale_id,
            ..fit
        }),
        points,
        residuals,
        diagnostics,
        valid,
    }
}

fn fit_scale(
    sample: &NormalizedCurve,
    buffer: &NormalizedCurve,
    definition: &ScaleCandidateDefinition,
    q_min: f64,
    q_max: f64,
) -> ScaleFit {
    let mut weighted_sum = 0.0;
    let mut weighted_square = 0.0;
    let mut count = 0usize;
    let mut pairs = Vec::new();
    for sample_point in &sample.points {
        if sample_point.q < q_min || sample_point.q > q_max {
            continue;
        }
        let Some(buffer_i) = buffer_value(&buffer.points, sample_point.q) else {
            continue;
        };
        let buffer_sigma = buffer_sigma(&buffer.points, sample_point.q).unwrap_or(0.0);
        let sigma = sample_point.sigma.hypot(buffer_sigma).max(f64::MIN_POSITIVE);
        let weight = 1.0 / (sigma * sigma);
        weighted_sum += weight * sample_point.i * buffer_i;
        weighted_square += weight * buffer_i * buffer_i;
        pairs.push((sample_point.i, buffer_i, sigma));
        count += 1;
    }
    if count == 0 || weighted_square <= 0.0 {
        return ScaleFit {
            candidate_id: definition.id.clone(),
            method: definition.method.clone(),
            scale: 0.0,
            scale_sigma: f64::NAN,
            weighted_rss: f64::NAN,
            reduced_chi_square: f64::NAN,
            accepted: false,
            valid: false,
            diagnostics: vec!["weighted scale fit has no usable overlap".to_string()],
        };
    }
    let scale = (weighted_sum / weighted_square).max(0.0);
    let scale_sigma = (1.0 / weighted_square).sqrt().max(definition.external_sigma);
    let weighted_rss: f64 = pairs
        .iter()
        .map(|(sample_i, buffer_i, sigma)| ((sample_i - scale * buffer_i) / sigma).powi(2))
        .sum();
    let reduced_chi_square = weighted_rss / count.saturating_sub(1).max(1) as f64;
    ScaleFit {
        candidate_id: definition.id.clone(),
        method: definition.method.clone(),
        scale,
        scale_sigma,
        weighted_rss,
        reduced_chi_square,
        accepted: false,
        valid: true,
        diagnostics: vec!["scale constrained to be non-negative".to_string()],
    }
}

fn nominal_scale(definition: &ScaleCandidateDefinition) -> ScaleFit {
    ScaleFit {
        candidate_id: definition.id.clone(),
        method: definition.method.clone(),
        scale: 1.0,
        scale_sigma: definition.external_sigma.max(f64::EPSILON),
        weighted_rss: 0.0,
        reduced_chi_square: 0.0,
        accepted: false,
        valid: true,
        diagnostics: vec!["nominal scale candidate uses propagated external uncertainty".to_string()],
    }
}

pub fn evaluate_intervals(result: &SubtractionResult, windows: &[IntervalWindow]) -> ModelEvidence {
    let mut guinier = Vec::new();
    let mut porod = Vec::new();
    for window in windows {
        let candidate = fit_interval(result, window);
        match window.kind {
            IntervalKind::Guinier => guinier.push(candidate),
            IntervalKind::Porod => porod.push(candidate),
        }
    }
    let mut diagnostics = vec!["Guinier and Porod remain separate evidence and are never auto-stitched".to_string()];
    if intervals_overlap(&guinier) || intervals_overlap(&porod) {
        diagnostics.push("overlapping interval candidates are retained as competing evidence".to_string());
    }
    ModelEvidence {
        node_id: format!("model_evidence:{}", result.node_id),
        subtraction_node_id: result.node_id.clone(),
        guinier,
        porod,
        stitched_model: None,
        diagnostics,
    }
}

fn fit_interval(result: &SubtractionResult, window: &IntervalWindow) -> IntervalCandidate {
    let positive: Vec<_> = result
        .points
        .iter()
        .filter(|point| point.q >= window.q_min && point.q <= window.q_max && point.i > 0.0)
        .collect();
    let includes_negative = result
        .points
        .iter()
        .any(|point| point.q >= window.q_min && point.q <= window.q_max && point.negative);
    let mut diagnostics = Vec::new();
    if includes_negative {
        diagnostics.push("negative points remain visible but are excluded from log-domain fits".to_string());
    }
    if positive.len() < 3 {
        diagnostics.push("interval has fewer than three positive points".to_string());
    }
    let (slope, intercept, r_square, rms_residual) = if positive.len() >= 3 {
        let pairs: Vec<(f64, f64)> = match window.kind {
            IntervalKind::Guinier => positive.iter().map(|p| (p.q * p.q, p.i.ln())).collect(),
            IntervalKind::Porod => positive.iter().map(|p| (p.q.ln(), p.i.ln())).collect(),
        };
        linear_regression(&pairs)
    } else {
        (f64::NAN, f64::NAN, f64::NAN, f64::NAN)
    };
    IntervalCandidate {
        stable_id: format!("interval:{}:{}", window.id, result.node_id),
        window_id: window.id.clone(),
        kind: window.kind,
        q_min: window.q_min,
        q_max: window.q_max,
        slope,
        intercept,
        r_square,
        rms_residual,
        point_count: positive.len(),
        includes_negative,
        valid: positive.len() >= 3 && slope.is_finite(),
        diagnostics,
    }
}

fn linear_regression(pairs: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    let n = pairs.len() as f64;
    let mean_x = pairs.iter().map(|p| p.0).sum::<f64>() / n;
    let mean_y = pairs.iter().map(|p| p.1).sum::<f64>() / n;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    let mut syy = 0.0;
    let mut sse = 0.0;
    for (x, y) in pairs {
        let dx = x - mean_x;
        let dy = y - mean_y;
        sxx += dx * dx;
        sxy += dx * dy;
        syy += dy * dy;
    }
    let slope = sxy / sxx;
    let intercept = mean_y - slope * mean_x;
    for (x, y) in pairs {
        let predicted = slope * x + intercept;
        sse += (y - predicted).powi(2);
    }
    let r_square = if syy > 0.0 { 1.0 - sse / syy } else { f64::NAN };
    let rms_residual = (sse / n).sqrt();
    (slope, intercept, r_square, rms_residual)
}

fn intervals_overlap(candidates: &[IntervalCandidate]) -> bool {
    candidates.windows(2).any(|pair| pair[0].q_max > pair[1].q_min)
}

fn same_q_grid(sample: &[NormalizedPoint], buffer: &[NormalizedPoint]) -> bool {
    sample.len() == buffer.len()
        && sample
            .iter()
            .zip(buffer)
            .all(|(left, right)| approx_eq(left.q, right.q))
}

fn overlap_bounds(sample: &[NormalizedPoint], buffer: &[NormalizedPoint]) -> Option<(f64, f64)> {
    let q_min = sample.first()?.q.max(buffer.first()?.q);
    let q_max = sample.last()?.q.min(buffer.last()?.q);
    if q_min < q_max || approx_eq(q_min, q_max) && sample.len() > 1 && buffer.len() > 1 {
        Some((q_min, q_max))
    } else {
        None
    }
}

fn buffer_value(points: &[NormalizedPoint], q: f64) -> Option<f64> {
    if let Some(exact) = points.iter().find(|point| approx_eq(point.q, q)) {
        return Some(exact.i);
    }
    interpolate_point(points, q).map(|(value, _, _)| value)
}

fn buffer_sigma(points: &[NormalizedPoint], q: f64) -> Option<f64> {
    if let Some(exact) = points.iter().find(|point| approx_eq(point.q, q)) {
        return Some(exact.sigma);
    }
    interpolate_raw(points, q, |point| point.sigma)
}

fn interpolate_point(points: &[NormalizedPoint], q: f64) -> Option<(f64, String, String)> {
    let index = points.iter().position(|point| point.q >= q)?;
    if index == 0 {
        return None;
    }
    let low = &points[index - 1];
    let high = &points[index];
    if !(low.q..=high.q).contains(&q) {
        return None;
    }
    let fraction = (q - low.q) / (high.q - low.q);
    Some((low.i + fraction * (high.i - low.i), low.source_point_id.clone(), high.source_point_id.clone()))
}

fn interpolate_raw(points: &[NormalizedPoint], q: f64, value: impl Fn(&NormalizedPoint) -> f64) -> Option<f64> {
    let index = points.iter().position(|point| point.q >= q)?;
    if index == 0 {
        return None;
    }
    let low = &points[index - 1];
    let high = &points[index];
    let fraction = (q - low.q) / (high.q - low.q);
    Some(value(low) + fraction * (value(high) - value(low)))
}

fn approx_eq(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1e-10
}
