use crate::model::*;
use crate::rules;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

/// In-memory workspace: sources + derived nodes + dependency pins.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Workspace {
    pub sources: BTreeMap<String, SourceRecord>,
    pub nodes: BTreeMap<String, DerivedNode>,
    /// content fingerprint per entity id (source version or node recompute counter)
    pub fingerprints: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecomputeReport {
    pub recomputed: Vec<(String, String)>,   // (node id, reason)
    pub still_valid: Vec<(String, String)>,  // (node id, why still valid)
    pub failed: Vec<(String, String)>,       // (node id, diagnostic code)
}

impl Workspace {
    pub fn fingerprint_of(&self, id: &str) -> u64 {
        self.fingerprints.get(id).copied().unwrap_or(0)
    }

    pub fn add_source(&mut self, src: SourceRecord) {
        self.fingerprints.insert(src.id.clone(), src.version);
        self.sources.insert(src.id.clone(), src);
    }

    pub fn bump_source(&mut self, id: &str) {
        if let Some(s) = self.sources.get_mut(id) {
            s.version += 1;
            self.fingerprints.insert(id.to_string(), s.version);
        }
    }

    fn inputs_match(&self, node: &DerivedNode) -> Option<String> {
        for (id, fp) in &node.inputs {
            let cur = self.fingerprint_of(id);
            if cur != *fp {
                return Some(format!("input {} changed (pin v{} -> current v{})", id, fp, cur));
            }
        }
        None
    }

    /// Dependency-graph propagation: recompute only nodes whose input pins no
    /// longer match; every other node keeps its result with an explanation.
    pub fn recompute(&mut self, sample_id: &str, buffer_ids: &[String], scale: f64, scale_sigma: f64) -> RecomputeReport {
        let mut report = RecomputeReport {
            recomputed: vec![],
            still_valid: vec![],
            failed: vec![],
        };
        // topological order: normalized -> subtracted -> residual/guinier/porod
        let norm_id = format!("node:normalized:{}", sample_id);
        self.ensure_normalized(&norm_id, sample_id, &mut report);
        for buf in buffer_ids {
            let sub_id = format!("node:subtracted:{}:{}", sample_id, buf);
            self.ensure_subtracted(&sub_id, &norm_id, sample_id, buf, scale, scale_sigma, &mut report);
            // downstream of subtraction
            self.ensure_residual(&sub_id, &mut report);
            self.ensure_intervals(&sub_id, NodeKind::Guinier, &mut report);
            self.ensure_intervals(&sub_id, NodeKind::Porod, &mut report);
        }
        report
    }

    fn touch(&mut self, id: &str) {
        let fp = self.fingerprint_of(id) + 1;
        self.fingerprints.insert(id.to_string(), fp);
    }

    fn ensure_normalized(&mut self, norm_id: &str, sample_id: &str, report: &mut RecomputeReport) {
        let need = match self.nodes.get(norm_id) {
            None => Some("no previous result".to_string()),
            Some(n) => self.inputs_match(n),
        };
        match need {
            None => {
                report.still_valid.push((norm_id.into(), "sample source unchanged; normalization pins match".into()));
            }
            Some(reason) => {
                let src = self.sources.get(sample_id).expect("sample source").clone();
                let (curve, diags) = rules::normalize(&src);
                let node = DerivedNode {
                    id: norm_id.into(),
                    kind: NodeKind::Normalized,
                    inputs: vec![(sample_id.into(), self.fingerprint_of(sample_id))],
                    params: json!({"exposure_s": src.meta.as_ref().unwrap().exposure_s,
                                   "thickness_mm": src.meta.as_ref().unwrap().thickness_mm,
                                   "transmission": src.meta.as_ref().unwrap().transmission}),
                    status: NodeStatus::Valid,
                    curve: Some(curve),
                    candidates: vec![],
                    diagnostics: diags,
                    validity_note: format!("recomputed: {}", reason),
                };
                self.nodes.insert(norm_id.into(), node);
                self.touch(norm_id);
                report.recomputed.push((norm_id.into(), reason));
            }
        }
    }

    fn ensure_subtracted(
        &mut self,
        sub_id: &str,
        norm_id: &str,
        sample_id: &str,
        buffer_id: &str,
        scale: f64,
        scale_sigma: f64,
        report: &mut RecomputeReport,
    ) {
        let params = json!({"scale": scale, "scale_sigma": scale_sigma, "buffer": buffer_id});
        let need = match self.nodes.get(sub_id) {
            None => Some("no previous result".to_string()),
            Some(n) if n.params != params => Some(format!("params changed: {} -> {}", n.params, params)),
            Some(n) => self.inputs_match(n),
        };
        let reason = match need {
            None => {
                report.still_valid.push((sub_id.into(), "normalized input and buffer unchanged".into()));
                return;
            }
            Some(r) => r,
        };
        let norm_curve = self.nodes.get(norm_id).and_then(|n| n.curve.clone()).expect("normalized curve");
        let buf = self.sources.get(buffer_id).expect("buffer source").clone();
        let buf_curve = buf.curve.clone().expect("buffer curve");
        let outcome = rules::subtract(
            &norm_curve,
            &buf_curve,
            &rules::SubtractParams { scale, scale_sigma },
            sample_id,
        );
        let inputs = vec![
            (norm_id.to_string(), self.fingerprint_of(norm_id)),
            (buffer_id.to_string(), self.fingerprint_of(buffer_id)),
        ];
        let status = outcome.status.clone();
        let node = DerivedNode {
            id: sub_id.into(),
            kind: NodeKind::Subtracted,
            inputs,
            params,
            status: status.clone(),
            curve: outcome.curve,
            candidates: vec![],
            diagnostics: outcome.diagnostics,
            validity_note: format!("recomputed: {}; mode={}", reason, outcome.mode),
        };
        match &status {
            NodeStatus::Failed(code) => report.failed.push((sub_id.into(), code.clone())),
            _ => report.recomputed.push((sub_id.into(), reason)),
        }
        self.nodes.insert(sub_id.into(), node);
        self.touch(sub_id);
    }

    fn ensure_residual(&mut self, sub_id: &str, report: &mut RecomputeReport) {
        let res_id = format!("{}#residual", sub_id);
        let need = match self.nodes.get(&res_id) {
            None => Some("no previous result".to_string()),
            Some(n) => self.inputs_match(n),
        };
        let reason = match need {
            None => {
                report.still_valid.push((res_id, "subtraction unchanged; residual kept".into()));
                return;
            }
            Some(r) => r,
        };
        let sub = self.nodes.get(sub_id).expect("subtracted node").clone();
        let (curve, mut diags, status) = match &sub.curve {
            Some(c) => (c.clone(), vec![], NodeStatus::Valid),
            None => (Curve { q_unit: String::new(), i_unit: String::new(), points: vec![] },
                     vec![Diagnostic { code: "upstream-failed".into(), message: "subtraction failed; residual unavailable".into(), subject: Some(sub_id.into()) }],
                     NodeStatus::Failed("upstream-failed".into())),
        };
        let stats = if sub.curve.is_some() {
            let hi = curve.q_max();
            let lo = (hi * 0.6).max(curve.q_min());
            rules::residual_stats(&curve, lo, hi)
        } else {
            json!(null)
        };
        diags.extend(sub.diagnostics.iter().filter(|d| d.code == "negative-intensity").cloned());
        let node = DerivedNode {
            id: res_id.clone(),
            kind: NodeKind::Residual,
            inputs: vec![(sub_id.into(), self.fingerprint_of(sub_id))],
            params: json!({"stats": stats}),
            status,
            curve: if sub.curve.is_some() { Some(curve) } else { None },
            candidates: vec![],
            diagnostics: diags,
            validity_note: format!("recomputed: {}", reason),
        };
        self.nodes.insert(res_id.clone(), node);
        self.touch(&res_id);
        report.recomputed.push((res_id, reason));
    }

    fn ensure_intervals(&mut self, sub_id: &str, kind: NodeKind, report: &mut RecomputeReport) {
        let tag = match kind { NodeKind::Guinier => "guinier", NodeKind::Porod => "porod", _ => unreachable!() };
        let id = format!("{}#{}", sub_id, tag);
        let need = match self.nodes.get(&id) {
            None => Some("no previous result".to_string()),
            Some(n) => self.inputs_match(n),
        };
        let reason = match need {
            None => {
                report.still_valid.push((id, "subtraction unchanged; interval candidates kept".into()));
                return;
            }
            Some(r) => r,
        };
        let sub = self.nodes.get(sub_id).expect("subtracted node").clone();
        let (candidates, status) = match &sub.curve {
            Some(c) => {
                let cands = match kind {
                    NodeKind::Guinier => rules::guinier_candidates(&id, c, 8, 2),
                    NodeKind::Porod => rules::porod_candidates(&id, c, 8, 2),
                    _ => unreachable!(),
                };
                (cands, NodeStatus::Valid)
            }
            None => (vec![], NodeStatus::Failed("upstream-failed".into())),
        };
        // overlap evidence between guinier & porod kept as diagnostics, never merged
        let mut diags = vec![];
        if kind == NodeKind::Porod {
            let gid = format!("{}#guinier", sub_id);
            if let Some(g) = self.nodes.get(&gid) {
                for (a, b) in rules::overlapping_pairs(&g.candidates, &candidates) {
                    diags.push(Diagnostic {
                        code: "interval-overlap".into(),
                        message: format!("guinier {} overlaps porod {}; kept as separate candidates", a, b),
                        subject: Some(id.clone()),
                    });
                }
            }
        }
        let node = DerivedNode {
            id: id.clone(),
            kind,
            inputs: vec![(sub_id.into(), self.fingerprint_of(sub_id))],
            params: json!({}),
            status,
            curve: None,
            candidates,
            diagnostics: diags,
            validity_note: format!("recomputed: {}", reason),
        };
        self.nodes.insert(id.clone(), node);
        self.touch(&id);
        report.recomputed.push((id, reason));
    }
}
