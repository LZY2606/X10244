//! 稳定身份（content-addressed）。
//!
//! 导入材料、规则版本、派生结果都有稳定 ID：
//! 同一内容（含来源指针）永远得到同一 ID，因此“导出 → 删除本地库 → 导入”后
//! 身份、顺序与来源关系保持不变。事件 ID 含纳秒时间戳，保持发生顺序。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub fn content_id(kind: &str, canonical_json: &str) -> String {
    let mut h = Sha256::new();
    h.update(kind.as_bytes());
    h.update(b"\0");
    h.update(canonical_json.as_bytes());
    format!("{}:{:.14}", kind, hex::encode(h.finalize()))
}

/// 确定性的短哈希（用于派生点 ID 等）。
pub fn short_hash(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"\0");
    }
    hex::encode(h.finalize())[..16].to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author(String);
impl Author {
    pub fn new(s: &str) -> Self {
        Author(s.trim().to_string())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
