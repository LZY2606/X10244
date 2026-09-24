use saxs_station::domain::*;
use saxs_station::rules::*;

fn rp(q: f64, u: &str, i: f64, sig: f64) -> RawPoint {
    RawPoint { q, q_unit: u.into(), i, sigma: sig }
}

#[test]
fn normalize_uses_exposure_thickness_transmission_and_keeps_uncertainty() {
    let raw = RawCurve {
        points: vec![rp(0.01, "1/A", 100.0, 2.0), rp(0.02, "1/A", 50.0, 1.0)],
        exposure_s: 2.0,
        thickness_cm: 0.1,
        transmission: 0.5,
        description: "t".into(),
    };
    let (pts, diags) = normalize(&raw, &NormParams { scale_uncertainty: 0.05 });
    assert!(diags.is_empty());
    let factor = 2.0 * 0.1 * 0.5; // 0.1
    assert!((pts[0].i - 1000.0).abs() < 1e-9);
    let count = 2.0 / factor;
    let norm = 0.05_f64 * 1000.0;
    assert!((pts[0].sigma - (count * count + norm * norm).sqrt()).abs() < 1e-6);
}

#[test]
fn nm_units_convert_and_unknown_units_stay_in_place_with_diagnostic() {
    let raw = RawCurve {
        points: vec![rp(0.01, "1/nm", 10.0, 1.0), rp(7.0, "detector-pixel", 9.0, 1.0)],
        exposure_s: 1.0,
        thickness_cm: 1.0,
        transmission: 1.0,
        description: "t".into(),
    };
    let (pts, diags) = normalize(&raw, &NormParams { scale_uncertainty: 0.0 });
    assert!((pts[0].q - 0.1).abs() < 1e-12);
    assert!(!pts[1].convertible);
    assert!(pts[1].i.is_nan(), "无法转换的点保留在原位置，不为了画图丢弃");
    assert_eq!(diags[0].code, "unit_unconvertible");
}

fn norm_curve(qs: &[f64], is: &[f64], sigmas: &[f64]) -> Vec<NormPoint> {
    qs.iter()
        .zip(is.iter().zip(sigmas.iter()))
        .enumerate()
        .map(|(idx, (q, (i, s)))| NormPoint { q: *q, i: *i, sigma: *s, convertible: true, source_index: idx })
        .collect()
}

#[test]
fn identical_grids_subtract_directly_with_full_error_budget() {
    let q = [0.01, 0.02, 0.03, 0.04];
    let s = norm_curve(&q, &[10.0, 8.0, 6.0, 5.0], &[0.1, 0.1, 0.1, 0.1]);
    let b = norm_curve(&q, &[2.0, 2.0, 2.0, 2.0], &[0.05, 0.05, 0.05, 0.05]);
    let out = subtract(&s, &b, &SubParams { scale: 1.0, scale_sigma: 0.03 });
    assert_eq!(out.mode, "direct");
    assert!((out.points[0].i - 8.0).abs() < 1e-12);
    // 误差包含归一化(已含)与背景缩放不确定度 (scale_sigma * I_b)^2
    let expect = (0.1_f64.powi(2) + 0.05_f64.powi(2) + (0.03_f64 * 2.0).powi(2)).sqrt();
    assert!((out.points[0].sigma - expect).abs() < 1e-10);
}

#[test]
fn mismatched_grids_interpolate_only_within_overlap_and_never_subtract_directly() {
    let q1 = [0.01, 0.02, 0.03, 0.04];
    let q2 = [0.0104, 0.0204, 0.0304, 0.0404];
    let s = norm_curve(&q1, &[10.0, 8.0, 6.0, 4.0], &[0.0, 0.0, 0.0, 0.0]);
    let b = norm_curve(&q2, &[2.0, 2.0, 2.0, 2.0], &[0.0, 0.0, 0.0, 0.0]);
    let out = subtract(&s, &b, &SubParams { scale: 1.0, scale_sigma: 0.0 });
    assert_eq!(out.mode, "interpolated");
    assert!(out.diagnostics.iter().any(|d| d.code == "q_grid_mismatch"));
    // 重叠范围 [0.0104, 0.04]：样品 q=0.01 落在重叠外，被排除
    assert_eq!(out.points[0].q, 0.02);
    assert!(out.overlap.unwrap().0 > 0.0103);
}

#[test]
fn no_overlap_region_yields_no_curve() {
    let s = norm_curve(&[0.01, 0.02], &[10.0, 9.0], &[0.1, 0.1]);
    let b = norm_curve(&[0.6, 0.7, 0.8], &[2.0, 2.0, 2.0], &[0.1, 0.1, 0.1]);
    let out = subtract(&s, &b, &SubParams { scale: 1.0, scale_sigma: 0.0 });
    assert_eq!(out.mode, "none");
    assert!(out.points.is_empty());
    assert!(out.diagnostics.iter().any(|d| d.code == "no_overlap"));
}

#[test]
fn negative_intensities_are_kept_and_flagged_not_clipped() {
    let q = [0.01, 0.02, 0.03];
    let s = norm_curve(&q, &[1.0, 1.0, 1.0], &[0.01, 0.01, 0.01]);
    let b = norm_curve(&q, &[1.1, 1.1, 1.1], &[0.01, 0.01, 0.01]);
    let out = subtract(&s, &b, &SubParams { scale: 1.0, scale_sigma: 0.0 });
    assert!(out.points.iter().all(|p| p.i < 0.0), "负值保留");
    assert_eq!(out.diagnostics.iter().filter(|d| d.code == "negative_intensity").count(), 3);
}

#[test]
fn all_scale_candidates_are_kept_and_ambiguity_is_reported() {
    let q: Vec<f64> = (0..20).map(|i| 0.01 + i as f64 * 0.01).collect();
    let si: Vec<f64> = (0..20).map(|i| 5.0 + (i as f64 * 1.3).sin()).collect();
    let s = norm_curve(&q, &si, &vec![1.0; 20]);
    let b = norm_curve(&q, &vec![5.0; 20], &vec![1.0; 20]);
    let out = subtract(&s, &b, &SubParams { scale: 1.0, scale_sigma: 0.03 });
    assert_eq!(out.candidates.len(), 3, "scale±σ 三个候选全部保留");
    assert!(out.candidates.iter().any(|c| c.adopted));
    assert!(out.diagnostics.iter().any(|d| d.code == "ambiguous_candidates"));
}

#[test]
fn guinier_and_porod_overlap_are_both_kept_and_never_merged() {
    let q: Vec<f64> = (0..60).map(|i| 0.006 + i as f64 * 0.008).collect();
    // Guinier: I = I0 exp(-q^2 Rg^2/3), Rg=20, I0=1000；高 q 加 Porod 尾巴
    let is: Vec<f64> = q
        .iter()
        .map(|q| 1000.0 * (-q * q * 400.0 / 3.0).exp() + 1e-9 / q.powi(4))
        .collect();
    let sig = vec![1e-6; q.len()];
    let pts: Vec<SubPoint> = q
        .iter()
        .zip(is.iter())
        .enumerate()
        .map(|(idx, (q, i))| SubPoint { q: *q, i: *i, sigma: sig[idx], sample_index: idx, buffer_index: Some(idx) })
        .collect();
    // 重叠区间：两个拟合均保留，但不会自动拼成单一模型
    let specs = vec![
        IntervalSpec { kind: "guinier".into(), q_min: 0.006, q_max: 0.03 },
        IntervalSpec { kind: "porod".into(), q_min: 0.025, q_max: 0.4 },
    ];
    let (fits, diags) = fit_intervals(&pts, &specs);
    assert_eq!(fits.len(), 2, "重叠区间两个拟合都保留");
    assert!(diags.iter().any(|d| d.code == "interval_overlap"));
    assert!(diags.iter().any(|d| d.code == "intervals_not_merged"));
    let rg = fits[0].params.iter().find(|(k, _)| k == "Rg_A").map(|(_, v)| *v).unwrap();
    assert!((rg - 20.0).abs() < 0.5, "Rg 恢复 ~20Å，实际 {rg}");

    // 物理参数在不重叠的干净区间分别恢复（Guinier 与 Porod 是并列证据）
    let clean = vec![
        IntervalSpec { kind: "guinier".into(), q_min: 0.006, q_max: 0.02 },
        IntervalSpec { kind: "porod".into(), q_min: 0.15, q_max: 0.4 },
    ];
    let (fits2, _) = fit_intervals(&pts, &clean);
    let exp = fits2[1].params.iter().find(|(k, _)| k == "exponent").map(|(_, v)| *v).unwrap();
    assert!((exp - (-4.0)).abs() < 0.2, "Porod 指数 ~-4，实际 {exp}");
}

#[test]
fn debug_porod() {
    let q: Vec<f64> = (0..60).map(|i| 0.006 + i as f64 * 0.008).collect();
    let is: Vec<f64> = q.iter().map(|q| 1000.0 * (-q * q * 400.0 / 3.0).exp() + 1e-9 / q.powi(4)).collect();
    eprintln!("qmax={} n={}", q.last().unwrap(), q.len());
    let pts: Vec<SubPoint> = q.iter().zip(is.iter()).enumerate()
        .map(|(idx, (q, i))| SubPoint { q: *q, i: *i, sigma: 1e-9, sample_index: idx, buffer_index: Some(idx) }).collect();
    let (fits, diags) = fit_intervals(&pts, &[IntervalSpec { kind: "porod".into(), q_min: 0.15, q_max: 0.4 }]);
    eprintln!("fits={fits:?} diags={diags:?}");
}
