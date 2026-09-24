use crate::domain::*;
use std::collections::BTreeMap;

fn quantity(value: f64, unit: &str, raw: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.to_string(),
        raw_representation: raw.to_string(),
        conversion_diagnostic: None,
    }
}

fn point(id: &str, index: usize, q: f64, intensity: f64, sigma: f64) -> RawPoint {
    RawPoint {
        id: id.to_string(),
        q,
        intensity,
        sigma,
        q_unit: "1/A".to_string(),
        intensity_unit: "counts".to_string(),
        masked: false,
        mask_source: "fixtures/case/mask-manual-v1.csv".to_string(),
        original_row: format!("row-{index:02}"),
    }
}

fn curve(
    id: &str,
    role: &str,
    label: &str,
    qs: &[f64],
    intensity: impl Fn(usize, f64) -> f64,
    sigma_scale: f64,
    masked: &[usize],
    thickness: f64,
    thickness_raw: &str,
) -> Curve {
    let points = qs
        .iter()
        .enumerate()
        .map(|(index, q)| {
            let value = intensity(index, *q);
            let mut raw = point(
                &format!("{id}-p{index:02}"),
                index + 2,
                *q,
                value,
                (value.abs() * 0.018 + 18.0).max(8.0) * sigma_scale,
            );
            raw.masked = masked.contains(&index);
            raw
        })
        .collect();
    Curve {
        id: id.to_string(),
        role: role.to_string(),
        label: label.to_string(),
        points,
        exposure: quantity(120.0, "s", "00:02:00"),
        thickness: quantity(thickness, "mm", thickness_raw),
        transmission: quantity(0.42, "fraction", "42.0 %"),
        instrument_response_id: "detector-eiger-silver-behenate".to_string(),
        instrument_response: quantity(1.0, "detector-count-scale", "calibration-2026-09"),
        mask_source: "fixtures/case/mask-manual-v1.csv".to_string(),
        diagnostics: Vec::new(),
    }
}

fn standard_q() -> Vec<f64> {
    (0..19)
        .map(|index| 0.012 + (index as f64) * 0.0072)
        .collect()
}

fn shifted_q() -> Vec<f64> {
    (0..19)
        .map(|index| 0.0132 + (index as f64) * 0.0071)
        .collect()
}

fn no_overlap_q() -> Vec<f64> {
    (0..8).map(|index| 0.22 + (index as f64) * 0.01).collect()
}

pub fn case_bundle() -> CaseBundle {
    let baseline_q = standard_q();
    let shifted_q = shifted_q();
    let no_overlap_q = no_overlap_q();

    let mut samples = BTreeMap::new();
    samples.insert(
        "sample-baseline".to_string(),
        SampleRecord {
            buffer_candidate_ids: vec!["buffer-a-v1".to_string(), "buffer-a-v2".to_string()],
            curve: curve(
                "sample-baseline",
                "sample",
                "Baseline protein sample",
                &baseline_q,
                |_, q| 9000.0 * (-2.3 * q * q).exp() + 90.0 / (q.powi(2) + 0.01),
                1.0,
                &[17],
                1.00,
                "1.00 mm",
            ),
        },
    );
    samples.insert(
        "sample-grid-shift".to_string(),
        SampleRecord {
            buffer_candidate_ids: vec!["buffer-a-v1".to_string()],
            curve: curve(
                "sample-grid-shift",
                "sample",
                "Integrated on a shifted q grid",
                &shifted_q,
                |_, q| 8800.0 * (-2.25 * q * q).exp() + 86.0 / (q.powi(2) + 0.011),
                1.0,
                &[],
                1.00,
                "1.00 mm",
            ),
        },
    );
    samples.insert(
        "sample-no-overlap".to_string(),
        SampleRecord {
            buffer_candidate_ids: vec!["buffer-a-v1".to_string()],
            curve: curve(
                "sample-no-overlap",
                "sample",
                "High-q sample with no buffer overlap",
                &no_overlap_q,
                |_, q| 72.0 / q.powi(2),
                1.2,
                &[],
                1.00,
                "1.00 mm",
            ),
        },
    );
    samples.insert(
        "sample-scale-uncertain".to_string(),
        SampleRecord {
            buffer_candidate_ids: vec!["buffer-noisy".to_string()],
            curve: curve(
                "sample-scale-uncertain",
                "sample",
                "Weak sample over noisy buffer",
                &baseline_q,
                |_, q| 500.0 + 42.0 / (q.powi(2) + 0.04),
                1.0,
                &[],
                1.00,
                "1.00 mm",
            ),
        },
    );
    samples.insert(
        "sample-negative".to_string(),
        SampleRecord {
            buffer_candidate_ids: vec!["buffer-a-v1".to_string()],
            curve: {
                let mut source = curve(
                    "sample-negative",
                    "sample",
                    "Contains a retained negative subtracted point",
                    &baseline_q,
                    |_, q| 8850.0 * (-2.3 * q * q).exp() + 82.0 / (q.powi(2) + 0.012),
                    1.0,
                    &[],
                    1.00,
                    "1.00 mm",
                );
                source.points[14].intensity = 35.0;
                source
            },
        },
    );
    samples.insert(
        "sample-unit-diagnostic".to_string(),
        SampleRecord {
            buffer_candidate_ids: vec!["buffer-a-v1".to_string()],
            curve: {
                let source = curve(
                    "sample-unit-diagnostic",
                    "sample",
                    "Original thickness representation retained",
                    &baseline_q,
                    |_, q| 8500.0 * (-2.3 * q * q).exp() + 84.0 / (q.powi(2) + 0.01),
                    1.0,
                    &[],
                    1.00,
                    "~1 mm (operator note)",
                );
                source
            },
        },
    );

    let mut buffers = BTreeMap::new();
    buffers.insert(
        "buffer-a-v1".to_string(),
        BufferRecord {
            version_of: "buffer-a".to_string(),
            curve: curve(
                "buffer-a-v1",
                "buffer",
                "Matching buffer v1",
                &baseline_q,
                |_, q| 900.0 * (-1.8 * q * q).exp() + 70.0 / (q.powi(2) + 0.012),
                1.0,
                &[17],
                1.00,
                "1.00 mm",
            ),
        },
    );
    buffers.insert(
        "buffer-a-v2".to_string(),
        BufferRecord {
            version_of: "buffer-a".to_string(),
            curve: {
                let mut source = curve(
                    "buffer-a-v2",
                    "buffer",
                    "Matching buffer v2 after refill",
                    &baseline_q,
                    |_, q| 990.0 * (-1.75 * q * q).exp() + 78.0 / (q.powi(2) + 0.011),
                    1.05,
                    &[17],
                    1.00,
                    "1.00 mm",
                );
                source.mask_source = "fixtures/case/mask-refill-v2.csv".to_string();
                source
            },
        },
    );
    buffers.insert(
        "buffer-noisy".to_string(),
        BufferRecord {
            version_of: "buffer-noise-series".to_string(),
            curve: curve(
                "buffer-noisy",
                "buffer",
                "Noisy buffer producing scale uncertainty",
                &baseline_q,
                |_, q| 430.0 + 24.0 / (q.powi(2) + 0.05),
                4.5,
                &[],
                1.00,
                "1.00 mm",
            ),
        },
    );

    let mut responses = BTreeMap::new();
    responses.insert(
        "detector-eiger-silver-behenate".to_string(),
        InstrumentResponse {
            id: "detector-eiger-silver-behenate".to_string(),
            label: "Eiger calibration against silver behenate".to_string(),
            scale_factor: 1.0,
            relative_uncertainty: 0.006,
            q_grid_basis: "integration-center-q-inverse-angstrom".to_string(),
            source: "fixtures/case/instrument-response.json".to_string(),
        },
    );

    let mut masks = BTreeMap::new();
    masks.insert(
        "mask-manual-v1".to_string(),
        Mask {
            id: "mask-manual-v1".to_string(),
            label: "Manual beamstop and detector edge mask v1".to_string(),
            source: "fixtures/case/mask-manual-v1.csv".to_string(),
        },
    );
    masks.insert(
        "mask-refill-v2".to_string(),
        Mask {
            id: "mask-refill-v2".to_string(),
            label: "Refill mask with changed beamstop edge".to_string(),
            source: "fixtures/case/mask-refill-v2.csv".to_string(),
        },
    );

    CaseBundle {
        case_id: "offline-saxs-reduction-evidence".to_string(),
        title: "散射曲线台：SAXS 归一化、背景扣除与多区间模型证据".to_string(),
        samples,
        buffers,
        responses,
        masks,
    }
}
