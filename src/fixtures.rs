use crate::domain::*;
use crate::store::ExportBundle;
use serde_json::json;

pub fn value(raw: &str, value: Option<f64>, unit: &str, sigma: Option<f64>) -> ValueWithUnit {
    ValueWithUnit {
        raw: raw.to_string(),
        value,
        unit: unit.to_string(),
        sigma,
        sigma_raw: sigma.map(|sigma| format!("{:.3}", sigma)),
        coordinate_basis: Some("2D-integrated lab q axis".to_string()),
    }
}

pub fn point(id: &str, q: f64, intensity: f64, sigma: f64) -> RawPoint {
    RawPoint {
        point_id: id.to_string(),
        q: value(&format!("{:.4}", q), Some(q), "1/angstrom", None),
        intensity: value(
            &format!("{:.4}", intensity),
            Some(intensity),
            "counts",
            None,
        ),
        sigma: value(&format!("{:.4}", sigma), Some(sigma), "counts", None),
        diagnostics: Vec::new(),
    }
}

fn unconvertible_point(id: &str, q: f64) -> RawPoint {
    RawPoint {
        point_id: id.to_string(),
        q: value(&format!("{:.4}", q), Some(q), "1/angstrom", None),
        intensity: value("N/A", None, "counts", None),
        sigma: value("N/A", None, "counts", None),
        diagnostics: vec![Diagnostic::new(
            "RAW_UNCONVERTIBLE",
            "warning",
            "导入时保留无法转换的强度和误差原位置。",
            vec![id],
        )],
    }
}

pub fn sample_decay(q: f64, scale: f64, slope: f64) -> f64 {
    scale * (-slope * q * q).exp() + 0.08 * (1.0 + 2.0 * q).powi(-4)
}

pub fn buffer_decay(q: f64, scale: f64) -> f64 {
    scale * (-1.2 * q * q).exp() + 0.03 * (1.0 + 1.7 * q).powi(-4)
}

pub fn initial_sample() -> Dataset {
    let mut points = Vec::new();
    for index in 0..10 {
        let q = 0.01 + index as f64 * 0.012;
        let intensity = sample_decay(q, 8.0, 28.0);
        points.push(point(
            &format!("sample-a.pt-{:02}", index),
            q,
            intensity,
            (0.012 + intensity * 0.02).max(0.006),
        ));
    }
    points.push(unconvertible_point("sample-a.pt-10", 0.13));
    Dataset {
        dataset_id: "sample-a".to_string(),
        role: "sample".to_string(),
        label: "Lysozyme SAXS A".to_string(),
        source_file: "fixtures/lysozyme_2d_integrated_a.tsv".to_string(),
        points,
        exposure: value("120", Some(120.0), "s", Some(0.5)),
        thickness: value("0.100", Some(0.100), "cm", Some(0.001)),
        transmission: value("0.820", Some(0.820), "dimensionless", Some(0.010)),
        instrument_response: Some(instrument("detector-flat-field-v1")),
        mask_source: "fixtures/mask/beamstop_and_detector_gaps_v1.msk".to_string(),
        buffer_candidate_ids: vec!["buffer-a".to_string()],
        selected_buffer_id: Some("buffer-a".to_string()),
    }
}

pub fn initial_buffer() -> Dataset {
    let mut points = Vec::new();
    for index in 0..10 {
        let q = 0.010 + index as f64 * 0.011;
        let intensity = buffer_decay(q, 2.25);
        points.push(point(
            &format!("buffer-a.pt-{:02}", index),
            q,
            intensity,
            (0.008 + intensity * 0.025).max(0.004),
        ));
    }
    Dataset {
        dataset_id: "buffer-a".to_string(),
        role: "buffer".to_string(),
        label: "Matching buffer A".to_string(),
        source_file: "fixtures/buffer_a_2d_integrated.tsv".to_string(),
        points,
        exposure: value("120", Some(120.0), "s", Some(0.5)),
        thickness: value("0.100", Some(0.100), "cm", Some(0.001)),
        transmission: value("0.880", Some(0.880), "dimensionless", Some(0.011)),
        instrument_response: Some(instrument("detector-flat-field-v1")),
        mask_source: "fixtures/mask/beamstop_and_detector_gaps_v1.msk".to_string(),
        buffer_candidate_ids: Vec::new(),
        selected_buffer_id: None,
    }
}

pub fn no_overlap_buffer() -> Dataset {
    let mut base = initial_buffer();
    base.dataset_id = "buffer-no-overlap".to_string();
    base.label = "No-overlap high-q buffer".to_string();
    base.source_file = "fixtures/buffer_high_q_no_overlap.tsv".to_string();
    base.points = (0..8)
        .map(|index| {
            let q = 0.60 + index as f64 * 0.03;
            let intensity = buffer_decay(q, 1.6);
            point(
                &format!("buffer-no-overlap.pt-{:02}", index),
                q,
                intensity,
                0.01,
            )
        })
        .collect();
    base
}

pub fn revised_buffer() -> Dataset {
    let mut base = initial_buffer();
    base.dataset_id = "buffer-b".to_string();
    base.label = "Matching buffer B (revised mask)".to_string();
    base.source_file = "fixtures/buffer_b_2d_integrated.tsv".to_string();
    base.points = (0..10)
        .map(|index| {
            let q = 0.010 + index as f64 * 0.012;
            let intensity = buffer_decay(q, 1.28) * (1.0 - 0.12 * q);
            let mut point = point(
                &format!("buffer-b.pt-{:02}", index),
                q,
                intensity,
                (0.008 + intensity * 0.03).max(0.004),
            );
            if index >= 8 {
                point.intensity = value("0.0300", Some(0.030), "counts", None);
                point.sigma = value("0.0040", Some(0.004), "counts", None);
            }
            point
        })
        .collect();
    base.instrument_response = Some(instrument("detector-flat-field-v2"));
    base.mask_source = "fixtures/mask/beamstop_and_detector_gaps_v2.msk".to_string();
    base
}

fn instrument(id: &str) -> InstrumentResponse {
    InstrumentResponse {
        response_id: id.to_string(),
        scale: value(
            "1.000",
            Some(1.0),
            "detector-count/count",
            Some(if id.ends_with("v2") { 0.02 } else { 0.015 }),
        ),
        mask_source: "embedded response manifest".to_string(),
        raw_record: json!({
            "source": "detector-calibration/saxs_station.poni",
            "q_basis": "1/angstrom",
            "mask_policy": "beamstop and intermodule gaps retained",
            "raw_manifest": true
        }),
    }
}

pub fn events() -> Vec<Event> {
    vec![
        event(
            "evt-0001",
            None,
            EventKind::ProjectCreated {
                project_id: "case-saxs-station".to_string(),
                name: "散射曲线台：SAXS 归一化、背景扣除与多区间模型证据".to_string(),
                description: "离线小角散射审阅案例".to_string(),
            },
        ),
        event(
            "evt-0002",
            Some("evt-0001"),
            EventKind::DatasetImported {
                dataset: initial_sample(),
            },
        ),
        event(
            "evt-0003",
            Some("evt-0002"),
            EventKind::DatasetImported {
                dataset: initial_buffer(),
            },
        ),
        event(
            "evt-0004",
            Some("evt-0003"),
            EventKind::ParameterRevision {
                dataset_id: "sample-a".to_string(),
                exposure: Some(value("118.4", Some(118.4), "s", Some(0.8))),
                thickness: None,
                transmission: None,
                instrument_response: None,
                reason: "复核快门日志后修订曝光；归一化版本应整体重算。".to_string(),
            },
        ),
        event(
            "evt-0005",
            Some("evt-0004"),
            EventKind::DatasetImported {
                dataset: no_overlap_buffer(),
            },
        ),
        event(
            "evt-0006",
            Some("evt-0005"),
            EventKind::BufferSelectionChanged {
                sample_id: "sample-a".to_string(),
                action: "add".to_string(),
                buffer_id: "buffer-no-overlap".to_string(),
                reason: "导入故意无重叠缓冲液，系统必须拒绝相减并保留候选诊断。".to_string(),
            },
        ),
        event(
            "evt-0007",
            Some("evt-0006"),
            EventKind::PointReplaced {
                dataset_id: "buffer-a".to_string(),
                point_id: "buffer-a.pt-00".to_string(),
                point: point("buffer-a.pt-00", 0.010, 2.72, 0.035),
                reason: "只替换一个缓冲液点；仅依赖该源点的局部结果需要重算。".to_string(),
            },
        ),
        event(
            "evt-0008",
            Some("evt-0007"),
            EventKind::DatasetImported {
                dataset: revised_buffer(),
            },
        ),
        event(
            "evt-0009",
            Some("evt-0008"),
            EventKind::BufferSelectionChanged {
                sample_id: "sample-a".to_string(),
                action: "switch".to_string(),
                buffer_id: "buffer-b".to_string(),
                reason: "切换到修订遮罩后的缓冲液版本；旧缓冲液候选仍保留。".to_string(),
            },
        ),
        event(
            "evt-0010",
            Some("evt-0009"),
            EventKind::CandidateDecision {
                candidate_id: "background:sample-a.buffer-b.residual_fit".to_string(),
                decision: "accepted".to_string(),
                actor: "reviewer-rina".to_string(),
                reason: "多候选残差比较后采用拟合缩放，但固定缩放候选继续保留。".to_string(),
            },
        ),
        event(
            "evt-0011",
            Some("evt-0010"),
            EventKind::CandidateDecision {
                candidate_id: "interval:background:sample-a.buffer-b.residual_fit.guinier.4.0"
                    .to_string(),
                decision: "accepted".to_string(),
                actor: "reviewer-rina".to_string(),
                reason: "接受一个 Guinier 区间证据，不自动与 Porod 拼接。".to_string(),
            },
        ),
    ]
}

pub fn bundle() -> ExportBundle {
    ExportBundle {
        format: "saxs-station-export/1".to_string(),
        schema_version: 2,
        events: events(),
        branches: vec![("main".to_string(), Some("evt-0011".to_string()))],
    }
}

fn event(event_id: &str, parent: Option<&str>, kind: EventKind) -> Event {
    Event {
        event_id: event_id.to_string(),
        parent_event_id: parent.map(str::to_string),
        branch: "main".to_string(),
        actor: "offline-case".to_string(),
        created_at: "2026-09-24T00:00:00Z".to_string(),
        kind,
    }
}
