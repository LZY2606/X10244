//! 单位与坐标基准。
//!
//! 规则 RULE_UNITS_V1：原始表示（raw_text/raw_unit）永远保留；只有在派生层才换算到
//! 规范单位。无法转换的值留在原处并附诊断，绝不丢弃。

use serde::{Deserialize, Serialize};

pub const RULE_UNITS: &str = "rules/units@1";

/// 规范坐标基准 / 单位表。
pub const CANON_Q: &str = "A^-1";
pub const CANON_INTENSITY: &str = "a.u."; // 绝对强度 cm^-1 与任意单位都保留其原始基准
pub const CANON_EXPOSURE: &str = "s";
pub const CANON_THICKNESS: &str = "cm";
pub const CANON_DIMENSIONLESS: &str = "1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Quant {
    /// 原始文本表示，逐字保留（例如 "1.20 mm"、"0,42" 未解析情形）。
    pub raw_text: String,
    /// 原始声明单位（逐字保留，可能为空）。
    pub raw_unit: String,
    /// 规范单位下的数值；无法转换时为 None。
    pub value: Option<f64>,
    /// 规范单位标签。
    pub canonical_unit: String,
    /// 转换诊断；None 表示成功。
    pub diagnostic: Option<String>,
}

impl Quant {
    pub fn raw(text: impl Into<String>, raw_unit: impl Into<String>, canonical_unit: &str) -> Self {
        let text = text.into();
        let raw_unit = raw_unit.into();
        let (value, diagnostic) = convert(&text, &raw_unit, canonical_unit);
        Quant { raw_text: text, raw_unit, value, canonical_unit: canonical_unit.to_string(), diagnostic }
    }

    pub fn from_value(value: f64, raw_unit: impl Into<String>, canonical_unit: &str) -> Self {
        let raw_unit = raw_unit.into();
        Quant {
            raw_text: trim_num(value),
            raw_unit: raw_unit.clone(),
            value: Some(value),
            canonical_unit: canonical_unit.to_string(),
            diagnostic: None,
        }
    }

    pub fn ok(&self) -> bool {
        self.value.is_some() && self.diagnostic.is_none()
    }
}

pub fn trim_num(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.10}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn parse_f64_lenient(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let v: f64 = s.parse().ok()?;
    if v.is_finite() { Some(v) } else { None }
}

/// 返回 (规范值, 诊断)。
pub fn convert(text: &str, raw_unit: &str, canonical: &str) -> (Option<f64>, Option<String>) {
    let v = match parse_f64_lenient(text) {
        Some(v) => v,
        None => return (None, Some(format!("{}: 数值无法解析 '{}'", RULE_UNITS, text))),
    };
    let u = raw_unit.trim();
    let factor = match (canonical, u) {
        (c, x) if c == x => 1.0,
        // q 坐标
        ("A^-1", "A^-1") | ("A^-1", "1/A") | ("A^-1", "Å^-1") | ("A^-1", "Å⁻¹") => 1.0,
        ("A^-1", "nm^-1") | ("A^-1", "1/nm") | ("A^-1", "nm⁻¹") => 0.1,
        // 曝光
        ("s", "s") | ("s", "sec") | ("s", "second") | ("s", "seconds") => 1.0,
        ("s", "ms") => 0.001,
        // 厚度
        ("cm", "cm") => 1.0,
        ("cm", "mm") => 0.1,
        ("cm", "um") | ("cm", "µm") | ("cm", "μm") => 1e-4,
        // 强度：绝对强度同义单位；任意单位不做物理换算
        ("cm^-1", "cm^-1") | ("cm^-1", "1/cm") | ("cm^-1", "cm⁻¹") => 1.0,
        ("a.u.", "a.u.") | ("a.u.", "counts") | ("a.u.", "arb") | ("a.u.", "") => 1.0,
        // 无量纲
        ("1", "1") | ("1", "") | ("1", "dimensionless") | ("1", "T") => 1.0,
        _ => return (None, Some(format!("{}: 单位 '{}' 无法转换到规范基准 '{}'，原始表示已保留", RULE_UNITS, u, canonical))),
    };
    (Some(v * factor), None)
}
