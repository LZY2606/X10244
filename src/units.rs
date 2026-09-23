//! 单位与坐标基准：所有领域量携带单位，导入时保留原始表示，
//! 派生层才做规范化；无法转换的值留在原位置并附诊断。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QUnit {
    InvAngstrom,
    InvNanometer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IUnit {
    InvCm,
    InvMm,
    Arbitrary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LenUnit {
    Cm,
    Mm,
}

/// 导入时保留原始文本表示的量。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawQuantity {
    pub raw: String,
    pub declared_unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConvertError {
    Unparseable(String),
    UnknownUnit(String),
}

pub fn parse_q_unit(s: &str) -> Option<QUnit> {
    match s.trim().to_ascii_lowercase().as_str() {
        "a^-1" | "1/a" | "angstrom^-1" | "ang^-1" | "1/angstrom" => Some(QUnit::InvAngstrom),
        "nm^-1" | "1/nm" => Some(QUnit::InvNanometer),
        _ => None,
    }
}

pub fn parse_i_unit(s: &str) -> Option<IUnit> {
    match s.trim().to_ascii_lowercase().as_str() {
        "cm^-1" | "1/cm" => Some(IUnit::InvCm),
        "mm^-1" | "1/mm" => Some(IUnit::InvMm),
        "arb" | "arbitrary" | "counts" => Some(IUnit::Arbitrary),
        _ => None,
    }
}

/// q 值换算到 Å⁻¹（派生层规范化目标）。
pub fn q_to_inv_angstrom(v: f64, u: QUnit) -> f64 {
    match u {
        QUnit::InvAngstrom => v,
        QUnit::InvNanometer => v / 10.0,
    }
}

pub fn i_to_inv_cm(v: f64, u: IUnit) -> Option<f64> {
    match u {
        IUnit::InvCm => Some(v),
        IUnit::InvMm => Some(v * 10.0),
        IUnit::Arbitrary => None, // 任意单位不可换算为绝对强度
    }
}

/// 解析厚度等长度量，返回厘米。无法解析时返回诊断，原值留在原位置。
pub fn parse_length_cm(raw: &RawQuantity) -> Result<f64, ConvertError> {
    let text = raw.raw.trim();
    let (num_part, unit_part) = split_num_unit(text);
    let value: f64 = num_part
        .parse()
        .map_err(|_| ConvertError::Unparseable(text.to_string()))?;
    let unit = raw
        .declared_unit
        .clone()
        .unwrap_or_else(|| unit_part.to_string());
    match unit.trim().to_ascii_lowercase().as_str() {
        "cm" => Ok(value),
        "mm" => Ok(value / 10.0),
        "" => Err(ConvertError::UnknownUnit("(missing)".into())),
        other => Err(ConvertError::UnknownUnit(other.to_string())),
    }
}

pub fn parse_f64(raw: &RawQuantity) -> Result<f64, ConvertError> {
    raw.raw
        .trim()
        .parse()
        .map_err(|_| ConvertError::Unparseable(raw.raw.clone()))
}

fn split_num_unit(s: &str) -> (&str, &str) {
    let idx = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E'))
        .unwrap_or(s.len());
    (s[..idx].trim(), s[idx..].trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_units_convert() {
        let q = RawQuantity { raw: "1.5 mm".into(), declared_unit: None };
        assert!((parse_length_cm(&q).unwrap() - 0.15).abs() < 1e-12);
        let q = RawQuantity { raw: "0.15".into(), declared_unit: Some("cm".into()) };
        assert!((parse_length_cm(&q).unwrap() - 0.15).abs() < 1e-12);
    }

    #[test]
    fn unconvertible_stays_with_diagnostic() {
        let q = RawQuantity { raw: "n/a".into(), declared_unit: Some("cm".into()) };
        assert!(matches!(parse_length_cm(&q), Err(ConvertError::Unparseable(_))));
        let q = RawQuantity { raw: "1.0".into(), declared_unit: Some("furlong".into()) };
        assert!(matches!(parse_length_cm(&q), Err(ConvertError::UnknownUnit(_))));
    }
}
