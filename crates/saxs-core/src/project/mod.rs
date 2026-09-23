//! Event-sourced project aggregate and the intent engine.

mod event;
mod intent;

pub use event::{Event, EventPayload};
pub use intent::Intent;

use crate::calc;
use engine::apply_payload;
use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const RULE_VERSION: &str = "saxs-rules-2026.1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleParams {
    pub qrg_max: f64,
    pub scale_rel_sigma_limit: f64,
    pub grid_tol: f64,
    pub response_interpolation: String,
}

impl Default for RuleParams {
    fn default() -> Self {
        Self {
            qrg_max: 1.3,
            scale_rel_sigma_limit: 0.05,
            grid_tol: 1e-9,
            response_interpolation: "linear".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectState {
    pub name: String,
    pub params: RuleParams,
    pub responses: BTreeMap<String, InstrumentResponse>,
    pub masks: BTreeMap<String, Mask>,
    pub runs: BTreeMap<String, ParsedRun>,
    pub run_revisions: BTreeMap<String, u32>,
    pub norms: BTreeMap<String, Vec<NormCurve>>,
    pub norm_head: BTreeMap<String, String>,
    pub sub_versions: BTreeMap<String, Vec<SubVersion>>,
    pub sub_heads: BTreeMap<String, String>,
    pub fits: BTreeMap<String, Vec<RegionFit>>,
    pub decisions: Vec<DecisionRecord>,
    pub conflict_resolutions: Vec<(u64, u64)>,
}

impl ProjectState {
    pub fn effective_run(&self, run_id: &str) -> Option<&ParsedRun> {
        self.runs.get(run_id)
    }
    pub fn norm_for(&self, run_id: &str) -> Option<&NormCurve> {
        let head = self.norm_head.get(run_id)?;
        self.norms.get(run_id).and_then(|v| v.iter().find(|c| &c.id == head))
    }
    pub fn sub_head(&self, artifact: &str) -> Option<&SubVersion> {
        let head = self.sub_heads.get(artifact)?;
        self.sub_versions.get(artifact).and_then(|v| v.iter().find(|s| &s.id == head))
    }
}

#[derive(Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub state: ProjectState,
    pub events: Vec<Event>,
    pub last_hash: String,
    pub next_seq: u64,
    /// fixed clock for deterministic offline case packs
    #[serde(skip)]
    pub clock: Option<Box<dyn Fn(u64) -> String + Send + Sync>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedEvent {
    pub event: Event,
    pub hash: String,
    /// ids of derived objects whose validity changed
    pub invalidated: Vec<String>,
}

impl Project {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        let mut state = ProjectState::default();
        state.name = name.into();
        Self {
            id: id.into(),
            state,
            events: Vec::new(),
            last_hash: String::new(),
            next_seq: 0,
            clock: None,
        }
    }

    pub fn with_fixed_timestamps(mut self, t0: &str) -> Self {
        let base = t0.to_string();
        self.clock = Some(Box::new(move |seq| format!("{}#{:03}", base, seq)));
        self
    }

    pub fn timestamp_for(&self, seq: u64) -> String {
        if let Some(c) = &self.clock {
            c(seq)
        } else {
            chrono::Utc::now().to_rfc3339()
        }
    }

    /// Replay an exported log into a fresh project. Verifies the hash chain
    /// and returns the first broken link, if any.
    pub fn replay(id: &str, events: Vec<Event>) -> Result<Self, String> {
        let mut p = Self::new(id, String::new());
        let mut parent = String::new();
        for ev in events {
            let h = ev.content_digest();
            if ev.parent_hash != parent {
                return Err(format!("event {} breaks parent chain", ev.seq));
            }
            parent = h;
            apply_payload(&mut p.state, &ev);
            p.events.push(ev);
        }
        p.last_hash = parent;
        p.next_seq = p.events.len() as u64;
        Ok(p)
    }

    /// Append an already-built event (used when replaying imports one-by-one).
    pub fn commit(&mut self, payload: EventPayload) -> AppliedEvent {
        let seq = self.next_seq;
        let ev = Event {
            id: crate::ids::event_id(&self.id, seq),
            project: self.id.clone(),
            seq,
            timestamp: self.timestamp_for(seq),
            rule_version: RULE_VERSION.to_string(),
            parent_hash: self.last_hash.clone(),
            payload,
        };
        let invalidated = affected_ids(&self.state, &ev);
        apply_payload(&mut self.state, &ev);
        let hash = ev.content_digest();
        self.last_hash = hash.clone();
        self.next_seq += 1;
        self.events.push(ev);
        AppliedEvent { event: self.events.last().unwrap().clone(), hash, invalidated }
    }
}

fn affected_ids(state: &ProjectState, ev: &Event) -> Vec<String> {
    let mut out = Vec::new();
    match &ev.payload {
        EventPayload::RunAmended { run_id, .. } => {
            if let Some(head) = state.norm_head.get(run_id) {
                out.push(head.clone());
            }
            for (art, head) in &state.sub_heads {
                if let Some(v) = state.sub_versions.get(art).and_then(|v| v.iter().find(|s| &s.id == head)) {
                    if v.sample_run == *run_id || v.buffer_run == *run_id {
                        out.push(v.id.clone());
                    }
                }
            }
        }
        EventPayload::Normalized { curve } => {
            for (art, head) in &state.sub_heads {
                if let Some(v) = state.sub_versions.get(art).and_then(|v| v.iter().find(|s| &s.id == head)) {
                    if v.sample_norm == curve.id || v.buffer_norm == curve.id {
                        out.push(v.id.clone());
                    }
                }
            }
        }
        EventPayload::Subtracted { version } => {
            for fits in state.fits.values() {
                for f in fits {
                    if f.sub_version == version.id {
                        out.push(f.id.clone());
                    }
                }
            }
        }
        EventPayload::RuleConfigured { .. } => {
            for fits in state.fits.values() {
                for f in fits {
                    out.push(f.id.clone());
                }
            }
        }
        _ => {}
    }
    out.into_iter().collect::<BTreeSet<_>>().into_iter().collect()
}

mod engine;
