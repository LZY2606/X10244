//! Review projection: dependency graph with "why valid / why stale" reasons,
//! point provenance tracing and the JSON shape consumed by the web UI.

use crate::model::*;
use crate::project::{Event, EventPayload, Project};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointRef {
    pub id: String,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceLink {
    pub point: PointRef,
    pub parents: Vec<PointRef>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub created_seq: u64,
    pub valid: bool,
    pub status: String, // "current" | "stale" | "blocked" | "rejected"
    pub reason: String,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleView {
    pub id: String,
    pub method: String,
    pub alpha: f64,
    pub sigma_alpha: f64,
    pub chi2: f64,
    pub rms_residual: f64,
    pub uncertain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubView {
    pub id: String,
    pub artifact_id: String,
    pub buffer_run: String,
    pub head: bool,
    pub direct_grids: bool,
    pub overlap: Option<(f64, f64)>,
    pub points: Vec<SubPoint>,
    pub candidates: Vec<ScaleView>,
    pub flags: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub created_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitView {
    pub id: String,
    pub artifact_id: String,
    pub sub_version: String,
    pub kind: String,
    pub q_min: f64,
    pub q_max: f64,
    pub n_points: usize,
    pub slope: f64,
    pub intercept: f64,
    pub chi2: f64,
    pub rg: Option<f64>,
    pub i0: Option<f64>,
    pub qrg_max: Option<f64>,
    pub overlap_with: Vec<String>,
    pub flags: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub created_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunView {
    pub id: String,
    pub label: String,
    pub role: RunRole,
    pub q_basis: String,
    pub raw_q_unit: String,
    pub exposure: RawScalar,
    pub thickness: RawScalar,
    pub transmission: RawScalar,
    pub response_id: Option<String>,
    pub mask_id: Option<String>,
    pub source_file: String,
    pub source_hash: String,
    pub revision: u32,
    pub raw_rows: Vec<RawRow>,
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub e: Vec<f64>,
    pub usable: Vec<bool>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormView {
    pub id: String,
    pub run_id: String,
    pub head: bool,
    pub q_basis: String,
    pub intensity_unit: String,
    pub meta: NormalizationMeta,
    pub points: Vec<NormPoint>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventView {
    pub seq: u64,
    pub id: String,
    pub timestamp: String,
    pub rule_version: String,
    pub kind: String,
    pub summary: String,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedView {
    pub seq: u64,
    pub kind: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionView {
    pub seq: u64,
    pub kind: String,
    pub target_artifact: String,
    pub chosen: Option<String>,
    pub rejected: Vec<String>,
    pub note: String,
    pub base_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectView {
    pub project_id: String,
    pub name: String,
    pub seq: u64,
    pub head_hash: String,
    pub rule_version: String,
    pub params: crate::project::RuleParams,
    pub runs: Vec<RunView>,
    pub norms: Vec<NormView>,
    pub subtractions: Vec<SubView>,
    pub fits: Vec<FitView>,
    pub events: Vec<EventView>,
    pub graph: Vec<GraphNode>,
    pub blocked: Vec<BlockedView>,
    pub decisions: Vec<DecisionView>,
    pub responses: Vec<InstrumentResponse>,
    pub masks: Vec<Mask>,
}

fn summarize(ev: &Event) -> String {
    match &ev.payload {
        EventPayload::ProjectCreated { name } => format!("项目创建：{}", name),
        EventPayload::RuleConfigured { note, .. } => format!("规则参数修订：{}", note),
        EventPayload::ResponseImported { response } => format!("导入仪器响应 {}", response.id),
        EventPayload::MaskImported { mask } => format!("导入遮罩 {}", mask.id),
        EventPayload::RunImported { parsed } => format!("导入曲线 {}（{}）", parsed.input.id, parsed.input.source_file),
        EventPayload::RunAmended { run_id, amendment } => format!("源数据修订 {} rev{}：{}", run_id, amendment.revision, amendment.reason),
        EventPayload::Normalized { curve } => format!("归一化 {}（{} 点）", curve.run_id, curve.points.len()),
        EventPayload::NormalizationBlocked { run_id, .. } => format!("归一化阻塞 {}", run_id),
        EventPayload::Subtracted { version } => format!("背景扣除 {}<-{}（{} 点，{} 候选）", version.artifact_id, version.buffer_run, version.points.len(), version.candidates.len()),
        EventPayload::SubtractionBlocked { artifact_id, diagnostics, .. } => {
            format!("背景扣除阻塞 {}：{}", artifact_id, diagnostics.first().map(|d| d.code.as_str()).unwrap_or("?"))
        }
        EventPayload::RegionFitted { fit } => format!("{} 区间拟合 [{:.3},{:.3}]", fit.kind, fit.q_min, fit.q_max),
        EventPayload::FitBlocked { kind, .. } => format!("{} 拟合阻塞", kind),
        EventPayload::Decision { record } => format!("裁定 {}（{}）", record.kind, record.note),
        EventPayload::ConflictResolved { resolution, .. } => format!("冲突解决：{}", resolution),
    }
}

fn kind_of(p: &EventPayload) -> &'static str {
    match p {
        EventPayload::ProjectCreated { .. } => "project-created",
        EventPayload::RuleConfigured { .. } => "rule-configured",
        EventPayload::ResponseImported { .. } => "response-imported",
        EventPayload::MaskImported { .. } => "mask-imported",
        EventPayload::RunImported { .. } => "run-imported",
        EventPayload::RunAmended { .. } => "run-amended",
        EventPayload::Normalized { .. } => "normalized",
        EventPayload::NormalizationBlocked { .. } => "normalization-blocked",
        EventPayload::Subtracted { .. } => "subtracted",
        EventPayload::SubtractionBlocked { .. } => "subtraction-blocked",
        EventPayload::RegionFitted { .. } => "region-fitted",
        EventPayload::FitBlocked { .. } => "fit-blocked",
        EventPayload::Decision { .. } => "decision",
        EventPayload::ConflictResolved { .. } => "conflict-resolved",
    }
}

impl Project {
    pub fn view(&self) -> ProjectView {
        let state = &self.state;
        let runs = state
            .runs
            .values()
            .map(|r| RunView {
                id: r.input.id.clone(),
                label: r.input.label.clone(),
                role: r.input.role,
                q_basis: "A^-1".to_string(),
                raw_q_unit: r.input.q_unit.clone(),
                exposure: r.input.exposure.clone(),
                thickness: r.input.thickness.clone(),
                transmission: r.input.transmission.clone(),
                response_id: r.input.response_id.clone(),
                mask_id: r.input.mask_id.clone(),
                source_file: r.input.source_file.clone(),
                source_hash: r.input.source_hash.clone(),
                revision: state.run_revisions.get(&r.input.id).copied().unwrap_or(1),
                raw_rows: r.input.rows.clone(),
                q: r.q.clone(),
                i: r.i.clone(),
                e: r.e.clone(),
                usable: r.row_usable.clone(),
                diagnostics: r.diagnostics.clone(),
            })
            .collect();

        let mut norms = Vec::new();
        for (run_id, curves) in &state.norms {
            for c in curves {
                norms.push(NormView {
                    id: c.id.clone(),
                    run_id: run_id.clone(),
                    head: state.norm_head.get(run_id) == Some(&c.id),
                    q_basis: c.q_basis.clone(),
                    intensity_unit: c.intensity_unit.clone(),
                    meta: c.meta.clone(),
                    points: c.points.clone(),
                    diagnostics: c.diagnostics.clone(),
                });
            }
        }

        let mut subtractions = Vec::new();
        for (art, versions) in &state.sub_versions {
            for v in versions {
                subtractions.push(SubView {
                    id: v.id.clone(),
                    artifact_id: art.clone(),
                    buffer_run: v.buffer_run.clone(),
                    head: state.sub_heads.get(art) == Some(&v.id),
                    direct_grids: v.direct_grids,
                    overlap: v.overlap,
                    points: v.points.clone(),
                    candidates: v
                        .candidates
                        .iter()
                        .map(|c| ScaleView {
                            id: c.id.clone(),
                            method: c.method.clone(),
                            alpha: c.alpha,
                            sigma_alpha: c.sigma_alpha,
                            chi2: c.chi2,
                            rms_residual: c.rms_residual,
                            uncertain: c.uncertain,
                        })
                        .collect(),
                    flags: v.flags.clone(),
                    diagnostics: v.diagnostics.clone(),
                    created_seq: v.created_seq,
                });
            }
        }

        let mut fits = Vec::new();
        for (art, list) in &state.fits {
            for f in list {
                fits.push(FitView {
                    id: f.id.clone(),
                    artifact_id: art.clone(),
                    sub_version: f.sub_version.clone(),
                    kind: f.kind.clone(),
                    q_min: f.q_min,
                    q_max: f.q_max,
                    n_points: f.n_points,
                    slope: f.slope,
                    intercept: f.intercept,
                    chi2: f.chi2,
                    rg: f.rg,
                    i0: f.i0,
                    qrg_max: f.qrg_max,
                    overlap_with: f.overlap_with.clone(),
                    flags: f.flags.clone(),
                    diagnostics: f.diagnostics.clone(),
                    created_seq: f.created_seq,
                });
            }
        }

        let events = self
            .events
            .iter()
            .map(|ev| EventView {
                seq: ev.seq,
                id: ev.id.clone(),
                timestamp: ev.timestamp.clone(),
                rule_version: ev.rule_version.clone(),
                kind: kind_of(&ev.payload).to_string(),
                summary: summarize(ev),
                payload: ev.payload.clone(),
            })
            .collect();

        let blocked = self
            .events
            .iter()
            .filter_map(|ev| match &ev.payload {
                EventPayload::NormalizationBlocked { diagnostics, .. } => Some(BlockedView {
                    seq: ev.seq,
                    kind: "normalization".to_string(),
                    diagnostics: diagnostics.clone(),
                }),
                EventPayload::SubtractionBlocked { diagnostics, .. } => Some(BlockedView {
                    seq: ev.seq,
                    kind: "subtraction".to_string(),
                    diagnostics: diagnostics.clone(),
                }),
                EventPayload::FitBlocked { diagnostic, .. } => Some(BlockedView {
                    seq: ev.seq,
                    kind: "fit".to_string(),
                    diagnostics: vec![diagnostic.clone()],
                }),
                _ => None,
            })
            .collect();

        let decisions = state
            .decisions
            .iter()
            .map(|d| DecisionView {
                seq: d.seq,
                kind: d.kind.clone(),
                target_artifact: d.target_artifact.clone(),
                chosen: d.chosen.clone(),
                rejected: d.rejected.clone(),
                note: d.note.clone(),
                base_seq: d.base_seq,
            })
            .collect();

        ProjectView {
            project_id: self.id.clone(),
            name: state.name.clone(),
            seq: self.next_seq,
            head_hash: self.last_hash.clone(),
            rule_version: crate::project::RULE_VERSION.to_string(),
            params: state.params.clone(),
            runs,
            norms,
            subtractions,
            fits,
            events,
            graph: self.dependency_graph(),
            blocked,
            decisions,
            responses: state.responses.values().cloned().collect(),
            masks: state.masks.values().cloned().collect(),
        }
    }
}

impl Project {
    /// Dependency graph node validity. Recomputation propagates through the
    /// graph: a derived node is stale when any input event with a higher seq
    /// touched one of its dependencies. The reason explains the verdict.
    pub fn dependency_graph(&self) -> Vec<GraphNode> {
        let st = &self.state;
        let mut nodes = Vec::new();

        for (run_id, run) in &st.runs {
            let rev = st.run_revisions.get(run_id).copied().unwrap_or(1);
            let amend_seq = self
                .events
                .iter()
                .rev()
                .find(|e| matches!(&e.payload, EventPayload::RunAmended { run_id: r, .. } if r == run_id))
                .map(|e| e.seq);
            let _ = amend_seq;
            nodes.push(GraphNode {
                id: format!("run:{}", run_id),
                kind: "run".to_string(),
                label: run.input.label.clone(),
                created_seq: 0,
                valid: true,
                status: "current".to_string(),
                reason: format!("原始材料保留（revision {}，来源 {}）", rev, run.input.source_file),
                depends_on: vec![],
            });
        }
        for (run_id, curves) in &st.norms {
            for c in curves {
                let head = st.norm_head.get(run_id) == Some(&c.id);
                let mut deps = vec![format!("run:{}", run_id)];
                if let Some(r) = &c.meta.response_id {
                    deps.push(format!("response:{}", r));
                }
                if let Some(m) = &c.meta.mask_id {
                    deps.push(format!("mask:{}", m));
                }
                deps.push("rules".to_string());
                let (valid, status, reason) = if head {
                    (true, "current".to_string(), "所有直接来源（原始曲线/曝光/厚度/透射率/响应/遮罩）均为最新，结果仍有效，无需重算".to_string())
                } else {
                    (false, "stale".to_string(), "同一来源已产生更新的归一化版本；此版本保留只读".to_string())
                };
                nodes.push(GraphNode {
                    id: format!("norm:{}", c.id),
                    kind: "norm".to_string(),
                    label: format!("归一化 {}", run_id),
                    created_seq: self
                        .events
                        .iter()
                        .find(|e| matches!(&e.payload, EventPayload::Normalized { curve } if curve.id == c.id))
                        .map(|e| e.seq)
                        .unwrap_or(0),
                    valid,
                    status,
                    reason,
                    depends_on: deps,
                });
            }
        }
        for (art, versions) in &st.sub_versions {
            for v in versions {
                let head = st.sub_heads.get(art) == Some(&v.id);
                let sample_head = st.norm_for(&v.sample_run).map(|n| n.id.clone());
                let buffer_head = st.norm_for(&v.buffer_run).map(|n| n.id.clone());
                let deps = vec![
                    format!("norm:{}", v.sample_norm),
                    format!("norm:{}", v.buffer_norm),
                    "rules".to_string(),
                ];
                let mut stale_reasons = Vec::new();
                if sample_head.as_deref() != Some(v.sample_norm.as_str()) {
                    stale_reasons.push(format!("样品 {} 已重新归一化", v.sample_run));
                }
                if buffer_head.as_deref() != Some(v.buffer_norm.as_str()) {
                    stale_reasons.push(format!("缓冲液 {} 已重新归一化", v.buffer_run));
                }
                let rule_rev = self
                    .events
                    .iter()
                    .filter(|e| matches!(e.payload, EventPayload::RuleConfigured { .. }))
                    .map(|e| e.seq)
                    .max();
                if let Some(rs) = rule_rev {
                    if rs > v.created_seq {
                        stale_reasons.push("规则参数已修订，需要基于新规则重算".to_string());
                    }
                }
                let (valid, status, reason) = if head && stale_reasons.is_empty() {
                    (true, "current".to_string(), "样品与缓冲液归一化版本、规则版本均未变；缩放候选与误差带仍有效".to_string())
                } else if !stale_reasons.is_empty() {
                    (false, "stale".to_string(), format!("需要重算：{}", stale_reasons.join("；")))
                } else {
                    (true, "rejected".to_string(), "未采纳分支：已由审阅裁定保留，可随时切回，不被覆盖".to_string())
                };
                nodes.push(GraphNode {
                    id: format!("sub:{}", v.id),
                    kind: "sub".to_string(),
                    label: format!("扣除 {} <- {}", art, v.buffer_run),
                    created_seq: v.created_seq,
                    valid,
                    status,
                    reason,
                    depends_on: deps,
                });
            }
        }
        for fits in st.fits.values() {
            for f in fits {
                let sub_head = self
                    .state
                    .sub_heads
                    .iter()
                    .find_map(|(a, h)| if *h == f.sub_version { Some(a) } else { None })
                    .is_some();
                let mut deps = vec![format!("sub:{}", f.sub_version), "rules".to_string()];
                let mut reasons = Vec::new();
                if !sub_head {
                    reasons.push("其背景扣除版本不再是当前指针".to_string());
                }
                let rule_rev = self
                    .events
                    .iter()
                    .filter(|e| matches!(e.payload, EventPayload::RuleConfigured { .. }))
                    .map(|e| e.seq)
                    .max();
                if let Some(rs) = rule_rev {
                    if rs > f.created_seq {
                        reasons.push(format!("qRg 等规则参数在事件 {} 之后修订", f.created_seq));
                    }
                }
                let (valid, status, reason) = if reasons.is_empty() {
                    (true, "current".to_string(), "依赖扣除版本与规则均为最新；独立区间证据有效".to_string())
                } else {
                    (false, "stale".to_string(), format!("需要重算：{}", reasons.join("；")))
                };
                nodes.push(GraphNode {
                    id: format!("fit:{}", f.id),
                    kind: "fit".to_string(),
                    label: format!("{} [{:.3},{:.3}]", f.kind, f.q_min, f.q_max),
                    created_seq: f.created_seq,
                    valid,
                    status,
                    reason,
                    depends_on: deps,
                });
            }
        }
        for (id, r) in &st.responses {
            nodes.push(GraphNode { id: format!("response:{}", id), kind: "response".into(), label: r.label.clone(), created_seq: 0, valid: true, status: "current".into(), reason: format!("仪器响应来源 {}", r.source), depends_on: vec![] });
        }
        for (id, m) in &st.masks {
            nodes.push(GraphNode { id: format!("mask:{}", id), kind: "mask".into(), label: m.label.clone(), created_seq: 0, valid: true, status: "current".into(), reason: format!("遮罩来源 {}", m.source), depends_on: vec![] });
        }
        nodes.push(GraphNode {
            id: "rules".to_string(),
            kind: "rules".to_string(),
            label: format!("规则 {}", crate::project::RULE_VERSION),
            created_seq: 0,
            valid: true,
            status: "current".to_string(),
            reason: format!("qRg 上限 {:.2}，缩放相对 sigma 上限 {:.0}%", st.params.qrg_max, st.params.scale_rel_sigma_limit * 100.0),
            depends_on: vec![],
        });
        nodes.sort_by_key(|n| (n.created_seq, n.id.clone()));
        nodes
    }

    /// Trace a derived subtraction point back to its two source points
    /// (sample normalized point and buffer normalized/interpolated pair).
    pub fn trace_point(&self, point_id: &str) -> Option<TraceLink> {
        let st = &self.state;
        for versions in st.sub_versions.values() {
            for v in versions {
                if let Some(sp) = v.points.iter().find(|p| p.id == point_id) {
                    let find_norm_point = |pid: &str| -> Option<(NormPoint, String)> {
                        for curves in st.norms.values() {
                            for c in curves {
                                if let Some(np) = c.points.iter().find(|p| p.id == pid) {
                                    return Some((np.clone(), c.run_id.clone()));
                                }
                            }
                        }
                        None
                    };
                    let mut parents = Vec::new();
                    if let Some((np, run)) = find_norm_point(&sp.sample_point_id) {
                        parents.push(PointRef { id: np.id, q: np.q, i: np.i, sigma: np.sigma_i, kind: "sample-norm".into(), label: run });
                    }
                    if sp.interpolated {
                        let key = sp.buffer_point_id.trim_start_matches("interp#");
                        let mut parts = key.splitn(2, '#');
                        let a = parts.next().unwrap_or("");
                        let b = parts.next().unwrap_or("");
                        for pid in [a, b] {
                            if let Some((np, run)) = find_norm_point(pid) {
                                parents.push(PointRef { id: np.id, q: np.q, i: np.i, sigma: np.sigma_i, kind: "buffer-interp".into(), label: run });
                            }
                        }
                    } else if let Some((np, run)) = find_norm_point(&sp.buffer_point_id) {
                        parents.push(PointRef { id: np.id, q: np.q, i: np.i, sigma: np.sigma_i, kind: "buffer-norm".into(), label: run });
                    }
                    let note = if sp.interpolated {
                        "q 网格不同：未直接相减；缓冲液在重叠区内线性插值到样品 q".to_string()
                    } else if sp.negative {
                        "该点 I-a·Ib < -2sigma：负值保留并标记".to_string()
                    } else {
                        "两条源曲线逐点相减，误差含归一化与缩放不确定度".to_string()
                    };
                    return Some(TraceLink {
                        point: PointRef { id: sp.id.clone(), q: sp.q, i: sp.i, sigma: sp.sigma_i, kind: "sub".into(), label: v.artifact_id.clone() },
                        parents,
                        note,
                    });
                }
            }
        }
        None
    }
}
