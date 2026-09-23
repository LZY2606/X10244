use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub seq: i64,
    pub at: String,
    pub actor: String,
    pub kind: String,
    pub branch_id: String,
    pub parent_event_id: String,
    pub base_refs: BTreeMap<String, String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDraft {
    pub actor: String,
    pub kind: String,
    pub branch_id: String,
    pub parent_event_id: Option<String>,
    pub base_refs: BTreeMap<String, String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: StableId,
    pub record_type: String,
    pub scope: String,
    pub order_key: i64,
    pub value: serde_json::Value,
    pub sources: Vec<SourceEdge>,
    pub created_event: String,
    pub retired_event: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StableId {
    pub local: String,
    pub origin: String,
    pub version: i64,
}

impl StableId {
    pub fn to_string(&self) -> String {
        format!("{}#{}@{}", self.local, self.origin, self.version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceEdge {
    pub source_id: String,
    pub role: String,
    pub slice: Option<SliceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SliceRef {
    pub axis: String,
    pub from_index: Option<usize>,
    pub to_index: Option<usize>,
    pub coordinate: Option<String>,
    pub basis: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
    pub original: String,
    pub basis: Option<String>,
    pub converted_from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unconverted {
    pub original: String,
    pub declared_unit: Option<String>,
    pub diagnostic: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Measure {
    Quantity(Quantity),
    Unconverted(Unconverted),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawPoint {
    pub index: usize,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawCurve {
    pub label: String,
    pub role: String,
    pub points: Vec<RawPoint>,
    pub exposure: Measure,
    pub thickness: Measure,
    pub transmission: Measure,
    pub instrument_response: SourceArtifact,
    pub mask: SourceArtifact,
    pub q_axis: AxisSpec,
    pub i_axis: AxisSpec,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AxisSpec {
    pub name: String,
    pub unit: String,
    pub coordinate_basis: String,
    pub original_representation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceArtifact {
    pub source: String,
    pub checksum: String,
    pub original_representation: String,
    pub coordinate_basis: String,
    pub retained: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub object_id: Option<String>,
    pub index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizationRule {
    pub sample_relative_uncertainty: f64,
    pub buffer_relative_uncertainty: f64,
    pub formula: String,
}

impl Default for NormalizationRule {
    fn default() -> Self {
        Self { sample_relative_uncertainty: 0.01, buffer_relative_uncertainty: 0.01, formula: "I_norm=(I/transmission/exposure/thickness)*response".to_string() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurvePoint {
    pub index: usize,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub origin: PointOrigin,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointOrigin {
    pub kind: String,
    pub source_index: Option<usize>,
    pub source_points: Vec<usize>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedCurve {
    pub label: String,
    pub role: String,
    pub raw_id: String,
    pub points: Vec<CurvePoint>,
    pub exposure: Quantity,
    pub thickness: Quantity,
    pub transmission: Quantity,
    pub scale: f64,
    pub rule: NormalizationRule,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtractionResult {
    pub sample_id: String,
    pub buffer_id: String,
    pub sample_q_min: f64,
    pub sample_q_max: f64,
    pub buffer_q_min: f64,
    pub buffer_q_max: f64,
    pub overlap_q_min: f64,
    pub overlap_q_max: f64,
    pub q_grid_mode: String,
    pub scale_mode: String,
    pub alpha: f64,
    pub sigma_alpha: f64,
    pub points: Vec<CurvePoint>,
    pub diagnostics: Vec<Diagnostic>,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FitCandidate {
    pub model_family: String,
    pub q_min: f64,
    pub q_max: f64,
    pub parameters: BTreeMap<String, f64>,
    pub parameter_units: BTreeMap<String, String>,
    pub chi_square: f64,
    pub residual_rms: f64,
    pub point_count: usize,
    pub evidence: f64,
    pub residuals: Vec<ResidualPoint>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidualPoint {
    pub index: usize,
    pub q: f64,
    pub observed: f64,
    pub expected: f64,
    pub normalized_residual: f64,
}

pub fn quantity(value: f64, unit: &str, original: &str, basis: Option<&str>) -> Measure {
    Measure::Quantity(Quantity { value, unit: unit.into(), original: original.into(), basis: basis.map(str::to_string), converted_from: None })
}

pub fn q_value(measure: &Measure) -> Option<f64> {
    match measure {
        Measure::Quantity(q) if q.unit == "1" || q.unit == "dimensionless" => Some(q.value),
        _ => None,
    }
}

pub fn as_quantity(measure: &Measure) -> Option<&Quantity> {
    match measure {
        Measure::Quantity(q) => Some(q),
        Measure::Unconverted(_) => None,
    }
}

pub fn normalize_curve(label: &str, role: &str, raw_id: &str, raw: &RawCurve, rule: &NormalizationRule) -> Result<NormalizedCurve, Diagnostic> {
    let exposure = as_quantity(&raw.exposure).ok_or_else(|| unconverted("exposure"))?;
    let thickness = as_quantity(&raw.thickness).ok_or_else(|| unconverted("thickness"))?;
    let transmission = as_quantity(&raw.transmission).ok_or_else(|| unconverted("transmission"))?;
    if exposure.value <= 0.0 || thickness.value <= 0.0 || transmission.value <= 0.0 {
        return Err(Diagnostic { code: "NON_POSITIVE_NORMALIZATION_INPUT".into(), severity: "error".into(), message: "Exposure, thickness and transmission must be positive.".into(), object_id: Some(raw_id.into()), index: None });
    }
    let scale = 1.0 / (transmission.value * exposure.value * thickness.value);
    let points = raw.points.iter().map(|p| CurvePoint {
        index: p.index,
        q: p.q,
        i: p.i * scale,
        sigma: (p.sigma * scale).hypot(p.i.abs() * scale * relative_uncertainty(role, rule)),
        origin: PointOrigin { kind: "raw-point".into(), source_index: Some(p.index), source_points: vec![p.index], note: Some("normalized without interpolation".into()) },
        flags: if p.i < 0.0 || p.sigma.is_nan() { vec!["negative-raw-intensity".into()] } else { vec![] },
    }).collect();
    Ok(NormalizedCurve {
        label: label.into(), role: role.into(), raw_id: raw_id.into(), points,
        exposure: exposure.clone(), thickness: thickness.clone(), transmission: transmission.clone(),
        scale, rule: rule.clone(), diagnostics: raw.diagnostics.clone(),
    })
}

fn relative_uncertainty(role: &str, rule: &NormalizationRule) -> f64 {
    if role == "buffer" { rule.buffer_relative_uncertainty } else { rule.sample_relative_uncertainty }
}

fn unconverted(field: &str) -> Diagnostic {
    Diagnostic { code: "UNCONVERTED_MEASURE".into(), severity: "error".into(), message: format!("{field} could not be converted and remains on the raw record."), object_id: None, index: None }
}

pub fn subtract_background(
    sample: &NormalizedCurve,
    buffer: &NormalizedCurve,
    sample_id: &str,
    buffer_id: &str,
    fixed_alpha: Option<f64>,
) -> SubtractionResult {
    let mut diagnostics = Vec::new();
    if sample.points.is_empty() || buffer.points.is_empty() {
        diagnostics.push(Diagnostic { code: "EMPTY_CURVE".into(), severity: "error".into(), message: "Sample or buffer has no points.".into(), object_id: None, index: None });
        return invalid_subtraction(sample_id, buffer_id, sample, buffer, "empty", fixed_alpha, diagnostics);
    }
    let sample_min = sample.points.first().unwrap().q;
    let sample_max = sample.points.last().unwrap().q;
    let buffer_min = buffer.points.first().unwrap().q;
    let buffer_max = buffer.points.last().unwrap().q;
    let overlap_min = sample_min.max(buffer_min);
    let overlap_max = sample_max.min(buffer_max);
    if overlap_max < overlap_min || !overlap_exists(&sample.points, &buffer.points) {
        diagnostics.push(Diagnostic { code: "NO_OVERLAP".into(), severity: "error".into(), message: "No shared q interval; no direct subtraction or extrapolation was performed.".into(), object_id: None, index: None });
        return invalid_subtraction(sample_id, buffer_id, sample, buffer, "no-overlap", fixed_alpha, diagnostics);
    }

    let same_grid = grids_match(&sample.points, &buffer.points);
    let q_grid_mode = if same_grid { "identical" } else { "interpolated-overlap" };
    if !same_grid {
        diagnostics.push(Diagnostic { code: "Q_GRID_MISMATCH".into(), severity: "warning".into(), message: "Q grids differ; buffer was linearly interpolated only inside the overlap.".into(), object_id: None, index: None });
    }

    let mut evaluated = Vec::new();
    for sp in &sample.points {
        if sp.q < overlap_min || sp.q > overlap_max {
            continue;
        }
        if let Some((b_i, b_sigma, origin, note)) = interpolate_buffer_at(&buffer.points, sp.q) {
            evaluated.push((sp, b_i, b_sigma, origin, note));
        }
    }
    if evaluated.is_empty() {
        diagnostics.push(Diagnostic { code: "NO_INTERPOLATED_POINTS".into(), severity: "error".into(), message: "Overlap interval exists but no interpolatable sample points were found.".into(), object_id: None, index: None });
        return invalid_subtraction(sample_id, buffer_id, sample, buffer, q_grid_mode, fixed_alpha, diagnostics);
    }

    let (alpha, sigma_alpha, scale_mode) = match fixed_alpha {
        Some(value) => (value, value * (sample.rule.sample_relative_uncertainty.hypot(buffer.rule.buffer_relative_uncertainty)), "fixed"),
        None => {
            let nums: f64 = evaluated.iter().map(|(s, b_i, bs, _, _)| s.i * b_i / (s.sigma.powi(2) + bs.powi(2))).sum();
            let dens: f64 = evaluated.iter().map(|(s, b_i, bs, _, _)| b_i.powi(2) / (s.sigma.powi(2) + bs.powi(2))).sum();
            if dens <= 0.0 {
                (f64::NAN, f64::NAN, "undefined")
            } else {
                let alpha = nums / dens;
                let reduced: f64 = evaluated.iter().map(|(s, b_i, bs, _, _)| {
                    let model = s.i - alpha * b_i;
                    model.powi(2) / (s.sigma.powi(2) + (alpha * bs).powi(2))
                }).sum::<f64>() / (evaluated.len() as f64 - 1.0).max(1.0);
                let variance = reduced / dens;
                (alpha, variance.max(0.0).sqrt(), "weighted-least-squares")
            }
        }
    };
    if !alpha.is_finite() || alpha < 0.0 {
        diagnostics.push(Diagnostic { code: "BACKGROUND_SCALE_UNDEFINED".into(), severity: "error".into(), message: "Background scale is non-finite or negative; subtraction is retained as invalid rather than clipped.".into(), object_id: None, index: None });
    }

    let mut points = Vec::new();
    for (sp, buffer_i, buffer_sigma, buffer_origin, note) in evaluated {
        let intensity = sp.i - alpha * buffer_i;
        let norm_term = sp.i.abs() * sample.rule.sample_relative_uncertainty;
        let buffer_norm_term = buffer_i.abs() * buffer.rule.buffer_relative_uncertainty;
        let sigma = (sp.sigma.powi(2)
            + (alpha * buffer_sigma).powi(2)
            + norm_term.powi(2)
            + (alpha * buffer_norm_term).powi(2)
            + (sigma_alpha * buffer_i).powi(2))
            .max(0.0)
            .sqrt();
        let mut flags = sp.flags.clone();
        if intensity < 0.0 { flags.push("negative-subtracted-intensity".into()); }
        if note.is_some() { flags.push("buffer-interpolated".into()); }
        points.push(CurvePoint {
            index: points.len(),
            q: sp.q,
            i: intensity,
            sigma,
            origin: PointOrigin { kind: "subtraction".into(), source_index: Some(sp.index), source_points: buffer_origin.source_points, note },
            flags,
        });
    }
    let negative_count = points.iter().filter(|p| p.i < 0.0).count();
    if negative_count > 0 {
        diagnostics.push(Diagnostic { code: "NEGATIVE_INTENSITY_RETAINED".into(), severity: "warning".into(), message: format!("{negative_count} subtracted points are negative and remain visible for review."), object_id: None, index: None });
    }
    SubtractionResult {
        sample_id: sample_id.into(), buffer_id: buffer_id.into(),
        sample_q_min: sample_min, sample_q_max: sample_max, buffer_q_min: buffer_min, buffer_q_max: buffer_max,
        overlap_q_min: overlap_min, overlap_q_max: overlap_max, q_grid_mode: q_grid_mode.into(),
        scale_mode: scale_mode.into(), alpha, sigma_alpha, points, diagnostics,
        valid: alpha.is_finite() && alpha >= 0.0,
    }
}

fn overlap_exists(sample: &[CurvePoint], buffer: &[CurvePoint]) -> bool {
    let smin = sample.first().unwrap().q;
    let smax = sample.last().unwrap().q;
    let bmin = buffer.first().unwrap().q;
    let bmax = buffer.last().unwrap().q;
    let lo = smin.max(bmin);
    let hi = smax.min(bmax);
    lo <= hi && sample.iter().any(|p| p.q >= lo && p.q <= hi && interpolate_buffer_at(buffer, p.q).is_some())
}

fn grids_match(sample: &[CurvePoint], buffer: &[CurvePoint]) -> bool {
    sample.len() == buffer.len()
        && sample.iter().zip(buffer).all(|(a, b)| (a.q - b.q).abs() <= 1e-10)
}

fn interpolate_buffer_at(buffer: &[CurvePoint], q: f64) -> Option<(f64, f64, PointOrigin, Option<String>)> {
    if buffer.is_empty() || q < buffer.first()?.q || q > buffer.last()?.q {
        return None;
    }
    match buffer.binary_search_by(|p| p.q.partial_cmp(&q).unwrap()) {
        Ok(index) => Some((buffer[index].i, buffer[index].sigma, PointOrigin { kind: "buffer-point".into(), source_index: Some(index), source_points: vec![index], note: None }, None)),
        Err(next) => {
            let right = next;
            let left = next.checked_sub(1)?;
            let a = buffer.get(left)?;
            let b = buffer.get(right)?;
            let fraction = (q - a.q) / (b.q - a.q);
            Some((
                a.i + fraction * (b.i - a.i),
                a.sigma.hypot(b.sigma) * fraction.hypot(1.0 - fraction),
                PointOrigin { kind: "linear-interpolation".into(), source_index: None, source_points: vec![left, right], note: Some("linear interpolation inside overlap".into()) },
                Some("buffer interpolated at sample q".into()),
            ))
        }
    }
}

fn invalid_subtraction(sample_id: &str, buffer_id: &str, sample: &NormalizedCurve, buffer: &NormalizedCurve, mode: &str, fixed_alpha: Option<f64>, diagnostics: Vec<Diagnostic>) -> SubtractionResult {
    let smin = sample.points.first().map(|p| p.q).unwrap_or(f64::NAN);
    let smax = sample.points.last().map(|p| p.q).unwrap_or(f64::NAN);
    let bmin = buffer.points.first().map(|p| p.q).unwrap_or(f64::NAN);
    let bmax = buffer.points.last().map(|p| p.q).unwrap_or(f64::NAN);
    SubtractionResult {
        sample_id: sample_id.into(), buffer_id: buffer_id.into(), sample_q_min: smin, sample_q_max: smax,
        buffer_q_min: bmin, buffer_q_max: bmax, overlap_q_min: smin.max(bmin), overlap_q_max: smax.min(bmax),
        q_grid_mode: mode.into(), scale_mode: fixed_alpha.map(|_| "fixed".into()).unwrap_or_else(|| "not-evaluated".into()),
        alpha: f64::NAN, sigma_alpha: f64::NAN, points: vec![], diagnostics, valid: false,
    }
}

pub fn fit_intervals(points: &[CurvePoint], intervals: &[(f64, f64)]) -> Vec<FitCandidate> {
    intervals.iter().filter_map(|&(qmin, qmax)| {
        let selected: Vec<&CurvePoint> = points.iter().filter(|p| p.q >= qmin && p.q <= qmax && p.sigma > 0.0).collect();
        if selected.len() < 3 {
            return Some(FitCandidate {
                model_family: "insufficient-points".into(), q_min: qmin, q_max: qmax,
                parameters: BTreeMap::new(), parameter_units: BTreeMap::new(), chi_square: f64::NAN,
                residual_rms: f64::NAN, point_count: selected.len(), evidence: f64::NAN, residuals: vec![],
                diagnostics: vec![Diagnostic { code: "INSUFFICIENT_FIT_POINTS".into(), severity: "warning".into(), message: "Interval contains fewer than three positive-error points.".into(), object_id: None, index: None }],
            });
        }
        let positive_i: Vec<&CurvePoint> = selected.iter().copied().filter(|p| p.i > 0.0).collect();
        if positive_i.len() < 3 {
            return Some(FitCandidate {
                model_family: "guinier-invalid".into(), q_min: qmin, q_max: qmax,
                parameters: BTreeMap::new(), parameter_units: BTreeMap::new(), chi_square: f64::NAN,
                residual_rms: f64::NAN, point_count: selected.len(), evidence: f64::NAN, residuals: vec![],
                diagnostics: vec![Diagnostic { code: "NON_POSITIVE_GUINIER_INPUT".into(), severity: "warning".into(), message: "Guinier logarithm requires positive intensity; candidate remains unresolved.".into(), object_id: None, index: None }],
            });
        }
        let xs: Vec<f64> = positive_i.iter().map(|p| p.q * p.q).collect();
        let ys: Vec<f64> = positive_i.iter().map(|p| p.i.ln()).collect();
        let ws: Vec<f64> = positive_i.iter().map(|p| 1.0 / ((p.sigma / p.i).powi(2))).collect();
        let (intercept, slope, residual_diagnostics) = weighted_linear_fit(&xs, &ys, &ws);
        let rg2 = if slope.is_finite() && slope < 0.0 { -3.0 * slope } else { f64::NAN };
        let rg = rg2.max(0.0).sqrt();
        let i0 = intercept.exp();
        let mut parameters = BTreeMap::new();
        parameters.insert("I0".into(), i0);
        parameters.insert("Rg".into(), rg);
        let mut parameter_units = BTreeMap::new();
        parameter_units.insert("I0".into(), "same as normalized I(q)".into());
        parameter_units.insert("Rg".into(), "same coordinate unit as q^-1".into());
        let (chi, rms, residuals) = residual_series(&positive_i, |q| intercept + slope * q * q, |q| i0 * (-q * q * rg2 / 3.0).exp());
        let qmax = positive_i.last().unwrap().q;
        let qrg = qmax * rg;
        let mut diagnostics = residual_diagnostics;
        if !rg.is_finite() || !slope.is_finite() || slope >= 0.0 {
            diagnostics.push(Diagnostic { code: "INVALID_GUINIER_SLOPE".into(), severity: "warning".into(), message: "Estimated slope is not negative; candidate is not accepted automatically.".into(), object_id: None, index: None });
        }
        if qrg > 1.3 {
            diagnostics.push(Diagnostic { code: "GUINIER_QRG_LIMIT".into(), severity: "warning".into(), message: "q_max Rg exceeds the conventional 1.3 review limit.".into(), object_id: None, index: None });
        }
        Some(FitCandidate {
            model_family: "Guinier".into(), q_min: qmin, q_max: qmax, parameters, parameter_units,
            chi_square: chi, residual_rms: rms, point_count: positive_i.len(), evidence: -chi / 2.0,
            residuals, diagnostics,
        })
    }).collect()
}

pub fn fit_porod_intervals(points: &[CurvePoint], intervals: &[(f64, f64)]) -> Vec<FitCandidate> {
    intervals.iter().filter_map(|&(qmin, qmax)| {
        let selected: Vec<&CurvePoint> = points.iter().filter(|p| p.q >= qmin && p.q <= qmax && p.i > 0.0 && p.sigma > 0.0).collect();
        if selected.len() < 3 {
            return Some(FitCandidate {
                model_family: "Porod".into(), q_min: qmin, q_max: qmax, parameters: BTreeMap::new(),
                parameter_units: BTreeMap::new(), chi_square: f64::NAN, residual_rms: f64::NAN,
                point_count: selected.len(), evidence: f64::NAN, residuals: vec![],
                diagnostics: vec![Diagnostic { code: "INSUFFICIENT_FIT_POINTS".into(), severity: "warning".into(), message: "Porod interval contains fewer than three positive points.".into(), object_id: None, index: None }],
            });
        }
        let xs: Vec<f64> = selected.iter().map(|p| p.q.ln()).collect();
        let ys: Vec<f64> = selected.iter().map(|p| (p.i * p.q.powi(4)).ln()).collect();
        let ws: Vec<f64> = selected.iter().map(|p| 1.0 / (p.sigma / p.i).powi(2)).collect();
        let (intercept, slope, fit_diags) = weighted_linear_fit(&xs, &ys, &ws);
        let mut parameters = BTreeMap::new();
        parameters.insert("ln(K)".into(), intercept);
        parameters.insert("Porod exponent deviation".into(), slope);
        let mut parameter_units = BTreeMap::new();
        parameter_units.insert("ln(K)".into(), "dimensionless log".into());
        parameter_units.insert("Porod exponent deviation".into(), "dimensionless".into());
        let (chi, rms, residuals) = residual_series(&selected, |q| intercept + slope * q.ln(), |q| (intercept + slope * q.ln()).exp() / q.powi(4));
        let mut diagnostics = fit_diags;
        diagnostics.push(Diagnostic { code: "SEPARATE_MODEL_FAMILY".into(), severity: "information".into(), message: "Porod evidence is stored independently and is not merged with a Guinier model.".into(), object_id: None, index: None });
        Some(FitCandidate { model_family: "Porod".into(), q_min: qmin, q_max: qmax, parameters, parameter_units, chi_square: chi, residual_rms: rms, point_count: selected.len(), evidence: -chi / 2.0, residuals, diagnostics })
    }).collect()
}

fn weighted_linear_fit(xs: &[f64], ys: &[f64], ws: &[f64]) -> (f64, f64, Vec<Diagnostic>) {
    let sw: f64 = ws.iter().sum();
    let sx: f64 = xs.iter().zip(ws).map(|(x, w)| x * w).sum();
    let sy: f64 = ys.iter().zip(ws).map(|(y, w)| y * w).sum();
    let sxx: f64 = xs.iter().zip(ws).map(|(x, w)| x * x * w).sum();
    let sxy: f64 = xs.iter().zip(ys).zip(ws).map(|((x, y), w)| x * y * w).sum();
    let denom = sw * sxx - sx * sx;
    if denom == 0.0 || !denom.is_finite() {
        return (f64::NAN, f64::NAN, vec![Diagnostic { code: "SINGULAR_FIT".into(), severity: "warning".into(), message: "Fit matrix is singular.".into(), object_id: None, index: None }]);
    }
    let intercept = (sy * sxx - sx * sxy) / denom;
    let slope = (sw * sxy - sx * sy) / denom;
    (intercept, slope, vec![])
}

fn residual_series(points: &[&CurvePoint], log_pred: impl Fn(f64) -> f64, intensity_pred: impl Fn(f64) -> f64) -> (f64, f64, Vec<ResidualPoint>) {
    let mut residuals = Vec::new();
    let mut chi = 0.0;
    let mut sq = 0.0;
    for p in points {
        let log_ok = log_pred(p.q).is_finite();
        let expected = intensity_pred(p.q);
        let residual = if log_ok && expected.is_finite() { (p.i - expected) / p.sigma } else { f64::NAN };
        if residual.is_finite() {
            chi += residual.powi(2);
            sq += residual.powi(2);
        }
        residuals.push(ResidualPoint { index: p.index, q: p.q, observed: p.i, expected, normalized_residual: residual });
    }
    let n = residuals.len().max(1) as f64;
    (chi, (sq / n).sqrt(), residuals)
}
