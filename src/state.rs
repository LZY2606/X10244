use crate::domain::{CaseBundle, ReductionRun};
use crate::events::*;
use crate::reductions::stable_hash;
use std::collections::{BTreeMap, BTreeSet};

pub fn replay(events: &[Event], at_event_id: Option<&str>) -> ReplayState {
    let ordered = topologically_ordered(events);
    let ordered = match at_event_id {
        None => ordered,
        Some(target) => ancestors_until(&ordered, target),
    };

    let mut bundle = None;
    let mut runs: Vec<ReductionRun> = Vec::new();
    let mut current_run_key = None;
    let mut selected_run_id = None;
    let mut rejected_run_ids = BTreeSet::new();
    let mut unresolved_conflict = None;
    let mut resolved_conflicts = Vec::new();
    let mut changed_inputs: Vec<String> = Vec::new();
    let mut affected_run_keys: BTreeSet<String> = BTreeSet::new();
    let mut interval_decisions: BTreeMap<(String, String), bool> = BTreeMap::new();
    let mut heads: BTreeSet<String> = BTreeSet::new();
    let mut id_set: BTreeSet<String> = BTreeSet::new();

    for event in &ordered {
        id_set.insert(event.id.clone());
        for parent in &event.parent_ids {
            heads.remove(parent);
        }
        heads.insert(event.id.clone());

        match &event.payload {
            EventPayload::CaseImported { bundle: imported } => {
                bundle = Some(imported.clone());
                changed_inputs.push("import".to_string());
            }
            EventPayload::ParameterRevised {
                curve_id,
                field,
                after,
                ..
            } => {
                if let Some(current) = bundle.as_mut() {
                    apply_parameter(current, curve_id, field, after);
                    changed_inputs.push(format!("parameter:{}:{}", curve_id, field));
                    for key in run_keys_for_curve(&runs, curve_id) {
                        affected_run_keys.insert(key);
                    }
                }
            }
            EventPayload::SourcePointPatched { patch } => {
                if let Some(current) = bundle.as_mut() {
                    apply_patch(current, patch);
                    changed_inputs.push(format!("point:{}:{}", patch.curve_id, patch.point_id));
                    for key in run_keys_for_curve(&runs, &patch.curve_id) {
                        affected_run_keys.insert(key);
                    }
                }
            }
            EventPayload::ReductionRecomputed { run } => {
                if let Some(position) = runs.iter().position(|existing| existing.id == run.id) {
                    runs[position] = run.clone();
                } else {
                    runs.push(run.clone());
                }
                affected_run_keys.remove(&run.key);
                if current_run_key.is_none() {
                    current_run_key = Some(run.key.clone());
                }
            }
            EventPayload::CandidateDecided {
                run_id,
                run_key,
                interval_id,
                accepted,
                ..
            } => {
                interval_decisions.insert((run_id.clone(), interval_id.clone()), *accepted);
                if let Some(index) = runs.iter().position(|run| run.id == *run_id) {
                    if let Some(interval) = runs[index]
                        .intervals
                        .iter_mut()
                        .find(|interval| interval.id == *interval_id)
                    {
                        interval.accepted = Some(*accepted);
                    }
                }
                let _ = run_key;
            }
            EventPayload::RunSelected {
                run_id,
                rejected_run_id,
                ..
            } => {
                if let Some(run) = runs.iter().find(|run| run.id == *run_id) {
                    current_run_key = Some(run.key.clone());
                    selected_run_id = Some(run_id.clone());
                }
                if let Some(rejected) = rejected_run_id {
                    rejected_run_ids.insert(rejected.clone());
                }
            }
            EventPayload::ConflictDetected { .. } => unresolved_conflict = Some(event.clone()),
            EventPayload::ConflictResolved {
                fork_id,
                winning_event_id,
                losing_event_id,
                ..
            } => {
                if unresolved_conflict
                    .as_ref()
                    .map(|event| match &event.payload {
                        EventPayload::ConflictDetected {
                            fork_id: existing, ..
                        } => existing == fork_id,
                        _ => false,
                    })
                    .unwrap_or(false)
                {
                    unresolved_conflict = None;
                }
                rejected_run_ids.insert(losing_event_id.clone());
                let _ = winning_event_id;
                resolved_conflicts.push(event.clone());
            }
        }
    }

    let _bundle_hash = bundle
        .as_ref()
        .map(stable_hash)
        .unwrap_or_else(|| "none".to_string());
    let visible_events = visible_event_set(&ordered);
    ReplayState {
        at_event_id: at_event_id
            .map(str::to_string)
            .or_else(|| ordered.last().map(|event| event.id.clone())),
        heads: heads
            .into_iter()
            .filter(|id| visible_events.contains(id))
            .collect(),
        bundle,
        runs,
        events: ordered
            .into_iter()
            .filter(|event| visible_events.contains(&event.id))
            .collect(),
        current_run_key,
        selected_run_id,
        rejected_run_ids: rejected_run_ids.into_iter().collect(),
        unresolved_conflict,
        resolved_conflicts,
        changed_inputs,
        affected_run_keys: affected_run_keys.into_iter().collect(),
        interval_decisions: interval_decisions
            .into_iter()
            .map(|((run, interval), accepted)| (run, interval, accepted))
            .collect(),
    }
}

fn topologically_ordered(events: &[Event]) -> Vec<Event> {
    let by_id: BTreeMap<&str, &Event> = events
        .iter()
        .map(|event| (event.id.as_str(), event))
        .collect();
    let mut permanent = BTreeSet::new();
    let mut temporary = BTreeSet::new();
    let mut ordered = Vec::new();

    fn visit<'a>(
        event: &'a Event,
        by_id: &BTreeMap<&'a str, &'a Event>,
        permanent: &mut BTreeSet<String>,
        temporary: &mut BTreeSet<String>,
        ordered: &mut Vec<Event>,
    ) {
        if permanent.contains(&event.id) {
            return;
        }
        if temporary.contains(&event.id) {
            return;
        }
        temporary.insert(event.id.clone());
        for parent in &event.parent_ids {
            if let Some(parent_event) = by_id.get(parent.as_str()) {
                visit(parent_event, by_id, permanent, temporary, ordered);
            }
        }
        temporary.remove(&event.id);
        permanent.insert(event.id.clone());
        ordered.push(event.clone());
    }

    for event in events.iter().rev() {
        visit(event, &by_id, &mut permanent, &mut temporary, &mut ordered);
    }
    ordered
}

fn ancestors_until(ordered: &[Event], target: &str) -> Vec<Event> {
    let wanted = ancestor_set(ordered, target);
    ordered
        .iter()
        .filter(|event| wanted.contains(&event.id))
        .cloned()
        .collect()
}

fn ancestor_set(ordered: &[Event], target: &str) -> BTreeSet<String> {
    let by_id: BTreeMap<&str, &Event> = ordered
        .iter()
        .map(|event| (event.id.as_str(), event))
        .collect();
    let mut wanted = BTreeSet::new();
    let mut stack = vec![target.to_string()];
    while let Some(id) = stack.pop() {
        if wanted.insert(id.clone()) {
            if let Some(event) = by_id.get(id.as_str()) {
                stack.extend(event.parent_ids.iter().cloned());
            }
        }
    }
    wanted
}

fn visible_event_set(ordered: &[Event]) -> BTreeSet<String> {
    let mut excluded = BTreeSet::new();
    for event in ordered {
        if let EventPayload::ConflictResolved {
            losing_event_id, ..
        } = &event.payload
        {
            let mut subtree = vec![losing_event_id.clone()];
            while let Some(id) = subtree.pop() {
                if excluded.insert(id.clone()) {
                    subtree.extend(ordered.iter().filter_map(|candidate| {
                        candidate
                            .parent_ids
                            .contains(&id)
                            .then(|| candidate.id.clone())
                    }));
                }
            }
        }
    }
    ordered
        .iter()
        .map(|event| event.id.clone())
        .filter(|id| !excluded.contains(id))
        .collect()
}

fn run_keys_for_curve(runs: &[ReductionRun], curve_id: &str) -> Vec<String> {
    runs.iter()
        .filter_map(|run| {
            let uses_sample = run.sample_id == curve_id;
            let uses_buffer = run.buffer_id == curve_id;
            (uses_sample || uses_buffer).then(|| run.key.clone())
        })
        .collect()
}

fn apply_parameter(
    bundle: &mut CaseBundle,
    curve_id: &str,
    field: &str,
    after: &serde_json::Value,
) {
    let mut curve = None;
    if let Some(record) = bundle.samples.get_mut(curve_id) {
        curve = Some(&mut record.curve);
    } else if let Some(record) = bundle.buffers.get_mut(curve_id) {
        curve = Some(&mut record.curve);
    }
    let Some(curve) = curve else { return };
    match field {
        "thickness_mm" => {
            if let Some(value) = after.as_f64() {
                curve.thickness.value = value;
                curve.thickness.unit = "mm".to_string();
                curve.thickness.raw_representation = format!("{value:.2} mm (revised)");
            }
        }
        "exposure_s" => {
            if let Some(value) = after.as_f64() {
                curve.exposure.value = value;
                curve.exposure.unit = "s".to_string();
                curve.exposure.raw_representation = format!("{value} s (revised)");
            }
        }
        "transmission" => {
            if let Some(value) = after.as_f64() {
                curve.transmission.value = value;
            }
        }
        _ => {}
    }
}

fn apply_patch(bundle: &mut CaseBundle, patch: &SourcePointPatch) {
    let mut point = None;
    if let Some(record) = bundle.samples.get_mut(&patch.curve_id) {
        point = record
            .curve
            .points
            .iter_mut()
            .find(|candidate| candidate.id == patch.point_id);
    } else if let Some(record) = bundle.buffers.get_mut(&patch.curve_id) {
        point = record
            .curve
            .points
            .iter_mut()
            .find(|candidate| candidate.id == patch.point_id);
    }
    let Some(point) = point else {
        return;
    };
    let value = patch.after.as_f64().unwrap_or_default();
    match patch.field.as_str() {
        "q" => point.q = value,
        "intensity" => point.intensity = value,
        "sigma" => point.sigma = value,
        "masked" => point.masked = patch.after.as_bool().unwrap_or(point.masked),
        _ => {}
    }
}

pub fn trace_point(state: &ReplayState, point_id: &str) -> Option<PointTrace> {
    for run in &state.runs {
        if let Some(point) = run
            .subtracted
            .iter()
            .find(|point| point.id == point_id || point.id_stable == point_id)
        {
            let sample_curve = state.bundle.as_ref().and_then(|bundle| {
                bundle
                    .samples
                    .get(&run.sample_id)
                    .map(|sample| &sample.curve)
                    .or_else(|| {
                        bundle
                            .buffers
                            .get(&run.sample_id)
                            .map(|buffer| &buffer.curve)
                    })
            });
            let buffer_curve = state.bundle.as_ref().and_then(|bundle| {
                bundle
                    .samples
                    .get(&run.buffer_id)
                    .map(|sample| &sample.curve)
                    .or_else(|| {
                        bundle
                            .buffers
                            .get(&run.buffer_id)
                            .map(|buffer| &buffer.curve)
                    })
            });
            let sample_point_id = point.sample_source.point_id.clone()?;
            let buffer_point_id = point.background_source.point_id.clone()?;
            let sample_raw = sample_curve.and_then(|curve| {
                curve.points.iter().find(|candidate| {
                    candidate.id == sample_point_id
                        || candidate.id == extract_raw_point(&sample_point_id)
                })
            });
            let buffer_raw = buffer_curve.and_then(|curve| {
                curve.points.iter().find(|candidate| {
                    candidate.id == buffer_point_id
                        || candidate.id == extract_raw_point(&buffer_point_id)
                })
            });
            return Some(PointTrace {
                derived_point_id: point.id.clone(),
                q: point.q,
                sample: source_view(
                    &run.sample_id,
                    &point.sample_source.version_id,
                    sample_raw,
                    None,
                ),
                background: source_view(
                    &run.buffer_id,
                    &point.background_source.version_id,
                    buffer_raw,
                    Some(point.background_source.point_id.clone().unwrap_or_default()),
                ),
            });
        }
    }
    None
}

fn extract_raw_point(derived_point_id: &str) -> String {
    derived_point_id
        .split(':')
        .next_back()
        .unwrap_or(derived_point_id)
        .to_string()
}

fn source_view(
    curve_id: &str,
    version: &str,
    raw: Option<&crate::domain::RawPoint>,
    interpolation_point: Option<String>,
) -> SourcePointView {
    SourcePointView {
        curve_id: curve_id.to_string(),
        curve_version: version.to_string(),
        point_id: raw
            .map(|point| point.id.clone())
            .unwrap_or_else(|| interpolation_point.unwrap_or_default()),
        q: raw.map(|point| point.q).unwrap_or_default(),
        intensity: raw.map(|point| point.intensity).unwrap_or_default(),
        sigma: raw.map(|point| point.sigma).unwrap_or_default(),
        masked: raw.map(|point| point.masked).unwrap_or(false),
        interpolation: None,
    }
}
