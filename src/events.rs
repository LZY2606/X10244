//! 机器可读事件：每次输入、推导、裁定都是不可变事件。
//! 回到旧事件即恢复当时的依赖版本与可见结论，而非用最新结果覆盖历史。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// 原始材料导入或修订。
    MaterialImport,
    /// 归一化 / 扣背景 / 模型拟合派生。
    Derived,
    /// 人工接受、否定或参数调整。
    Decision,
    /// 冲突解决（两位使用者从同一版本编辑）。
    ConflictResolution,
    /// 从归档导入（身份保持校验）。
    Imported,
    /// 重放标记。
    ReplayMark,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Blocked,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub seq: u64,
    pub kind: EventKind,
    pub action: String,
    pub summary: String,
    pub author: String,
    /// 编辑分支序号：相同 edit_group 上的第二次提交会产生可解决冲突。
    pub edit_group: String,
    /// 该事件提交时看到的 edit_group 头（seq）。
    pub expected_head: Option<u64>,
    /// 被 superseded 的事件 seq 列表（旧决定/旧指针仍保留，仅当前指针移动）。
    pub supersedes: Vec<u64>,
    pub outcome: Outcome,
    /// 受影响对象（material id / node id / version id / point id）。
    pub affected: Vec<String>,
    /// 产生或引用的版本 id（用于时间旅行恢复当时依赖）。
    pub versions: Vec<String>,
    /// 规则版本。
    pub rules: Vec<String>,
    /// 诊断码（与版本诊断对齐）。
    pub codes: Vec<String>,
    /// 结构化负载（参数、候选 id 等）。
    pub payload_json: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionBody {
    pub node_id: String,
    pub version_id: String,
    /// accept / reject / retune
    pub decision: String,
    pub reason: String,
    /// 参数调整时的新参数（会产生新候选版本，旧候选保留）。
    pub new_params_json: Option<String>,
}
