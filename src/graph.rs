//! 依赖图传播：每个派生节点版本由「输入版本 + 规则版本」指纹决定，
//! 未变化的节点不重算；当前指针只在完整成功后切换。

use crate::domain::{Payload, RULE_VERSION};
use serde::{Deserialize, Serialize};

/// 确定性 64 位 FNV-1a 指纹（输入是规范 JSON）。
pub fn fingerprint(canonical: &str) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut h = OFFSET;
    for b in canonical.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(PRIME);
    }
    format!("{:016x}", h)
}

/// 版本是否有效的结构标记（无效版本保留，指针不切换）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Validity {
    pub ok: bool,
    pub codes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeState {
    pub entity_id: String,
    pub kind: &'static str,
    pub current_version: Option<String>,
    pub reused: bool,
    pub created_version: Option<String>,
    pub valid: Validity,
    pub reason: String,
}

pub fn payload_fingerprint(entity_id: &str, inputs: &[String], payload_json: &str) -> String {
    let mut canon = String::new();
    canon.push_str(RULE_VERSION);
    canon.push('|');
    canon.push_str(entity_id);
    canon.push('|');
    canon.push_str(&inputs.join(","));
    canon.push('|');
    canon.push_str(payload_json);
    fingerprint(&canon)
}

/// 从归一化/扣除结果提取判定代码（决定版本是否 valid 可采用）。
pub fn validity_codes(p: &Payload) -> Vec<String> {
    let mut codes = Vec::new();
    match p {
        Payload::Norm(n) => {
            for d in &n.diagnostics {
                codes.push(d.code.clone());
            }
        }
        Payload::Sub(s) => {
            for d in &s.diagnostics {
                codes.push(d.code.clone());
            }
        }
        Payload::Raw(_) | Payload::Intervals(_) => {}
    }
    codes
}

pub fn is_adoptable(p: &Payload) -> bool {
    // NO_OVERLAP / SCALE_UNDETERMINED 使扣除结果不可采用；
    // 网格错位（走插值）与负强度仍是可审阅结果，只是带诊断。
    !validity_codes(p).iter().any(|c| c == "NO_OVERLAP" || c == "SCALE_UNDETERMINED")
}
