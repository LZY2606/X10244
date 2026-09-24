//! 领域量的单位与坐标基准。
//! 导入层保留原始表示；派生层才调用本模块做规范化。
//! 无法转换的值返回诊断，由调用方留在原位置，不得丢弃。

use serde::{Deserialize, Serialize};

/// 规范化后的坐标基准：q 一律 nm^-1，强度一律 cm^-1，时间 s，厚度 cm，透射率无量纲。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub raw_value: f64,
    pub raw_unit: String,
}

pub type ConvResult = Result<f64, Diagnostic>;

fn diag(code: &str, msg: impl Into<String>, raw_value: f64, raw_unit: &str) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        message: msg.into(),
        raw_value,
        raw_unit: raw_unit.to_string(),
    }
}

pub fn q_to_nm_inv(value: f64, unit: &str) -> ConvResult {
    match unit {
        "nm^-1" | "1/nm" | "nm-1" => Ok(value),
        "A^-1" | "1/A" | "angstrom^-1" => Ok(value * 10.0),
        other => Err(diag(
            "unknown_q_unit",
            format!("无法识别的 q 单位 '{other}'，值保留在原始层"),
            value,
            other,
        )),
    }
}

pub fn intensity_to_cm_inv(value: f64, unit: &str) -> ConvResult {
    match unit {
        "cm^-1" | "1/cm" | "cm-1" => Ok(value),
        "m^-1" | "1/m" => Ok(value / 100.0),
        "arb" | "a.u." | "counts" => Err(diag(
            "non_absolute_intensity",
            "强度为任意单位，缺少绝对标定，不能规范化为 cm^-1",
            value,
            unit,
        )),
        other => Err(diag(
            "unknown_intensity_unit",
            format!("无法识别的强度单位 '{other}'"),
            value,
            other,
        )),
    }
}

pub fn time_to_s(value: f64, unit: &str) -> ConvResult {
    match unit {
        "s" | "sec" => Ok(value),
        "ms" => Ok(value / 1000.0),
        "min" => Ok(value * 60.0),
        other => Err(diag("unknown_time_unit", format!("无法识别的时间单位 '{other}'"), value, other)),
    }
}

pub fn thickness_to_cm(value: f64, unit: &str) -> ConvResult {
    match unit {
        "cm" => Ok(value),
        "mm" => Ok(value / 10.0),
        other => Err(diag(
            "unknown_thickness_unit",
            format!("无法识别的厚度单位 '{other}'"),
            value,
            other,
        )),
    }
}

/// 透射率必须是无量纲且在 (0, 1]；超出范围是物理诊断而非单位问题。
pub fn transmission(value: f64, unit: &str) -> ConvResult {
    match unit {
        "1" | "dimensionless" | "" => {
            if value > 0.0 && value <= 1.0 {
                Ok(value)
            } else {
                Err(diag(
                    "transmission_out_of_range",
                    format!("透射率 {value} 不在 (0,1] 区间"),
                    value,
                    unit,
                ))
            }
        }
        "%" => {
            let v = value / 100.0;
            if v > 0.0 && v <= 1.0 {
                Ok(v)
            } else {
                Err(diag("transmission_out_of_range", format!("透射率 {value}% 不在 (0,100]"), value, unit))
            }
        }
        other => Err(diag(
            "unknown_transmission_unit",
            format!("无法识别的透射率单位 '{other}'"),
            value,
            other,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q_units_convert() {
        assert_eq!(q_to_nm_inv(0.1, "A^-1").unwrap(), 1.0);
        assert_eq!(q_to_nm_inv(0.5, "nm^-1").unwrap(), 0.5);
        assert!(q_to_nm_inv(0.5, "pixels").is_err());
    }

    #[test]
    fn unconvertible_kept_with_diagnostic() {
        let d = intensity_to_cm_inv(12.0, "counts").unwrap_err();
        assert_eq!(d.code, "non_absolute_intensity");
        assert_eq!(d.raw_value, 12.0);
    }

    #[test]
    fn transmission_range_checked() {
        assert!(transmission(0.32, "1").is_ok());
        assert!(transmission(1.4, "1").is_err());
        assert_eq!(transmission(32.0, "%").unwrap(), 0.32);
    }
}
