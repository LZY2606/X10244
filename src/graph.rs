//! Dependency graph with lazy, selective propagation.
//! A revision of one retained source only invalidates derived objects whose
//! content actually changed; unchanged nodes stay valid and are never recomputed.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::Obj;

#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub kind: &'static str,
    pub inputs: Vec<String>,
    pub rule_version: String,
}

#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub nodes: BTreeMap<String, Node>,
    /// object id -> object ids that consume it
    pub dependents: BTreeMap<String, BTreeSet<String>>,
}

impl Graph {
    pub fn from_objects(objs: &[Obj]) -> Graph {
        let mut g = Graph::default();
        for o in objs {
            g.add_obj(o);
        }
        g
    }

    pub fn add_obj(&mut self, o: &Obj) {
        let id = o.object_id_ref().to_string();
        let kind = match o {
            Obj::Raw { .. } => "raw",
            Obj::Norm { .. } => "norm",
            Obj::Sub { .. } => "sub",
            Obj::Candidates { .. } => "candidates",
        };
        let rule = match o {
            Obj::Raw { .. } => "import-1.0",
            Obj::Norm { curve, .. } => curve.params.rule_version.as_str(),
            Obj::Sub { curve, .. } => curve
                .params
                .as_ref()
                .map(|p| p.rule_version.as_str())
                .unwrap_or(crate::engine::RULE_VERSION),
            Obj::Candidates { provenance, .. } => provenance.rule_version.as_str(),
        };
        let node = Node {
            id: id.clone(),
            kind,
            inputs: o.source_inputs(),
            rule_version: rule.to_string(),
        };
        for input in &node.inputs {
            self.dependents
                .entry(input.clone())
                .or_default()
                .insert(id.clone());
        }
        self.nodes.insert(id, node);
    }

    /// All transitive dependents of the given source object ids.
    pub fn downstream(&self, roots: &[String]) -> BTreeSet<String> {
        let mut affected = BTreeSet::new();
        let mut stack: Vec<String> = roots.to_vec();
        while let Some(id) = stack.pop() {
            if let Some(deps) = self.dependents.get(&id) {
                for d in deps {
                    if affected.insert(d.clone()) {
                        stack.push(d.clone());
                    }
                }
            }
        }
        affected
    }

    /// Compare a pre-revision and post-revision graph (same node ids for
    /// unchanged content, new content-hash ids for changed content) and
    /// report which previously existing derived ids must be recomputed.
    pub fn propagation_report(
        &self,
        new_graph: &Graph,
        changed_sources: &[String],
    ) -> PropagationReport {
        let mut recompute = Vec::new();
        let mut still_valid = Vec::new();
        let affected = self.downstream(changed_sources);
        for id in &affected {
            if new_graph.nodes.contains_key(id) {
                still_valid.push(id.clone());
            } else {
                recompute.push(id.clone());
            }
        }
        PropagationReport {
            changed_sources: changed_sources.to_vec(),
            recompute,
            still_valid,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PropagationReport {
    pub changed_sources: Vec<String>,
    pub recompute: Vec<String>,
    pub still_valid: Vec<String>,
}

impl PropagationReport {
    pub fn explain(&self, g: &Graph) -> Vec<String> {
        let mut out = Vec::new();
        for id in &self.recompute {
            let node = g.nodes.get(id);
            out.push(format!(
                "RECOMPUTE {} ({}): input content changed",
                id,
                node.map(|n| n.kind).unwrap_or("?")
            ));
        }
        for id in &self.still_valid {
            let node = g.nodes.get(id);
            out.push(format!(
                "STILL_VALID {} ({}): content-addressed id unchanged, inputs identical",
                id,
                node.map(|n| n.kind).unwrap_or("?")
            ));
        }
        out
    }
}
