use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ValueWithUnit {
    pub raw: String,
    pub value: Option<f64>,
    pub unit: String,
    pub sigma: Option<f64>,
    pub sigma_raw: Option<String>,
    pub coordinate_basis: Option<String>,
}

impl ValueWithUnit {
    pub fn new(raw: &str, unit: &str) -> Self {
        let parsed: Option<f64> = raw.trim().parse::<f64>().ok().filter(|v| v.is_finite());
        Self {
            raw: raw.to_string(),
            value: parsed,
            unit: unit.to_string(),
            sigma: None,
            sigma_raw: None,
            coordinate_basis: None,
        }
    }

    pub fn with_basis(mut self, basis: &str) -> Self {
        self.coordinate_basis = Some(basis.to_string());
        self
    }

    pub fn finite(&self) -> Option<f64> {
        self.value.filter(|v| v.is_finite())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RawPoint {
    pub point_id: String,
    pub q: ValueWithUnit,
    pub intensity: ValueWithUnit,
    pub sigma: ValueWithUnit,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InstrumentResponse {
    pub response_id: String,
    pub scale: ValueWithUnit,
    pub mask_source: String,
    pub raw_record: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Dataset {
    pub dataset_id: String,
    pub role: String,
    pub label: String,
    pub source_file: String,
    pub points: Vec<RawPoint>,
    pub exposure: ValueWithUnit,
    pub thickness: ValueWithUnit,
    pub transmission: ValueWithUnit,
    pub instrument_response: Option<InstrumentResponse>,
    pub mask_source: String,
    pub buffer_candidate_ids: Vec<String>,
    pub selected_buffer_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub related: Vec<String>,
}

impl Diagnostic {
    pub fn new(code: &str, severity: &str, message: &str, related: Vec<&str>) -> Self {
        Self {
            code: code.to_string(),
            severity: severity.to_string(),
            message: message.to_string(),
            related: related.into_iter().map(str::to_string).collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    ProjectCreated {
        project_id: String,
        name: String,
        description: String,
    },
    DatasetImported {
        dataset: Dataset,
    },
    ParameterRevision {
        dataset_id: String,
        exposure: Option<ValueWithUnit>,
        thickness: Option<ValueWithUnit>,
        transmission: Option<ValueWithUnit>,
        instrument_response: Option<InstrumentResponse>,
        reason: String,
    },
    PointReplaced {
        dataset_id: String,
        point_id: String,
        point: RawPoint,
        reason: String,
    },
    BufferSelectionChanged {
        sample_id: String,
        action: String,
        buffer_id: String,
        reason: String,
    },
    CandidateDecision {
        candidate_id: String,
        decision: String,
        actor: String,
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub event_id: String,
    pub parent_event_id: Option<String>,
    pub branch: String,
    pub actor: String,
    pub created_at: String,
    pub kind: EventKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NormalizedPoint {
    pub point_id: String,
    pub source_point_id: String,
    pub q: f64,
    pub q_unit: String,
    pub intensity: Option<f64>,
    pub sigma: Option<f64>,
    pub intensity_unit: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NormalizedCurve {
    pub artifact_id: String,
    pub version: String,
    pub dataset_id: String,
    pub label: String,
    pub role: String,
    pub points: Vec<NormalizedPoint>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BackgroundPoint {
    pub point_id: String,
    pub sample_point_id: String,
    pub buffer_point_id: String,
    pub buffer_source_point_ids: Vec<String>,
    pub q: f64,
    pub sample_intensity: f64,
    pub buffer_intensity_raw: f64,
    pub buffer_intensity_on_sample_grid: f64,
    pub alpha: f64,
    pub value: f64,
    pub sigma: f64,
    pub residual: f64,
    pub residual_sigma: f64,
    pub normalized_variance: f64,
    pub buffer_normalized_variance: f64,
    pub background_scale_variance: f64,
    pub interpolated: bool,
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BackgroundCandidate {
    pub artifact_id: String,
    pub version: String,
    pub candidate_id: String,
    pub sample_id: String,
    pub buffer_id: String,
    pub label: String,
    pub alpha_mode: String,
    pub alpha: f64,
    pub alpha_sigma: f64,
    pub grid_relation: String,
    pub q_min: Option<f64>,
    pub q_max: Option<f64>,
    pub chi_square: Option<f64>,
    pub rss: Option<f64>,
    pub status: String,
    pub points: Vec<BackgroundPoint>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IntervalCandidate {
    pub artifact_id: String,
    pub version: String,
    pub interval_id: String,
    pub background_candidate_id: String,
    pub kind: String,
    pub q_min: f64,
    pub q_max: f64,
    pub point_ids: Vec<String>,
    pub slope: f64,
    pub intercept: f64,
    pub r_squared: f64,
    pub rss: f64,
    pub status: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GraphNode {
    pub node_id: String,
    pub kind: String,
    pub label: String,
    pub status: String,
    pub artifact_id: String,
    pub artifact_version: String,
    pub inputs: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PropagationEvent {
    pub event_id: String,
    pub changed_nodes: Vec<String>,
    pub reused_nodes: Vec<String>,
    pub invalidated_nodes: Vec<String>,
    pub rationale: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProjectState {
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub datasets: BTreeMap<String, Dataset>,
    pub normalized_curves: Vec<NormalizedCurve>,
    pub background_candidates: Vec<BackgroundCandidate>,
    pub interval_candidates: Vec<IntervalCandidate>,
    pub graph: Vec<GraphNode>,
    pub judgments: Vec<Judgment>,
    pub propagation: Vec<PropagationEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Judgment {
    pub judgment_id: String,
    pub event_id: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    pub related: Vec<String>,
    pub evidence: Value,
}

pub fn dataset_points(dataset: &Dataset) -> Vec<(&RawPoint, bool)> {
    dataset
        .points
        .iter()
        .map(|point| {
            let valid = point.q.finite().is_some()
                && point.intensity.finite().is_some()
                && point.sigma.finite().is_some_and(|sigma| sigma >= 0.0);
            (point, valid)
        })
        .collect()
}

pub fn normalize_dataset(dataset: &Dataset) -> NormalizedCurve {
    let response_scale = dataset
        .instrument_response
        .as_ref()
        .and_then(|response| response.scale.finite())
        .unwrap_or(1.0);
    let exposure = dataset.exposure.finite();
    let thickness = dataset.thickness.finite();
    let transmission = dataset.transmission.finite();
    let divisor = match (exposure, thickness, transmission) {
        (Some(exposure), Some(thickness), Some(transmission))
            if exposure > 0.0 && thickness > 0.0 && transmission > 0.0 =>
        {
            exposure * thickness * transmission * response_scale
        }
        _ => 0.0,
    };

    let mut points = Vec::new();
    let mut diagnostics = Vec::new();
    for point in &dataset.points {
        let mut point_diagnostics = point.diagnostics.clone();
        let q = point.q.finite();
        let raw_i = point.intensity.finite();
        let raw_sigma = point.sigma.finite();
        let valid = q.is_some()
            && raw_i.is_some()
            && raw_sigma.is_some_and(|sigma| sigma >= 0.0)
            && divisor > 0.0;
        if point.q.value.is_some() != point.q.raw.trim().parse::<f64>().is_ok() {
            point_diagnostics.push(Diagnostic::new(
                "UNCONVERTIBLE_VALUE_RETAINED",
                "warning",
                "原始 q 保留原位置，未参与规范化。",
                vec![&point.point_id],
            ));
        }
        if point.intensity.value.is_none() {
            point_diagnostics.push(Diagnostic::new(
                "UNCONVERTIBLE_VALUE_RETAINED",
                "warning",
                "原始强度无法转换，保留原始表示且不丢弃。",
                vec![&point.point_id],
            ));
        }
        if point.sigma.value.is_none() {
            point_diagnostics.push(Diagnostic::new(
                "UNCONVERTIBLE_VALUE_RETAINED",
                "warning",
                "原始误差无法转换，保留原始表示且不参与拟合。",
                vec![&point.point_id],
            ));
        }
        if divisor <= 0.0 {
            point_diagnostics.push(Diagnostic::new(
                "NORMALIZATION_FACTOR_INVALID",
                "error",
                "曝光、厚度、透射率或仪器响应不能形成正的归一化因子。",
                vec![&dataset.dataset_id, &point.point_id],
            ));
        }

        let (intensity, sigma) = if valid {
            let _q = q.unwrap();
            let raw_i = raw_i.unwrap();
            let raw_sigma = raw_sigma.unwrap();
            let normalized = raw_i / divisor;
            let mut relative_variance = (raw_sigma / raw_i).powi(2);
            for (value, sigma, _code) in [
                (exposure, dataset.exposure.sigma, "EXPOSURE_UNCERTAINTY"),
                (thickness, dataset.thickness.sigma, "THICKNESS_UNCERTAINTY"),
                (
                    transmission,
                    dataset.transmission.sigma,
                    "TRANSMISSION_UNCERTAINTY",
                ),
                (
                    dataset
                        .instrument_response
                        .as_ref()
                        .and_then(|r| r.scale.finite()),
                    dataset
                        .instrument_response
                        .as_ref()
                        .and_then(|r| r.scale.sigma),
                    "INSTRUMENT_RESPONSE_UNCERTAINTY",
                ),
            ] {
                if let (Some(value), Some(sigma)) = (value, sigma) {
                    if value > 0.0 {
                        relative_variance += (sigma / value).powi(2);
                    }
                }
            }
            let normalized_sigma = normalized.abs() * relative_variance.max(0.0).sqrt();
            (Some(normalized), Some(normalized_sigma))
        } else {
            (None, None)
        };
        points.push(NormalizedPoint {
            point_id: format!("{}.normalized.{}", dataset.dataset_id, point.point_id),
            source_point_id: point.point_id.clone(),
            q: q.unwrap_or(f64::NAN),
            q_unit: point.q.unit.clone(),
            intensity,
            sigma,
            intensity_unit: "cm^-1".to_string(),
            diagnostics: point_diagnostics,
        });
    }

    if divisor <= 0.0 {
        diagnostics.push(Diagnostic::new(
            "NORMALIZATION_FACTOR_INVALID",
            "error",
            "归一化因子无效，派生曲线保留诊断而不是使用零替代。",
            vec![&dataset.dataset_id],
        ));
    }
    let artifact_id = format!("normalized:{}", dataset.dataset_id);
    let version = artifact_version(&artifact_id, &(dataset, &points, response_scale));
    NormalizedCurve {
        artifact_id,
        version,
        dataset_id: dataset.dataset_id.clone(),
        label: dataset.label.clone(),
        role: dataset.role.clone(),
        points,
        diagnostics,
    }
}

pub fn stable_hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = FnvHasher(0xcbf29ce484222325);
    bytes.hash(&mut hasher);
    hasher.finish()
}

pub fn artifact_version<T: Serialize>(artifact_id: &str, value: &T) -> String {
    let serialized = serde_json::to_vec(&(artifact_id, value)).unwrap_or_default();
    format!("v-{:016x}", stable_hash_bytes(&serialized))
}

struct FnvHasher(u64);

impl Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= *byte as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

fn valid_xy<'a>(curve: &'a NormalizedCurve) -> Vec<&'a NormalizedPoint> {
    curve
        .points
        .iter()
        .filter(|point| point.q.is_finite() && point.intensity.is_some_and(f64::is_finite))
        .collect()
}

fn sample_grid_relation(sample: &NormalizedCurve, buffer: &NormalizedCurve) -> String {
    let sample_q: Vec<f64> = valid_xy(sample).iter().map(|point| point.q).collect();
    let buffer_q: Vec<f64> = valid_xy(buffer).iter().map(|point| point.q).collect();
    if sample_q.len() == buffer_q.len()
        && sample_q
            .iter()
            .zip(&buffer_q)
            .all(|(left, right)| (*left - *right).abs() < 1e-12)
    {
        "identical".to_string()
    } else {
        "interpolated_overlap".to_string()
    }
}

struct InterpolatedBufferPoint {
    intensity: f64,
    sigma: f64,
    method: &'static str,
    source_point_ids: Vec<String>,
}

fn interpolate_log_intensity(
    q: f64,
    buffer_points: &[&NormalizedPoint],
) -> Option<InterpolatedBufferPoint> {
    let points: Vec<&NormalizedPoint> = buffer_points
        .iter()
        .filter(|point| point.intensity.is_some_and(f64::is_finite))
        .copied()
        .collect();
    if points.is_empty() || q < points.first()?.q || q > points.last()?.q {
        return None;
    }
    if let Some(exact) = points.iter().find(|point| (point.q - q).abs() < 1e-12) {
        return Some(InterpolatedBufferPoint {
            intensity: exact.intensity?,
            sigma: exact.sigma.unwrap_or(exact.intensity? * 0.05),
            method: "exact",
            source_point_ids: vec![exact.point_id.clone()],
        });
    }
    let position = points.partition_point(|point| point.q < q);
    if position == 0 || position >= points.len() {
        return None;
    }
    let left = points[position - 1];
    let right = points[position];
    let left_i = left.intensity?;
    let right_i = right.intensity?;
    if left_i <= 0.0 || right_i <= 0.0 {
        return None;
    }
    let fraction = (q - left.q) / (right.q - left.q);
    let interpolated = (left_i.ln() * (1.0 - fraction) + right_i.ln() * fraction).exp();
    let left_sigma = left.sigma.unwrap_or(left_i * 0.05);
    let right_sigma = right.sigma.unwrap_or(right_i * 0.05);
    Some(InterpolatedBufferPoint {
        intensity: interpolated,
        sigma: left_sigma * (1.0 - fraction) + right_sigma * fraction,
        method: "log_linear",
        source_point_ids: vec![left.point_id.clone(), right.point_id.clone()],
    })
}

fn weighted_alpha(points: &[(f64, f64, f64, f64)]) -> Option<(f64, f64, f64)> {
    let usable: Vec<(f64, f64, f64, f64)> = points
        .iter()
        .copied()
        .filter(|(sample_i, sample_sigma, buffer_i, buffer_sigma)| {
            sample_i.is_finite() && *buffer_i > 0.0 && *sample_sigma >= 0.0 && *buffer_sigma >= 0.0
        })
        .collect();
    let mut sum_weight = 0.0;
    let mut sum_xy = 0.0;
    let mut sum_y2 = 0.0;
    for (sample_i, sample_sigma, buffer_i, buffer_sigma) in &usable {
        let variance = sample_sigma.powi(2) + buffer_sigma.powi(2);
        let weight = 1.0 / variance.max(1e-12);
        sum_weight += weight;
        sum_xy += weight * buffer_i * sample_i;
        sum_y2 += weight * buffer_i * buffer_i;
    }
    if sum_weight < 2.0 || sum_y2 <= 0.0 {
        return None;
    }
    let alpha = sum_xy / sum_y2;
    let mut rss = 0.0;
    for (sample_i, sample_sigma, buffer_i, buffer_sigma) in &usable {
        let variance = sample_sigma.powi(2) + (alpha * buffer_sigma).powi(2);
        let uncertainty = variance.sqrt().max(1e-12);
        rss += ((sample_i - alpha * buffer_i) / uncertainty).powi(2);
    }
    let alpha_sigma = (1.0 / sum_y2.max(1e-300)).sqrt();
    Some((alpha, alpha_sigma, rss))
}

pub fn background_candidates(
    sample: &NormalizedCurve,
    buffer: &NormalizedCurve,
    buffer_label: &str,
    selected: bool,
    decisions: &[(String, String)],
) -> Vec<BackgroundCandidate> {
    let buffer_points = valid_xy(buffer);
    let mut diagnostics = Vec::new();
    let grid_relation = sample_grid_relation(sample, buffer);
    let buffer_q_min_global = buffer_points.first().map(|point| point.q);
    let buffer_q_max_global = buffer_points.last().map(|point| point.q);
    let sample_points = valid_xy(sample);
    let sample_q_min_global = sample_points.first().map(|point| point.q);
    let sample_q_max_global = sample_points.last().map(|point| point.q);
    let has_global_overlap = match (
        sample_q_min_global,
        sample_q_max_global,
        buffer_q_min_global,
        buffer_q_max_global,
    ) {
        (Some(sample_min), Some(sample_max), Some(buffer_min), Some(buffer_max)) => {
            sample_max >= buffer_min && sample_min <= buffer_max
        }
        _ => false,
    };
    if grid_relation != "identical" && has_global_overlap {
        diagnostics.push(Diagnostic::new(
            "Q_GRID_MISMATCH_INTERPOLATE_OVERLAP_ONLY",
            "warning",
            "样品和缓冲液 q 网格不同：不直接相减，只在共同 q 范围内插值。",
            vec![&sample.artifact_id, &buffer.artifact_id],
        ));
    }
    if !has_global_overlap {
        diagnostics.push(Diagnostic::new(
            "NO_OVERLAP_REGION",
            "error",
            "样品和缓冲液 q 区间互不重叠；不做插值、不相减。",
            vec![&sample.artifact_id, &buffer.artifact_id],
        ));
    }
    if buffer_points.is_empty() || sample_points.is_empty() || !has_global_overlap {
        let candidate_id = format!(
            "background:{}.{}.fixed_unity",
            sample.dataset_id, buffer.dataset_id
        );
        return vec![BackgroundCandidate {
            artifact_id: candidate_id.clone(),
            version: artifact_version(
                &candidate_id,
                &(sample.version.clone(), buffer.version.clone(), "no_overlap"),
            ),
            candidate_id,
            sample_id: sample.dataset_id.clone(),
            buffer_id: buffer.dataset_id.clone(),
            label: format!("{} / fixed α=1", buffer_label),
            alpha_mode: "fixed_unity".to_string(),
            alpha: f64::NAN,
            alpha_sigma: f64::NAN,
            grid_relation: if has_global_overlap {
                grid_relation.clone()
            } else {
                "no_overlap".to_string()
            },
            q_min: None,
            q_max: None,
            chi_square: None,
            rss: None,
            status: "invalid_no_overlap".to_string(),
            points: Vec::new(),
            diagnostics,
        }];
    }

    let overlap_points: Vec<(&NormalizedPoint, InterpolatedBufferPoint)> = sample
        .points
        .iter()
        .filter(|point| point.q.is_finite())
        .filter_map(|point| {
            interpolate_log_intensity(point.q, &buffer_points)
                .map(|interpolated| (point, interpolated))
        })
        .collect();
    let in_range: Vec<f64> = overlap_points.iter().map(|(point, _)| point.q).collect();
    let Some(first_q) = in_range.first() else {
        diagnostics.push(Diagnostic::new(
            "NO_OVERLAP_REGION",
            "error",
            "共同 q 范围内没有可用于样品网格的点。",
            vec![&sample.artifact_id, &buffer.artifact_id],
        ));
        let candidate_id = format!(
            "background:{}.{}.fixed_unity",
            sample.dataset_id, buffer.dataset_id
        );
        return vec![BackgroundCandidate {
            artifact_id: candidate_id.clone(),
            version: artifact_version(
                &candidate_id,
                &(
                    sample.version.clone(),
                    buffer.version.clone(),
                    "empty_overlap",
                ),
            ),
            candidate_id,
            sample_id: sample.dataset_id.clone(),
            buffer_id: buffer.dataset_id.clone(),
            label: format!("{} / fixed α=1", buffer_label),
            alpha_mode: "fixed_unity".to_string(),
            alpha: f64::NAN,
            alpha_sigma: f64::NAN,
            grid_relation,
            q_min: None,
            q_max: None,
            chi_square: None,
            rss: None,
            status: "invalid_no_overlap".to_string(),
            points: Vec::new(),
            diagnostics,
        }];
    };
    let Some(last_q) = in_range.last() else {
        unreachable!("非空重叠范围必须有最后一个 q 点");
    };
    let q_min = *first_q;
    let q_max = *last_q;

    let fit_inputs: Vec<(f64, f64, f64, f64)> = overlap_points
        .iter()
        .map(|(point, interpolated)| {
            let sample_i = point.intensity.unwrap_or(f64::NAN);
            let sample_sigma = point.sigma.unwrap_or(sample_i.abs() * 0.05);
            (
                sample_i,
                sample_sigma,
                interpolated.intensity,
                interpolated.sigma,
            )
        })
        .collect();
    let fit = weighted_alpha(&fit_inputs);
    let mut modes: Vec<(&str, f64, f64)> = vec![
        ("fixed_unity", 1.0, 0.0),
        ("fixed_uncertainty_0p95", 0.95, 0.05),
    ];
    if let Some((alpha, alpha_sigma, _)) = fit {
        modes.push(("residual_fit", alpha, alpha_sigma));
    } else {
        diagnostics.push(Diagnostic::new(
            "BACKGROUND_SCALE_NOT_IDENTIFIABLE",
            "error",
            "重叠区不足以唯一估计背景缩放，保留固定候选。",
            vec![&sample.artifact_id, &buffer.artifact_id],
        ));
    }

    modes
        .into_iter()
        .map(|(mode, alpha, alpha_sigma)| {
            let mut points = Vec::new();
            let mut mode_diagnostics = diagnostics.clone();
            for (point, interpolated_point) in &overlap_points {
                let sample_i = point.intensity.unwrap_or(f64::NAN);
                let sample_sigma = point.sigma.unwrap_or(sample_i.abs() * 0.05);
                let raw_buffer_i = interpolated_point.intensity;
                let raw_buffer_sigma = interpolated_point.sigma;
                let scaled_buffer = alpha * raw_buffer_i;
                let value = sample_i - scaled_buffer;
                let normalized_variance = sample_sigma.powi(2);
                let buffer_normalized_variance = (alpha * raw_buffer_sigma).powi(2);
                let background_scale_variance = (raw_buffer_i * alpha_sigma).powi(2);
                let sigma =
                    (normalized_variance + buffer_normalized_variance + background_scale_variance)
                        .sqrt();
                let residual = if sigma > 0.0 { value / sigma } else { f64::NAN };
                let mut point_diagnostics = Vec::new();
                if value < 0.0 {
                    point_diagnostics.push(Diagnostic::new(
                        "NEGATIVE_SUBTRACTED_INTENSITY",
                        "warning",
                        "扣背景后强度为负，保留数值供审阅，不裁剪为零。",
                        vec![&point.point_id],
                    ));
                }
                if interpolated_point.method == "log_linear" {
                    point_diagnostics.push(Diagnostic::new(
                        "Q_GRID_MISMATCH_INTERPOLATE_OVERLAP_ONLY",
                        "warning",
                        "该点使用缓冲液曲线在重叠范围内的对数线性插值。",
                        vec![&point.point_id],
                    ));
                }
                points.push(BackgroundPoint {
                    point_id: format!("{}.{}.{}", point.point_id, buffer.dataset_id, mode),
                    sample_point_id: point.point_id.clone(),
                    buffer_point_id: format!(
                        "interpolated:{}:{}",
                        buffer.artifact_id,
                        interpolated_point.source_point_ids.join("+")
                    ),
                    buffer_source_point_ids: interpolated_point.source_point_ids.clone(),
                    q: point.q,
                    sample_intensity: sample_i,
                    buffer_intensity_raw: raw_buffer_i,
                    buffer_intensity_on_sample_grid: scaled_buffer,
                    alpha,
                    value,
                    sigma,
                    residual,
                    residual_sigma: sigma,
                    normalized_variance,
                    buffer_normalized_variance,
                    background_scale_variance,
                    interpolated: interpolated_point.method == "log_linear",
                    valid: value.is_finite() && sigma.is_finite(),
                    diagnostics: point_diagnostics,
                });
            }
            let valid: Vec<&BackgroundPoint> = points.iter().filter(|point| point.valid).collect();
            let rss = valid
                .iter()
                .map(|point| point.residual.powi(2))
                .sum::<f64>();
            let chi_square = if valid.len() > 1 {
                Some(rss / valid.len() as f64)
            } else {
                None
            };
            if alpha_sigma > 0.1 {
                mode_diagnostics.push(Diagnostic::new(
                    "BACKGROUND_SCALE_UNCERTAINTY",
                    "warning",
                    "背景缩放不确定度较大，误差带包含该项并保留多候选。",
                    vec![&sample.artifact_id, &buffer.artifact_id],
                ));
            }
            let candidate_id = format!(
                "background:{}.{}.{}",
                sample.dataset_id,
                buffer.dataset_id,
                if mode == "fixed_uncertainty_0p95" {
                    "fixed_0p95"
                } else {
                    mode
                }
            );
            let status = decisions
                .iter()
                .find_map(|(id, decision)| {
                    if id == &candidate_id {
                        Some(decision.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| {
                    if selected {
                        "available_selected_buffer"
                    } else {
                        "available_alternative_buffer"
                    }
                    .to_string()
                });
            BackgroundCandidate {
                artifact_id: candidate_id.clone(),
                version: artifact_version(
                    &candidate_id,
                    &(
                        sample.version.clone(),
                        buffer.version.clone(),
                        mode,
                        alpha,
                        alpha_sigma,
                    ),
                ),
                candidate_id,
                sample_id: sample.dataset_id.clone(),
                buffer_id: buffer.dataset_id.clone(),
                label: format!("{} / {}", buffer_label, mode),
                alpha_mode: mode.to_string(),
                alpha,
                alpha_sigma,
                grid_relation: grid_relation.clone(),
                q_min: Some(q_min),
                q_max: Some(q_max),
                chi_square,
                rss: Some(rss),
                status: status.to_string(),
                points,
                diagnostics: mode_diagnostics,
            }
        })
        .collect()
}

fn linear_fit(x_values: &[f64], y_values: &[f64]) -> Option<(f64, f64, f64, f64)> {
    let count = x_values.len();
    if count < 3 {
        return None;
    }
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xx = 0.0;
    let mut sum_xy = 0.0;
    for (x, y) in x_values.iter().zip(y_values) {
        sum_x += x;
        sum_y += y;
        sum_xx += x * x;
        sum_xy += x * y;
    }
    let denominator = count as f64 * sum_xx - sum_x * sum_x;
    if denominator.abs() < 1e-12 {
        return None;
    }
    let slope = (count as f64 * sum_xy - sum_x * sum_y) / denominator;
    let intercept = (sum_y - slope * sum_x) / count as f64;
    let mean_y = sum_y / count as f64;
    let mut rss = 0.0;
    let mut tss = 0.0;
    for (x, y) in x_values.iter().zip(y_values) {
        let predicted = intercept + slope * x;
        rss += (y - predicted).powi(2);
        tss += (y - mean_y).powi(2);
    }
    let r_squared = if tss > 1e-12 { 1.0 - rss / tss } else { 0.0 };
    Some((slope, intercept, r_squared, rss))
}

fn window_candidates(
    background: &BackgroundCandidate,
    kind: &str,
    sizes: &[usize],
) -> Vec<IntervalCandidate> {
    let usable: Vec<&BackgroundPoint> = background
        .points
        .iter()
        .filter(|point| point.valid && point.value > 0.0)
        .collect();
    let mut result = Vec::new();
    for (window_index, size) in sizes.iter().enumerate() {
        if *size > usable.len() {
            continue;
        }
        let window = if kind == "guinier" {
            &usable[..*size]
        } else {
            &usable[usable.len() - *size..]
        };
        let x_values: Vec<f64> = window
            .iter()
            .map(|point| {
                if kind == "guinier" {
                    point.q.powi(2)
                } else {
                    point.q.ln()
                }
            })
            .collect();
        let y_values: Vec<f64> = window
            .iter()
            .map(|point| {
                if kind == "guinier" {
                    point.value.ln()
                } else {
                    point.value.ln()
                }
            })
            .collect();
        let Some((slope, intercept, r_squared, rss)) = linear_fit(&x_values, &y_values) else {
            continue;
        };
        let interval_id = format!(
            "interval:{}.{}.{}.{}",
            background.candidate_id, kind, size, window_index
        );
        let status = if r_squared > 0.90 {
            "legal_candidate"
        } else {
            "weak_evidence_retained"
        };
        let mut diagnostics = vec![Diagnostic::new(
            "INTERVAL_EVIDENCE_INDEPENDENT",
            "info",
            "Guinier 与 Porod 只作为独立区间证据，不自动拼成单一模型。",
            vec![&interval_id],
        )];
        if kind == "guinier" && (!slope.is_finite() || slope >= 0.0) {
            diagnostics.push(Diagnostic::new(
                "GUINIER_SLOPE_EXPECTED_NEGATIVE",
                "warning",
                "ln(I) 对 q² 斜率应为负，窗口保留为弱证据。",
                vec![&interval_id],
            ));
        }
        if kind == "porod" && (!slope.is_finite() || slope >= 0.0) {
            diagnostics.push(Diagnostic::new(
                "POROD_SLOPE_EXPECTED_NEGATIVE",
                "warning",
                "ln(I) 对 ln(q) 斜率应为负，窗口保留为弱证据。",
                vec![&interval_id],
            ));
        }
        result.push(IntervalCandidate {
            artifact_id: interval_id.clone(),
            version: artifact_version(
                &interval_id,
                &(
                    background.version.clone(),
                    kind,
                    size,
                    window_index,
                    slope,
                    intercept,
                ),
            ),
            interval_id,
            background_candidate_id: background.candidate_id.clone(),
            kind: kind.to_string(),
            q_min: window.first().map(|point| point.q).unwrap_or_default(),
            q_max: window.last().map(|point| point.q).unwrap_or_default(),
            point_ids: window.iter().map(|point| point.point_id.clone()).collect(),
            slope,
            intercept,
            r_squared,
            rss,
            status: status.to_string(),
            diagnostics,
        });
    }
    result
}

pub fn interval_candidates(background: &BackgroundCandidate) -> Vec<IntervalCandidate> {
    if !background
        .points
        .iter()
        .any(|point| point.valid && point.value > 0.0)
    {
        return vec![IntervalCandidate {
            artifact_id: format!("interval:{}.none", background.candidate_id),
            version: artifact_version(
                &format!("interval:{}.none", background.candidate_id),
                &(background.version.clone(), "no_positive_overlap"),
            ),
            interval_id: format!("interval:{}.none", background.candidate_id),
            background_candidate_id: background.candidate_id.clone(),
            kind: "none".to_string(),
            q_min: f64::NAN,
            q_max: f64::NAN,
            point_ids: Vec::new(),
            slope: f64::NAN,
            intercept: f64::NAN,
            r_squared: f64::NAN,
            rss: f64::NAN,
            status: "no_positive_interval".to_string(),
            diagnostics: vec![Diagnostic::new(
                "NO_POSITIVE_INTERVAL_EVIDENCE",
                "warning",
                "扣背景结果没有正强度点，不能生成 Guinier 或 Porod 证据。",
                vec![&background.candidate_id],
            )],
        }];
    }
    let mut candidates = Vec::new();
    candidates.extend(window_candidates(background, "guinier", &[4, 3, 5]));
    candidates.extend(window_candidates(background, "porod", &[3, 4, 5]));
    for guinier_index in 0..candidates.len() {
        if candidates[guinier_index].kind != "guinier" {
            continue;
        }
        let mut guinier = candidates[guinier_index].clone();
        if guinier.kind != "guinier" {
            continue;
        }
        for porod in &mut candidates {
            if porod.kind == "porod" && guinier.q_max > porod.q_min {
                let diagnostic = Diagnostic::new(
                    "INTERVAL_CANDIDATES_OVERLAP_NOT_MERGED",
                    "warning",
                    "Guinier 与 Porod 候选区间重叠，但仍独立保留，不自动拼接。",
                    vec![&guinier.interval_id, &porod.interval_id],
                );
                guinier.diagnostics.push(diagnostic.clone());
                porod.diagnostics.push(diagnostic);
            }
        }
    }
    candidates
}
