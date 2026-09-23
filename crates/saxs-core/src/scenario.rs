//! Offline domain case pack:
//! "Scattering-curve bench — SAXS normalization, background subtraction and
//! multi-region model evidence".
//!
//! Deterministic synthetic 1-D integrated I(q) runs (no external files).
//! Replays: grid mismatch, no-overlap, scale uncertainty, negative intensity,
//! interval overlap, buffer version switch and multi-candidate residuals; then
//! a parameter revision, a local-only source amendment and a human conflict.

use crate::model::*;
use crate::project::{Intent, Project, RULE_VERSION};
use serde::{Deserialize, Serialize};

pub const CASE_ID: &str = "casepack-001";
pub const CASE_TITLE: &str = "散射曲线台：SAXS 归一化、背景扣除与多区间模型证据";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioStep {
    pub index: u32,
    pub title: String,
    pub expect: String,
    pub tags: Vec<String>,
}

/// Physical-ish generator: Guinier plateau + two decays, buffer weaker and
/// scaled differently; noise is deterministic so tests are repeatable.
fn isample(q: f64) -> f64 {
    let base = 2.4 * (-q * q / (2.0 * 0.0045)).exp() + 0.004 / (q * q + 0.002) + 0.00015;
    base
}
fn ibuffer(q: f64, scale: f64) -> f64 {
    scale * (0.004 / (q * q + 0.002) + 0.00015 * 1.0)
}
fn pseudo_noise(k: usize, salt: u32) -> f64 {
    // deterministic in (-1, 1)
    let x = ((k as u64).wrapping_mul(2654435761).wrapping_add(salt as u64 * 40503) % 1_000_000) as f64 / 1_000_000.0;
    (x * 2.0 - 1.0)
}

fn make_run(
    id: &str,
    label: &str,
    role: RunRole,
    qs: &[f64],
    f: impl Fn(f64, usize) -> (f64, f64),
    exposure_s: f64,
    thickness_mm: f64,
    transmission: f64,
    q_sym: &str,
    salt: u32,
    source: &str,
) -> RunImportInput {
    let qfactor = crate::units::unit_factor("q", q_sym).unwrap();
    let rows = qs
        .iter()
        .enumerate()
        .map(|(k, q)| {
            let (i, rel_e) = f(*q, k);
            let n = pseudo_noise(k, salt) * rel_e * i;
            let iv = i + n;
            let e = (rel_e * i).max(1e-6);
            let r = RawRow {
                q: format!("{:.6}", q / qfactor),
                i: format!("{:.6}", iv),
                e: format!("{:.6}", e),
            };
            r
        })
        .collect();
    RunImportInput {
        id: id.to_string(),
        role,
        label: label.to_string(),
        q_unit: q_sym.to_string(),
        i_unit: "rel".to_string(),
        err_unit: "rel".to_string(),
        exposure: RawScalar { raw: format!("{:.3}", exposure_s), unit: "s".to_string() },
        thickness: RawScalar { raw: format!("{:.3}", thickness_mm), unit: "mm".to_string() },
        transmission: RawScalar { raw: format!("{:.4}", transmission), unit: "1".to_string() },
        response_id: Some("resp-pilatus".to_string()),
        mask_id: Some("mask-beamstop".to_string()),
        rows,
        source_file: source.to_string(),
        source_hash: crate::ids::digest(&["source", id, &qs.len().to_string(), q_sym, source]),
    }
}

pub fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    (0..n).map(|k| a + (b - a) * k as f64 / (n - 1) as f64).collect()
}

/// Build the material inputs (also serialized into fixtures/casepack-001).
pub fn materials() -> serde_json::Value {
    let q_sample = linspace(0.006, 0.35, 40);
    let q_b1 = linspace(0.006, 0.35, 40);
    // shifted grid: sample-equivalent spacing but offset -> interpolation path
    let q_b2: Vec<f64> = linspace(0.0062, 0.352, 39);
    // no overlap: starts beyond sample's maximum
    let q_b3 = linspace(0.40, 0.80, 20);

    let sample = make_run(
        "run-sample",
        "ProteinA 5 mg/mL",
        RunRole::Sample,
        &q_sample,
        |q, _| (isample(q), 0.012),
        10.0,
        1.0,
        0.72,
        "A^-1",
        11,
        "integration/sample_5mgml_qI.dat",
    );
    let buffer1 = make_run(
        "run-buffer-v1",
        "Buffer v1 (batch 2026-09-18)",
        RunRole::Buffer,
        &q_b1,
        |q, _| (ibuffer(q, 0.90), 0.015),
        10.0,
        1.0,
        0.74,
        "A^-1",
        23,
        "integration/buffer_batch0918_qI.dat",
    );
    // buffer v2 imported from a nm^-1 pipeline: tests q-grid + unit mismatch
    let mut buffer2 = make_run(
        "run-buffer-v2",
        "Buffer v2 (nm^-1 pipeline, shifted grid)",
        RunRole::Buffer,
        &q_b2,
        |q, _| (ibuffer(q, 0.88), 0.015),
        10.0,
        1.0,
        0.735,
        "nm^-1",
        37,
        "integration/buffer_nmpipeline_qI.csv",
    );
    // mark it explicitly as a different buffer version candidate
    buffer2.mask_id = Some("mask-beamstop".to_string());

    let buffer3 = make_run(
        "run-buffer-v3",
        "Buffer v3 (high-q cell only, no overlap)",
        RunRole::Buffer,
        &q_b3,
        |q, _| (ibuffer(q, 0.95), 0.02),
        8.0,
        1.0,
        0.70,
        "A^-1",
        41,
        "integration/buffer_highqcell_qI.dat",
    );

    // unconvertible unit row/material: must remain at source with diagnostic
    let mut bad = make_run(
        "run-bad-units",
        "Import with parsec exposure (unconvertible)",
        RunRole::Sample,
        &q_sample,
        |q, _| (isample(q), 0.02),
        1.0,
        1.0,
        0.7,
        "A^-1",
        5,
        "integration/legacy_run_qI.dat",
    );
    bad.exposure = RawScalar { raw: "300".to_string(), unit: "parsec".to_string() };
    // also corrupt one row's q text: it stays in place with a PARSE_Q diagnostic
    bad.rows[7].q = "n/a".to_string();

    let response = InstrumentResponse {
        id: "resp-pilatus".to_string(),
        label: "Pilatus flat-field + solid-angle".to_string(),
        source: "calibration/pilatus_response_2026.json".to_string(),
        q_unit: "A^-1".to_string(),
        points: vec![
            ResponsePoint { q: 0.0, factor: 1.0 },
            ResponsePoint { q: 0.5, factor: 1.0 },
        ],
        rule_version: RULE_VERSION.to_string(),
    };
    let mask = Mask {
        id: "mask-beamstop".to_string(),
        label: "Beamstop + intermodule gaps".to_string(),
        source: "calibration/beamstop_mask.h5".to_string(),
        q_unit: "A^-1".to_string(),
        q_ranges: vec![(0.0, 0.0055)],
        index_masks: vec![],
        raw_repr: "q<0.0055 A^-1 beamstop; detector module gaps {127,128}".to_string(),
    };

    serde_json::json!({
        "case_id": CASE_ID,
        "title": CASE_TITLE,
        "materials": {
            "runs": [sample, buffer1, buffer2, buffer3, bad],
            "response": response,
            "mask": mask
        }
    })
}

pub fn step_plan() -> Vec<ScenarioStep> {
    vec![
        ScenarioStep { index: 0, title: "导入仪器响应与遮罩来源".to_string(), expect: "响应与遮罩按来源文件登记，保留原始坐标基准".to_string(), tags: vec!["provenance".into()] },
        ScenarioStep { index: 1, title: "导入样品与三条缓冲液候选".to_string(), expect: "原始 q/I/误差、曝光、厚度、透射率、候选与来源全部保留".to_string(), tags: vec!["baseline".into()] },
        ScenarioStep { index: 2, title: "核对不可转换单位材料".to_string(), expect: "parsec 曝光与损坏 q 留在原位置并附诊断，不删除、不上图伪造".to_string(), tags: vec!["diagnostics".into()] },
        ScenarioStep { index: 3, title: "归一化样品与缓冲液".to_string(), expect: "I/(曝光·厚度·透射率·响应)，遮罩点保留但标记排除".to_string(), tags: vec!["normalization".into()] },
        ScenarioStep { index: 4, title: "尝试归一化不可转换材料".to_string(), expect: "阻塞并保留 NormalizationBlocked 诊断，不移动当前指针".to_string(), tags: vec!["blocked".into()] },
        ScenarioStep { index: 5, title: "缓冲液 v1 背景扣除（多缩放窗口）".to_string(), expect: "多个 alpha 候选、残差与 chi² 并列，缩放不确定候选被标记".to_string(), tags: vec!["multi-candidate".into(), "scale-uncertain".into()] },
        ScenarioStep { index: 6, title: "缓冲液 v2（q 网格错位）扣除".to_string(), expect: "不逐点直接相减；仅重叠区内线性插值，点带来源插值标记".to_string(), tags: vec!["grid-mismatch".into()] },
        ScenarioStep { index: 7, title: "缓冲液 v3（无重叠区）扣除".to_string(), expect: "SubtractionBlocked: no overlapping q range，不产出派生点".to_string(), tags: vec!["no-overlap".into()] },
        ScenarioStep { index: 8, title: "比较两个分支并裁定继续 v2".to_string(), expect: "v1/v2 版本并列；ContinueBranch 记录裁定，v1 保留未删除".to_string(), tags: vec!["branch".into(), "decision".into()] },
        ScenarioStep { index: 9, title: "Guinier 区间候选（重叠窗口）".to_string(), expect: "三个独立 Guinier 候选，重叠区间互标，qRg 越限告警".to_string(), tags: vec!["guinier".into(), "interval-overlap".into()] },
        ScenarioStep { index: 10, title: "Porod 区间候选".to_string(), expect: "独立 Porod 证据，不与 Guinier 自动拼成单一模型".to_string(), tags: vec!["porod".into()] },
        ScenarioStep { index: 11, title: "修订规则参数 qRg 上限".to_string(), expect: "仅规则依赖的区间拟合变陈旧，原始与扣除仍有效".to_string(), tags: vec!["dependency".into()] },
        ScenarioStep { index: 12, title: "局部源数据修订（仅 v2 一个点）".to_string(), expect: "仅 v2 分支及下游陈旧；样品、v1 分支仍有效".to_string(), tags: vec!["local-invalidation".into()] },
        ScenarioStep { index: 13, title: "两位审阅者基于同一版本裁定（冲突）".to_string(), expect: "第二次接受检测到 seq 漂移并拒绝；冲突被记录并可解决".to_string(), tags: vec!["conflict".into()] },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioReport {
    pub project_id: String,
    pub events: usize,
    pub head_hash: String,
    pub artifact_id: String,
    pub v1_version: String,
    pub v2_version: String,
    pub no_overlap_blocked: bool,
    pub bad_units_blocked: bool,
    pub v2_interpolated: bool,
    pub multi_candidate: bool,
    pub scale_uncertain: bool,
    pub negative_points: usize,
    pub guinier_fits: usize,
    pub porod_fits: usize,
    pub overlap_marks: usize,
    pub conflict_seen: String,
    pub resolved: bool,
}

/// Run the complete narrative. Returns the resulting project plus a report
/// used by the review UI and the automated tests.
pub fn build_case() -> (Project, ScenarioReport) {
    let m = materials();
    let mats = &m["materials"];
    let runs = mats["runs"].as_array().unwrap();
    let sample: RunImportInput = serde_json::from_value(runs[0].clone()).unwrap();
    let buf1: RunImportInput = serde_json::from_value(runs[1].clone()).unwrap();
    let buf2: RunImportInput = serde_json::from_value(runs[2].clone()).unwrap();
    let buf3: RunImportInput = serde_json::from_value(runs[3].clone()).unwrap();
    let bad: RunImportInput = serde_json::from_value(runs[4].clone()).unwrap();
    let response: InstrumentResponse = serde_json::from_value(mats["response"].clone()).unwrap();
    let mask: Mask = serde_json::from_value(mats["mask"].clone()).unwrap();

    let mut p = Project::new(CASE_ID, CASE_TITLE).with_fixed_timestamps("2026-09-23T00:00:00Z");
    let mut fire = |p: &mut Project, i: Intent| p.apply_intent(i).expect("scenario intent must apply");

    fire(&mut p, Intent::CreateProject { id: CASE_ID.into(), name: CASE_TITLE.into() });
    fire(&mut p, Intent::ImportResponse { response });
    fire(&mut p, Intent::ImportMask { mask });
    fire(&mut p, Intent::ImportRun { input: sample });
    fire(&mut p, Intent::ImportRun { input: buf1 });
    fire(&mut p, Intent::ImportRun { input: buf2 });
    fire(&mut p, Intent::ImportRun { input: buf3 });
    fire(&mut p, Intent::ImportRun { input: bad });

    // normalization chain
    for id in ["run-sample", "run-buffer-v1", "run-buffer-v2", "run-buffer-v3"] {
        fire(&mut p, Intent::Normalize {
            run_id: id.into(),
            response_id: Some("resp-pilatus".into()),
            mask_id: Some("mask-beamstop".into()),
        });
    }
    // bad material: blocked, pointer untouched
    fire(&mut p, Intent::Normalize {
        run_id: "run-bad-units".into(),
        response_id: Some("resp-pilatus".into()),
        mask_id: Some("mask-beamstop".into()),
    });

    let artifact = "sub:proteinA";

    // v1 subtraction with multiple scale windows; one deliberately narrow
    // window yields a poorly determined (uncertain) alpha.
    fire(&mut p, Intent::Subtract {
        artifact_id: artifact.into(),
        sample_run: "run-sample".into(),
        buffer_run: "run-buffer-v1".into(),
        windows: vec![(0.18, 0.34), (0.26, 0.30)],
        manual_alpha: None,
    });
    let v1_id = p.state.sub_head(artifact).unwrap().id.clone();

    // v2: shifted grid, nm^-1 input; same windows
    fire(&mut p, Intent::Subtract {
        artifact_id: artifact.into(),
        sample_run: "run-sample".into(),
        buffer_run: "run-buffer-v2".into(),
        windows: vec![(0.18, 0.34), (0.26, 0.30)],
        manual_alpha: None,
    });
    let v2_id = p.state.sub_head(artifact).unwrap().id.clone();

    // v3: no overlap -> blocked, head remains v2
    fire(&mut p, Intent::Subtract {
        artifact_id: "sub:proteinA-highq".into(),
        sample_run: "run-sample".into(),
        buffer_run: "run-buffer-v3".into(),
        windows: vec![(0.4, 0.8)],
        manual_alpha: None,
    });

    // reviewer compares and continues the v2 branch explicitly
    let seq_before_decision = p.next_seq;
    fire(&mut p, Intent::ContinueBranch {
        artifact_id: artifact.into(),
        chosen_version: v2_id.clone(),
        base_seq: seq_before_decision,
        note: "v2 与当批缓冲液批次一致；v1 保留作未采纳候选".into(),
    });

    // independent interval evidence
    for (lo, hi) in [(0.008, 0.045), (0.012, 0.060), (0.015, 0.075)] {
        fire(&mut p, Intent::FitRegion {
            artifact_id: artifact.into(),
            kind: "guinier".into(),
            q_min: lo,
            q_max: hi,
        });
    }
    for (lo, hi) in [(0.12, 0.30), (0.18, 0.34)] {
        fire(&mut p, Intent::FitRegion {
            artifact_id: artifact.into(),
            kind: "porod".into(),
            q_min: lo,
            q_max: hi,
        });
    }

    // parameter revision: tighten qRg limit -> prior fits become stale
    let mut params = p.state.params.clone();
    params.qrg_max = 1.0;
    fire(&mut p, Intent::ConfigureRules { params, note: "审阅会后将 qRg 上限收紧到 1.0".into() });

    // local source amendment: one buffer v2 point corrected
    let v2_run = p.state.runs.get("run-buffer-v2").unwrap();
    let idx = 20;
    let mut fixed = v2_run.input.rows[idx].clone();
    let v: f64 = fixed.i.parse().unwrap();
    fixed.i = format!("{:.6}", v * 0.98);
    fire(&mut p, Intent::AmendRun {
        amendment: RunAmendment {
            run_id: "run-buffer-v2".into(),
            reason: "积分复核：第 21 点束流补偿系数修正 2%".into(),
            rows: vec![(idx, fixed)],
            revision: 2,
        },
    });

    // recompute normalization + subtraction downstream (dependency propagation)
    fire(&mut p, Intent::Normalize {
        run_id: "run-buffer-v2".into(),
        response_id: Some("resp-pilatus".into()),
        mask_id: Some("mask-beamstop".into()),
    });
    let head = p.state.sub_head(artifact).unwrap();
    let windows = head.windows.clone();
    fire(&mut p, Intent::Subtract {
        artifact_id: artifact.into(),
        sample_run: "run-sample".into(),
        buffer_run: "run-buffer-v2".into(),
        windows,
        manual_alpha: None,
    });

    // concurrent decisions: editor B prepared against an old seq
    let seq_now = p.next_seq;
    let stale_base = seq_now - 3;
    let current = p.state.sub_head(artifact).unwrap().clone();
    let cand = current.candidates[0].id.clone();
    let conflict = p.apply_intent(Intent::AcceptScale {
        artifact_id: artifact.into(),
        sub_version_id: current.id.clone(),
        candidate_id: cand.clone(),
        base_seq: stale_base,
        note: "编辑 B 基于旧版本的接受".into(),
    });
    let conflict_seen = conflict.unwrap_err();
    fire(&mut p, Intent::ResolveConflict {
        base_seq: stale_base,
        incoming_seq: seq_now,
        resolution: "keep-current".into(),
        note: "保留编辑 A 的 v2 分支，编辑 B 需基于新版本复核".into(),
    });
    // editor A's valid acceptance at the current version succeeds
    let final_base = p.next_seq;
    fire(&mut p, Intent::AcceptScale {
        artifact_id: artifact.into(),
        sub_version_id: current.id.clone(),
        candidate_id: cand,
        base_seq: final_base,
        note: "编辑 A 接受低残差缩放".into(),
    });

    let head = p.state.sub_head(artifact).unwrap();
    let fits = p.state.fits.get(&head.artifact_id).cloned().unwrap_or_default();
    let report = ScenarioReport {
        project_id: p.id.clone(),
        events: p.events.len(),
        head_hash: p.last_hash.clone(),
        artifact_id: artifact.into(),
        v1_version: v1_id,
        v2_version: v2_id,
        no_overlap_blocked: p
            .events
            .iter()
            .any(|e| matches!(&e.payload, crate::project::EventPayload::SubtractionBlocked { artifact_id: a, .. } if a == "sub:proteinA-highq")),
        bad_units_blocked: p
            .events
            .iter()
            .any(|e| matches!(&e.payload, crate::project::EventPayload::NormalizationBlocked { run_id, .. } if run_id == "run-bad-units")),
        v2_interpolated: head.flags.iter().any(|f| f == "grid_interpolated")
            || p.state.sub_versions[artifact].iter().any(|v| v.flags.contains(&"grid_interpolated".to_string())),
        multi_candidate: head.candidates.len() >= 2
            || p.state.sub_versions[artifact].iter().any(|v| v.candidates.len() >= 2),
        scale_uncertain: p
            .state
            .sub_versions
            .values()
            .flatten()
            .flat_map(|v| v.candidates.iter())
            .any(|c| c.uncertain),
        negative_points: head
            .points
            .iter()
            .filter(|x| x.negative)
            .count(),
        guinier_fits: fits.iter().filter(|f| f.kind == "guinier").count(),
        porod_fits: fits.iter().filter(|f| f.kind == "porod").count(),
        overlap_marks: fits.iter().map(|f| f.overlap_with.len()).sum(),
        conflict_seen,
        resolved: !p.state.conflict_resolutions.is_empty(),
    };
    (p, report)
}
