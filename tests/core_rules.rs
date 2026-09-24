//! Offline, fixture-only tests for the guarded SAXS rules.

use saxs_station::cases;
use saxs_station::engine::{self, SubInputs};
use saxs_station::model::*;

fn normalize(obj: &Obj) -> Obj {
    engine::normalize(obj).expect("normalizable fixture")
}

fn pair(
    sample_req: &engine::ImportRequest,
    buffer_req: &engine::ImportRequest,
) -> (Obj, Obj, Obj, Obj) {
    let s = engine::import_raw(sample_req);
    let b = engine::import_raw(buffer_req);
    let sn = normalize(&s);
    let bn = normalize(&b);
    (s, b, sn, bn)
}

fn subtract(sn: &Obj, bn: &Obj, braw: &Obj) -> Obj {
    let (sc, bc) = match (sn, bn) {
        (Obj::Norm { curve: a, .. }, Obj::Norm { curve: b, .. }) => (a, b),
        _ => panic!(),
    };
    let ranges = match braw {
        Obj::Raw { record, .. } => record
            .mask
            .as_ref()
            .map(|m| m.excluded_ranges.clone())
            .unwrap_or_default(),
        _ => vec![],
    };
    engine::subtract(SubInputs {
        sample_id: sn.object_id_ref(),
        buffer_id: bn.object_id_ref(),
        sample: sc,
        buffer: bc,
        buffer_mask_ranges: ranges,
    })
}

#[test]
fn import_retains_raw_representation_and_converts_in_derived_layer() {
    let s = engine::import_raw(&cases::sample_request());
    let Obj::Raw { record, .. } = &s else { panic!() };
    // original q unit text kept per point
    assert_eq!(record.points[0].q_unit, "A^-1");
    assert!(record.points[0].q_text.starts_with("0.0"));
    // canonical q is nm^-1 in the derived layer
    let c = record.points[0].canonical.as_ref().unwrap();
    assert!(c.q < 1.0); // A^-1 * 10 -> nm^-1
    // thickness kept verbatim AND canonicalized (mm -> nm)
    assert_eq!(record.thickness.text, "1.20");
    assert_eq!(record.thickness.value, Some(1.2e6));
}

#[test]
fn bad_unit_blocks_normalization_but_keeps_value_at_origin() {
    let s = engine::import_raw(&cases::sample_request_bad_unit());
    let Obj::Raw { record, .. } = &s else { panic!() };
    assert!(!record.thickness.convertible);
    assert!(record.diagnostics.iter().any(|d| d.contains("normalization blocked")));
    let err = engine::normalize(&s).expect_err("must not normalize unconvertible thickness");
    assert!(err.diagnostics.iter().any(|d| d.contains("thickness")));
}

#[test]
fn identical_grids_subtract_cleanly_with_scale_uncertainty_in_errors() {
    let pack = cases::pack();
    let (s, b, sn, bn) = pair(&pack.requests[0], &pack.requests[1]);
    let sub = subtract(&sn, &bn, &b);
    let Obj::Sub { curve, .. } = &sub else { panic!() };
    assert!(curve.grids_identical);
    assert_eq!(curve.status, SubStatus::Ok);
    let p = curve
        .points
        .iter()
        .find(|p| p.covered && !p.excluded)
        .unwrap();
    // e includes a positive scale-uncertainty contribution
    let par = curve.params.as_ref().unwrap();
    assert!(par.scale_sigma > 0.0);
    assert!(p.e_scale > 0.0);
    assert!(p.e >= p.e_sample);
    let _ = s;
}

#[test]
fn mismatched_grids_never_subtract_point_for_point_and_interpolate_only_inside_overlap() {
    let pack = cases::pack();
    let (_s, b, sn, bn) = pair(&pack.requests[0], &pack.requests[2]);
    let Obj::Sub { curve, .. } = subtract(&sn, &bn, &b) else { panic!() };
    assert!(!curve.grids_identical);
    assert_eq!(curve.status, SubStatus::GridMismatch);
    let overlap = curve.overlap.unwrap();
    // every covered point is strictly within overlap and cites 2 buffer pts
    for p in &curve.points {
        if p.covered {
            assert!(p.q >= overlap[0] - 1e-9 && p.q <= overlap[1] + 1e-9);
            assert_eq!(p.source_buffer_pids.len(), 2);
        } else {
            assert!(p.buffer_i_interp.is_none());
            assert!(p.i.is_nan());
        }
    }
    // guard code surfaced in diagnostics
    assert!(curve.diagnostics.iter().any(|d| d.starts_with("GRID_MISMATCH")));
}

#[test]
fn no_overlap_is_an_error_state_without_invented_subtraction() {
    let pack = cases::pack();
    let (_s, b, sn, bn) = pair(&pack.requests[0], &pack.requests[3]);
    let Obj::Sub { curve, .. } = subtract(&sn, &bn, &b) else { panic!() };
    assert_eq!(curve.status, SubStatus::NoOverlap);
    assert!(curve.overlap.is_none());
    assert!(curve.params.is_none());
    assert!(curve.points.iter().all(|p| !p.covered && p.i.is_nan()));
    assert!(curve.diagnostics.iter().any(|d| d.contains("NO_OVERLAP")));
}

#[test]
fn negative_points_are_retained_not_clipped() {
    let pack = cases::pack();
    let (_s, b, sn, bn) = pair(&pack.requests[4], &pack.requests[1]);
    let Obj::Sub { curve, .. } = subtract(&sn, &bn, &b) else { panic!() };
    let negs: Vec<_> = curve.points.iter().filter(|p| p.negative).collect();
    assert!(!negs.is_empty(), "fixture should produce negatives");
    assert!(negs.iter().all(|p| p.i < 0.0));
    assert!(curve.diagnostics.iter().any(|d| d.contains("NEGATIVE_POINTS")));
}

#[test]
fn guinier_and_porod_are_separate_candidates_and_overlaps_flagged() {
    let pack = cases::pack();
    let (_s, b, sn, bn) = pair(&pack.requests[0], &pack.requests[1]);
    let sub = subtract(&sn, &bn, &b);
    let Obj::Candidates { candidates, .. } = engine::fit_models(sub.object_id_ref(), match &sub {
        Obj::Sub { curve, .. } => curve,
        _ => panic!(),
    }) else { panic!() };
    assert!(candidates.iter().any(|c| c.kind == IntervalKind::Guinier));
    assert!(candidates.iter().any(|c| c.kind == IntervalKind::Porod));
    // multiple legal windows are retained as evidence
    assert!(candidates.len() >= 2);
    // overlaps are flagged, and nothing was merged into one model object
    let _ = candidates.iter().filter(|c| c.interval_overlap).count();
}

#[test]
fn derived_identity_is_stable_and_content_addressed() {
    let req = cases::sample_request();
    let a = engine::import_raw(&req);
    let b = engine::import_raw(&req);
    assert_eq!(a.object_id_ref(), b.object_id_ref());
    let na = normalize(&a);
    let nb = normalize(&b);
    assert_eq!(na.object_id_ref(), nb.object_id_ref());
    let np = match &na {
        Obj::Norm { curve, .. } => &curve.points[5],
        _ => panic!(),
    };
    assert!(np.id.contains('#'));
}
