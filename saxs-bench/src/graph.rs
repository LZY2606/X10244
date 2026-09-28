//! Versioned derivation graph and the event-sourced state machine.
//!
//! Every derivation is an independent, content-addressed version stored as a
//! node. A lineage pointer names the current version for a logical chain.
//! Decisions move pointers but never overwrite or delete the unchosen version.

use crate::derived::{ModelAnalysis, NormalizedCurve, Subtraction};
use crate::domain::{CurveKind, ImportBundle, InstrumentResponse, MaskSource, RawCurve};
use serde::{Deserialize, Serialize};

pub type LineageId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Raw,
    Response,
    Mask,
    Normalized,
    Subtraction,
    Model,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub logical: String,
    pub version: String,
    /// Direct dependency node ids (the graph edges).
    pub deps: Vec<String>,
    /// Short hash of the inputs that produced this node, for staleness checks.
    pub input_hash: String,
    pub created_by_event: u64,
    pub payload: NodePayload,
    pub findings_ok: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodePayload {
    Raw(RawCurve),
    Response(InstrumentResponse),
    Mask(MaskSource),
    Normalized(Box<NormalizedCurve>),
    Subtraction(Box<Subtraction>),
    Model(Box<ModelAnalysis>),
}

impl NodePayload {
    pub fn kind(&self) -> NodeKind {
        match self {
            NodePayload::Raw(_) => NodeKind::Raw,
            NodePayload::Response(_) => NodeKind::Response,
            NodePayload::Mask(_) => NodeKind::Mask,
            NodePayload::Normalized(_) => NodeKind::Normalized,
            NodePayload::Subtraction(_) => NodeKind::Subtraction,
            NodePayload::Model(_) => NodeKind::Model,
        }
    }
}

/// Current pointer for one logical chain, with optimistic version token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pointer {
    pub lineage: LineageId,
    pub node_id: String,
    /// Monotonic revision of this pointer; concurrent edits must conflict.
    pub rev: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct State {
    pub nodes: Vec<Node>,
    pub pointers: Vec<Pointer>,
}

impl State {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn pointer(&self, lineage: &str) -> Option<&Pointer> {
        self.pointers.iter().find(|p| p.lineage == lineage)
    }
    pub fn current_node(&self, lineage: &str) -> Option<&Node> {
        self.pointer(lineage).and_then(|p| self.node(&p.node_id))
    }
    pub fn lineage_for_kind(kind: CurveKind, logical: &str) -> LineageId {
        let k = match kind {
            CurveKind::Sample => "sample",
            CurveKind::Buffer => "buffer",
        };
        format!("raw:{k}:{logical}")
    }
}

// ---------------------------------------------------------------------------
// Events: the only thing that is persisted. State is a pure fold of events.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub at: String,
    pub actor: String,
    pub kind: String,
    pub machine_readable: bool,
    pub detail: EventDetail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventDetail {
    Imported {
        bundle: ImportBundle,
        node_ids: Vec<String>,
    },
    Derived {
        node: Node,
        /// Lineages whose pointers moved as a result.
        pointer_moves: Vec<PointerMove>,
    },
    Decided {
        lineage: String,
        from_node: String,
        to_node: String,
        reason: String,
        /// The losing candidate node is retained and recorded here.
        rejected_node: Option<String>,
    },
    SourceRevised {
        logical: String,
        old_node: String,
        new_node: String,
        reason: String,
        /// Dependent nodes reported stale by the dependency graph.
        stale: Vec<String>,
    },
    Anomaly {
        code: String,
        severity: String,
        message: String,
        affected: Vec<String>,
        input_excerpt: String,
    },
    ConflictObserved {
        lineage: String,
        expected_rev: u64,
        actual_rev: u64,
        attempted_by: String,
        message: String,
        resolution: Option<String>,
    },
    ReplayStep {
        step: String,
        label: String,
        produced_nodes: Vec<String>,
        affected: Vec<String>,
        verdict: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointerMove {
    pub lineage: String,
    pub from: Option<String>,
    pub to: String,
}
