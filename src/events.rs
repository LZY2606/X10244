//! 机器可读事件：每个判定/接受/参数修订都落一条 append-only 事件。
//! 关键守护语义以结构化 `judgement` 表达，便于自动测试与界面回读。

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Import,
    ParameterRevision,
    SourcePatch,
    Derive,
    GridMismatch,
    NoOverlap,
    ScaleUncertain,
    NegativeIntensity,
    IntervalOverlap,
    BufferSwitch,
    MultiCandidateResidual,
    DecisionAccept,
    DecisionReject,
    ConflictDetected,
    ConflictResolved,
    ReplayMarker,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub seq: i64,
    pub event_id: String,
    pub timestamp: String,
    pub actor: String,
    pub event_type: EventType,
    /// 关联实体（曲线/版本）。
    pub subject: String,
    /// 版本指针（若有）。
    pub version_id: Option<String>,
    /// 机器可读判断依据。
    pub judgement: serde_json::Value,
    /// 人类可读说明。
    pub note: String,
    /// 上一个已接受决定（用于冲突/分支比较）。
    pub parent_event_id: Option<String>,
}

pub fn guard_grid_mismatch(
    subject: &str,
    sample_id: &str,
    buffer_id: &str,
    grids_match: bool,
    has_overlap: bool,
    q_min: Option<f64>,
    q_max: Option<f64>,
) -> serde_json::Value {
    serde_json::json!({
        "rule": "grid_alignment",
        "rule_version": crate::domain::RULE_VERSION,
        "sample_id": sample_id,
        "buffer_id": buffer_id,
        "grids_match": grids_match,
        "direct_subtraction": false,
        "interpolation": "only-within-overlap",
        "extrapolation": false,
        "has_overlap": has_overlap,
        "overlap_q_min_1/nm": q_min,
        "overlap_q_max_1/nm": q_max,
        "subject": subject,
    })
}

pub fn guard_uncertainty(
    norm_rel: f64,
    alpha: Option<f64>,
    sigma_alpha: Option<f64>,
) -> serde_json::Value {
    serde_json::json!({
        "rule": "uncertainty_budget",
        "rule_version": crate::domain::RULE_VERSION,
        "components": ["statistical", "normalization", "background_scale"],
        "normalization_relative_uncertainty": norm_rel,
        "alpha": alpha,
        "sigma_alpha": sigma_alpha,
        "scale_uncertain": sigma_alpha.map(|s| s > 0.0).unwrap_or(false),
    })
}

pub fn guard_intervals(
    n_guinier_accepted: usize,
    n_porod_accepted: usize,
    n_overlaps: usize,
) -> serde_json::Value {
    serde_json::json!({
        "rule": "multi_interval_evidence",
        "rule_version": crate::domain::RULE_VERSION,
        "guinier_candidates_accepted": n_guinier_accepted,
        "porod_candidates_accepted": n_porod_accepted,
        "overlapping_pairs": n_overlaps,
        "auto_joined_into_single_model": false,
        "all_legal_candidates_retained": true,
    })
}

pub fn guard_residuals(scores: &[crate::ops::ResidualScore]) -> serde_json::Value {
    serde_json::json!({
        "rule": "multi_candidate_residual",
        "rule_version": crate::domain::RULE_VERSION,
        "unique_winner": false,
        "selection_policy": "manual-review-required",
        "candidates": scores,
    })
}
