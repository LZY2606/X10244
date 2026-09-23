use crate::domain::*;
use crate::saxs;
use crate::state::{Action, AppState};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct MetadataFile {
    case_id: String,
    coordinate_frame: BTreeMap<String, AxisBasis>,
    sample: CurveMetadata,
    buffers: Vec<CurveMetadata>,
    accepted_buffer_candidate: String,
    scale_candidates: Vec<ScaleCandidateDefinition>,
    interval_windows: Vec<IntervalWindow>,
}

#[derive(Debug, Deserialize)]
struct CurveMetadata {
    logical_curve_id: String,
    current_version: String,
    source_file: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    candidate_id: Option<String>,
    exposure: Quantity,
    thickness: Quantity,
    transmission: Quantity,
    instrument_response: Quantity,
    mask_source: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct ScenarioFile {
    pub case_id: String,
    pub title: String,
    pub narrative_stages: Vec<String>,
    pub deterministic_actions: Vec<ScenarioAction>,
}

#[derive(Debug, Deserialize)]
pub struct ScenarioAction {
    pub seq: i64,
    pub action: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub canonical_value: Option<f64>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub uncertainty: Option<f64>,
    #[serde(default)]
    pub candidate: Option<String>,
    #[serde(default)]
    pub expected: Option<String>,
    #[serde(default)]
    pub sigma: Option<f64>,
    #[serde(default)]
    pub logical_curve: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub point_index: Option<usize>,
    #[serde(default)]
    pub intensity: Option<f64>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub guinier: Option<String>,
    #[serde(default)]
    pub porod: Option<String>,
    #[serde(default)]
    pub q_above: Option<f64>,
    #[serde(default)]
    pub delta_q: Option<f64>,
    #[serde(default)]
    pub first: Option<String>,
    #[serde(default)]
    pub second: Option<String>,
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub must_record: Option<Vec<String>>,
}

pub fn load_case(root: &Path) -> Result<CaseDefinition, String> {
    let metadata_path = root.join("metadata.json");
    let metadata: MetadataFile = serde_json::from_slice(
        &std::fs::read(metadata_path).map_err(|error| format!("read metadata: {error}"))?,
    )
    .map_err(|error| format!("parse metadata: {error}"))?;
    let sample = load_curve(root, &metadata.sample)?;
    let buffers = metadata
        .buffers
        .iter()
        .map(|item| load_curve(root, item))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CaseDefinition {
        case_id: metadata.case_id,
        coordinate_frame: metadata.coordinate_frame,
        sample,
        buffers,
        accepted_buffer_candidate: metadata.accepted_buffer_candidate,
        scale_candidates: metadata.scale_candidates,
        interval_windows: metadata.interval_windows,
    })
}

pub fn load_scenario(root: &Path) -> Result<ScenarioFile, String> {
    serde_json::from_slice(&std::fs::read(root.join("scenario.json")).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}

fn load_curve(root: &Path, metadata: &CurveMetadata) -> Result<RawCurve, String> {
    let csv = std::fs::read_to_string(root.join(&metadata.source_file)).map_err(|error| error.to_string())?;
    let mut points = saxs::parse_curve_csv(&csv)?;
    points.sort_by(|left, right| left.q.total_cmp(&right.q));
    let mut curve = RawCurve {
        logical_curve_id: metadata.logical_curve_id.clone(),
        current_version: metadata.current_version.clone(),
        source_file: metadata.source_file.clone(),
        label: metadata.label.clone(),
        candidate_id: metadata.candidate_id.clone(),
        exposure: metadata.exposure.clone(),
        thickness: metadata.thickness.clone(),
        transmission: metadata.transmission.clone(),
        instrument_response: metadata.instrument_response.clone(),
        mask_source: metadata.mask_source.clone(),
        points,
        diagnostics: Vec::new(),
    };
    validate_curve(&mut curve);
    Ok(curve)
}

pub fn run_demo(root: &Path) -> Result<(AppState, Vec<Action>, ScenarioFile), String> {
    let case = load_case(root)?;
    let scenario = load_scenario(root)?;
    let mut state = AppState::from_case(case);
    let mut actions = Vec::new();
    for item in &scenario.deterministic_actions {
        let action = scenario_action(item);
        if item.action == "import_case" || item.action == "finish_review" {
            state.apply(action)?;
            if item.action == "finish_review" {
                actions.push(action);
            }
            continue;
        }
        state.apply(action.clone())?;
        actions.push(action);
    }
    Ok((state, actions, scenario))
}

fn scenario_action(item: &ScenarioAction) -> Action {
    let mut action = Action {
        action: item.action.clone(),
        target: item.target.clone().or_else(|| item.logical_curve.clone()),
        candidate: item.candidate.clone(),
        value: item.canonical_value.map(serde_json::Value::from),
        raw: item.raw.clone(),
        unit: item.unit.clone(),
        uncertainty: item.uncertainty,
        point_index: item.point_index,
        intensity: item.intensity,
        q_above: item.q_above,
        delta_q: item.delta_q,
        reason: item.reason.clone(),
        guinier: item.guinier.clone(),
        porod: item.porod.clone(),
        sigma: item.sigma,
        decision: None,
        subject: None,
        comment: item.resolution.clone(),
        expected_version: None,
    };
    if item.action == "attempt_conflicting_decisions" {
        action.action = "attempt_conflicting_decisions".to_string();
    }
    if item.action == "compare_residual_candidates" {
        action.action = "compare_residual_candidates".to_string();
    }
    if item.action == "finish_review" {
        action.action = "finish_review".to_string();
    }
    if item.action == "patch_source" {
        if let Some(version) = item.version.clone() {
            action.value = Some(serde_json::Value::String(version));
        }
        action.target = Some("sample".to_string());
    }
    action
}
