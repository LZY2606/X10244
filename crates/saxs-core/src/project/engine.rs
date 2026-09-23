//! Intent -> event application, including optimistic-concurrency checks.

use super::*;
use crate::calc;
use crate::model::*;

pub(crate) fn apply_payload(state: &mut ProjectState, ev: &Event) {
    match &ev.payload {
        EventPayload::ProjectCreated { name } => state.name = name.clone(),
        EventPayload::RuleConfigured { params, .. } => state.params = params.clone(),
        EventPayload::ResponseImported { response } => {
            state.responses.insert(response.id.clone(), response.clone());
        }
        EventPayload::MaskImported { mask } => {
            state.masks.insert(mask.id.clone(), mask.clone());
        }
        EventPayload::RunImported { parsed } => {
            state.run_revisions.insert(parsed.input.id.clone(), 1);
            state.runs.insert(parsed.input.id.clone(), parsed.clone());
        }
        EventPayload::RunAmended { run_id, amendment } => {
            if let Some(base) = state.runs.get(run_id).cloned() {
                let next = calc::apply_amendment(&base, amendment);
                state.runs.insert(run_id.clone(), next);
                state.run_revisions.insert(run_id.clone(), amendment.revision);
            }
        }
        EventPayload::Normalized { curve } => {
            let list = state.norms.entry(curve.run_id.clone()).or_default();
            if !list.iter().any(|c| c.id == curve.id) {
                list.push(curve.clone());
            }
            state.norm_head.insert(curve.run_id.clone(), curve.id.clone());
        }
        EventPayload::NormalizationBlocked { .. } => {}
        EventPayload::Subtracted { version } => {
            let list = state.sub_versions.entry(version.artifact_id.clone()).or_default();
            if !list.iter().any(|v| v.id == version.id) {
                list.push(version.clone());
            }
            state.sub_heads.insert(version.artifact_id.clone(), version.id.clone());
        }
        EventPayload::SubtractionBlocked { .. } => {}
        EventPayload::RegionFitted { fit } => {
            // mark overlapping independent intervals of the same kind
            let mut fit = fit.clone();
            if let Some(others) = state.fits.get(&fit.artifact_id) {
                for o in others {
                    if o.kind == fit.kind
                        && o.id != fit.id
                        && fit.q_min < o.q_max
                        && o.q_min < fit.q_max
                    {
                        if !fit.overlap_with.contains(&o.id) {
                            fit.overlap_with.push(o.id.clone());
                        }
                    }
                }
            }
            if !fit.overlap_with.is_empty() && !fit.flags.contains(&"interval_overlap".to_string()) {
                fit.flags.push("interval_overlap".to_string());
            }
            let list = state.fits.entry(fit.artifact_id.clone()).or_default();
            if !list.iter().any(|f| f.id == fit.id) {
                list.push(fit);
            }
        }
        EventPayload::FitBlocked { .. } => {}
        EventPayload::Decision { record } => {
            if let Some(chosen) = &record.chosen {
                if record.kind == "continue_branch" {
                    state.sub_heads.insert(record.target_artifact.clone(), chosen.clone());
                }
            }
            state.decisions.push(record.clone());
        }
        EventPayload::ConflictResolved { base_seq, incoming_seq, .. } => {
            state.conflict_resolutions.push((*base_seq, *incoming_seq));
        }
    }
}

fn conflict(base_seq: u64, current: u64) -> Result<(), String> {
    if base_seq != current {
        Err(format!(
            "CONFLICT: decision was prepared against seq {} but project is now at seq {}",
            base_seq, current
        ))
    } else {
        Ok(())
    }
}

impl Project {
    pub fn apply_intent(&mut self, intent: Intent) -> Result<Vec<AppliedEvent>, String> {
        let mut applied = Vec::new();
        match intent {
            Intent::CreateProject { id, name } => {
                self.id = id;
                applied.push(self.commit(EventPayload::ProjectCreated { name }));
            }
            Intent::ConfigureRules { params, note } => {
                applied.push(self.commit(EventPayload::RuleConfigured { params, note }));
            }
            Intent::ImportResponse { response } => {
                applied.push(self.commit(EventPayload::ResponseImported { response }));
            }
            Intent::ImportMask { mask } => {
                applied.push(self.commit(EventPayload::MaskImported { mask }));
            }
            Intent::ImportRun { input } => {
                let parsed = calc::parse_run(input);
                applied.push(self.commit(EventPayload::RunImported { parsed }));
            }
            Intent::AmendRun { amendment } => {
                if !self.state.runs.contains_key(&amendment.run_id) {
                    return Err(format!("unknown run {}", amendment.run_id));
                }
                let rev = self.state.run_revisions.get(&amendment.run_id).copied().unwrap_or(0) + 1;
                let mut amendment = amendment;
                if amendment.revision == 0 {
                    amendment.revision = rev;
                }
                applied.push(self.commit(EventPayload::RunAmended {
                    run_id: amendment.run_id.clone(),
                    amendment,
                }));
            }
            Intent::Normalize { run_id, response_id, mask_id } => {
                let run = self.state.runs.get(&run_id).cloned().ok_or("unknown run")?;
                let resp = response_id.and_then(|id| self.state.responses.get(&id).cloned());
                let mask = mask_id.and_then(|id| self.state.masks.get(&id).cloned());
                match calc::normalize(&run, resp.as_ref(), mask.as_ref(), RULE_VERSION) {
                    Ok(mut curve) => {
                        let rev = self.state.run_revisions.get(&run_id).copied().unwrap_or(1);
                        curve.revision = rev;
                        applied.push(self.commit(EventPayload::Normalized { curve }));
                    }
                    Err(diags) => {
                        applied.push(self.commit(EventPayload::NormalizationBlocked {
                            run_id,
                            diagnostics: diags,
                        }));
                    }
                }
            }
            Intent::Subtract { artifact_id, sample_run, buffer_run, windows, manual_alpha } => {
                let s = self.state.norm_for(&sample_run).cloned().ok_or("sample not normalized")?;
                let b = self.state.norm_for(&buffer_run).cloned().ok_or("buffer not normalized")?;
                let outcome = calc::subtract(calc::SubtractParams {
                    sample: &s,
                    buffer: &b,
                    artifact_id: &artifact_id,
                    windows: &windows,
                    manual_alpha,
                    rule_version: RULE_VERSION,
                    created_seq: self.next_seq,
                });
                match outcome.version {
                    Some(version) => {
                        applied.push(self.commit(EventPayload::Subtracted { version }));
                    }
                    None => {
                        applied.push(self.commit(EventPayload::SubtractionBlocked {
                            artifact_id,
                            sample_run,
                            buffer_run,
                            diagnostics: outcome.diagnostics,
                        }));
                    }
                }
            }
            Intent::FitRegion { artifact_id, kind, q_min, q_max } => {
                let sub = self
                    .state
                    .sub_head(&artifact_id)
                    .cloned()
                    .ok_or("no current subtraction version for artifact")?;
                match calc::fit_region(calc::FitParams {
                    sub: &sub,
                    kind: &kind,
                    q_min,
                    q_max,
                    qrg_limit: self.state.params.qrg_max,
                    rule_version: RULE_VERSION,
                    created_seq: self.next_seq,
                }) {
                    Ok(fit) => applied.push(self.commit(EventPayload::RegionFitted { fit })),
                    Err(diagnostic) => applied.push(self.commit(EventPayload::FitBlocked {
                        sub_version: sub.id,
                        kind,
                        q_min,
                        q_max,
                        diagnostic,
                    })),
                }
            }
            Intent::AcceptScale { artifact_id, sub_version_id, candidate_id, base_seq, note } => {
                conflict(base_seq, self.next_seq)?;
                let version = self
                    .state
                    .sub_versions
                    .get(&artifact_id)
                    .and_then(|v| v.iter().find(|s| s.id == sub_version_id))
                    .cloned()
                    .ok_or("subtraction version not found")?;
                let candidate = version
                    .candidates
                    .iter()
                    .find(|c| c.id == candidate_id)
                    .cloned()
                    .ok_or("candidate not found")?;
                let s = self.state.norms.get(&version.sample_run).and_then(|v| v.iter().find(|c| c.id == version.sample_norm)).cloned().unwrap();
                let b = self.state.norms.get(&version.buffer_run).and_then(|v| v.iter().find(|c| c.id == version.buffer_norm)).cloned().unwrap();
                let windows: Vec<(f64, f64)> = candidate.fit_window.into_iter().collect();
                let outcome = calc::subtract(calc::SubtractParams {
                    sample: &s,
                    buffer: &b,
                    artifact_id: &artifact_id,
                    windows: &windows,
                    manual_alpha: Some(candidate.alpha),
                    rule_version: RULE_VERSION,
                    created_seq: self.next_seq,
                });
                let new_version = outcome.version.ok_or("could not rebuild subtraction with chosen scale")?;
                applied.push(self.commit(EventPayload::Subtracted { version: new_version.clone() }));
                let rejected: Vec<String> = version
                    .candidates
                    .iter()
                    .filter(|c| c.id != candidate_id)
                    .map(|c| c.id.clone())
                    .collect();
                let record = DecisionRecord {
                    event_id: String::new(),
                    seq: self.next_seq,
                    kind: "accept_scale".to_string(),
                    target_artifact: artifact_id,
                    chosen: Some(candidate_id),
                    rejected,
                    params_adjusted: serde_json::json!({"alpha": candidate.alpha, "sigma_alpha": candidate.sigma_alpha}),
                    note,
                    base_seq,
                };
                applied.push(self.commit(EventPayload::Decision { record }));
            }
            Intent::ContinueBranch { artifact_id, chosen_version, base_seq, note } => {
                conflict(base_seq, self.next_seq)?;
                let versions = self.state.sub_versions.get(&artifact_id).ok_or("artifact not found")?;
                if !versions.iter().any(|v| v.id == chosen_version) {
                    return Err("chosen version does not belong to artifact".to_string());
                }
                let rejected: Vec<String> = versions.iter().filter(|v| v.id != chosen_version).map(|v| v.id.clone()).collect();
                let record = DecisionRecord {
                    event_id: String::new(),
                    seq: self.next_seq,
                    kind: "continue_branch".to_string(),
                    target_artifact: artifact_id.clone(),
                    chosen: Some(chosen_version),
                    rejected,
                    params_adjusted: serde_json::json!({}),
                    note,
                    base_seq,
                };
                applied.push(self.commit(EventPayload::Decision { record }));
            }
            Intent::ResolveConflict { base_seq, incoming_seq, resolution, note } => {
                applied.push(self.commit(EventPayload::ConflictResolved {
                    base_seq,
                    incoming_seq,
                    resolution,
                    note,
                }));
            }
        }
        Ok(applied)
    }
}
