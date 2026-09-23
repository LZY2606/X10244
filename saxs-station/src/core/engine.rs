//! 依赖图与增量重算。
//!
//! 图：Dataset → Normalized → Subtraction → ModelFit。
//! 节点身份是内容寻址（输入哈希 + 规则版本）：源数据改动后，
//! 只有受影响的下游节点被重算；其它节点原样复用并可说明“为何仍有效”。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::core::id::content_id;
use crate::core::model::{
    Dataset, Diagnostic, ModelFit, Normalized, Subtraction,
};
use crate::core::rules::{fit_models, normalize, subtract_background};
use crate::RULE_VERSION;

/// 派生节点载荷（成功或携带错误诊断的失败）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeOutput {
    Normalized(Normalized),
    Subtraction(Subtraction),
    ModelFit(ModelFit),
    Failed { diagnostics: Vec<Diagnostic> },
}

impl NodeOutput {
    pub fn label(&self) -> &'static str {
        match self {
            NodeOutput::Normalized(_) => "normalized",
            NodeOutput::Subtraction(_) => "subtraction",
            NodeOutput::ModelFit(_) => "model_fit",
            NodeOutput::Failed { .. } => "failed",
        }
    }
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        match self {
            NodeOutput::Normalized(_) => Vec::new(),
            NodeOutput::Subtraction(s) => s.diagnostics.clone(),
            NodeOutput::ModelFit(m) => m.diagnostics.clone(),
            NodeOutput::Failed { diagnostics } => diagnostics.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivNode {
    pub id: String,
    pub kind: String,
    /// 有向依赖（节点 id）。
    pub inputs: Vec<String>,
    pub output: NodeOutput,
    pub created_event: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Explanation {
    pub node_id: String,
    pub recomputed: bool,
    pub reason: String,
    pub inputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecomputeReport {
    pub touched: Vec<Explanation>,
    pub changed: Vec<String>,
}

/// 一个可重放的依赖图状态。槽位用稳定 key 寻址：
/// - `norm:<dataset_id>`
/// - `sub:<session_id>`
/// - `fit:<session_id>`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    pub datasets: HashMap<String, Dataset>,
    pub dataset_order: Vec<String>,
    pub nodes: HashMap<String, DerivNode>,
    pub slots: HashMap<String, String>,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn node(&self, id: &str) -> Option<&DerivNode> {
        self.nodes.get(id)
    }

    pub fn slot(&self, key: &str) -> Option<&DerivNode> {
        self.slots.get(key).and_then(|id| self.nodes.get(id))
    }
}

fn hash_inputs(kind: &str, inputs: &[String], extra: &str) -> String {
    let canonical = serde_json::json!({
        "rule": RULE_VERSION,
        "inputs": inputs,
        "extra": extra,
    })
    .to_string();
    content_id(kind, &canonical)
}

impl Graph {
    /// 加入（或确认已存在）一份原始材料，返回其稳定 ID。
    pub fn add_dataset(&mut self, dataset: Dataset) -> String {
        let canonical = serde_json::to_string(&dataset).unwrap_or_default();
        let id = content_id("dataset", &canonical);
        if !self.datasets.contains_key(&id) {
            self.dataset_order.push(id.clone());
            self.datasets.insert(id.clone(), dataset);
        }
        id
    }

    /// 计算/复用一份材料的归一化节点。
    pub fn ensure_normalized(
        &mut self,
        dataset_id: &str,
        report: Option<&mut Vec<Explanation>>,
    ) -> Option<String> {
        let dataset = self.datasets.get(dataset_id)?.clone();
        let canonical = serde_json::to_string(&dataset).unwrap_or_default();
        let id = hash_inputs("norm", &[dataset_id.into()], &canonical);
        if let Some(node) = self.nodes.get(&id) {
            if let Some(r) = report {
                r.push(Explanation {
                    node_id: id.clone(),
                    recomputed: false,
                    reason: "输入材料内容与规则版本均未变化，归一化节点仍有效，直接复用".into(),
                    inputs: vec![dataset_id.into()],
                });
            }
            self.slots.insert(format!("norm:{dataset_id}"), id.clone());
            return Some(id);
        }
        let output = match normalize(&dataset) {
            Ok(n) => NodeOutput::Normalized(n),
            Err(diagnostics) => NodeOutput::Failed { diagnostics },
        };
        self.nodes.insert(
            id.clone(),
            DerivNode { id: id.clone(), kind: "normalized".into(), inputs: vec![dataset_id.into()], output, created_event: None },
        );
        self.slots.insert(format!("norm:{dataset_id}"), id.clone());
        if let Some(r) = report {
            r.push(Explanation {
                node_id: id.clone(),
                recomputed: true,
                reason: "材料内容或规则版本变化，重新归一化".into(),
                inputs: vec![dataset_id.into()],
            });
        }
        Some(id)
    }

    /// 建立一次背景扣除会话（槽位 `sub:<session>`），指定样品/缓冲液材料与可选候选。
    pub fn start_subtraction(
        &mut self,
        session: &str,
        sample_id: &str,
        buffer_id: &str,
        candidate_id: Option<&str>,
        report: Option<&mut Vec<Explanation>>,
    ) -> Result<String, String> {
        let mut local = Vec::new();
        let sn = self
            .ensure_normalized(sample_id, Some(&mut local))
            .ok_or_else(|| format!("未知样品材料 {sample_id}"))?;
        let bn = self
            .ensure_normalized(buffer_id, Some(&mut local))
            .ok_or_else(|| format!("未知缓冲液材料 {buffer_id}"))?;
        let sample = match &self.nodes[&sn].output {
            NodeOutput::Normalized(n) => n.clone(),
            _ => return Err("样品归一化失败，无法扣除背景".into()),
        };
        let buffer = match &self.nodes[&bn].output {
            NodeOutput::Normalized(n) => n.clone(),
            _ => return Err("缓冲液归一化失败，无法扣除背景".into()),
        };
        let sample_name = self.datasets[sample_id].name.clone();
        let buffer_name = self.datasets[buffer_id].name.clone();

        let mut sub = match subtract_background(&sample, &buffer, &sample_name, &buffer_name) {
            Ok(s) => s,
            Err(diagnostics) => {
                let id = hash_inputs(
                    "sub",
                    &[sn.clone(), bn.clone(), candidate_id.unwrap_or("adopted").into()],
                    &serde_json::to_string(&candidate_id).unwrap_or_default(),
                );
                self.nodes.insert(
                    id.clone(),
                    DerivNode {
                        id: id.clone(),
                        kind: "subtraction".into(),
                        inputs: vec![sn.clone(), bn.clone()],
                        output: NodeOutput::Failed { diagnostics },
                        created_event: None,
                    },
                );
                self.slots.insert(format!("sub:{session}"), id.clone());
                return Ok(id);
            }
        };

        // 若指定非采纳候选，则以该候选重算（多解全部可实例化）。
        if let Some(want) = candidate_id {
            let chosen = want.to_string();
            // 从已计算候选中找：当前规则内置两候选，直接重估。
            let cand = estimate_named_candidate(&sample, &buffer, &chosen);
            match cand {
                Some((scale, sigma, cid)) => {
                    sub.scale_candidate_id = cid;
                    sub.scale = scale;
                    sub.scale_sigma = sigma;
                    sub = rebuild_subtraction_with_scale(&sample, &buffer, sub, scale, sigma, &chosen);
                }
                None => {
                    let id = hash_inputs(
                        "sub",
                        &[sn.clone(), bn.clone(), chosen.clone()],
                        &chosen,
                    );
                    self.nodes.insert(
                        id.clone(),
                        DerivNode {
                            id: id.clone(),
                            kind: "subtraction".into(),
                            inputs: vec![sn, bn],
                            output: NodeOutput::Failed {
                                diagnostics: vec![Diagnostic {
                                    code: "CANDIDATE_NOT_FOUND".into(),
                                    severity: "error".into(),
                                    message: format!("候选 {chosen} 无法在当前输入上重算"),
                                }],
                            },
                            created_event: None,
                        },
                    );
                    self.slots.insert(format!("sub:{session}"), id.clone());
                    return Ok(id);
                }
            }
        }

        let key_extra = serde_json::to_string(&candidate_id).unwrap_or_default();
        let id = hash_inputs("sub", &[sn.clone(), bn.clone(), key_extra.clone()], &key_extra);
        self.nodes.insert(
            id.clone(),
            DerivNode {
                id: id.clone(),
                kind: "subtraction".into(),
                inputs: vec![sn, bn],
                output: NodeOutput::Subtraction(sub),
                created_event: None,
            },
        );
        self.slots.insert(format!("sub:{session}"), id.clone());
        if let Some(r) = report {
            r.push(Explanation {
                node_id: id.clone(),
                recomputed: true,
                reason: "建立/更新背景扣除会话".into(),
                inputs: local.iter().map(|e| e.node_id.clone()).collect(),
            });
        }
        Ok(id)
    }

    /// 对会话当前扣除结果做 Guinier/Porod 多区间拟合。
    pub fn ensure_fit(
        &mut self,
        session: &str,
        report: Option<&mut Vec<Explanation>>,
    ) -> Option<String> {
        let sub_id = self.slots.get(&format!("sub:{session}"))?.clone();
        let id = hash_inputs("fit", &[sub_id.clone()], session);
        if self.nodes.contains_key(&id) {
            if let Some(r) = report {
                r.push(Explanation {
                    node_id: id.clone(),
                    recomputed: false,
                    reason: "上游扣除节点未变化，多区间模型候选仍有效".into(),
                    inputs: vec![sub_id],
                });
            }
            self.slots.insert(format!("fit:{session}"), id.clone());
            return Some(id);
        }
        let output = match &self.nodes.get(&sub_id)?.output {
            NodeOutput::Subtraction(s) => NodeOutput::ModelFit(fit_models(s)),
            NodeOutput::Failed { diagnostics } => {
                NodeOutput::Failed { diagnostics: diagnostics.clone() }
            }
            _ => NodeOutput::Failed {
                diagnostics: vec![Diagnostic {
                    code: "UPSTREAM_NOT_SUBTRACTION".into(),
                    severity: "error".into(),
                    message: "上游不是扣除结果".into(),
                }],
            },
        };
        self.nodes.insert(
            id.clone(),
            DerivNode { id: id.clone(), kind: "model_fit".into(), inputs: vec![sub_id], output, created_event: None },
        );
        self.slots.insert(format!("fit:{session}"), id.clone());
        if let Some(r) = report {
            r.push(Explanation {
                node_id: id.clone(),
                recomputed: true,
                reason: "扣除结果变化，重算 Guinier/Porod 候选".into(),
                inputs: vec![],
            });
        }
        Some(id)
    }
}
