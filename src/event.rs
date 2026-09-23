//! Commands (intent) and events (the durable ledger facts).

use crate::domain::{Artifacts, Ref, Verdict};
use serde::{Deserialize, Serialize};

/// User/import intents. Applied by the rule engine; only a fully successful
/// command becomes an event and switches current pointers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Import (or import a new version of) a material record.
    ImportMaterial { material_id: String, label: String, kind: String, seed: ImportSeed },
    /// Revise a normalization parameter (exposure/thickness/transmission...).
    ReviseParameter { material_id: String, field: String, raw: String, unit: String, reason: String },
    /// Change a local slice of the source curve (one source only).
    PatchSource { material_id: String, index: usize, field: String, raw: String, reason: String },
    /// Switch which buffer version a sample subtraction uses.
    SwitchBuffer { sample: Ref, old_buffer: Ref, new_buffer: Ref, reason: String },
    /// Choose a background-scale candidate.
    ChooseCandidate { sub_node: String, candidate_id: String },
    /// Accept or reject a branch.
    DecideBranch { sub_node: String, decision: String, note: String },
    /// Two reviewers editing from the same base: a conflicting human decision.
    Conflict { base_event: u32, local_decision: String, incoming_decision: String, resolution: String, sub_node: String },
    /// Replay/reset control used by the offline casepack.
    ReplayCasepack { name: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportSeed {
    pub kind: String, // "sample" | "buffer"
    pub label: String,
    pub q_unit: String,
    pub i_unit: String,
    pub exposure_raw: String,
    pub exposure_unit: String,
    pub thickness_raw: String,
    pub thickness_unit: String,
    pub transmission_raw: String,
    pub transmission_unit: String,
    pub transmission_error_raw: String,
    pub response_id: Option<String>,
    pub mask_id: Option<String>,
    pub rows: Vec<SeedRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeedRow {
    pub q: String,
    pub i: String,
    pub e: String,
    pub masked: bool,
}

/// A durable event. Events are append-only and never overwritten.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u32,
    pub parent_seq: u32,
    pub author: String,
    pub command: Command,
    /// Input fragments that were seen when judging.
    pub input_fragment: serde_json::Value,
    pub verdicts: Vec<Verdict>,
    pub artifacts: Artifacts,
    /// Pointers (node slug -> Ref) as of this event; historical snapshot.
    pub pointers: std::collections::BTreeMap<String, String>,
    /// Per-node recompute status for this command.
    pub recompute: Vec<RecomputeStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecomputeStatus {
    pub node: String,
    pub status: String, // "recomputed" | "still_valid" | "needs_recompute" | "invalid"
    pub because: String,
}
