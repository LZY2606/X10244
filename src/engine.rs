//! 编排：导入原始曲线、依赖图自底向上传播、版本复用与守护事件。

use crate::domain::*;
use crate::events::*;
use crate::graph;
use crate::ops;
use crate::store::Store;
use std::collections::BTreeMap;

pub struct Engine<'a> {
    pub store: &'a Store,
    pub actor: String,
}

pub struct PropagationReport {
    pub nodes: BTreeMap<String, graph::NodeState>,
    pub events: Vec<i64>,
}

impl<'a> Engine<'a> {
    pub fn new(store: &'a Store, actor: &str) -> Self {
        Engine { store, actor: actor.to_string() }
    }

    fn emit(&self, et: EventType, subject: &str, version_id: Option<String>, judgement: serde_json::Value, note: impl Into<String>, parent: Option<String>) -> i64 {
        let evt = Event {
            seq: 0,
            event_id: format!("evt:{}:{}", self.actor, nanoid_like()),
            timestamp: current_ts(),
            actor: self.actor.clone(),
            event_type: et,
            subject: subject.to_string(),
            version_id,
            judgement,
            note: note.into(),
            parent_event_id: parent,
        };
        self.store.append_event(evt).expect("append event")
    }

    pub fn import_raw(&self, raw: RawCurve, note: &str) -> String {
        let payload = Payload::Raw(raw.clone());
        let inputs: Vec<String> = vec![];
        let fp = graph::payload_fingerprint(&raw.id, &inputs, &serde_json::to_string(&payload).unwrap());
        let version_id = self
            .store
            .write_version(&raw.id, "raw", &payload, None, &[], &fp, true, None, true)
            .expect("write raw");
        self.emit(
            EventType::Import,
            &raw.id,
            Some(version_id.clone()),
            serde_json::json!({
                "rule": "provenance_preserved",
                "rule_version": RULE_VERSION,
                "fields": ["q","i","sigma","exposure","thickness","transmission","buffer_candidates","instrument_response","mask"],
                "source_excerpt": raw.source_excerpt,
                "n_points": raw.points.len(),
                "mask_source": raw.mask.source,
                "instrument_source": raw.instrument.source,
                "q_unit_raw": raw.points.first().map(|p| p.q_unit.clone()),
                "i_unit_raw": raw.points.first().map(|p| p.i_unit.clone()),
            }),
            note,
            None,
        );
        version_id
    }
