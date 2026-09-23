//! Event sourcing + dependency graph.
//!
//! Only raw inputs and human decisions are events. Derived curves/branches are
//! deterministic functions of (events-at-a-point, rules version) and are
//! materialised lazily; revisiting an old event replays from there, so history
//! is never overwritten by newer computations.

use crate::model::*;
use crate::saxs::{normalize, subtract_branch, RULES_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    ProjectCreated { name: String },
    /// Add or fully replace a raw profile (source-data edit keyed by profile id).
    ProfileAdded { profile: RawProfile },
    ProfileUpdated { profile: RawProfile },
    /// Declare which profiles are buffer candidates for a sample.
    BufferCandidate { sample_id: String, buffer_id: String },
    /// Parameter revision: fit window used to build a branch for a pair.
    FitWindow { sample_id: String, buffer_id: String, fit_lo: f64, fit_hi: f64 },
    /// Human adjudication, appended only.
    Decision { sample_id: String, branch_id: String, decision: Decision },
    /// Rule-set activation (records which rule version produced which results).
    RuleActivated { version: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Monotonic, gap-free on commit (auto-increment handled by the store).
    pub seq: i64,
    pub kind: EventKind,
    /// Epoch ms; informational, ordering is by seq.
    pub at_ms: i64,
    /// Branch this event was committed on (parent head seq), for forking.
    pub parent_seq: i64,
    /// Stable content id of the event itself.
    pub id: String,
}

/// One dependency-graph node: a raw input or a derived object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    /// Direct predecessors (raw/rule ids).
    pub parents: Vec<String>,
    /// Rules version that produced a derived node.
    pub rules_version: Option<String>,
    pub detail: Value,
}

#[derive(Debug, Clone, Default)]
pub struct State {
    pub name: String,
    pub rules_version: String,
    pub profiles: Vec<RawProfile>,
    pub pairs: Vec<PairSpec>,
    /// seq order
    pub events: Vec<Event>,
    pub norms: Vec<NormCurve>,
    pub branches: Vec<SubBranch>,
    /// branch_id -> decision (latest explicit adjudication only for the head).
    pub decisions: Vec<(String, Decision, i64)>,
    pub graph: Vec<GraphNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitWindow {
    pub lo: f64,
    pub hi: f64,
    /// Seq of the event that introduced this window.
    pub since_seq: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairSpec {
    pub sample_id: String,
    pub buffer_id: String,
    /// Every fit window ever requested becomes its own retained branch; the
    /// latest entry is the current proposal, older ones stay as candidates.
    pub windows: Vec<FitWindow>,
}

impl PairSpec {
}

impl State {
    pub fn profile(&self, id: &str) -> Option<&RawProfile> {
        self.profiles.iter().find(|p| p.id == id)
    }
    pub fn norm(&self, id: &str) -> Option<&NormCurve> {
        self.norms.iter().find(|n| n.id == id)
    }
    pub fn branch(&self, id: &str) -> Option<&SubBranch> {
        self.branches.iter().find(|b| b.id == id)
    }

    /// Latest decision recorded for a branch (head view).
    pub fn decision_for(&self, branch_id: &str) -> Option<&(String, Decision, i64)> {
        self.decisions.iter().rev().find(|(bid, _, _)| bid == branch_id)
    }

    /// Resolve a decision branch selector. Supports concrete branch ids plus
    /// `LATEST:<BUF_PROF>` / `FIRST:<BUF_PROF>` so replay steps stay meaningful
    /// even though branch ids are content-derived.
    pub fn resolve_branch(&self, selector: &str) -> Option<String> {
        if let Some(spec) = selector.strip_prefix("LATEST:") {
            let norm = self.norm_for_profile(spec).map(|n| n.id.clone());
            if let Some(nid) = norm {
                return self
                    .branches
                    .iter()
                    .filter(|b| b.buffer_norm == nid)
                    .last()
                    .map(|b| b.id.clone());
            }
            return None;
        }
        if let Some(spec) = selector.strip_prefix("FIRST:") {
            let norm = self.norm_for_profile(spec).map(|n| n.id.clone());
            if let Some(nid) = norm {
                return self
                    .branches
                    .iter()
                    .find(|b| b.buffer_norm == nid)
                    .map(|b| b.id.clone());
            }
            return None;
        }
        if self.branches.iter().any(|b| b.id == selector) {
            Some(selector.to_string())
        } else {
            None
        }
    }

    /// Recompute the whole derived layer from the current raw inputs and rule
    /// version. Deterministic; ids change iff an input or rules version does.
    pub fn recompute(&mut self) {
        self.norms = self.profiles.iter().map(normalize).collect();

        let mut branches = Vec::new();
        for pair in &self.pairs {
            let sm = match self.norm_for_profile(&pair.sample_id) {
                Some(n) => n,
                None => continue,
            };
            let bf = match self.norm_for_profile(&pair.buffer_id) {
                Some(n) => n,
                None => continue,
            };
            for w in &pair.windows {
                branches.push(subtract_branch(sm, bf, w.lo, w.hi));
            }
        }
        self.branches = branches;
        self.build_graph();
    }

    fn norm_for_profile(&self, profile_id: &str) -> Option<&NormCurve> {
        self.norms.iter().find(|n| n.source_profile == profile_id)
    }

    fn build_graph(&mut self) {
        let mut nodes = Vec::new();
        for p in &self.profiles {
            nodes.push(GraphNode {
                id: p.id.clone(),
                kind: "raw".into(),
                label: p.label.clone(),
                parents: vec![],
                rules_version: None,
                detail: serde_json::json!({"role": p.role, "version": p.version,
                    "points": p.points.len(), "masked": p.mask_source.masked_count}),
            });
        }
        for n in &self.norms {
            nodes.push(GraphNode {
                id: n.id.clone(),
                kind: "norm".into(),
                label: format!("归一化 {}", n.label),
                parents: vec![n.source_profile.clone(), format!("rules:{}", self.rules_version)],
                rules_version: Some(self.rules_version.clone()),
                detail: serde_json::json!({"points": n.points.len(),
                    "blocked": n.judgments.iter().any(|j| j.severity == Severity::Blocked)}),
            });
        }
        for b in &self.branches {
            nodes.push(GraphNode {
                id: b.id.clone(),
                kind: "branch".into(),
                label: format!("扣背景 {}", b.buffer_label),
                parents: vec![b.sample_norm.clone(), b.buffer_norm.clone()],
                rules_version: Some(self.rules_version.clone()),
                detail: serde_json::json!({"blocked": b.blocked,
                    "scale": b.scale, "overlap_points": b.overlap_points,
                    "buffer_version": b.buffer_version}),
            });
        }
        nodes.push(GraphNode {
            id: format!("rules:{}", self.rules_version),
            kind: "rules".into(),
            label: format!("规则 {}", self.rules_version),
            parents: vec![],
            rules_version: Some(self.rules_version.clone()),
            detail: serde_json::json!({}),
        });
        self.graph = nodes;
    }

    /// Transitive descendants of a set of changed node ids — the only objects
    /// a recompute must refresh.
    pub fn affected_by(&self, changed: &[String]) -> Vec<String> {
        let mut result = Vec::new();
        let mut frontier: Vec<String> = changed.to_vec();
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = frontier.pop() {
            for node in &self.graph {
                if node.parents.contains(&id) && seen.insert(node.id.clone()) {
                    result.push(node.id.clone());
                    frontier.push(node.id.clone());
                }
            }
        }
        result.sort();
        result.dedup();
        result
    }

    /// Explain why a node is still valid or must be recomputed, given the set
    /// of ids that changed since the node's stored materialisation.
    pub fn validity(&self, node_id: &str, changed_since: &[String]) -> Validity {
        let node = match self.graph.iter().find(|n| n.id == node_id) {
            Some(n) => n,
            None => {
                return Validity {
                    node_id: node_id.into(),
                    status: "unknown".into(),
                    reason: "节点不存在".into(),
                    changed_ancestors: vec![],
                }
            }
        };
        let ancestors = self.ancestors(node_id);
        let changed_ancestors: Vec<String> = changed_since
            .iter()
            .filter(|c| ancestors.contains(c) || **c == node_id)
            .cloned()
            .collect();
        if changed_ancestors.is_empty() {
            Validity {
                node_id: node_id.into(),
                status: "valid".into(),
                reason: format!("{} 的来源与规则均未变化，结果仍然有效", node.label),
                changed_ancestors: vec![],
            }
        } else {
            Validity {
                node_id: node_id.into(),
                status: "stale".into(),
                reason: format!("来源 {} 已变化，需要沿依赖图重算", changed_ancestors.join(", ")),
                changed_ancestors,
            }
        }
    }

    fn ancestors(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![id.to_string()];
        while let Some(cur) = stack.pop() {
            if let Some(n) = self.graph.iter().find(|n| n.id == cur) {
                for par in &n.parents {
                    if !out.contains(par) {
                        out.push(par.clone());
                        stack.push(par.clone());
                    }
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Validity {
    pub node_id: String,
    /// "valid" | "stale" | "unknown"
    pub status: String,
    pub reason: String,
    pub changed_ancestors: Vec<String>,
}

/// Fold a prefix of events into a State. `head_seq = None` means every event.
pub fn fold(events: &[Event], head_seq: Option<i64>) -> State {
    let limit = head_seq.unwrap_or(i64::MAX);
    let mut st = State {
        rules_version: RULES_VERSION.into(),
        ..Default::default()
    };
    for ev in events {
        if ev.seq > limit {
            break;
        }
        match &ev.kind {
            EventKind::ProjectCreated { name } => st.name = name.clone(),
            EventKind::ProfileAdded { profile } | EventKind::ProfileUpdated { profile } => {
                if let Some(slot) = st.profiles.iter_mut().find(|p| p.id == profile.id) {
                    *slot = profile.clone();
                } else {
                    st.profiles.push(profile.clone());
                }
            }
            EventKind::BufferCandidate { sample_id, buffer_id } => {
                ensure_pair(&mut st.pairs, sample_id, buffer_id);
            }
            EventKind::FitWindow { sample_id, buffer_id, fit_lo, fit_hi } => {
                if let Some(pair) = pair_mut(&mut st.pairs, sample_id, buffer_id) {
                    if !pair.windows.iter().any(|w| w.lo == *fit_lo && w.hi == *fit_hi) {
                        pair.windows.push(FitWindow { lo: *fit_lo, hi: *fit_hi, since_seq: ev.seq });
                    }
                } else {
                    st.pairs.push(PairSpec {
                        sample_id: sample_id.clone(),
                        buffer_id: buffer_id.clone(),
                        windows: vec![FitWindow { lo: *fit_lo, hi: *fit_hi, since_seq: ev.seq }],
                    });
                }
            }
            EventKind::Decision { sample_id: _, branch_id, decision } => {
                st.recompute();
                let resolved = st.resolve_branch(branch_id).unwrap_or_else(|| branch_id.clone());
                let mut decision = decision.clone();
                decision.branch_id = resolved.clone();
                st.decisions.push((resolved, decision, ev.seq));
            }
            EventKind::RuleActivated { version } => st.rules_version = version.clone(),
        }
        st.events.push(ev.clone());
    }
    // Pairs without an explicit window get a single full-overlap proposal so
    // the chain is still inspectable; this never replaces a later window.
    for pair in &mut st.pairs {
        if pair.windows.is_empty() {
            pair.windows.push(FitWindow { lo: 0.0, hi: f64::MAX, since_seq: 1 });
        }
    }
    st.recompute();
    st
}

fn ensure_pair(pairs: &mut Vec<PairSpec>, sample_id: &str, buffer_id: &str) {
    if !pairs.iter().any(|p| p.sample_id == sample_id && p.buffer_id == buffer_id) {
        pairs.push(PairSpec { sample_id: sample_id.into(), buffer_id: buffer_id.into(), windows: vec![] });
    }
}

fn pair_mut<'a>(
    pairs: &'a mut Vec<PairSpec>,
    sample_id: &str,
    buffer_id: &str,
) -> Option<&'a mut PairSpec> {
    pairs.iter_mut().find(|p| p.sample_id == sample_id && p.buffer_id == buffer_id)
}
