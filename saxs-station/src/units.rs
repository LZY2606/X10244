//! 领域量的单位与坐标基准。
//!
//! 规则：导入层保留原始表示（字符串），派生层才规范化；
//! 无法转换的值留在原位置并附诊断（`ConvertIssue`），绝不丢弃。

use serde::{Deserialize, Serialize};

/// 规范化后的坐标/物理基准。所有派生量统一使用这些单位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CanonicalUnit {
    /// q：1/Å
    QInvAngstrom,
    /// I：cm⁻¹（绝对尺度），原始探测器计数也换算到这一尺度
    IcmInv,
    /// 曝光：s
    Seconds,
    /// 样品厚度：cm
    Cm,
    /// 透射率：无量纲
    Dimensionless,
    /// 仪器响应：无量纲乘子
    ResponseFactor,
}

impl CanonicalUnit {
    pub fn label(&self) -> &'static str {
        match self {
            CanonicalUnit::QInvAngstrom => "1/A",
            CanonicalUnit::IcmInv => "cm^-1",
            CanonicalUnit::Seconds => "s",
            CanonicalUnit::Cm => "cm",
            CanonicalUnit::Dimensionless => "-",
            CanonicalUnit::ResponseFactor => "response factor (-)",
        }
    }
}

/// 一次单位转换：原始字符串 → 数值（规范单位），或一条诊断。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvertIssue {
    pub field: String,
    pub raw: String,
    pub problem: String,
}

/// 单标量的“原始表示 + 规范值”。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WithUnit {
    pub raw: String,
    pub canonical: CanonicalUnit,
    /// 转换成功为 Some；无法转换时为 None，问题记录在 issue。
    pub value: Option<f64>,
    pub issue: Option<ConvertIssue>,
}

impl WithUnit {
    pub fn new(field: &str, raw: &str, canonical: CanonicalUnit, value: Option<f64>) -> Self {
        let issue = value.is_none().then(|| ConvertIssue {
            field: field.to_string(),
            raw: raw.to_string(),
            problem: format!("无法转换为规范单位 {}", canonical.label()),
        });
        WithUnit { raw: raw.to_string(), canonical, value, issue }
    }

    pub fn ok(field: &str, raw: &str, canonical: CanonicalUnit, value: f64) -> Self {
        Self::new(field, raw, canonical, Some(value))
    }
}

fn parse_num(raw: &str) -> Option<f64> {
    let t = raw.trim();
    // 接受 "1.2e-3"、"0.05 mm"、"1,5 mm"（部分仪器导出使用逗号小数）
    let t = t.replace(',', ".");
    let mut num = String::new();
    for ch in t.chars() {
        if ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+' | 'e' | 'E') {
            num.push(ch);
        } else if !num.is_empty() {
            break;
        }
    }
    num.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// 曝光时间：接受 s / sec / seconds / ms。
pub fn convert_exposure(field: &str, raw: &str) -> WithUnit {
    let r = raw.trim().to_lowercase();
    let n = parse_num(raw);
    let value = n.and_then(|n| {
        if r.ends_with("ms") {
            Some(n / 1000.0)
        } else if r.is_empty() || r.ends_with('s') || r.contains("sec") {
            Some(n)
        } else {
            None
        }
    });
    WithUnit::new(field, raw, CanonicalUnit::Seconds, value)
}

/// 厚度：cm / mm / m → cm。
pub fn convert_thickness(field: &str, raw: &str) -> WithUnit {
    let r = raw.trim().to_lowercase();
    let n = parse_num(raw);
    let value = n.and_then(|n| {
        if r.ends_with("mm") {
            Some(n / 10.0)
        } else if r.ends_with("cm") || (!r.is_empty() && !r.contains('m')) {
            Some(n)
        } else if r.ends_with("m ") || r == "m" {
            Some(n * 100.0)
        } else {
            None
        }
    });
    WithUnit::new(field, raw, CanonicalUnit::Cm, value)
}

/// 透射率：0..=1 数值或百分数。
pub fn convert_transmission(field: &str, raw: &str) -> WithUnit {
    let r = raw.trim().to_lowercase();
    let n = parse_num(raw);
    let value = n.map(|mut n| {
        if r.contains('%') {
            n /= 100.0;
        }
        n
    });
    let value = value.filter(|t| (0.0..=1.0).contains(t));
    WithUnit::new(field, raw, CanonicalUnit::Dimensionless, value)
}

/// q 数组：1/Å 与 nm⁻¹。
pub fn convert_q(field: &str, raw: &str, q: f64) -> WithUnit {
    let r = raw.trim().to_lowercase();
    let factor = if r.contains("nm") { 0.1 } else { 1.0 };
    WithUnit::ok(field, raw, CanonicalUnit::QInvAngstrom, q * factor)
}

/// 仪器响应乘子（1.0 = 无修正）。
pub fn convert_response(raw: &str) -> WithUnit {
    let n = parse_num(raw).filter(|n| *n > 0.0);
    WithUnit::new("response", raw, CanonicalUnit::ResponseFactor, n)
}

/// 探测器计数 → 绝对尺度的转换元数据（保留原始表示）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScaleToAbsolute {
    /// 原始文件中声明的单位，如 "counts/s"、"a.u."、"cm^-1"。
    pub raw_unit: String,
    /// 乘到归一化结果上的系数；None 表示保持相对尺度。
    pub factor: Option<f64>,
    pub note: String,
}

impl Default for ScaleToAbsolute {
    fn default() -> Self {
        ScaleToAbsolute { raw_unit: "counts/s".into(), factor: None, note: "相对尺度，未做绝对定标".into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_and_failures() {
        assert_eq!(convert_exposure("exposure", "1.0 s").value, Some(1.0));
        assert_eq!(convert_exposure("exposure", "200 ms").value, Some(0.2));
        assert!(convert_exposure("exposure", "abc").value.is_none());
        assert_eq!(convert_thickness("thickness", "1.5 mm").value, Some(0.15));
        assert_eq!(convert_thickness("thickness", "0.2cm").value, Some(0.2));
        assert_eq!(convert_transmission("transmission", "87%").value, Some(0.87));
        assert!(convert_transmission("transmission", "1.4").value.is_none());
        assert!((convert_q("q", "nm^-1", 1.0).value.unwrap() - 0.1).abs() < 1e-12);
        // 失败时原始字符串保留且附诊断
        let bad = convert_exposure("exposure", "zzz");
        assert!(bad.value.is_none());
        assert!(bad.issue.is_some());
        assert_eq!(bad.raw, "zzz");
    }
}
