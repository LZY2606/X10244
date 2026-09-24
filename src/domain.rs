//! 领域模型：稳定身份、单位/坐标基准、机器可读事件与规则判定。

use serde::{Deserialize, Serialize};

pub type Id = String;

// ===== 机器可读规则（事件 detail 中引用，供复核界面与测试断言） =====
pub const RULE_GRID_NO_DIRECT_SUBTRACT: &str =
    "rule.grid.q_grids_differ_never_subtract_pointwise";
pub const RULE_INTERP_OVERLAP_ONLY: &str =
    "rule.grid.interpolation_takes_effect_only_inside_overlap";
pub const RULE_NO_OVERLAP_REFUSE: &str =
    "rule.grid.no_overlap_region_subtraction_refused";
pub const RULE_ERROR_BUDGET: &str =
    "rule.error.includes_normalization_and_background_scale_uncertainty";
pub const RULE_GUINIER_POROD_NO_MERGE: &str =
    "rule.model.guinier_and_porod_ranges_are_not_merged_into_single_model";
pub const RULE_NEGATIVE_KEPT: &str =
    "rule.intensity.negative_results_kept_flagged_not_clamped";
pub const RULE_MULTICANDIDATE_RETAINED: &str =
    "rule.candidate.all_legal_candidates_kept_until_adjudication";
pub const RULE_DAG_AFFECTED_ONLY: &str =
    "rule.dag.only_affected_descendants_recompute";
pub const RULE_DAG_UNAFFECTED_VALID: &str =
    "rule.dag.unchanged_inputs_result_still_valid";
pub const RULE_RAW_RETAINED: &str =
    "rule.import.raw_q_i_err_and_sources_are_retained";
pub const RULE_UNIT_DIAGNOSTIC: &str =
    "rule.unit.unconvertible_value_stays_in_place_with_diagnostic";

pub fn new_id(prefix: &str, seq: u64) -> Id {
    format!("{prefix}-{seq:06}")
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CurveRole {
    Sample,
    Buffer,
    InstrumentResponse,
    Mask,
}

impl CurveRole {
    pub fn as_str(self) -> &'static str {
        match self {
            CurveRole::Sample => "sample",
            CurveRole::Buffer => "buffer",
            CurveRole::InstrumentResponse => "instrument_response",
            CurveRole::Mask => "mask",
        }
    }
}

/// 原始曲线。q/I/误差与原始文本表示全部保留；规范化只发生在派生层。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Curve {
    pub id: Id,
    pub name: String,
    pub role: CurveRole,
    pub q_unit: String,
    pub i_unit: String,
    pub coord_basis: String,
    pub q: Vec<f64>,
    pub intensity: Vec<f64>,
    pub err: Vec<f64>,
    pub raw: String,
    pub diagnostics: Vec<String>,
}

/// 样品获取参数，均携带单位。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SampleMeta {
    pub sample_name: String,
    pub exposure_s: f64,
    pub thickness_mm: f64,
    pub transmission: f64,
    pub transmission_unc: f64,
    pub q_unit: String,
    pub i_unit: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    Valid,
    Failed,
    NotSelected,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Normalized,
    Subtracted,
    GuinierFit,
    PorodFit,
    ResidualCandidates,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Normalized => "normalized",
            ArtifactKind::Subtracted => "subtracted",
            ArtifactKind::GuinierFit => "guinier_fit",
            ArtifactKind::PorodFit => "porod_fit",
            ArtifactKind::ResidualCandidates => "residual_candidates",
        }
    }
}

/// 派生节点：版本号随重算递增，inputs 是依赖图边。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub id: Id,
    pub kind: ArtifactKind,
    pub generation: u64,
    pub inputs: Vec<Id>,
    pub params: serde_json::Value,
    pub status: ArtifactStatus,
    pub reason: String,
    pub rule_hits: Vec<String>,
    pub payload: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Import,
    Normalize,
    Subtract,
    Fit,
    Adjudicate,
    ReviseParam,
    PatchSource,
    ConflictDetected,
    ConflictResolved,
    TimeTravel,
    Export,
    Imported,
    Migration,
    WriteInterrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: Id,
    pub seq: u64,
    pub version_id: Id,
    pub kind: EventKind,
    pub actor: String,
    pub summary: String,
    /// 机器可读判定：规则命中、受影响对象、输入片段、多候选。
    pub detail: serde_json::Value,
}

/// 某个版本下的完整工作区快照。版本切换是指针原子切换。
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub curves: Vec<Curve>,
    pub metas: Vec<SampleMeta>,
    pub artifacts: Vec<Artifact>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub id: Id,
    pub seq: u64,
    pub parent_id: Option<Id>,
    pub event_seq: u64,
    pub label: String,
    pub snapshot: Snapshot,
}

/// 完整归档（导出/导入格式）：身份、顺序、来源关系自描述。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Archive {
    pub format: String,
    pub created_at: String,
    pub current_version_id: Id,
    pub events: Vec<Event>,
    pub versions: Vec<Version>,
}
