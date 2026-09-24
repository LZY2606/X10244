use crate::domain::*;
use serde_json::json;
use std::collections::BTreeMap;

pub fn replay(events: &[Event]) -> ProjectState {
    let mut engine = ReplayEngine::default();
    for event in events {
        engine.apply(event);
    }
    engine.finish()
}

#[derive(Default)]
struct ReplayEngine {
    state: ProjectState,
    decisions: Vec<(String, String)>,
    previous_nodes: BTreeMap<String, String>,
}

impl ReplayEngine {
    fn apply(&mut self, event: &Event) {
        self.state.judgments.push(Judgment {
            judgment_id: format!("judgment:{}:event", event.event_id),
            event_id: event.event_id.clone(),
            code: "EVENT_RECORDED".to_string(),
            severity: "info".to_string(),
            message: "事件以只追加形式进入重放链。".to_string(),
            related: vec![event.event_id.clone()],
            evidence: serde_json::to_value(&event.kind).unwrap_or(json!({})),
        });

        match &event.kind {
            EventKind::ProjectCreated {
                project_id,
                name,
                description,
            } => {
                self.state.project_id = project_id.clone();
                self.state.name = name.clone();
                self.state.description = description.clone();
            }
            EventKind::DatasetImported { dataset } => {
                self.state
                    .datasets
                    .insert(dataset.dataset_id.clone(), dataset.clone());
            }
            EventKind::ParameterRevision {
                dataset_id,
                exposure,
                thickness,
                transmission,
                instrument_response,
                ..
            } => {
                if let Some(dataset) = self.state.datasets.get_mut(dataset_id) {
                    if let Some(value) = exposure {
                        dataset.exposure = value.clone();
                    }
                    if let Some(value) = thickness {
                        dataset.thickness = value.clone();
                    }
                    if let Some(value) = transmission {
                        dataset.transmission = value.clone();
                    }
                    if let Some(value) = instrument_response {
                        dataset.instrument_response = Some(value.clone());
                    }
                }
            }
            EventKind::PointReplaced {
                dataset_id,
                point_id,
                point,
                ..
            } => {
                if let Some(dataset) = self.state.datasets.get_mut(dataset_id) {
                    if let Some(existing) = dataset
                        .points
                        .iter_mut()
                        .find(|item| item.point_id == *point_id)
                    {
                        *existing = point.clone();
                    } else {
                        dataset.points.push(point.clone());
                    }
                }
            }
            EventKind::BufferSelectionChanged {
                sample_id,
                action,
                buffer_id,
                ..
            } => {
                if let Some(sample) = self.state.datasets.get_mut(sample_id) {
                    if !sample.buffer_candidate_ids.contains(buffer_id) {
                        sample.buffer_candidate_ids.push(buffer_id.clone());
                    }
                    if action == "select" || action == "switch" {
                        sample.selected_buffer_id = Some(buffer_id.clone());
                    }
                }
            }
            EventKind::CandidateDecision {
                candidate_id,
                decision,
                ..
            } => {
                self.decisions.retain(|(id, _)| id != candidate_id);
                self.decisions
                    .push((candidate_id.clone(), decision.clone()));
            }
        }

        self.rebuild_derived(event);
    }

    fn rebuild_derived(&mut self, event: &Event) {
        let normalized: Vec<NormalizedCurve> = self
            .state
            .datasets
            .values()
            .map(normalize_dataset)
            .collect();

        let mut backgrounds = Vec::new();
        let mut intervals = Vec::new();
        for sample in &normalized {
            let Some(dataset) = self.state.datasets.get(&sample.dataset_id) else {
                continue;
            };
            for buffer_id in &dataset.buffer_candidate_ids {
                let Some(buffer) = normalized
                    .iter()
                    .find(|curve| curve.dataset_id == *buffer_id)
                else {
                    continue;
                };
                let selected = dataset.selected_buffer_id.as_deref() == Some(buffer_id);
                let label = buffer.label.clone();
                for candidate in
                    background_candidates(sample, buffer, &label, selected, &self.decisions)
                {
                    intervals.extend(interval_candidates(&candidate));
                    backgrounds.push(candidate);
                }
            }
        }

        self.state.normalized_curves = normalized;
        self.state.background_candidates = backgrounds.clone();
        self.state.interval_candidates = intervals;
        self.rebuild_graph(event, &backgrounds);
    }

    fn rebuild_graph(&mut self, event: &Event, backgrounds: &[BackgroundCandidate]) {
        let mut current_nodes = BTreeMap::new();
        let mut graph = Vec::new();
        for dataset in self.state.datasets.values() {
            let node_id = format!("raw:{}", dataset.dataset_id);
            let version = artifact_version(
                &node_id,
                &serde_json::to_value(dataset).unwrap_or(json!({})),
            );
            current_nodes.insert(node_id.clone(), version.clone());
            graph.push(GraphNode {
                node_id,
                kind: "raw".to_string(),
                label: format!("原始记录 {}", dataset.label),
                status: "source".to_string(),
                artifact_id: format!("raw:{}", dataset.dataset_id),
                artifact_version: version,
                inputs: vec![
                    dataset.source_file.clone(),
                    dataset.mask_source.clone(),
                    dataset
                        .instrument_response
                        .as_ref()
                        .map(|response| response.response_id.clone())
                        .unwrap_or_else(|| "instrument:none".to_string()),
                ],
                reason: "原始 q/I/误差、曝光、厚度、透射率、缓冲液、响应和遮罩来源均不可变保留。"
                    .to_string(),
            });
        }
        for curve in &self.state.normalized_curves {
            current_nodes.insert(curve.artifact_id.clone(), curve.version.clone());
            graph.push(GraphNode {
                node_id: curve.artifact_id.clone(),
                kind: "normalized".to_string(),
                label: format!("归一化 {}", curve.label),
                status: curve
                    .points
                    .iter()
                    .any(|point| point.diagnostics.iter().any(|d| d.severity == "error"))
                    .then(|| "invalid".to_string())
                    .unwrap_or_else(|| "derived".to_string()),
                artifact_id: curve.artifact_id.clone(),
                artifact_version: curve.version.clone(),
                inputs: vec![format!("raw:{}", curve.dataset_id)],
                reason: "按曝光、厚度、透射率与仪器响应归一化；误差传播包含归一化不确定度。"
                    .to_string(),
            });
        }
        for candidate in backgrounds {
            current_nodes.insert(candidate.artifact_id.clone(), candidate.version.clone());
            let severity = if candidate.status.starts_with("invalid")
                || candidate.diagnostics.iter().any(|d| d.severity == "error")
            {
                "invalid"
            } else {
                "derived"
            };
            graph.push(GraphNode {
                node_id: candidate.artifact_id.clone(),
                kind: "background".to_string(),
                label: candidate.label.clone(),
                status: severity.to_string(),
                artifact_id: candidate.artifact_id.clone(),
                artifact_version: candidate.version.clone(),
                inputs: vec![
                    format!("normalized:{}", candidate.sample_id),
                    format!("normalized:{}", candidate.buffer_id),
                ],
                reason: "q 网格不同时不直接相减；只在共同 q 范围内插值，误差含背景缩放不确定度。"
                    .to_string(),
            });
        }
        for interval in &self.state.interval_candidates {
            current_nodes.insert(interval.artifact_id.clone(), interval.version.clone());
            graph.push(GraphNode {
                node_id: interval.artifact_id.clone(),
                kind: "interval".to_string(),
                label: format!(
                    "{} 区间 {:.3}–{:.3}",
                    interval.kind, interval.q_min, interval.q_max
                ),
                status: interval.status.clone(),
                artifact_id: interval.artifact_id.clone(),
                artifact_version: interval.version.clone(),
                inputs: vec![interval.background_candidate_id.clone()],
                reason: "Guinier 与 Porod 仅保留独立区间证据，不自动拼成单一模型。".to_string(),
            });
        }

        let mut changed_nodes = Vec::new();
        let mut reused_nodes = Vec::new();
        for (node_id, version) in &current_nodes {
            if self.previous_nodes.get(node_id) == Some(version) {
                reused_nodes.push(node_id.clone());
            } else {
                changed_nodes.push(node_id.clone());
            }
        }
        let invalidated_nodes: Vec<String> = self
            .previous_nodes
            .keys()
            .filter(|node_id| !current_nodes.contains_key(*node_id))
            .cloned()
            .collect();
        for node in graph.iter_mut() {
            if changed_nodes.contains(&node.node_id) {
                node.status = format!("{}:changed", node.status);
                node.reason = format!(
                    "{} 输入或决定已变化，版本指纹 {} 已重算。",
                    node.reason, node.artifact_version
                );
            } else if reused_nodes.contains(&node.node_id) {
                node.status = format!("{}:reused", node.status);
                node.reason = format!(
                    "{} 当前依赖指纹 {} 与上一事件一致，因此仍有效并复用。",
                    node.reason, node.artifact_version
                );
            }
        }

        self.state.graph = graph;
        self.state.propagation.push(PropagationEvent {
            event_id: event.event_id.clone(),
            changed_nodes,
            reused_nodes,
            invalidated_nodes,
            rationale: "依据原始记录、归一化、背景候选和区间候选的依赖图比较版本指纹。".to_string(),
        });
        self.previous_nodes = current_nodes;
    }

    fn finish(self) -> ProjectState {
        self.state
    }
}
