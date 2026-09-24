use serde_json::Value;
use sha2::{Digest, Sha256};

/// 稳定身份：内容寻址哈希。相同内容在任何进程、任何机器上得到相同 id，
/// 因此导出/删除/再导入后身份保持不变。
pub fn content_id(kind: &str, payload: &Value) -> String {
    let canonical = serde_json::to_string(payload).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(kind.as_bytes());
    h.update(b"\0");
    h.update(canonical.as_bytes());
    let digest = h.finalize();
    let hex: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
    format!("{kind}-{hex}")
}

/// 事件 id：由序号 + 种类 + 负载决定，重放时完全确定。
pub fn event_id(seq: i64, kind: &str, payload: &Value) -> String {
    let mut p = payload.clone();
    p["__seq"] = Value::from(seq);
    content_id(&format!("evt-{kind}"), &p)
}
