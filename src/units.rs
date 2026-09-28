//! 单位与坐标基准。所有领域量都携带单位；导入时保留原始表示，
//! 派生层才做规范化；无法转换的值留在原位置并附诊断。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QUnit {
    #[serde(rename = "1/angstrom")]
    InvAngstrom,
    #[serde(rename = "1/nm")]
    InvNanometer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IUnit {
    #[serde(rename = "1/cm")]
    InvCentimeter,
    #[serde(rename = "arbitrary")]
    Arbitrary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
}

/// 转换 q 到规范单位 1/angstrom。无法识别单位时返回 None（调用方保留原值并记录诊断）。
pub fn q_to_angstrom(value: f64, unit: &str) -> Option<f64> {
    match unit {
        "1/angstrom" | "1/A" | "A^-1" => Some(value),
        "1/nm" | "nm^-1" => Some(value / 10.0),
        _ => None,
    }
}

/// 转换强度到规范单位 1/cm。
pub fn i_to_inv_cm(value: f64, unit: &str) -> Option<f64> {
    match unit {
        "1/cm" | "cm^-1" => Some(value),
        "arbitrary" | "arb" => Some(value), // 保留数值但语义上仍是任意单位，由调用方决定是否接受
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub context: String,
}

impl Diagnostic {
    pub fn new(code: &str, message: impl Into<String>, context: impl Into<String>) -> Self {
        Self { code: code.to_string(), message: message.into(), context: context.into() }
    }
}
