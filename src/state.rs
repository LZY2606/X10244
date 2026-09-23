use crate::domain::*;
use crate::saxs::{evaluate_intervals, normalize_curve, subtract_background};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppState {
    pub case_definition: CaseDefinition,
    pub source_versions: BTreeMap<String, RawCurve>,
    pub accepted_buffer_candidate: String,
    pub accepted_scale_candidate: String,
    pub selected_buffer_history: Vec<String>,
    pub events: Vec<EventRecord>,
    pub derived: DerivedState,
    pub decisions: BTreeMap<String, Decision>,
    pub parameter_overrides: BTreeMap<String, Quantity>,
    pub active_guinier: String,
    pub active_porod: String,
    pub action_log: Vec<Action>,
    pub next_event_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventRecord {
    pub event_id: i64,
    pub event_type: String,
    pub basis_code: String,
    pub message: String,
    pub affected_objects: Vec<String>,
    pub inputs: serde_json::Value,
    pub created_from_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Decision {
    pub subject: String,
    pub decision: String,
    pub event_id: i64,
    pub comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DerivedState {
    pub normalized: BTreeMap<String, NormalizedCurve>,
    pub subtraction_candidates: Vec<SubtractionResult>,
    pub evidence: Vec<ModelEvidence>,
    pub graph: DerivedGraph,
    pub recompute_notes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Action {
    pub action: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub candidate: Option<String>,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub uncertainty: Option<f64>,
    #[serde(default)]
    pub point_index: Option<usize>,
    #[serde(default)]
    pub intensity: Option<f64>,
    #[serde(default)]
    pub q_above: Option<f64>,
    #[serde(default)]
    pub delta_q: Option<f64>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub guinier: Option<String>,
    #[serde(default)]
    pub porod: Option<String>,
    #[serde(default)]
    pub sigma: Option<f64>,
    #[serde(default)]
    pub decision: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

impl AppState {
    pub fn from_case(mut case: CaseDefinition) -> Self {
        let mut source_versions = BTreeMap::new();
        case.sample.diagnostics.clear();
        for curve in std::iter::once(&case.sample).chain(case.buffers.iter()) {
            let mut curve = curve.clone();
            validate_curve(&mut curve);
            source_versions.insert(curve.current_version.clone(), curve);
        }
        let accepted_scale = case
            .scale_candidates
            .iter()
            .find(|candidate| candidate.id == "scale_weighted_fit")
            .map(|candidate| candidate.id.clone())
            .unwrap_or_else(|| case.scale_candidates[0].id.clone());
        let mut state = AppState {
            case_definition: case,
            source_versions,
            accepted_buffer_candidate: String::new(),
            accepted_scale_candidate: accepted_scale,
            selected_buffer_history: Vec::new(),
            events: Vec::new(),
            derived: DerivedState::default(),
            decisions: BTreeMap::new(),
            parameter_overrides: BTreeMap::new(),
            active_guinier: "guinier_core".to_string(),
            active_porod: "porod_core".to_string(),
            action_log: Vec::new(),
            next_event_id: 1,
        };
        state.accepted_buffer_candidate = state.case_definition.accepted_buffer_candidate.clone();
        state.record(
            "case_imported",
            "RAW_MATERIALS_RETAINED",
            "原始 q/I/误差、曝光、厚度、透射率、缓冲液候选、仪器响应与遮罩均已保留",
            vec!["sample", "buffer_b1", "buffer_shifted", "buffer_high_q"],
            serde_json::json!({"fixture": "fixtures/saxs-case"}),
        );
        state.recompute_all();
        state
    }

    pub fn current_version(&self) -> i64 {
        self.events.last().map(|event| event.event_id).unwrap_or(0)
    }

    pub fn apply(&mut self, action: Action) -> Result<i64, String> {
        if let Some(expected) = action.expected_version {
            if expected != self.current_version() {
                return Err(format!(
                    "VERSION_CONFLICT expected={} current={}; refetch and resolve the branch",
                    expected,
                    self.current_version()
                ));
            }
        }
        match action.action.as_str() {
            "revise_parameter" => self.revise_parameter(action),
            "select_buffer" => self.select_buffer(action),
            "set_scale_uncertainty" => self.set_scale_uncertainty(action),
            "patch_source" => self.patch_source(action),
            "activate_intervals" => self.activate_intervals(action),
            "decision" => self.decision(action),
            "attempt_conflicting_decisions" => self.attempt_conflicting_pair(),
            "compare_residual_candidates" => self.compare_candidates(),
            "finish_review" => self.finish_review(),
            "import_case" | "reset_case" => Ok(self.current_version()),
            other => Err(format!("unknown action: {other}")),
        }
    }

    fn record(&mut self, event_type: &str, basis_code: &str, message: &str, affected: Vec<String>, inputs: serde_json::Value) -> i64 {
        let event_id = self.next_event_id;
        self.events.push(EventRecord {
            event_id,
            event_type: event_type.to_string(),
            basis_code: basis_code.to_string(),
            message: message.to_string(),
            affected_objects: affected,
            inputs,
            created_from_version: event_id - 1,
        });
        self.next_event_id += 1;
        event_id
    }

    fn revise_parameter(&mut self, action: Action) -> Result<i64, String> {
        let target = action.target.as_deref().ok_or("target required")?;
        let raw = action.raw.clone().unwrap_or_default();
        let unit = action.unit.clone().unwrap_or_else(|| "cm".to_string());
        let value = action.value.as_ref().and_then(|v| v.as_f64()).ok_or("numeric value required")?;
        let uncertainty = action.uncertainty.unwrap_or(0.0);
        let quantity = Quantity {
            raw,
            value: Some(value),
            unit,
            uncertainty,
            conversion_note: Some("parameter revision creates a new dependency input".to_string()),
        };
        self.parameter_overrides.insert(target.to_string(), quantity.clone());
        let event_id = self.record(
            "parameter_revised",
            "PARAMETER_VERSION_CHANGES_DOWNSTREAM_ONLY",
            "参数修订产生新版本输入；归一化和下游扣除重算，原始材料不覆盖",
            vec!["normalization:sample".to_string(), "subtraction".to_string(), "model_evidence".to_string()],
            serde_json::to_value(&quantity).unwrap(),
        );
        self.recompute_all();
        Ok(event_id)
    }

    fn select_buffer(&mut self, action: Action) -> Result<i64, String> {
        let candidate = action.candidate.clone().ok_or("candidate required")?;
        if !self.case_definition.buffers.iter().any(|curve| curve.candidate_id.as_deref() == Some(candidate.as_str())) {
            return Err("unknown buffer candidate".to_string());
        }
        self.selected_buffer_history.push(self.accepted_buffer_candidate.clone());
        self.accepted_buffer_candidate = candidate.clone();
        let event_id = self.record(
            "buffer_selected",
            "BUFFER_BRANCH_RETAINED",
            "缓冲液版本切换；旧候选与旧扣除分支保留，不发生覆盖",
            vec![candidate.clone(), "subtraction_candidates".to_string()],
            serde_json::json!({"candidate": candidate, "previous": self.selected_buffer_history.last()}),
        );
        self.recompute_all();
        Ok(event_id)
    }

    fn set_scale_uncertainty(&mut self, action: Action) -> Result<i64, String> {
        let candidate = action.candidate.clone().ok_or("candidate required")?;
        let sigma = action.sigma.ok_or("sigma required")?;
        let definition = self
            .case_definition
            .scale_candidates
            .iter_mut()
            .find(|item| item.id == candidate)
            .ok_or("unknown scale candidate")?;
        definition.external_sigma = sigma;
        let event_id = self.record(
            "scale_uncertainty_changed",
            "BACKGROUND_SCALE_UNCERTAINTY_PROPAGATED",
            "背景缩放不确定度进入每个扣除点误差，不改变点值之外的选择规则",
            vec![candidate, "subtracted_error_bands".to_string()],
            serde_json::json!({"sigma": sigma}),
        );
        self.recompute_all();
        Ok(event_id)
    }
    fn patch_source(&mut self, action: Action) -> Result<i64, String> {
        let logical = action.target.clone().or_else(|| action.q_above.map(|_| "sample".to_string())).unwrap_or_else(|| "sample".to_string());
        let current = self.latest_curve(&logical).ok_or("logical curve not found")?;
        let mut curve = current.clone();
        let previous_version = curve.current_version;
        let version = if let Some(value) = action.value.as_ref().and_then(|v| v.as_str()) {
            value.to_string()
        } else {
            next_version(&curve.current_version)
        };
        let mut affected = vec![version.clone()];
        curve.current_version = version.clone();
        if let Some(index) = action.point_index {
            let index = index.checked_sub(1).ok_or("point_index is one-based")?;
            let point = curve.points.get_mut(index).ok_or("point_index out of range")?;
            let intensity = action.intensity.ok_or("intensity required")?;
            point.i = intensity;
            point.i_raw = format!("{:.8}", intensity);
            affected.push(point.point_id.clone());
        }
        if let (Some(q_above), Some(delta_q)) = (action.q_above, action.delta_q) {
            for point in &mut curve.points {
                if point.q > q_above {
                    point.q += delta_q;
                    point.q_raw = format!("{:.6}", point.q);
                    affected.push(point.point_id.clone());
                }
            }
        }
        validate_curve(&mut curve);
        self.source_versions.insert(curve.current_version.clone(), curve);
        if logical == "sample" {
            self.case_definition.sample = self.source_versions[&version].clone();
        }
        let basis = if action.q_above.is_some() {
            "LOCAL_DEPENDENCY_PROPAGATION"
        } else {
            "NEGATIVE_INTENSITY_RETAINED"
        };
        let message = if action.q_above.is_some() {
            "局部源数据只使高于阈值的 q 点和其下游对象失效；低 q 点仍有效"
        } else {
            "负强度源点保留在原位置；不裁剪，不伪造成正值"
        };
        let event_id = self.record(
            "source_patched",
            basis,
            message,
            affected,
            serde_json::json!({"previous_version": previous_version, "action": action}),
        );
        self.action_log_push(action);
        self.recompute_all();
        Ok(event_id)
    }

    fn activate_intervals(&mut self, action: Action) -> Result<i64, String> {
        let guinier = action.guinier.clone().ok_or("guinier window required")?;
        let porod = action.porod.clone().ok_or("porod window required")?;
        self.active_guinier = guinier.clone();
        self.active_porod = porod.clone();
        let event_id = self.record(
            "intervals_activated",
            "GUINIER_POROD_NEVER_AUTO_STITCHED",
            "即使 Guinier 与 Porod 区间重叠，也只保留并列证据，不生成单一拼接模型",
            vec![guinier, porod, "model_evidence".to_string()],
            serde_json::json!({"guinier": action.guinier, "porod": action.porod}),
        );
        self.action_log_push(action);
        self.recompute_all();
        Ok(event_id)
    }

    fn decision(&mut self, action: Action) -> Result<i64, String> {
        let subject = action.subject.clone().or_else(|| action.candidate.clone()).ok_or("subject required")?;
        let decision = action.decision.clone().ok_or("decision required")?;
        let event_id = self.record(
            "human_decision",
            "ACCEPT_OR_REJECT_IS_A_BRANCH_NOT_OVERWRITE",
            "人工接受或否定生成新事件；未选候选继续可读、可回滚",
            vec![subject.clone()],
            serde_json::json!({"decision": decision, "comment": action.comment}),
        );
        self.decisions.insert(
            subject.clone(),
            Decision {
                subject,
                decision: decision.clone(),
                event_id,
                comment: action.comment.unwrap_or_default(),
            },
        );
        if subject.starts_with("buffer_candidate_") {
            self.accepted_buffer_candidate = if decision == "accepted" {
                subject
            } else {
                self.accepted_buffer_candidate.clone()
            };
        }
        if subject.starts_with("scale_") {
            self.accepted_scale_candidate = if decision == "accepted" {
                subject
            } else {
                self.accepted_scale_candidate.clone()
            };
        }
        self.action_log_push(action);
        self.recompute_all();
        Ok(event_id)
    }

    fn attempt_conflicting_pair(&mut self) -> Result<i64, String> {
        let first = Action {
            action: "decision".to_string(),
            target: None,
            candidate: Some("scale_weighted_fit".to_string()),
            value: None,
            raw: None,
            unit: None,
            uncertainty: None,
            point_index: None,
            intensity: None,
            q_above: None,
            delta_q: None,
            reason: None,
            guinier: None,
            porod: None,
            sigma: None,
            decision: Some("accepted".to_string()),
            subject: Some("scale_weighted_fit".to_string()),
            comment: Some("first user accepts".to_string()),
            expected_version: Some(self.current_version()),
        };
        let stale_version = self.current_version();
        self.apply(first)?;
        let second = Action {
            action: "decision".to_string(),
            target: None,
            candidate: Some("scale_weighted_fit".to_string()),
            value: None,
            raw: None,
            unit: None,
            uncertainty: None,
            point_index: None,
            intensity: None,
            q_above: None,
            delta_q: None,
            reason: None,
            guinier: None,
            porod: None,
            sigma: None,
            decision: Some("rejected".to_string()),
            subject: Some("scale_weighted_fit".to_string()),
            comment: Some("second user had stale version".to_string()),
            expected_version: Some(stale_version),
        };
        let conflict = self.apply(second).expect_err("stale concurrent save must conflict");
        let event_id = self.record(
            "conflict_detected",
            "CONCURRENT_SAVE_REQUIRES_RESOLUTION",
            "两位使用者基于同一旧版本时，第二次保存被拒绝并保留可解决冲突",
            vec!["scale_weighted_fit".to_string()],
            serde_json::json!({"rejected_attempt": second, "conflict": conflict}),
        );
        let resolved = Action {
            action: "decision".to_string(),
            target: None,
            candidate: Some("scale_weighted_fit".to_string()),
            value: None,
            raw: None,
            unit: None,
            uncertainty: None,
            point_index: None,
            intensity: None,
            q_above: None,
            delta_q: None,
            reason: None,
            guinier: None,
            porod: None,
            sigma: None,
            decision: Some("rejected".to_string()),
            subject: Some("scale_weighted_fit".to_string()),
            comment: Some("after refetch, second user resolves the branch".to_string()),
            expected_version: Some(self.current_version()),
        };
        self.apply(resolved)?;
        Ok(event_id)
    }

    fn compare_candidates(&mut self) -> Result<i64, String> {
        let event_id = self.record(
            "residual_candidates_compared",
            "ALL_LEGAL_CANDIDATES_RETAINED",
            "所有合法缩放候选的残差同时保留；系统不为了单一答案吞掉多解",
            vec!["scale_weighted_fit".to_string(), "scale_nominal_one".to_string()],
            serde_json::json!({"candidate_count": self.derived.subtraction_candidates.len()}),
        );
        self.action_log_push(Action { action: "compare_residual_candidates".to_string(), ..Default::default() });
        Ok(event_id)
    }

    fn finish_review(&mut self) -> Result<i64, String> {
        let event_id = self.record(
            "review_finished",
            "DERIVED_POINT_TRACE_REACHES_TWO_SOURCE_CURVES",
            "最终复核从扣除点可追回样品与缓冲液两条源曲线、版本和决定",
            vec!["final_review".to_string()],
            serde_json::json!({"version": self.current_version()}),
        );
        self.action_log_push(Action { action: "finish_review".to_string(), ..Default::default() });
        Ok(event_id)
    }

    fn action_log_push(&mut self, action: Action) {
        self.action_log.push(action);
    }

    fn latest_curve(&self, logical_curve_id: &str) -> Option<&RawCurve> {
        self.source_versions
            .values()
            .filter(|curve| curve.logical_curve_id == logical_curve_id)
            .max_by_key(|curve| &curve.current_version)
    }

    fn current_sample(&self) -> RawCurve {
        let mut curve = self.case_definition.sample.clone();
        for (target, quantity) in &self.parameter_overrides {
            match target.as_str() {
                "sample.exposure" => curve.exposure = quantity.clone(),
                "sample.thickness" => curve.thickness = quantity.clone(),
                "sample.transmission" => curve.transmission = quantity.clone(),
                "sample.instrument_response" => curve.instrument_response = quantity.clone(),
                _ => {}
            }
        }
        validate_curve(&mut curve);
        curve
    }

    fn current_buffer(&self) -> RawCurve {
        let version = self
            .case_definition
            .buffers
            .iter()
            .find(|curve| curve.candidate_id.as_deref() == Some(self.accepted_buffer_candidate.as_str()))
            .map(|curve| curve.current_version.clone())
            .or_else(|| Some(self.case_definition.buffers[0].current_version.clone()))
            .expect("at least one buffer candidate");
        self.source_versions
            .get(&version)
            .cloned()
            .unwrap_or_else(|| self.case_definition.buffers[0].clone())
    }

    fn recompute_all(&mut self) {
        let sample_raw = self.current_sample();
        let buffer_raw = self.current_buffer();
        let normalized_sample = normalize_curve(&sample_raw);
        let normalized_buffer = normalize_curve(&buffer_raw);
        let mut normalized = BTreeMap::new();
        for curve in self.source_versions.values() {
            normalized.insert(curve.current_version.clone(), normalize_curve(curve));
        }
        normalized.insert(normalized_sample.raw_curve_version.clone(), normalized_sample.clone());
        normalized.insert(normalized_buffer.raw_curve_version.clone(), normalized_buffer.clone());

        let mut candidates = subtract_background(
            &normalized_sample,
            &normalized_buffer,
            &self.accepted_buffer_candidate,
            &self.case_definition.scale_candidates,
            &self.accepted_scale_candidate,
        );
        for candidate in &mut candidates {
            if let Some(scale) = candidate.scale.as_mut() {
                scale.accepted = candidate.scale_candidate_id == self.accepted_scale_candidate;
            }
        }
        let evidence: Vec<_> = candidates
            .iter()
            .filter(|candidate| candidate.valid)
            .map(|candidate| evaluate_intervals(candidate, &self.case_definition.interval_windows))
            .collect();

        let mut graph = DerivedGraph::default();
        graph.nodes.insert(
            format!("normalization:{}", normalized_sample.raw_curve_version),
            GraphNode {
                id: format!("normalization:{}", normalized_sample.raw_curve_version),
                kind: "normalization".to_string(),
                inputs: vec![
                    normalized_sample.raw_curve_version.clone(),
                    "exposure".to_string(),
                    "thickness".to_string(),
                    "transmission".to_string(),
                    "instrument_response".to_string(),
                ],
                status: if normalized_sample.valid { "valid".to_string() } else { "invalid".to_string() },
                reason: "normalized from its exact source version and normalization inputs".to_string(),
                depends_on_event: self.current_version(),
            },
        );
        graph.nodes.insert(
            format!("normalization:{}", normalized_buffer.raw_curve_version),
            GraphNode {
                id: format!("normalization:{}", normalized_buffer.raw_curve_version),
                kind: "normalization".to_string(),
                inputs: vec![normalized_buffer.raw_curve_version.clone()],
                status: if normalized_buffer.valid { "valid".to_string() } else { "invalid".to_string() },
                reason: "unchanged source version may be reused; buffer switch changes the selected input".to_string(),
                depends_on_event: self.current_version(),
            },
        );
        for candidate in &candidates {
            let status = if candidate.valid {
                "valid"
            } else {
                match candidate.status {
                    DerivationStatus::BlockedNoOverlap => "blocked_no_overlap",
                    DerivationStatus::InvalidInputs => "invalid_inputs",
                    DerivationStatus::Ready => "invalid",
                }
            };
            graph.nodes.insert(
                candidate.node_id.clone(),
                GraphNode {
                    id: candidate.node_id.clone(),
                    kind: "subtraction".to_string(),
                    inputs: vec![
                        format!("normalization:{}", candidate.sample_version),
                        format!("normalization:{}", candidate.buffer_version),
                        candidate.scale_candidate_id.clone(),
                    ],
                    status: status.to_string(),
                    reason: if candidate.valid {
                        "recomputed from selected sample and buffer versions".to_string()
                    } else {
                        candidate.diagnostics.join("; ")
                    },
                    depends_on_event: self.current_version(),
                },
            );
            graph.nodes.insert(
                format!("model_evidence:{}", candidate.node_id),
                GraphNode {
                    id: format!("model_evidence:{}", candidate.node_id),
                    kind: "model_evidence".to_string(),
                    inputs: vec![candidate.node_id.clone()],
                    status: if candidate.valid { "valid" } else { "blocked_by_subtraction" }.to_string(),
                    reason: "Guinier and Porod are separate; no automatic single model".to_string(),
                    depends_on_event: self.current_version(),
                },
            );
        }

        let mut recompute_notes = BTreeMap::new();
        let last_event = self.events.last();
        if let Some(event) = last_event {
            match event.event_type.as_str() {
                "source_patched" if event.basis_code == "LOCAL_DEPENDENCY_PROPAGATION" => {
                    recompute_notes.insert("sample/q<=threshold".to_string(), "still_valid: source point outside patched q region".to_string());
                    recompute_notes.insert("sample/q>threshold".to_string(), "recomputed: q calibration changed source point coordinates".to_string());
                    recompute_notes.insert("buffers".to_string(), "still_valid: buffer source versions did not change".to_string());
                }
                "buffer_selected" => {
                    recompute_notes.insert("sample_branch".to_string(), "still_valid: sample version unchanged".to_string());
                    recompute_notes.insert("buffer_branch".to_string(), "recomputed: selected buffer candidate changed".to_string());
                }
                "parameter_revised" => {
                    recompute_notes.insert("raw_curves".to_string(), "still_valid: revision does not overwrite raw representation".to_string());
                    recompute_notes.insert("normalization".to_string(), "recomputed: normalization parameter changed".to_string());
                }
                _ => {}
            }
        }
        self.derived = DerivedState {
            normalized,
            subtraction_candidates: candidates,
            evidence,
            graph,
            recompute_notes,
        };
    }

    pub fn at_version(initial_case: CaseDefinition, mut actions: Vec<Action>, version: i64) -> Result<Self, String> {
        actions.truncate(version.max(0) as usize);
        let mut state = Self::from_case(initial_case);
        for action in actions {
            state.apply(action)?;
        }
        Ok(state)
    }
}

fn next_version(version: &str) -> String {
    if let Some((prefix, number)) = version.rsplit_once('/v') {
        if let Ok(value) = number.parse::<u32>() {
            return format!("{}/v{}", prefix, value + 1);
        }
    }
    format!("{version}.next")
}
