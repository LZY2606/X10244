use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type ArtifactId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Artifact {
    RawCurve(RawCurve),
    InstrumentResponse(InstrumentResponse),
    Mask(Mask),
    Normalization(Normalization),
    BackgroundSubtraction(BackgroundSubtraction),
    ModelAnalysis(ModelAnalysis),
    Decision(DecisionRecord),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactEnvelope {
    pub id: ArtifactId,
    pub version: u32,
    pub event_id: String,
    pub created_at: u64,
    pub deprecated_by_event: Option<String>,
    pub inputs: BTreeMap<String, String>,
    pub artifact: Artifact,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScalarRecord {
    pub value: f64,
    pub raw_text: String,
    pub unit: String,
    pub conversion_note: Option<String>,
}

impl ScalarRecord {
    pub fn new(value: f64, raw_text: impl Into<String>, unit: impl Into<String>) -> Self {
        Self { value, raw_text: raw_text.into(), unit: unit.into(), conversion_note: None }
    }

    pub fn converted(mut self, raw_unit: &str, canonical_unit: &str) -> Self {
        self.conversion_note = Some(format!("imported as {}; normalized to {}", raw_unit, canonical_unit));
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub id: String,
    pub q: ScalarRecord,
    pub intensity: ScalarRecord,
    pub sigma: ScalarRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawCurve {
    pub label: String,
    pub role: String,
    pub exposure: ScalarRecord,
    pub thickness: ScalarRecord,
    pub transmission: ScalarRecord,
    pub response_id: String,
    pub mask_id: String,
    pub source_file: String,
    pub q_frame: String,
    pub points: Vec<RawPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstrumentResponse {
    pub label: String,
    pub relative_scale: f64,
    pub relative_scale_sigma: f64,
    pub source_file: String,
    pub rule_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Mask {
    pub label: String,
    pub source_file: String,
    pub masked_ranges_q: Vec<(f64, f64)>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedPoint {
    pub id: String,
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub source_point_id: String,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Normalization {
    pub label: String,
    pub q_unit: String,
    pub intensity_unit: String,
    pub scale_factor: f64,
    pub scale_sigma: f64,
    pub points: Vec<NormalizedPoint>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtractionPoint {
    pub id: String,
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
    pub sample_point_id: String,
    pub background_point_id: String,
    pub background_was_interpolated: bool,
    pub background_scale: f64,
    pub background_scale_sigma: f64,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlapRange {
    pub q_min: f64,
    pub q_max: f64,
    pub sample_points: usize,
    pub interpolated_background_points: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackgroundSubtraction {
    pub label: String,
    pub q_unit: String,
    pub intensity_unit: String,
    pub status: SubtractionStatus,
    pub background_scale: f64,
    pub background_scale_sigma: f64,
    pub overlap: Option<OverlapRange>,
    pub points: Vec<SubtractionPoint>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SubtractionStatus {
    Complete,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelFit {
    pub model: String,
    pub q_min: f64,
    pub q_max: f64,
    pub point_count: usize,
    pub valid_point_count: usize,
    pub intercept: Option<f64>,
    pub slope: Option<f64>,
    pub i0: Option<f64>,
    pub rg: Option<f64>,
    pub residual_rms: Option<f64>,
    pub chi_square: Option<f64>,
    pub status: String,
    pub reason: String,
    pub point_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelAnalysis {
    pub label: String,
    pub q_unit: String,
    pub intensity_unit: String,
    pub fits: Vec<ModelFit>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionRecord {
    pub decision: String,
    pub target_id: String,
    pub summary: String,
    pub branch_of: Option<String>,
    pub supersedes_candidate_ids: Vec<String>,
    pub retained_candidate_ids: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub q_min: Option<f64>,
    pub q_max: Option<f64>,
}

pub fn normalize_curve(raw: &RawCurve, response: &InstrumentResponse, id_prefix: &str, event_id: &str) -> Normalization {
    let exposure = raw.exposure.value;
    let transmission = raw.transmission.value;
    let factor = response.relative_scale / (exposure * transmission);
    let calibration_rel = response.relative_scale_sigma / response.relative_scale;
    let scale_sigma = factor * calibration_rel;
    let mut points = Vec::new();
    let mut diagnostics = Vec::new();
    if !exposure.is_finite() || exposure <= 0.0 {
        diagnostics.push(bad_value("invalid_exposure", "曝光必须为正数"));
    }
    if !transmission.is_finite() || transmission <= 0.0 || transmission > 1.0 {
        diagnostics.push(bad_value("invalid_transmission", "透射率必须位于 (0, 1]"));
    }
    for point in &raw.points {
        let intensity = point.intensity.value * factor;
        let counting_term = if point.sigma.value > 0.0 {
            point.sigma.value * factor
        } else {
            diagnostics.push(bad_value("nonpositive_sigma", "误差必须为正数"));
            0.0
        };
        let normalization_term = intensity * calibration_rel;
        let sigma = (counting_term.powi(2) + normalization_term.powi(2)).sqrt();
        let mut flags = Vec::new();
        if intensity < 0.0 || point.intensity.value < 0.0 {
            flags.push("negative_intensity".to_string());
            diagnostics.push(q_diagnostic("negative_intensity", "warn", "保留负强度点，不参与 Guinier 线性拟合", point.q.value, point.q.value));
        }
        points.push(NormalizedPoint {
            id: format!("{}.n{}", id_prefix, points.len()),
            q: point.q.value,
            intensity,
            sigma,
            source_point_id: point.id.clone(),
            flags,
        });
    }
    Normalization {
        label: format!("normalized {}", raw.label),
        q_unit: raw.q_frame.clone(),
        intensity_unit: canonical_intensity_unit(&raw.intensity.unit),
        scale_factor: factor,
        scale_sigma,
        points,
        diagnostics,
    }
}

fn bad_value(code: &str, message: &str) -> Diagnostic {
    Diagnostic { code: code.to_string(), severity: "error".to_string(), message: message.to_string(), q_min: None, q_max: None }
}

fn q_diagnostic(code: &str, severity: &str, message: &str, q_min: f64, q_max: f64) -> Diagnostic {
    Diagnostic { code: code.to_string(), severity: severity.to_string(), message: message.to_string(), q_min: Some(q_min), q_max: Some(q_max) }
}

fn canonical_intensity_unit(raw_unit: &str) -> String {
    if raw_unit.contains("1/cm/sr") || raw_unit.contains("cm^-1 sr^-1") {
        "cm^-1 sr^-1".to_string()
    } else {
        raw_unit.to_string()
    }
}

pub fn subtract_background(
    sample: &Normalization,
    background: &Normalization,
    id_prefix: &str,
    background_scale: f64,
    background_scale_sigma: f64,
    transmission_ratio: f64,
) -> BackgroundSubtraction {
    let sample_grid: Vec<f64> = sample.points.iter().map(|p| p.q).collect();
    let bg_grid: Vec<f64> = background.points.iter().map(|p| p.q).collect();
    let min_q = sample_grid.first().copied().unwrap_or(f64::NAN).max(bg_grid.first().copied().unwrap_or(f64::NAN));
    let max_q = sample_grid.last().copied().unwrap_or(f64::NAN).min(bg_grid.last().copied().unwrap_or(f64::NAN));
    let same_grid = grids_match(&sample_grid, &bg_grid);
    let mut diagnostics = Vec::new();
    if !same_grid {
        diagnostics.push(Diagnostic {
            code: "q_grid_mismatch".to_string(),
            severity: "info".to_string(),
            message: "q 网格不同；不直接相减，仅在重叠范围内插值背景".to_string(),
            q_min: Some(min_q),
            q_max: Some(max_q),
        });
    }
    if !min_q.is_finite() || !max_q.is_finite() || min_q > max_q {
        return BackgroundSubtraction {
            label: format!("{} - {}", sample.label, background.label),
            q_unit: sample.q_unit.clone(),
            intensity_unit: sample.intensity_unit.clone(),
            status: SubtractionStatus::Blocked,
            background_scale,
            background_scale_sigma,
            overlap: None,
            points: Vec::new(),
            diagnostics: vec![Diagnostic {
                code: "no_overlap_q_range".to_string(),
                severity: "error".to_string(),
                message: "两条曲线 q 范围无重叠；禁止外推或直接相减".to_string(),
                q_min: None,
                q_max: None,
            }],
        };
    }

    let mut points = Vec::new();
    let mut interpolated = 0usize;
    for sp in sample.points.iter().filter(|p| p.q >= min_q - 1e-12 && p.q <= max_q + 1e-12) {
        let found = background.points.iter().find(|bp| (bp.q - sp.q).abs() < 1e-10);
        let (bg, bp_id, was_interp) = if let Some(bp) = found {
            (bp.intensity, bp.id.clone(), false)
        } else {
            let bp = interpolate_point(background, sp.q).expect("overlap guarantees interpolation");
            (bp.intensity, bp.source_point_id.clone(), true)
        };
        if was_interp { interpolated += 1; }
        let scaled_bg = bg * background_scale * transmission_ratio;
        let value = sp.intensity - scaled_bg;
        let bg_counting_term = if let Some(bp) = background.points.iter().find(|bp| (bp.q - sp.q).abs() < 1e-10) {
            bp.sigma * background_scale * transmission_ratio
        } else {
            interpolated_sigma(background, sp.q) * background_scale * transmission_ratio
        };
        let background_scale_term = scaled_bg * background_scale_sigma / background_scale.max(1e-300);
        let sigma = (sp.sigma.powi(2) + bg_counting_term.powi(2) + background_scale_term.powi(2)).sqrt();
        let mut flags = Vec::new();
        if value < 0.0 {
            flags.push("negative_subtracted_intensity".to_string());
            diagnostics.push(q_diagnostic("negative_intensity", "warn", "扣背景后强度为负；保留该点并标记", sp.q, sp.q));
        }
        points.push(SubtractionPoint {
            id: format!("{}.s{}", id_prefix, points.len()),
            q: sp.q,
            intensity: value,
            sigma,
            sample_point_id: sp.id.clone(),
            background_point_id: bp_id,
            background_was_interpolated: was_interp,
            background_scale,
            background_scale_sigma,
            flags,
        });
    }
    BackgroundSubtraction {
        label: format!("{} - {}", sample.label, background.label),
        q_unit: sample.q_unit.clone(),
        intensity_unit: sample.intensity_unit.clone(),
        status: SubtractionStatus::Complete,
        background_scale,
        background_scale_sigma,
        overlap: Some(OverlapRange { q_min: min_q, q_max: max_q, sample_points: points.len(), interpolated_background_points: interpolated }),
        points,
        diagnostics,
    }
}

pub fn grids_match(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-10)
}

fn interpolate_point(curve: &Normalization, q: f64) -> Option<&NormalizedPoint> {
    curve.points.iter().find(|p| (p.q - q).abs() < 1e-10).or_else(|| {
        curve.points.windows(2).find(|w| q >= w[0].q && q <= w[1].q).map(|w| &w[0])
    })
}

fn interpolated_sigma(curve: &Normalization, q: f64) -> f64 {
    for w in curve.points.windows(2) {
        if q >= w[0].q && q <= w[1].q {
            let t = (q - w[0].q) / (w[1].q - w[0].q);
            return (1.0 - t) * w[0].sigma + t * w[1].sigma;
        }
    }
    f64::NAN
}

pub fn analyze_models(curve: &BackgroundSubtraction, id_prefix: &str) -> ModelAnalysis {
    let mut fits = Vec::new();
    let mut diagnostics = Vec::new();
    let points: Vec<&SubtractionPoint> = curve.points.iter().collect();
    let windows = vec![(0.07, 0.25), (0.07, 0.35), (0.07, 0.50), (0.15, 0.50)];
    for (index, (q_min, q_max)) in windows.iter().enumerate() {
        let in_window: Vec<&SubtractionPoint> = points.iter().filter(|p| p.q >= *q_min && p.q <= *q_max).cloned().collect();
        let valid: Vec<&SubtractionPoint> = in_window.iter().filter(|p| p.intensity > 0.0 && p.sigma > 0.0).cloned().collect();
        let mut fit = ModelFit {
            model: "guinier".to_string(),
            q_min: *q_min,
            q_max: *q_max,
            point_count: in_window.len(),
            valid_point_count: valid.len(),
            intercept: None,
            slope: None,
            i0: None,
            rg: None,
            residual_rms: None,
            chi_square: None,
            status: "rejected".to_string(),
            reason: if in_window.len() < 3 { "区间点数不足".to_string() } else { "正强度且正误差的有效点不足三个".to_string() },
            point_ids: in_window.iter().map(|p| p.id.clone()).collect(),
        };
        if valid.len() >= 3 {
            let xs: Vec<f64> = valid.iter().map(|p| p.q * p.q).collect();
            let ys: Vec<f64> = valid.iter().map(|p| p.intensity.ln()).collect();
            let ws: Vec<f64> = valid.iter().map(|p| 1.0 / (p.sigma / p.intensity).powi(2)).collect();
            if let Some(LineFit { slope, intercept, rms, chi }) = weighted_line(&xs, &ys, &ws) {
                let valid_shape = slope < 0.0 && q_max * q_max * slope.abs() < 1.3;
                fit.slope = Some(slope);
                fit.intercept = Some(intercept);
                fit.i0 = Some(intercept.exp());
                fit.rg = Some((-3.0 * slope).sqrt());
                fit.residual_rms = Some(rms);
                fit.chi_square = Some(chi);
                fit.status = if valid_shape { "candidate".to_string() } else { "rejected".to_string() };
                fit.reason = if valid_shape { "线性 Guinier 候选；不自动扩展或拼接 Porod".to_string() } else { "斜率不符合 Guinier 形状或 qRg 过大".to_string() };
            }
        }
        fits.push(fit);
        let _ = index;
    }

    let porod_windows = vec![(0.40, 0.70), (0.50, 0.85), (0.60, 1.00)];
    for (q_min, q_max) in porod_windows {
        let in_window: Vec<&SubtractionPoint> = points.iter().filter(|p| p.q >= q_min && p.q <= q_max).cloned().collect();
        let valid: Vec<&SubtractionPoint> = in_window.iter().filter(|p| p.intensity > 0.0 && p.sigma > 0.0).cloned().collect();
        let mut fit = ModelFit {
            model: "porod".to_string(),
            q_min,
            q_max,
            point_count: in_window.len(),
            valid_point_count: valid.len(),
            intercept: None,
            slope: None,
            i0: None,
            rg: None,
            residual_rms: None,
            chi_square: None,
            status: "rejected".to_string(),
            reason: if in_window.len() < 3 { "区间点数不足".to_string() } else { "有效点不足三个".to_string() },
            point_ids: in_window.iter().map(|p| p.id.clone()).collect(),
        };
        if valid.len() >= 3 {
            let xs: Vec<f64> = valid.iter().map(|p| p.q.powi(4)).collect();
            let ys: Vec<f64> = valid.iter().map(|p| (p.intensity * p.q.powi(4)).ln()).collect();
            let ws: Vec<f64> = valid.iter().map(|p| 1.0 / ((p.q.powi(4) * p.sigma) / (p.intensity * p.q.powi(4)).max(1e-300)).powi(2)).collect();
            if let Some(LineFit { slope, intercept, rms, chi }) = weighted_line(&xs, &ys, &ws) {
                fit.slope = Some(slope);
                fit.intercept = Some(intercept);
                fit.i0 = Some(intercept.exp());
                fit.residual_rms = Some(rms);
                fit.chi_square = Some(chi);
                fit.status = if slope < 0.0 { "candidate".to_string() } else { "rejected".to_string" };
                fit.reason = if slope < 0.0 { "Porod 变换候选；独立于 Guinier 保留".to_string() } else { "Porod 变换斜率非负".to_string() };
            }
        }
        fits.push(fit);
    }

    let candidates: Vec<&ModelFit> = fits.iter().filter(|f| f.status == "candidate").collect();
    for i in 0..candidates.len() {
        for j in (i + 1)..candidates.len() {
            if candidates[i].model == candidates[j].model && candidates[i].q_min < candidates[j].q_max && candidates[j].q_min < candidates[i].q_max {
                diagnostics.push(Diagnostic {
                    code: "interval_overlap".to_string(),
                    severity: "info".to_string(),
                    message: format!("{} 候选区间重叠；保留多候选，不合并成单一模型", candidates[i].model),
                    q_min: Some(candidates[i].q_min.max(candidates[j].q_min)),
                    q_max: Some(candidates[i].q_max.min(candidates[j].q_max)),
                });
            }
        }
    }
    if diagnostics.is_empty() {
        diagnostics.push(Diagnostic { code: "multi_candidate_evidence".to_string(), severity: "info".to_string(), message: "所有合法模型候选均保留；Guinier 与 Porod 不自动拼接".to_string(), q_min: None, q_max: None });
    }
    let _ = id_prefix;
    ModelAnalysis { label: format!("model evidence for {}", curve.label), q_unit: curve.q_unit.clone(), intensity_unit: curve.intensity_unit.clone(), fits, diagnostics }
}

struct LineFit { slope: f64, intercept: f64, rms: f64, chi: f64 }

fn weighted_line(xs: &[f64], ys: &[f64], ws: &[f64]) -> Option<LineFit> {
    let n = xs.len() as f64;
    let sw: f64 = ws.iter().sum();
    let sx: f64 = xs.iter().zip(ws).map(|(x, w)| x * w).sum();
    let sy: f64 = ys.iter().zip(ws).map(|(y, w)| y * w).sum();
    let sxx: f64 = xs.iter().zip(ws).map(|(x, w)| x * x * w).sum();
    let sxy: f64 = xs.iter().zip(ys).zip(ws).map(|((x, y), w)| x * y * w).sum();
    let denom = sw * sxx - sx * sx;
    if denom.abs() < 1e-18 { return None; }
    let slope = (sw * sxy - sx * sy) / denom;
    let intercept = (sy - slope * sx) / sw;
    let mut se = 0.0;
    let mut chi = 0.0;
    for ((x, y), w) in xs.iter().zip(ys).zip(ws) {
        let residual = y - (intercept + slope * x);
        se += residual.powi(2);
        chi += w * residual.powi(2);
    }
    Some(LineFit { slope, intercept, rms: (se / n).sqrt(), chi })
}

pub fn impacted_targets(changed: &str, edges: &[(String, String)]) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    let mut queue = vec![changed.to_string()];
    while let Some(node) = queue.pop() {
        for (from, to) in edges {
            if *from == node && result.insert(to.clone()) {
                queue.push(to.clone());
            }
        }
    }
    result
}
