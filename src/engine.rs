//! Event-sourced rule engine.
//!
//! Applying a command recomputes only the nodes affected by the changed
//! dependency, and reports why every other node remained valid. A command is
//! committed as an event only when it fully succeeds.

use crate::domain::*;
use crate::event::*;
use crate::ids;
use crate::{models, rules, units};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct EngineState {
    pub events: Vec<Event>,
    pub materials: BTreeMap<String, Vec<MaterialVersion>>,
    pub responses: BTreeMap<String, Vec<InstrumentResponse>>,
    pub masks: BTreeMap<String, Vec<Mask>>,
    pub normalized: BTreeMap<String, Normalized>,
    pub subtracted: BTreeMap<String, Subtracted>,
    pub models: BTreeMap<String, ModelEvidence>,
    /// sub node slug -> its version
    pub sub_version: BTreeMap<String, u32>,
    /// sample material id -> current buffer Ref
    pub buffer_pref: BTreeMap<String, Ref>,
    /// sub node slug -> branch decision
    pub decisions: BTreeMap<String, String>,
    pub pointers: BTreeMap<String, String>,
}

#[derive(Debug)]
pub struct EngineError(pub String);
impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for EngineError {}

type EngineResult<T> = Result<T, EngineError>;

impl EngineState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn head(&self) -> u32 {
        self.events.last().map(|e| e.seq).unwrap_or(0)
    }

    pub fn material(&self, id: &str, version: u32) -> EngineResult<&MaterialVersion> {
        self.materials
            .get(id)
            .and_then(|v| v.iter().find(|m| m.version == version))
            .ok_or_else(|| EngineError(format!("材料 {}#v{} 不存在", id, version)))
    }

    pub fn latest_material(&self, id: &str) -> EngineResult<&MaterialVersion> {
        self.materials
            .get(id)
            .and_then(|v| v.last())
            .ok_or_else(|| EngineError(format!("材料 {} 不存在", id)))
    }

    fn response_for(&self, mat: &MaterialVersion) -> Option<&InstrumentResponse> {
        let r = mat.response_ref.as_ref()?;
        self.responses.get(&r.id).and_then(|v| v.iter().find(|x| x.version == r.version))
    }

    pub fn apply(&mut self, author: &str, cmd: Command) -> EngineResult<&Event> {
        let mut artifacts = Artifacts::default();
        let mut verdicts = Vec::new();
        let mut recompute = Vec::new();
        let mut input_fragment = serde_json::json!({});

        match cmd.clone() {
            Command::ImportMaterial { material_id, label, kind, seed } => {
                self.cmd_import(&material_id, &label, &kind, &seed, &mut artifacts, &mut verdicts, &mut recompute, &mut input_fragment)?;
            }
            Command::ReviseParameter { material_id, field, raw, unit, reason } => {
                self.cmd_revise(&material_id, &field, &raw, &unit, &reason, &mut artifacts, &mut verdicts, &mut recompute, &mut input_fragment)?;
            }
            Command::PatchSource { material_id, index, field, raw, reason } => {
                self.cmd_patch(&material_id, index, &field, &raw, &reason, &mut artifacts, &mut verdicts, &mut recompute, &mut input_fragment)?;
            }
            Command::SwitchBuffer { sample, old_buffer, new_buffer, reason } => {
                self.cmd_switch(&sample, &old_buffer, &new_buffer, &reason, &mut artifacts, &mut verdicts, &mut recompute, &mut input_fragment)?;
            }
            Command::ChooseCandidate { sub_node, candidate_id } => {
                self.cmd_choose(&sub_node, &candidate_id, &mut artifacts, &mut verdicts, &mut recompute)?;
            }
            Command::DecideBranch { sub_node, decision, note } => {
                self.cmd_decide(&sub_node, &decision, &note, &mut verdicts, &mut artifacts)?;
            }
            Command::Conflict { base_event, local_decision, incoming_decision, resolution, sub_node } => {
                self.cmd_conflict(base_event, &local_decision, &incoming_decision, &resolution, &sub_node, &mut verdicts, &mut artifacts)?;
            }
            Command::ReplayCasepack { .. } => {
                return Err(EngineError("ReplayCasepack 由 casepack 驱动器处理，而非引擎".into()));
            }
        }

        let seq = self.head() + 1;
        let parent = self.head();
        self.rebuild_pointers();
        let event = Event {
            seq,
            parent_seq: parent,
            author: author.to_string(),
            command: cmd,
            input_fragment,
            verdicts,
            artifacts,
            pointers: self.pointers.clone(),
            recompute,
        };
        self.events.push(event);
        Ok(self.events.last().unwrap())
    }

    fn rebuild_pointers(&mut self) {
        self.pointers.clear();
        for (id, versions) in &self.materials {
            if let Some(latest) = versions.last() {
                self.pointers.insert(format!("material:{}", id), Ref::new(id, latest.version).slug());
            }
        }
        for (sample, bref) in &self.buffer_pref {
            self.pointers.insert(format!("buffer_of:{}", sample), bref.slug());
        }
        for (node, ver) in &self.sub_version {
            self.pointers.insert(format!("sub:{}", node), format!("{}#v{}", node, ver));
        }
        for (node, _m) in &self.models {
            self.pointers.insert(format!("model:{}", node), node.clone());
        }
    }

    fn build_material(&self, material_id: &str, version: u32, kind: MaterialKind, label: &str, seed: &ImportSeed) -> MaterialVersion {
        let mut points = Vec::new();
        let mut diagnostics = Vec::new();
        for (i, row) in seed.rows.iter().enumerate() {
            let q = units::qty(&row.q, &seed.q_unit, units::Family::Q);
            let intensity = units::qty(&row.i, &seed.i_unit, units::Family::Intensity);
            let error = units::qty(&row.e, &seed.i_unit, units::Family::Error);
            if q.diagnostic.is_some() || intensity.diagnostic.is_some() || error.diagnostic.is_some() {
                diagnostics.push(format!("第 {} 行含不可转换值，保留原位并附诊断", i));
            }
            points.push(RawPoint {
                id: ids::point_id(material_id, i, &row.q, &row.i, &row.e),
                index: i,
                q,
                intensity,
                error,
                masked: row.masked,
            });
        }
        let exposure = units::qty(&seed.exposure_raw, &seed.exposure_unit, units::Family::Exposure);
        let thickness = units::qty(&seed.thickness_raw, &seed.thickness_unit, units::Family::Thickness);
        let transmission = units::qty(&seed.transmission_raw, &seed.transmission_unit, units::Family::Transmission);
        let transmission_error = units::qty(&seed.transmission_error_raw, "", units::Family::Transmission);
        for q in [&exposure, &thickness, &transmission] {
            if let Some(d) = &q.diagnostic {
                diagnostics.push(d.clone());
            }
        }
        let response_ref = seed.response_id.as_ref().map(|id| {
            let v = self.responses.get(id).and_then(|x| x.last()).map(|r| r.version).unwrap_or(1);
            Ref::new(id.clone(), v)
        });
        let mask_ref = seed.mask_id.as_ref().map(|id| {
            let v = self.masks.get(id).and_then(|x| x.last()).map(|m| m.version).unwrap_or(1);
            Ref::new(id.clone(), v)
        });
        MaterialVersion {
            material_id: material_id.to_string(),
            version,
            kind,
            label: label.to_string(),
            points,
            exposure,
            thickness,
            transmission,
            transmission_error,
            response_ref,
            mask_ref,
            diagnostics,
        }
    }
}

fn kind_from(s: &str) -> MaterialKind {
    if s == "buffer" { MaterialKind::Buffer } else { MaterialKind::Sample }
}

impl EngineState {
    fn add_response(&mut self, r: InstrumentResponse, art: &mut Artifacts) {
        self.responses.entry(r.id.clone()).or_default().push(r.clone());
        art.responses.push(r);
    }
    fn add_mask(&mut self, m: Mask, art: &mut Artifacts) {
        self.masks.entry(m.id.clone()).or_default().push(m.clone());
        art.masks.push(m);
    }

    fn cmd_import(
        &mut self,
        material_id: &str,
        label: &str,
        kind: &str,
        seed: &ImportSeed,
        art: &mut Artifacts,
        verdicts: &mut Vec<Verdict>,
        recompute: &mut Vec<RecomputeStatus>,
        input: &mut serde_json::Value,
    ) -> EngineResult<()> {
        let existing = self.materials.get(material_id).map(|v| v.len() as u32).unwrap_or(0);
        let version = existing + 1;
        let mk = kind_from(kind);
        let mat = self.build_material(material_id, version, mk, label, seed);
        let is_new = existing == 0;

        *input = serde_json::json!({
            "material_id": material_id, "version": version, "points": mat.points.len(),
            "q_unit": seed.q_unit, "i_unit": seed.i_unit,
            "exposure": seed.exposure_raw, "thickness": seed.thickness_raw,
            "transmission": seed.transmission_raw,
            "response": seed.response_id, "mask": seed.mask_id,
        });

        let norm = rules::normalize(&mat, self.response_for(&mat));
        art.materials.push(mat.clone());
        art.normalized.push(norm.clone());
        self.materials.entry(material_id.to_string()).or_default().push(mat.clone());
        self.normalized.insert(norm.node.clone(), norm.clone());

        verdicts.push(Verdict {
            code: VerdictCode::Imported,
            detail: format!(
                "{} {}#v{} 导入：保留原始 q/I/误差、曝光、厚度、透射率、缓冲候选、仪器响应与遮罩来源",
                if is_new { "新材料" } else { "新版本" }, material_id, version
            ),
        });
        if !mat.diagnostics.is_empty() {
            for d in mat.diagnostics.clone() {
                verdicts.push(Verdict { code: VerdictCode::QGridMismatch, detail: d });
            }
        }
        recompute.push(RecomputeStatus {
            node: norm.node.clone(),
            status: "recomputed".into(),
            because: "新材料/新版本导入，建立归一化节点".into(),
        });

        // Auto chain: sample against current preferred buffer; buffer has no
        // subtraction. Existing sample->buffer subtractions stay as branches.
        if mk == MaterialKind::Sample {
            if let Some(bref) = self.buffer_pref.get(material_id).cloned().or_else(|| {
                self.materials
                    .iter()
                    .find(|(_, vs)| vs.last().map(|m| m.kind == MaterialKind::Buffer).unwrap_or(false))
                    .map(|(k, vs)| Ref::new(k.clone(), vs.last().unwrap().version))
            }) {
                self.buffer_pref.insert(material_id.to_string(), bref.clone());
                self.derive_subtraction(material_id, version, &bref, art, verdicts, recompute)?;
            }
        } else {
            // A new buffer becomes a candidate for every sample; attach branch.
            let sample_ids: Vec<String> = self
                .materials
                .iter()
                .filter(|(_, vs)| vs.last().map(|m| m.kind == MaterialKind::Sample).unwrap_or(false))
                .map(|(k, _)| k.clone())
                .collect();
            let bref = Ref::new(material_id.to_string(), version);
            for sid in sample_ids {
                let sver = self.latest_material(&sid)?.version;
                self.derive_subtraction(&sid, sver, &bref, art, verdicts, recompute)?;
            }
        }
        Ok(())
    }

    /// Build normalized + subtraction + model for a sample/buffer pair.
    fn derive_subtraction(
        &mut self,
        sample_id: &str,
        sample_version: u32,
        bref: &Ref,
        art: &mut Artifacts,
        verdicts: &mut Vec<Verdict>,
        recompute: &mut Vec<RecomputeStatus>,
    ) -> EngineResult<()> {
        let sample_mat = self.material(sample_id, sample_version)?.clone();
        let buffer_mat = self.material(&bref.id, bref.version)?.clone();
        let sample_norm = rules::normalize(&sample_mat, self.response_for(&sample_mat));
        let buffer_norm = rules::normalize(&buffer_mat, self.response_for(&buffer_mat));
        let preselect = self
            .subtracted
            .get(&ids::sub_node(&sample_mat.material_id, sample_mat.version, &buffer_mat.material_id, buffer_mat.version))
            .and_then(|old| old.chosen_candidate.clone());
        let sub = rules::subtract(
            &sample_mat,
            &buffer_mat,
            &sample_norm,
            &buffer_norm,
            preselect.as_deref(),
        );
        let node = sub.node.clone();

        let existed = self.subtracted.contains_key(&node);
        let prev_decision = self.decisions.get(&node).cloned();

        // Translate machine diagnostics into verdict codes.
        for d in &sub.diagnostics {
            let code = if d.contains("没有重叠") { VerdictCode::NoOverlap }
                else if d.contains("q 网格不同") { VerdictCode::QGridMismatch }
                else if d.contains("插值") { VerdictCode::InterpolatedOverlap }
                else if d.contains("缩放相对不确定度") { VerdictCode::BgScaleUncertain }
                else if d.contains("负强度") { VerdictCode::NegativeIntensity }
                else if d.contains("多候选残差") { VerdictCode::MultiCandidateResidual }
                else { VerdictCode::Recomputed };
            verdicts.push(Verdict { code, detail: d.clone() });
        }

        let sub = sub;

        let sub_version = self.sub_version.get(&node).copied().unwrap_or(0) + 1;
        self.sub_version.insert(node.clone(), sub_version);
        self.subtracted.insert(node.clone(), sub.clone());
        art.subtracted.push(sub.clone());
        recompute.push(RecomputeStatus {
            node: node.clone(),
            status: if existed { "recomputed" } else { "recomputed" }.into(),
            because: format!("样品 {}#v{} 与缓冲液 {} 派生扣背景", sample_id, sample_version, bref.slug()),
        });

        // Model evidence (independent intervals).
        if !sub.points.is_empty() {
            let mut ev = models::build_evidence(&sub, sub_version);
            for d in &ev.diagnostics {
                let code = if d.contains("重叠") { VerdictCode::IntervalOverlap } else { VerdictCode::Recomputed };
                verdicts.push(Verdict { code, detail: d.clone() });
            }
            if let Some(dec) = &prev_decision {
                ev.diagnostics.push(format!("既有分支裁定：{}", dec));
            }
            self.models.insert(node.clone(), ev.clone());
            art.models.push(ev);
        }
        Ok(())
    }
}


impl EngineState {
    fn revise_clone(
        &mut self,
        material_id: &str,
        mutate: impl Fn(&mut MaterialVersion),
    ) -> EngineResult<MaterialVersion> {
        let latest = self.latest_material(material_id)?.clone();
        let mut m = latest;
        m.version += 1;
        m.diagnostics.clear();
        mutate(&mut m);
        self.materials.entry(material_id.to_string()).or_default().push(m.clone());
        Ok(m)
    }

    fn affected_samples_after_material(
        &mut self,
        changed_id: &str,
        new_version: u32,
        art: &mut Artifacts,
        verdicts: &mut Vec<Verdict>,
        recompute: &mut Vec<RecomputeStatus>,
    ) -> EngineResult<()> {
        // Recompute normalised node for the changed material.
        let mat = self.material(changed_id, new_version)?.clone();
        let norm = rules::normalize(&mat, self.response_for(&mat));
        self.normalized.insert(norm.node.clone(), norm.clone());
        art.normalized.push(norm);

        // Dependency propagation: find every subtraction that used this material.
        let mut affected: Vec<(String, u32, Ref)> = Vec::new();
        if mat.kind == MaterialKind::Sample {
            for (node, sub) in &self.subtracted {
                if sub.sample.id == changed_id {
                    affected.push((mat.material_id.clone(), new_version, sub.buffer.clone()));
                    let _ = node;
                }
            }
            if affected.is_empty() {
                if let Some(bref) = self.buffer_pref.get(changed_id).cloned() {
                    affected.push((changed_id.to_string(), new_version, bref));
                }
            }
        } else {
            for (_node, sub) in &self.subtracted {
                if sub.buffer.id == changed_id {
                    affected.push((sub.sample.id.clone(), sub.sample.version, Ref::new(changed_id, new_version)));
                }
            }
        }
        let affected_keys: BTreeSet<String> =
            affected.iter().map(|(s, sv, b)| ids::sub_node(s, *sv, &b.id, b.version)).collect();

        for (sid, sver, bref) in affected {
            self.derive_subtraction(&sid, sver, &bref, art, verdicts, recompute)?;
        }

        // Everything else stays valid: state why.
        for node in self.subtracted.keys() {
            if !affected_keys.contains(node) {
                recompute.push(RecomputeStatus {
                    node: node.clone(),
                    status: "still_valid".into(),
                    because: "其样品与缓冲液版本指纹未变化，依赖图中无失效输入".into(),
                });
            }
        }
        Ok(())
    }

    fn cmd_revise(
        &mut self,
        material_id: &str,
        field: &str,
        raw: &str,
        unit: &str,
        reason: &str,
        art: &mut Artifacts,
        verdicts: &mut Vec<Verdict>,
        recompute: &mut Vec<RecomputeStatus>,
        input: &mut serde_json::Value,
    ) -> EngineResult<()> {
        let prev_version = self.latest_material(material_id)?.version;
        let m = self.revise_clone(material_id, |m| {
            let assign = |slot: &mut Qty, fam: units::Family, raw: &str, unit: &str| {
                *slot = units::qty(raw, unit, fam);
            };
            match field {
                "exposure" => assign(&mut m.exposure, units::Family::Exposure, raw, unit),
                "thickness" => assign(&mut m.thickness, units::Family::Thickness, raw, unit),
                "transmission" => assign(&mut m.transmission, units::Family::Transmission, raw, unit),
                "transmission_error" => assign(&mut m.transmission_error, units::Family::Transmission, raw, unit),
                other => m.diagnostics.push(format!("未知参数字段 {}", other)),
            }
        })?;
        art.materials.push(m.clone());
        *input = serde_json::json!({"material_id": material_id, "field": field, "raw": raw, "unit": unit, "reason": reason, "from_version": prev_version});
        verdicts.push(Verdict { code: VerdictCode::ParameterRevised, detail: format!("参数修订 {} -> {} {}（{}）", field, raw, unit, reason) });
        self.affected_samples_after_material(material_id, m.version, art, verdicts, recompute)?;
        Ok(())
    }

    fn cmd_patch(
        &mut self,
        material_id: &str,
        index: usize,
        field: &str,
        raw: &str,
        reason: &str,
        art: &mut Artifacts,
        verdicts: &mut Vec<Verdict>,
        recompute: &mut Vec<RecomputeStatus>,
        input: &mut serde_json::Value,
    ) -> EngineResult<()> {
        let prev_version = self.latest_material(material_id)?.version;
        let m = self.revise_clone(material_id, |m| {
            if let Some(pt) = m.points.get_mut(index) {
                match field {
                    "q" => pt.q = units::qty(raw, &pt.q.unit, units::Family::Q),
                    "i" | "intensity" => pt.intensity = units::qty(raw, &pt.intensity.unit, units::Family::Intensity),
                    "e" | "error" => pt.error = units::qty(raw, &pt.error.unit, units::Family::Error),
                    other => m.diagnostics.push(format!("未知源字段 {}", other)),
                }
                // recompute stable point id from new content
                pt.id = ids::point_id(&m.material_id, pt.index, &pt.q.raw, &pt.intensity.raw, &pt.error.raw);
            } else {
                m.diagnostics.push(format!("源数据索引 {} 越界", index));
            }
        })?;
        art.materials.push(m.clone());
        *input = serde_json::json!({"material_id": material_id, "index": index, "field": field, "raw": raw, "reason": reason, "from_version": prev_version});
        verdicts.push(Verdict { code: VerdictCode::SourceChanged, detail: format!("局部源数据改动：{}[#{}].{}（只影响局部依赖）", material_id, index, reason) });
        self.affected_samples_after_material(material_id, m.version, art, verdicts, recompute)?;
        Ok(())
    }

    fn cmd_switch(
        &mut self,
        sample: &Ref,
        old_buffer: &Ref,
        new_buffer: &Ref,
        reason: &str,
        art: &mut Artifacts,
        verdicts: &mut Vec<Verdict>,
        recompute: &mut Vec<RecomputeStatus>,
        input: &mut serde_json::Value,
    ) -> EngineResult<()> {
        // Both branches must exist (old) or be derivable (new); nothing is deleted.
        self.buffer_pref.insert(sample.id.clone(), new_buffer.clone());
        self.derive_subtraction(&sample.id, sample.version, new_buffer, art, verdicts, recompute)?;
        let old_node = ids::sub_node(&sample.id, sample.version, &old_buffer.id, old_buffer.version);
        let new_node = ids::sub_node(&sample.id, sample.version, &new_buffer.id, new_buffer.version);
        recompute.push(RecomputeStatus {
            node: old_node,
            status: "still_valid".into(),
            because: "旧缓冲液分支保留为未选候选，不因切换被覆盖".into(),
        });
        *input = serde_json::json!({"sample": sample, "old_buffer": old_buffer, "new_buffer": new_buffer, "reason": reason});
        verdicts.push(Verdict { code: VerdictCode::BufferVersionSwitched, detail: format!("缓冲液版本切换 {} -> {}（{}），旧分支保留", old_buffer.slug(), new_buffer.slug(), reason) });
        let _ = new_node;
        Ok(())
    }

    fn cmd_choose(
        &mut self,
        sub_node: &str,
        candidate_id: &str,
        art: &mut Artifacts,
        verdicts: &mut Vec<Verdict>,
        recompute: &mut Vec<RecomputeStatus>,
    ) -> EngineResult<()> {
        let old = self.subtracted.get(sub_node).cloned().ok_or_else(|| EngineError(format!("扣背景节点 {} 不存在", sub_node)))?;
        if !old.candidates.iter().any(|c| c.id == candidate_id) {
            return Err(EngineError(format!("候选 {} 不是 {} 的合法候选", candidate_id, sub_node)));
        }
        let sample_mat = self.material(&old.sample.id, old.sample.version)?.clone();
        let buffer_mat = self.material(&old.buffer.id, old.buffer.version)?.clone();
        let sample_norm = rules::normalize(&sample_mat, self.response_for(&sample_mat));
        let buffer_norm = rules::normalize(&buffer_mat, self.response_for(&buffer_mat));
        let sub = rules::subtract(&sample_mat, &buffer_mat, &sample_norm, &buffer_norm, Some(candidate_id));

        // preserve multi-candidate residual evidence verdict
        for d in &sub.diagnostics {
            let code = if d.contains("多候选残差") { VerdictCode::MultiCandidateResidual }
                else if d.contains("缩放相对不确定度") { VerdictCode::BgScaleUncertain }
                else if d.contains("负强度") { VerdictCode::NegativeIntensity }
                else { VerdictCode::Recomputed };
            verdicts.push(Verdict { code, detail: d.clone() });
        }

        let ver = self.sub_version.get(sub_node).copied().unwrap_or(1) + 1;
        self.sub_version.insert(sub_node.to_string(), ver);
        self.subtracted.insert(sub_node.to_string(), sub.clone());
        art.subtracted.push(sub.clone());
        if !sub.points.is_empty() {
            let ev = models::build_evidence(&sub, ver);
            self.models.insert(sub_node.to_string(), ev.clone());
            art.models.push(ev);
        }
        recompute.push(RecomputeStatus { node: sub_node.into(), status: "recomputed".into(), because: "采纳不同背景缩放候选，重算点、误差带与模型".into() });
        verdicts.push(Verdict { code: VerdictCode::MultiCandidateResidual, detail: format!("已选择候选 {}，其余候选仍保留为未选结果", candidate_id) });
        Ok(())
    }

    fn cmd_decide(
        &mut self,
        sub_node: &str,
        decision: &str,
        note: &str,
        verdicts: &mut Vec<Verdict>,
        art: &mut Artifacts,
    ) -> EngineResult<()> {
        if !self.subtracted.contains_key(sub_node) {
            return Err(EngineError(format!("分支 {} 不存在", sub_node)));
        }
        if !["accepted", "rejected"].contains(&decision) {
            return Err(EngineError("裁定必须是 accepted 或 rejected".into()));
        }
        self.decisions.insert(sub_node.to_string(), decision.to_string());
        let code = if decision == "accepted" { VerdictCode::Accepted } else { VerdictCode::Rejected };
        verdicts.push(Verdict { code, detail: format!("分支 {} 被{}：{}", sub_node, if decision == "accepted" { "接受" } else { "否定" }, note) });
        // keep an artifact echo so the decision is attached/auditable
        if let Some(sub) = self.subtracted.get(sub_node).cloned() {
            art.subtracted.push(sub);
        }
        Ok(())
    }

    fn cmd_conflict(
        &mut self,
        base_event: u32,
        local_decision: &str,
        incoming_decision: &str,
        resolution: &str,
        sub_node: &str,
        verdicts: &mut Vec<Verdict>,
        _art: &mut Artifacts,
    ) -> EngineResult<()> {
        if base_event > self.head() {
            return Err(EngineError("冲突基准事件不存在".into()));
        }
        if local_decision == incoming_decision {
            return Err(EngineError("只有不同决定才构成冲突".into()));
        }
        self.decisions.insert(sub_node.to_string(), resolution.to_string());
        verdicts.push(Verdict {
            code: VerdictCode::Conflict,
            detail: format!(
                "同一版本 #{} 上两位使用者给出不同裁定（本地={}，传入={}），解决为={}",
                base_event, local_decision, incoming_decision, resolution
            ),
        });
        Ok(())
    }
}

/// A read-only view of the ledger at one point in time.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Snapshot {
    pub head: u32,
    pub events: Vec<Event>,
    pub materials: Vec<MaterialVersion>,
    pub responses: Vec<InstrumentResponse>,
    pub masks: Vec<Mask>,
    pub normalized: Vec<Normalized>,
    pub subtracted: Vec<Subtracted>,
    pub models: Vec<ModelEvidence>,
    pub sub_versions: BTreeMap<String, u32>,
    pub decisions: BTreeMap<String, String>,
    pub pointers: BTreeMap<String, String>,
}

impl EngineState {
    /// Replay the ledger from genesis up to and including `up_to`. Replay is
    /// pure/deterministic so any historical event can be revisited without the
    /// latest computation overwriting it.
    pub fn replay(events: &[Event], up_to: u32) -> EngineResult<EngineState> {
        let mut state = EngineState::new();
        for ev in events.iter().filter(|e| e.seq <= up_to) {
            // Re-apply only ledger commands; casepack commands are expanded
            // before being stored, so they never appear here.
            state.apply(&ev.author, ev.command.clone())?;
        }
        Ok(state)
    }

    pub fn snapshot_at(&self, up_to: u32) -> EngineResult<Snapshot> {
        let state = if up_to >= self.head() { self.clone_view() } else { Self::replay(&self.events, up_to)? };
        Ok(state.into_snapshot())
    }

    pub fn clone_view(&self) -> EngineState {
        self.clone()
    }

    pub fn into_snapshot(self) -> Snapshot {
        Snapshot {
            head: self.head(),
            events: self.events,
            materials: self.materials.values().flatten().cloned().collect(),
            responses: self.responses.values().flatten().cloned().collect(),
            masks: self.masks.values().flatten().cloned().collect(),
            normalized: self.normalized.values().cloned().collect(),
            subtracted: self.subtracted.values().cloned().collect(),
            models: self.models.values().cloned().collect(),
            sub_versions: self.sub_version.clone(),
            decisions: self.decisions.clone(),
            pointers: self.pointers.clone(),
        }
    }

    /// Register a calibration response (loaded from casepack or fixture).
    pub fn put_response(&mut self, r: InstrumentResponse) {
        self.responses.entry(r.id.clone()).or_default().push(r);
    }
    pub fn put_mask(&mut self, m: Mask) {
        self.masks.entry(m.id.clone()).or_default().push(m);
    }
}
