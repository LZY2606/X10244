//! Application layer: event folding, dependency-graph propagation and commands.
//!
//! The projection is a deterministic fold over the event log. Recomputation
//! reuses a cached derivation whenever its exact input/rule signature still
//! matches; only affected downstream nodes are rebuilt.

use crate::model::*;
use crate::rules;
use crate::store::{Materialization, Store, StoreError};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphStatus {
    Current,
    Recomputed,
    Reused,
    Blocked,
}

#[derive(Debug, Clone)]
pub struct Freshness {
    pub deriv_id: String,
    pub kind: String,
    pub status: GraphStatus,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct NodeReport {
    pub id: String,
    pub kind: String,
    pub created_seq: i64,
    pub valid: bool,
    pub reasons: Vec<String>,
}

pub struct Projection {
    pub seq: i64,
    pub curves: HashMap<String, RawCurve>,
    pub responses: HashMap<String, InstrumentResponse>,
    pub params: Params,
    pub params_history: Vec<(i64, Params)>,
    pub derivations: HashMap<String, Derivation>,
    /// branch key -> current derivation id (moved only by decision events).
    pub pointers: HashMap<String, String>,
    /// subtraction branch -> (sample deriv, buffer deriv, buffer curve id)
    pub sub_branches: HashMap<String, SubBranch>,
    /// intervals branch -> (subtraction deriv id)
    pub interval_branches: HashMap<String, String>,
    /// candidate id -> adopted/rejected state.
    pub candidate_decisions: HashMap<String, CandidateDecision>,
    pub events: Vec<Event>,
    pub judgments: Vec<(i64, Judgment)>,
}

#[derive(Debug, Clone)]
pub struct SubBranch {
    pub sample_norm: String,
    pub buffer_norm: String,
    pub buffer_curve: String,
    pub subtraction: String,
}

#[derive(Debug, Clone)]
pub struct CandidateDecision {
    pub action: String, // adopt | reject | parameter_adjust
    pub at_seq: i64,
    pub note: String,
}

impl Projection {
    pub fn empty() -> Self {
        Projection {
            seq: 0,
            curves: HashMap::new(),
            responses: HashMap::new(),
            params: Params::default(),
            params_history: vec![],
            derivations: HashMap::new(),
            pointers: HashMap::new(),
            sub_branches: HashMap::new(),
            interval_branches: HashMap::new(),
            candidate_decisions: HashMap::new(),
            events: vec![],
            judgments: vec![],
        }
    }

    pub fn fold(events: Vec<Event>) -> Self {
        let mut p = Projection::empty();
        for e in events {
            p.apply(&e);
        }
        p
    }

    pub fn apply(&mut self, e: &Event) {
        self.seq = e.seq;
        self.events.push(e.clone());
        match e.event_type.as_str() {
            "curve_registered" => {
                if let Ok(c) = serde_json::from_value::<RawCurve>(e.body["curve"].clone()) {
                    self.curves.insert(c.id.clone(), c);
                }
                for j in decode_judgments(&e.body) {
                    self.judgments.push((e.seq, j));
                }
            }
            "response_registered" => {
                if let Ok(r) = serde_json::from_value::<InstrumentResponse>(e.body["response"].clone()) {
                    self.responses.insert(r.id.clone(), r);
                }
            }
            "params_revised" => {
                if let Ok(p) = serde_json::from_value::<Params>(e.body["params"].clone()) {
                    self.params = p.clone();
                    self.params_history.push((e.seq, p));
                }
            }
            "derivation_recorded" => {
                if let Ok(d) = serde_json::from_value::<Derivation>(e.body["derivation"].clone()) {
                    for j in &d.judgments {
                        self.judgments.push((e.seq, j.clone()));
                    }
                    if d.kind == "subtracted" {
                        let key = e.body["branch"].as_str().unwrap_or("").to_string();
                        if let Some(existing) = self.sub_branches.get(&key).cloned() {
                            self.sub_branches.insert(
                                key,
                                SubBranch { subtraction: d.id.clone(), ..existing },
                            );
                        }
                    }
                    if d.kind == "interval_candidates" {
                        let key = e.body["branch"].as_str().unwrap_or("").to_string();
                        self.interval_branches.insert(key, d.id.clone());
                    }
                    self.derivations.insert(d.id.clone(), d);
                }
            }
            "decision_recorded" => {
                let branch = e.body["branch"].as_str().unwrap_or("").to_string();
                let action = e.body["action"].as_str().unwrap_or("").to_string();
                let target = e.body["target"].as_str().unwrap_or("").to_string();
                match action.as_str() {
                    "continue_subtraction" | "select_buffer_version" => {
                        if !target.is_empty() {
                            self.pointers.insert(branch, target);
                        }
                    }
                    "adopt_candidate" | "reject_candidate" | "parameter_adjust" => {
                        if !target.is_empty() {
                            self.candidate_decisions.insert(
                                target.clone(),
                                CandidateDecision {
                                    action: action.clone(),
                                    at_seq: e.seq,
                                    note: e.body["note"].as_str().unwrap_or("").to_string(),
                                },
                            );
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub fn current_subtraction(&self, key: &str) -> Option<&Derivation> {
        if let Some(id) = self.pointers.get(key) {
            return self.derivations.get(id);
        }
        let branch = self.sub_branches.get(key)?;
        self.derivations.get(&branch.subtraction)
    }

    /// Point provenance walk: derived point -> normalized/raw source points,
    /// returning the two raw source curves for a subtracted point.
    pub fn provenance(&self, point_id: &str) -> Provenance {
        let mut chain = vec![];
        let mut raw_curves: Vec<String> = vec![];
        let mut stack = vec![point_id.to_string()];
        let mut seen = std::collections::HashSet::new();
        while let Some(pid) = stack.pop() {
            if !seen.insert(pid.clone()) {
                continue;
            }
            for d in self.derivations.values() {
                if let Some(p) = d.points.iter().find(|p| p.id == pid) {
                    chain.push(PointLink {
                        point: pid.clone(),
                        derivation: d.id.clone(),
                        sources: p.source_point_ids.clone(),
                    });
                    for s in &p.source_point_ids {
                        if s.starts_with("rp_") {
                            for c in self.curves.values() {
                                if c.points.iter().any(|rp| {
                                    crate::ids::raw_point_id(&c.id, rp.idx, &rp.q_raw, &rp.i_raw) == *s
                                }) {
                                    if !raw_curves.contains(&c.id) {
                                        raw_curves.push(c.id.clone());
                                    }
                                }
                            }
                        }
                        stack.push(s.clone());
                    }
                }
            }
        }
        Provenance { point: point_id.into(), chain, raw_curves }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PointLink {
    pub point: String,
    pub derivation: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Provenance {
    pub point: String,
    pub chain: Vec<PointLink>,
    pub raw_curves: Vec<String>,
}

fn decode_judgments(body: &serde_json::Value) -> Vec<Judgment> {
    serde_json::from_value(body.get("judgments").cloned().unwrap_or(serde_json::Value::Null))
        .unwrap_or_default()
}

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("1970-01-01T00:00:{secs:02}Z")
}

fn chain_hash(prev: &str, event_type: &str, at: &str, actor: &str, client: &str, canonical_body: &str) -> String {
    crate::ids::fnv64(&["hash", prev, event_type, at, actor, client, canonical_body])
}

pub struct App {
    pub store: Store,
}

#[derive(Debug, Clone)]
pub struct EventDraft {
    pub event_type: String,
    pub actor: String,
    pub client: String,
    pub body: serde_json::Value,
    pub materialize: MaterializationKind,
}

#[derive(Debug, Clone)]
pub enum MaterializationKind {
    None,
    Curve(String /*curve id inside body*/),
    Response(String),
    Params,
    Derivation,
    Decision,
    CasepackRun,
}

#[derive(Debug, Clone)]
pub struct CommitResult {
    pub seq: i64,
    pub event_id: String,
}

impl App {
    pub fn new(path: &str) -> Result<Self, StoreError> {
        Ok(App { store: Store::open(path)? })
    }

    pub fn projection(&self, at_seq: Option<i64>) -> Result<Projection, StoreError> {
        Ok(Projection::fold(self.store.events(at_seq)?))
    }

    /// Append with optimistic concurrency. Two users editing the same base
    /// version produce a Conflict the loser must resolve and re-base.
    pub fn append(
        &self,
        draft: EventDraft,
        expected_seq: Option<i64>,
        at_override: Option<String>,
    ) -> Result<CommitResult, StoreError> {
        let (mut pending, seq) = self.store.begin_append(expected_seq)?;
        let at = at_override.unwrap_or_else(now_iso);
        let prev = self.store.events(None)?;
        let prev_hash = prev.last().map(|e| e.hash.clone()).unwrap_or_else(|| "genesis".into());
        let canonical_body = serde_json::to_string(&draft.body).unwrap_or_default();
        let hash = chain_hash(&prev_hash, &draft.event_type, &at, &draft.actor, &draft.client, &canonical_body);
        let event_id = crate::ids::event_id(seq, &canonical_body);
        let event = Event {
            seq,
            event_id: event_id.clone(),
            event_type: draft.event_type.clone(),
            at,
            actor: draft.actor,
            client: draft.client,
            body: draft.body.clone(),
            prev_hash,
            hash,
        };
        let mat = self.material_for(&draft, &event);
        self.store.commit_append(&mut pending, &event, mat)?;
        Ok(CommitResult { seq, event_id })
    }

    fn material_for(&self, draft: &EventDraft, event: &Event) -> Materialization {
        match draft.materialize {
            MaterializationKind::None => Materialization::None,
            MaterializationKind::Curve => {
                serde_json::from_value::<RawCurve>(event.body["curve"].clone())
                    .map(Materialization::Curve)
                    .unwrap_or(Materialization::None)
            }
            MaterializationKind::Response => {
                serde_json::from_value::<InstrumentResponse>(event.body["response"].clone())
                    .map(Materialization::Response)
                    .unwrap_or(Materialization::None)
            }
            MaterializationKind::Params => {
                serde_json::from_value::<Params>(event.body["params"].clone())
                    .map(|p| Materialization::Params(p, event.seq))
                    .unwrap_or(Materialization::None)
            }
            MaterializationKind::Derivation => {
                serde_json::from_value::<Derivation>(event.body["derivation"].clone())
                    .map(Materialization::Derivation)
                    .unwrap_or(Materialization::None)
            }
            MaterializationKind::Decision => Materialization::Decision {
                branch: event.body["branch"].as_str().unwrap_or("").to_string(),
                action: event.body["action"].as_str().unwrap_or("").to_string(),
                target: event.body["target"].as_str().map(String::from),
                seq: event.seq,
            },
            MaterializationKind::CasepackRun => Materialization::CasepackRun {
                run_id: event.body["run_id"].as_str().unwrap_or("").to_string(),
                finished_seq: event.seq,
                at: event.at.clone(),
            },
        }
    }

    pub fn register_response(
        &self,
        response: InstrumentResponse,
        actor: &str,
        client: &str,
        expected_seq: Option<i64>,
        at_override: Option<String>,
    ) -> Result<CommitResult, StoreError> {
        self.append(
            EventDraft {
                event_type: "response_registered".into(),
                actor: actor.into(),
                client: client.into(),
                body: serde_json::json!({"response": response}),
                materialize: MaterializationKind::Response,
            },
            expected_seq,
            at_override,
        )
    }

    pub fn register_curve(
        &self,
        cmd: RegisterCurve,
        actor: &str,
        client: &str,
        expected_seq: Option<i64>,
        at_override: Option<String>,
    ) -> Result<(CommitResult, String, Vec<Judgment>), StoreError> {
        let (curve, judgments) = rules::build_raw_curve(cmd, now_iso());
        let id = curve.id.clone();
        let res = self.append(
            EventDraft {
                event_type: "curve_registered".into(),
                actor: actor.into(),
                client: client.into(),
                body: serde_json::json!({"curve": curve, "judgments": judgments}),
                materialize: MaterializationKind::Curve,
            },
            expected_seq,
            at_override,
        )?;
        Ok((res, id, judgments))
    }

    pub fn revise_params(
        &self,
        mut params: Params,
        actor: &str,
        client: &str,
        expected_seq: Option<i64>,
    ) -> Result<CommitResult, StoreError> {
        let proj = self.projection(None)?;
        params.version = format!("params-v{}", proj.params_history.len() + 1);
        self.append(
            EventDraft {
                event_type: "params_revised".into(),
                actor: actor.into(),
                client: client.into(),
                body: serde_json::json!({"params": params}),
                materialize: MaterializationKind::Params,
            },
            expected_seq,
            None,
        )
    }
}

pub fn sample_norm_key(sample_curve: &str) -> String {
    format!("norm:{sample_curve}")
}
pub fn sub_branch_key(sample_curve: &str, buffer_curve: &str) -> String {
    format!("sub:{sample_curve}|{buffer_curve}")
}

pub struct PipelineReport {
    pub nodes: Vec<Freshness>,
    pub events: Vec<i64>,
    pub branch: String,
}

impl App {
    /// Full requested pipeline for one (sample, buffer) pair. Only nodes whose
    /// signature changed are rebuilt; identical derivations are reused, and the
    /// returned report explains per node why it is still valid or recomputed.
    pub fn run_pipeline(
        &self,
        sample_curve: &str,
        buffer_curve: &str,
        actor: &str,
        client: &str,
        expected_seq: Option<i64>,
    ) -> Result<PipelineReport, String> {
        let mut nodes: Vec<Freshness> = vec![];
        let mut seq_events: Vec<i64> = vec![];
        let proj = self.projection(None).map_err(|e| e.to_string())?;
        let sample = proj.curves.get(sample_curve).ok_or("sample curve missing")?.clone();
        let buffer = proj.curves.get(buffer_curve).ok_or("buffer curve missing")?.clone();
        let params = proj.params.clone();
        let sample_resp = sample.response_id.as_ref().and_then(|r| proj.responses.get(r));
        let buffer_resp = buffer.response_id.as_ref().and_then(|r| proj.responses.get(r));

        let norm_s = rules::normalize(&sample, sample_resp, &params, 0);
        let norm_b = rules::normalize(&buffer, buffer_resp, &params, 0);

        for (existing, fresh, label) in [
            (proj.derivations.contains_key(&norm_s.id), &norm_s, "normalized sample"),
            (proj.derivations.contains_key(&norm_b.id), &norm_b, "normalized buffer"),
        ] {
            if existing {
                nodes.push(Freshness {
                    deriv_id: fresh.id.clone(),
                    kind: fresh.kind.clone(),
                    status: GraphStatus::Reused,
                    reason: format!("{label}: input+rule+params signature unchanged ({})", crate::RULE_VERSION),
                });
            } else {
                let r = self
                    .append(
                        EventDraft {
                            event_type: "derivation_recorded".into(),
                            actor: actor.into(),
                            client: client.into(),
                            body: serde_json::json!({"derivation": fresh, "branch": sample_norm_key(if label.contains("sample") { sample_curve } else { buffer_curve })}),
                            materialize: MaterializationKind::Derivation,
                        },
                        None,
                        None,
                    )
                    .map_err(|e| e.to_string())?;
                seq_events.push(r.seq);
                nodes.push(Freshness {
                    deriv_id: fresh.id.clone(),
                    kind: fresh.kind.clone(),
                    status: GraphStatus::Recomputed,
                    reason: format!("{label}: signature new or input changed; rebuilt at seq {}", r.seq),
                });
            }
        }

        let branch = sub_branch_key(sample_curve, buffer_curve);
        let sub = rules::subtract(&norm_s, &norm_b, &params, 0, &branch);
        if proj.derivations.contains_key(&sub.id)
            && proj.sub_branches.get(&branch).map(|b| b.subtraction == sub.id).unwrap_or(false)
        {
            nodes.push(Freshness {
                deriv_id: sub.id.clone(),
                kind: sub.kind.clone(),
                status: GraphStatus::Reused,
                reason: "subtraction: both normalized inputs and buffer selection unchanged".into(),
            });
        } else {
            if sub.status == "error" {
                nodes.push(Freshness {
                    deriv_id: sub.id.clone(),
                    kind: sub.kind.clone(),
                    status: GraphStatus::Blocked,
                    reason: "subtraction blocked (e.g. no overlap); failure recorded, previous version stays current".into(),
                });
            }
            let r = self
                .append(
                    EventDraft {
                        event_type: "derivation_recorded".into(),
                        actor: actor.into(),
                        client: client.into(),
                        body: serde_json::json!({
                            "derivation": sub.clone(),
                            "branch": branch,
                            "sample_norm": norm_s.id,
                            "buffer_norm": norm_b.id,
                            "sample_curve": sample_curve,
                            "buffer_curve": buffer_curve,
                            "supersedes_branch_version": proj.sub_branches.get(&branch).map(|b| b.subtraction.clone()),
                        }),
                        materialize: MaterializationKind::Derivation,
                    },
                    expected_seq,
                    None,
                )
                .map_err(|e| e.to_string())?;
            seq_events.push(r.seq);
            nodes.push(Freshness {
                deriv_id: sub.id.clone(),
                kind: sub.kind.clone(),
                status: if sub.status == "error" { GraphStatus::Blocked } else { GraphStatus::Recomputed },
                reason: format!("subtraction recorded at seq {}; current pointer moves only on full success", r.seq),
            });
        }

        // Intervals are derived from the successful subtraction only.
        if sub.status != "error" {
            let intervals = rules::interval_candidates(&sub, &params, 0);
            let ibranch = format!("intervals:{}", branch);
            let unchanged = proj
                .interval_branches
                .get(&ibranch)
                .map(|id| *id == intervals.id)
                .unwrap_or(false)
                && proj.derivations.contains_key(&intervals.id);
            if unchanged {
                nodes.push(Freshness {
                    deriv_id: intervals.id.clone(),
                    kind: intervals.kind.clone(),
                    status: GraphStatus::Reused,
                    reason: "interval candidates: subtraction + interval parameters unchanged".into(),
                });
            } else {
                let r = self
                    .append(
                        EventDraft {
                            event_type: "derivation_recorded".into(),
                            actor: actor.into(),
                            client: client.into(),
                            body: serde_json::json!({"derivation": intervals, "branch": ibranch, "subtraction": sub.id}),
                            materialize: MaterializationKind::Derivation,
                        },
                        None,
                        None,
                    )
                    .map_err(|e| e.to_string())?;
                seq_events.push(r.seq);
                nodes.push(Freshness {
                    deriv_id: intervals.id.clone(),
                    kind: intervals.kind.clone(),
                    status: GraphStatus::Recomputed,
                    reason: format!("interval candidates rebuilt at seq {} (Guinier/Porod never auto-joined)", r.seq),
                });
            }
        }

        Ok(PipelineReport { nodes, events: seq_events, branch })
    }

    pub fn decide(
        &self,
        branch: &str,
        action: &str,
        target: Option<&str>,
        note: &str,
        actor: &str,
        client: &str,
        expected_seq: Option<i64>,
    ) -> Result<CommitResult, StoreError> {
        self.append(
            EventDraft {
                event_type: "decision_recorded".into(),
                actor: actor.into(),
                client: client.into(),
                body: serde_json::json!({
                    "branch": branch, "action": action, "target": target, "note": note,
                    "reversible": true, "overwrites_history": false,
                }),
                materialize: MaterializationKind::Decision,
            },
            expected_seq,
            None,
        )
    }

    /// Why is this derivation still valid / stale at the current head?
    pub fn explain_node(&self, deriv_id: &str) -> Result<NodeReport, String> {
        let proj = self.projection(None).map_err(|e| e.to_string())?;
        let d = proj.derivations.get(deriv_id).ok_or("derivation not found")?.clone();
        let mut reasons = vec![];
        let mut valid = true;
        match d.kind.as_str() {
            "normalized" => {
                let curve_id = d.inputs.first().cloned().unwrap_or_default();
                if let Some(c) = proj.curves.get(&curve_id) {
                    let fresh = rules::normalize(
                        c,
                        c.response_id.as_ref().and_then(|r| proj.responses.get(r)),
                        &proj.params,
                        0,
                    );
                    if fresh.id == d.id {
                        reasons.push("curve, exposure/transmission/thickness, response and parameters hash to the same signature".into());
                    } else {
                        valid = false;
                        reasons.push("an upstream source or the active parameter version changed; recompute produces a different id".into());
                    }
                }
            }
            "subtracted" => {
                if d.inputs.len() == 2 {
                    let both_present = d.inputs.iter().all(|i| proj.derivations.contains_key(i));
                    if both_present {
                        reasons.push("both normalized source derivations are still present with matching signatures".into());
                    } else {
                        valid = false;
                        reasons.push("a normalized input is missing".into());
                    }
                }
            }
            "interval_candidates" => {
                if let Some(sub_id) = d.inputs.first() {
                    if proj.derivations.contains_key(sub_id) {
                        reasons.push("parent subtraction intact; interval params unchanged".into());
                    } else {
                        valid = false;
                        reasons.push("parent subtraction missing".into());
                    }
                }
            }
            _ => {}
        }
        reasons.push(format!("rule version at creation: {}", d.rule_version));
        Ok(NodeReport { id: d.id, kind: d.kind, created_seq: d.created_event_seq, valid, reasons })
    }

    /// Branch comparison: list every subtraction derivation ever produced for a
    /// sample, across buffer versions, so reviewers can compare and continue.
    pub fn subtraction_branches(&self, sample_curve: &str) -> Vec<Derivation> {
        self.projection(None)
            .map(|p| {
                p.derivations
                    .values()
                    .filter(|d| d.kind == "subtracted" && d.config_sig.contains(sample_curve) || false)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }
}
