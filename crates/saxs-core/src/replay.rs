//! Offline case-pack export/import. The event log plus material manifest is
//! the single portable artifact; ids, order and provenance survive
//! export -> delete local database -> import.

use crate::project::{Event, Project};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialFile {
    pub path: String,
    pub role: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub case_id: String,
    pub title: String,
    pub pack_version: u32,
    pub rule_version: String,
    pub exported_at: String,
    pub project_id: String,
    pub project_name: String,
    pub events: Vec<Event>,
    pub materials: Vec<MaterialFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verification {
    pub events: usize,
    pub head_hash: String,
    pub chain_ok: bool,
}

impl Bundle {
    pub fn verify(&self) -> Result<Verification, String> {
        let mut parent = String::new();
        let mut ok = true;
        for ev in &self.events {
            if ev.parent_hash != parent {
                ok = false;
            }
            parent = ev.content_digest();
        }
        Ok(Verification { events: self.events.len(), head_hash: parent, chain_ok: ok })
    }

    pub fn into_project(self) -> Result<Project, String> {
        Project::replay(&self.project_id, self.events)
    }
}
