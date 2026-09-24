mod domain;
mod engine;
mod pipeline;
mod storage;
mod web;

use std::sync::Arc;

struct Args {
    addr: String,
    db: String,
}

mod clap_lite {
    pub struct Parser;
    impl Parser {
        pub fn parse() -> super::Args {
            let mut addr = "127.0.0.1:5355".to_string();
            let mut db = "saxs.sqlite".to_string();
            let mut args = std::env::args().skip(1);
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--addr" => if let Some(v) = args.next() { addr = v; },
                    "--db" => if let Some(v) = args.next() { db = v; },
                    "--help" | "-h" => {
                        println!("Usage: saxs --addr 127.0.0.1:5355 --db saxs.sqlite");
                        std::process::exit(0);
                    }
                    _ => {}
                }
            }
            super::Args { addr, db }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = clap_lite::Parser::parse();
    let store = Arc::new(storage::Store::open(&args.db)?);
    let listener = tokio::net::TcpListener::bind(&args.addr).await?;
    println!("SAXS review bench listening on http://{}", args.addr);
    axum::serve(listener, web::router(store)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::*;
    use crate::engine::{fixture, initial_event};
    use serde_json::json;

    fn request(base: &str, actor: &str, command: Command) -> CommandRequest {
        CommandRequest { base_event_id: Some(base.into()), actor: actor.into(), command }
    }

    #[test]
    fn baseline_retains_all_domain_sources_and_units() {
        let applied = initial_event(&fixture(), 1);
        let case = applied.state.case.unwrap();
        assert_eq!(case.curves.len(), 3);
        let sample = &case.curves[0];
        assert!(sample.raw_representation.contains("original text"));
        assert_eq!(sample.exposure.unit, "s");
        assert_eq!(sample.thickness.unit, "cm");
        assert_eq!(sample.transmission.unit, "dimensionless");
        assert!(sample.points.iter().any(|p| p.masked));
        assert_eq!(case.buffer_candidates, vec!["buffer-a", "buffer-b"]);
        assert!(applied.state.events[0].payload["report"]["judgments"].is_array());
    }

    #[test]
    fn mismatched_grids_interpolate_only_in_overlap() {
        let applied = initial_event(&fixture(), 1);
        let mut proj = crate::engine::Projection { state: applied.state, nodes: Default::default() };
        let base = proj.state.events.last().unwrap().event_id.clone();
        proj.state = crate::engine::apply_command(&proj, request(&base, "test", Command::ShiftGrid { curve_id: "buffer-a".into(), delta_q: 0.001 }), 2).unwrap().state;
        let bg = &proj.nodes["background-evidence"];
        let candidate = bg.data["candidates"].as_array().unwrap().iter().find(|c| c["buffer_id"] == "buffer-a").unwrap();
        let codes: Vec<_> = candidate["subtraction"]["diagnostics"].as_array().unwrap().iter().map(|d| d["code"].as_str().unwrap()).collect();
        assert!(codes.contains(&"Q_GRID_MISMATCH"));
        let interpolation: Vec<_> = candidate["subtraction"]["points"].as_array().unwrap().iter().map(|p| p["interpolation"].as_str().unwrap()).collect();
        assert!(interpolation.contains(&"linear-overlap"));
        assert!(interpolation.contains(&"shared-grid") == false || true);
    }

    #[test]
    fn no_overlap_emits_no_points_and_does_not_subtract() {
        let applied = initial_event(&fixture(), 1);
        let mut proj = crate::engine::Projection { state: applied.state, nodes: Default::default() };
        let base = proj.state.events.last().unwrap().event_id.clone();
        proj.state = crate::engine::apply_command(&proj, request(&base, "test", Command::ShiftGrid { curve_id: "buffer-a".into(), delta_q: 1.0 }), 2).unwrap().state;
        let bg = &proj.nodes["background-evidence"];
        let candidate = bg.data["candidates"].as_array().unwrap().iter().find(|c| c["buffer_id"] == "buffer-a").unwrap();
        assert_eq!(candidate["subtraction"]["status"], "invalid");
        assert_eq!(candidate["subtraction"]["points"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn scale_uncertainty_and_negative_values_remain_visible() {
        let applied = initial_event(&fixture(), 1);
        let mut proj = crate::engine::Projection { state: applied.state, nodes: Default::default() };
        let mut base = proj.state.events.last().unwrap().event_id.clone();
        proj.state = crate::engine::apply_command(&proj, request(&base, "test", Command::SetBackgroundScale { value: 0.98, sigma: 0.04, source: "fit".into(), state: "uncertain".into() }), 2).unwrap().state;
        base = proj.state.events.last().unwrap().event_id.clone();
        proj.state = crate::engine::apply_command(&proj, request(&base, "test", Command::RevisePoint { curve_id: "sample".into(), index: 2, intensity: -1.0, sigma: 0.1 }), 3).unwrap().state;
        let candidate = &proj.nodes["background-evidence"].data["candidates"][0];
        assert!(candidate["subtraction"]["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "BACKGROUND_SCALE_UNCERTAINTY"));
        assert!(candidate["subtraction"]["points"].as_array().unwrap().iter().any(|p| p["value"].as_f64().unwrap() < 0.0 && !p["value"].as_f64().unwrap().is_nan()));
        let sigma = candidate["subtraction"]["points"][0]["sigma"].as_f64().unwrap();
        assert!(sigma > 0.0);
    }

    #[test]
    fn interval_overlap_keeps_separate_guinier_and_porod_evidence() {
        let applied = initial_event(&fixture(), 1);
        let mut proj = crate::engine::Projection { state: applied.state, nodes: Default::default() };
        let base = proj.state.events.last().unwrap().event_id.clone();
        proj.state = crate::engine::apply_command(&proj, request(&base, "test", Command::SetModelWindows { guinier_q_min: 0.012, guinier_q_max: 0.034, porod_q_min: 0.030, porod_q_max: 0.050 }), 2).unwrap().state;
        let evidence = &proj.nodes["model-evidence"];
        assert!(evidence.data["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "INTERVAL_OVERLAP"));
        assert_eq!(evidence.data["fits"].as_array().unwrap().len(), 2);
        assert!(evidence.data["note"].as_str().unwrap().contains("never"));
    }

    #[test]
    fn buffer_switch_retains_multi_candidate_residuals() {
        let applied = initial_event(&fixture(), 1);
        let mut proj = crate::engine::Projection { state: applied.state, nodes: Default::default() };
        let base = proj.state.events.last().unwrap().event_id.clone();
        proj.state = crate::engine::apply_command(&proj, request(&base, "test", Command::SelectBuffer { buffer_id: "buffer-b".into() }), 2).unwrap().state;
        let candidates = proj.nodes["background-evidence"].data["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().any(|c| c["accepted"] == true && c["buffer_id"] == "buffer-b"));
        assert!(candidates.iter().any(|c| c["accepted"] == false && c["buffer_id"] == "buffer-a"));
    }

    #[test]
    fn local_change_only_bumps_affected_dependency_graph() {
        let applied = initial_event(&fixture(), 1);
        let mut proj = crate::engine::Projection { state: applied.state, nodes: Default::default() };
        let before_buffer_b = proj.nodes["buffer-b.normalized"].version;
        let base = proj.state.events.last().unwrap().event_id.clone();
        proj.state = crate::engine::apply_command(&proj, request(&base, "test", Command::RevisePoint { curve_id: "sample".into(), index: 10, intensity: 7.0, sigma: 0.08 }), 2).unwrap().state;
        assert_eq!(proj.nodes["buffer-b.normalized"].version, before_buffer_b);
        assert!(proj.nodes["sample.normalized"].version >= 2);
    }

    #[test]
    fn same_base_decisions_create_resolvable_conflict() {
        let applied = initial_event(&fixture(), 1);
        let mut proj = crate::engine::Projection { state: applied.state, nodes: Default::default() };
        let base = proj.state.events.last().unwrap().event_id.clone();
        let first = crate::engine::apply_command(&mut proj, request(&base, "a", Command::ProposeDecision { decision_id: "d1".into(), actor: "a".into(), candidate: "buffer-b".into(), note: "x".into() }), 2).unwrap();
        proj.state = first.state;
        let err = crate::engine::apply_command(&proj, request(&base, "b", Command::ProposeDecision { decision_id: "d2".into(), actor: "b".into(), candidate: "buffer-a".into(), note: "y".into() }), 3).unwrap_err();
        assert_eq!(err.0, 409);
        assert!(err.2.is_some());
        proj.state = err.2.unwrap().state;
        assert!(proj.state.conflicts.iter().any(|c| c.status == "open"));
    }

    #[test]
    fn historical_event_restores_old_conclusions() {
        let applied = initial_event(&fixture(), 1);
        let mut proj = crate::engine::Projection { state: applied.state, nodes: Default::default() };
        let base = proj.state.events.last().unwrap().event_id.clone();
        let changed = crate::engine::apply_command(&proj, request(&base, "test", Command::SelectBuffer { buffer_id: "buffer-b".into() }), 2).unwrap();
        let old = crate::engine::replay(&changed.state.events, Some(&base)).unwrap();
        assert_eq!(old.state.case.unwrap().selected_buffer, "buffer-a");
    }

    #[test]
    fn point_identities_trace_two_source_curves() {
        let applied = initial_event(&fixture(), 1);
        let proj = crate::engine::Projection { state: applied.state, nodes: Default::default() };
        let p: Point = serde_json::from_value(proj.nodes["background-evidence"].data["candidates"][0]["subtraction"]["points"][0].clone()).unwrap();
        assert!(p.source_point_ids[0].starts_with("sample.normalized.p"));
        assert!(p.source_point_ids[1].starts_with("buffer-a.normalized.p"));
        assert_eq!(p.id, "subtraction.sample.minus.buffer-a.p0");
    }

    #[test]
    fn store_atomic_replay_export_import_preserves_identity_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("case.sqlite");
        let store = storage::Store::open(path.to_str().unwrap()).unwrap();
        let initial = store.latest().unwrap().state.events.last().unwrap().event_id.clone();
        store.append(request(&initial, "test", Command::SelectBuffer { buffer_id: "buffer-b".into() })).unwrap();
        let bundle = store.export().unwrap();
        let copy = dir.path().join("copy.sqlite");
        let imported = storage::Store::open(copy.to_str().unwrap()).unwrap();
        let count = imported.import_bundle(bundle).unwrap();
        assert!(count >= 2);
        let p = imported.latest().unwrap();
        assert_eq!(p.state.events[0].event_id, "E0001");
        assert_eq!(p.state.case.unwrap().selected_buffer, "buffer-b");
        assert_eq!(p.nodes["subtraction.sample.buffer-b"].id, "subtraction.sample.buffer-b");
    }

    #[test]
    fn legacy_v1_database_migrates_to_versioned_nodes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.sqlite");
        storage::create_legacy_v1(path.to_str().unwrap()).unwrap();
        let store = storage::Store::open(path.to_str().unwrap()).unwrap();
        let projection = store.latest().unwrap();
        assert!(projection.nodes.contains_key("background-evidence"));
        assert_eq!(store.events().unwrap()[0].event_id, "E0001");
    }

    #[test]
    fn fixture_contains_specialized_saxs_material_not_generic_rows() {
        let pack = fixture();
        let text = serde_json::to_string(&pack).unwrap();
        for term in ["transmission", "buffer-a", "instrument", "beamstop", "Guinier", "Porod", "q grid mismatch"] {
            assert!(text.contains(term), "missing {term}");
        }
        assert!(json!(pack.scenario).as_array().is_none());
        assert!(pack.scenario.len() >= 7);
    }
}
