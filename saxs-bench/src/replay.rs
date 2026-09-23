//! Replay script for the offline case pack.
//!
//! Produces a deterministic ordered list of events that actually traverses:
//! q-grid misalignment, no-overlap, background-scale uncertainty, negative
//! intensity, interval overlap, buffer version switching and multi-candidate
//! residuals — plus one parameter revision, one local dependency-only source
//! edit and one conflicting human decision.

use crate::case::*;
use crate::events::EventKind;
use crate::model::*;

#[derive(Debug, Clone)]
pub struct Step {
    pub title: String,
    pub kind: EventKind,
    /// Rule ids the reviewer should be able to read back after this step.
    pub expect_rules: Vec<&'static str>,
}

/// Full replay: baseline import, revision, local edit, conflicting decisions.
pub fn script(seed: i64, start_ms: i64) -> Vec<Step> {
    let mut kinds = base_event_kinds(seed);
    let mut steps = Vec::new();
    for k in kinds.drain(..) {
        steps.push(Step { title: "导入原始材料".into(), kind: k, expect_rules: vec![] });
    }

    // --- 参数修订：收窄 v1 的拟合窗（保留原宽窗分支为未采纳候选）---
    steps.push(Step {
        title: "参数修订：缓冲液 v1 拟合窗收窄到高 q 尾".into(),
        kind: EventKind::FitWindow {
            sample_id: SAMPLE.into(),
            buffer_id: BUF_V1.into(),
            fit_lo: 0.18,
            fit_hi: 0.34,
        },
        expect_rules: vec!["grid.misaligned", "bg.scale_uncertain", "sub.negative_intensity", "model.intervals_separate"],
    });

    // --- 局部依赖的源数据改动：仅修正一个样品点（保留原始与遮罩来源）---
    let mut edited = sample_profile(if seed % 2 == 0 { 0.012 } else { -0.014 });
    if let Some(pt) = edited.points.get_mut(20) {
        let v = pt.i.unwrap() * 0.90;
        pt.i = Some(v);
        pt.i_raw = format!("{v:.6}");
    }
    edited.version = "import+pointfix".into();
    steps.push(Step {
        title: "源数据局部修订：仅改样品第 21 点（沿依赖图局部传播）".into(),
        kind: EventKind::ProfileUpdated { profile: edited },
        expect_rules: vec!["grid.misaligned"],
    });

    // --- 人工决定（可冲突）：先接受 v2，再否定 v1 宽窗候选 ---
    // 分支 id 在运行期由规则+输入决定，因此这里用占位标记，UI 用最新分支解析。
    steps.push(Step {
        title: "人工裁定：接受缓冲液 v2 当前分支".into(),
        kind: EventKind::Decision {
            sample_id: SAMPLE.into(),
            branch_id: "LATEST:BUF_V2".into(),
            decision: Decision {
                branch_id: "LATEST:BUF_V2".into(),
                kind: DecisionKind::Accept,
                note: "v2 残差无系统趋势，作为当前采纳版本".into(),
                params: serde_json::json!({}),
            },
        },
        expect_rules: vec![],
    });
    steps.push(Step {
        title: "冲突的人工裁定：否决 v1 宽窗原始候选（保留未选分支）".into(),
        kind: EventKind::Decision {
            sample_id: SAMPLE.into(),
            branch_id: "FIRST:BUF_V1".into(),
            decision: Decision {
                branch_id: "FIRST:BUF_V1".into(),
                kind: DecisionKind::Reject,
                note: "宽窗在低 q 处系统偏高，否决但不覆盖其它候选".into(),
                params: serde_json::json!({}),
            },
        },
        expect_rules: vec![],
    });

    // attach sequential timestamps handled by build_events
    let _ = start_ms;
    steps
}

/// The boundary scenarios this replay is designed to evidence, with the
/// concrete material/rule/decision that drives each (for the UI guide).
pub fn scenario_index() -> Vec<(&'static str, &'static str)> {
    vec![
        ("q 网格错位", "样品 0.01–0.40 与缓冲液 v1 0.012–0.395 网格不同 → grid.misaligned，仅在重叠区插值"),
        ("无重叠区", "高 q 缓冲液 0.80–1.60 与样品不相交 → grid.no_overlap，分支 blocked 且不相减"),
        ("背景缩放不确定度", "窄窗拟合 v1，scale_sigma 计入 σ_total → bg.scale_uncertain"),
        ("负强度", "样品尾部第 51 点保留一个下凹，扣除后为负 → sub.negative_intensity，不截断"),
        ("区间重叠", "多个 Guinier 窗互相重叠，标记为多候选证据而非合并 → model.intervals_separate"),
        ("缓冲液版本切换", "v1/v2 同时为候选，拟合窗不同，残差并列保留"),
        ("多候选残差", "每个 (缓冲液版本, 拟合窗) 产生独立分支，接受/否决均为只追加事件"),
    ]
}
