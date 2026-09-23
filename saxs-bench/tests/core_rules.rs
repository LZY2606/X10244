//! Tests for the fixed SAXS semantics. All data comes from the in-repo
//! deterministic fixture builder (`saxs_bench::case`) — no network/table.

use saxs_bench::case::*;
use saxs_bench::events::{fold, EventKind};
use saxs_bench::model::{IntervalKind, Role, Severity};
use saxs_bench::replay::script;
use saxs_bench::saxs::*;

fn head_state() -> saxs_bench::events::State {
    let kinds = base_event_kinds(0);
    let events = build_events(kinds, 1_700_000_000_000);
    fold(&events, None)
}

fn branch_for(st: &saxs_bench::events::State, buffer_id: &str) -> &saxs_bench::model::SubBranch {
    let norm = st
        .norms
        .iter()
        .find(|n| n.source_profile == buffer_id)
        .unwrap();
    st.branches
        .iter()
        .find(|b| b.buffer_norm == norm.id && b.fit_lo == 0.02)
        .unwrap()
}

#[test]
fn raw_material_is_fully_retained() {
    let s = sample_profile(0.012);
    // masked points are present in the raw record
    assert!(s.points.iter().any(|p| p.masked));
    // original textual representations preserved
    assert!(!s.points[1].q_raw.is_empty());
    assert_eq!(s.transmission.raw, "0.42");
    assert_eq!(s.exposure.unit, "s");
    assert_eq!(s.instrument_response.reference, "resp/cal_20260910.json");
    assert!(s.mask_source.masked_count >= 2);
}

#[test]
fn normalization_divides_by_transmission_times_exposure_and_converts_q() {
    let mut p = sample_profile(0.0);
    p.q_basis_raw = "1/nm".into();
    use saxs_bench::units::QBasis;
    p.q_basis = Some(QBasis::QInvNm);
    let n = normalize(&p);
    let raw_first = p.points.iter().find(|p| !p.masked).unwrap();
    let out = n.points.iter().find(|p| p.source_index == raw_first.index).unwrap();
    let expected_i = raw_first.i.unwrap() / (0.42 * 300.0);
    assert!((out.i - expected_i).abs() < 1e-9);
    // 1 nm^-1 = 0.1 Å^-1
    assert!((out.q - raw_first.q.unwrap() * 0.1).abs() < 1e-12);
}

#[test]
fn unconvertible_value_stays_in_place_and_blocks_normalization() {
    let mut p = sample_profile(0.0);
    p.transmission.unit = "absorbance".into(); // not a dimensionless factor
    let n = normalize(&p);
    assert!(n.points.is_empty());
    assert!(n
        .judgments
        .iter()
        .any(|j| j.rule_id == "norm.transmission_unconvertible" && j.severity == Severity::Blocked));
    // raw value retained
    assert_eq!(p.transmission.raw, "0.42");
}

#[test]
fn differing_grids_never_subtract_pointwise_and_overlap_only() {
    let st = head_state();
    let b = branch_for(&st, BUF_V1);
    assert!(!b.blocked);
    // grids differ in length (60 usable vs 70)
    assert!(b
        .judgments
        .iter()
        .any(|j| j.rule_id == "grid.misaligned"));
    // every subtracted point lies within the overlap; some sample points near
    // the sample-only edge must be outside overlap and carry None
    let outside = b.points.iter().filter(|p| !p.in_overlap).count();
    assert!(outside >= 1, "sample-only q region must be outside overlap");
    for p in &b.points {
        if p.in_overlap {
            assert!(p.i_sub.is_some());
            assert!(p.source_buffer_lo.is_some());
        } else {
            assert!(p.i_sub.is_none());
            assert!(p.buffer_i.is_none());
        }
    }
}

#[test]
fn no_overlap_blocks_subtraction_entirely() {
    let st = head_state();
    let b = branch_for(&st, BUF_NOOVERLAP);
    assert!(b.blocked);
    assert!(b.judgments.iter().any(|j| j.rule_id == "grid.no_overlap"));
    assert!(b.points.iter().all(|p| p.i_sub.is_none()));
    assert_eq!(b.overlap_points, 0);
}

#[test]
fn uncertainty_includes_normalization_and_background_scale() {
    let st = head_state();
    let b = branch_for(&st, BUF_V1);
    let p = b.points.iter().find(|p| p.in_overlap).unwrap();
    // sigma_total must exceed pure sample statistical sigma once scale and
    // normalisation variance are added.
    let sample = st.norm(&b.sample_norm).unwrap();
    let sp = sample.points.iter().find(|x| x.source_index == p.source_sample_index).unwrap();
    assert!(p.sigma_total.unwrap() > sp.sigma_stat);
    assert!(b.scale_sigma.unwrap() > 0.0);
}

#[test]
fn negative_intensities_are_retained_not_clipped() {
    let st = head_state();
    let b = branch_for(&st, BUF_V1);
    let negs = b.points.iter().filter(|p| matches!(p.i_sub, Some(v) if v < 0.0)).count();
    assert!(negs >= 1, "fixture dip must yield a retained negative point");
    assert!(b.judgments.iter().any(|j| j.rule_id == "sub.negative_intensity"));
}

#[test]
fn guinier_and_porod_are_separate_never_stitched() {
    let st = head_state();
    let b = branch_for(&st, BUF_V1);
    let g: Vec<_> = b.intervals.iter().filter(|c| c.kind == IntervalKind::Guinier).collect();
    let por: Vec<_> = b.intervals.iter().filter(|c| c.kind == IntervalKind::Porod).collect();
    assert!(!g.is_empty());
    assert!(!por.is_empty());
    // there is no combined/stitched model entity in the data model; both
    // families coexist as independent candidates.
    assert!(b.judgments.iter().any(|j| j.rule_id == "model.intervals_separate"));
}

#[test]
fn multiple_legal_candidates_are_all_kept() {
    let st = head_state();
    let b = branch_for(&st, BUF_V1);
    let valid_g = b.intervals.iter().filter(|c| c.kind == IntervalKind::Guinier && c.valid).count();
    assert!(valid_g >= 1, "multiple window lengths produce competing candidates");
}

#[test]
fn interpolation_is_linear_within_overlap() {
    let qb = [0.0, 1.0, 2.0, 3.0];
    let yb = [10.0, 20.0, 30.0, 40.0];
    let (y, lo, hi) = interp_within(1.5, &qb, &yb, 0.0, 3.0).unwrap();
    assert_eq!((lo, hi), (1, 2));
    assert!((y - 25.0).abs() < 1e-12);
    assert!(interp_within(3.5, &qb, &yb, 0.0, 3.0).is_none());
}

#[test]
fn fit_scale_is_positive_and_weighted() {
    let st = head_state();
    let s = st.norms.iter().find(|n| n.source_profile == SAMPLE).unwrap();
    let bf = st.norms.iter().find(|n| n.source_profile == BUF_V1).unwrap();
    let (k, sk, _, _, n) = fit_scale(s, bf, 0.02, 0.35).unwrap();
    assert!(k > 0.0);
    assert!(sk > 0.0);
    assert!(n >= 5);
}

#[test]
fn replay_traverses_all_seven_boundary_scenarios() {
    let steps = script(0, 0);
    let kinds: Vec<EventKind> = steps.iter().map(|s| s.kind.clone()).collect();
    let events = build_events(kinds, 1_700_000_000_000);
    let st = fold(&events, None);

    // buffer version switch: both v1 and v2 yield branches
    let nbuf = |id: &str| st.norms.iter().find(|n| n.source_profile == id).unwrap().id.clone();
    let v1 = nbuf(BUF_V1);
    let v2 = nbuf(BUF_V2);
    assert!(st.branches.iter().any(|b| b.buffer_norm == v1));
    assert!(st.branches.iter().any(|b| b.buffer_norm == v2));

    // no-overlap branch present and blocked
    assert!(st.branches.iter().any(|b| b.buffer_norm == nbuf(BUF_NOOVERLAP) && b.blocked));

    // at least one branch evidences the grid/negative/scale rules
    let all_rules: std::collections::HashSet<_> = st
        .branches
        .iter()
        .flat_map(|b| b.judgments.iter().map(|j| j.rule_id.clone()))
        .collect();
    for r in ["grid.misaligned", "grid.no_overlap", "sub.negative_intensity", "model.intervals_separate"] {
        assert!(all_rules.contains(r), "missing {r}");
    }
    // scale uncertainty is represented (possibly ok or warn depending window)
    assert!(all_rules.contains("bg.scale_uncertain"));

    // both human decisions retained (accept v2 latest, reject v1 first)
    assert_eq!(st.decisions.len(), 2);
}

#[test]
fn sample_has_sample_role() {
    assert_eq!(sample_profile(0.0).role, Role::Sample);
}
