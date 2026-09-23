//! 领域模型：导入材料（原始表示保留）与派生曲线/模型结果。

use serde::{Deserialize, Serialize};

use crate::units::{
    convert_exposure, convert_q, convert_response, convert_thickness, convert_transmission,
    ScaleToAbsolute, WithUnit,
};

/// 二维积分导入的一条 q/I/sigma 记录及其全部原始采集上下文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dataset {
    pub role: DatasetRole,
    pub name: String,
    /// 原始 q 的坐标基准（曲线级，如 "1/A" 或 "nm^-1"）。
    pub q_unit_raw: String,
    /// 原始 I 的单位声明（仅记录，不猜测）。
    pub i_unit_raw: String,
    /// 原始行：(q 原始串, I 原始串, sigma 原始串, mask, 原始行号)。
    pub points: Vec<RawPoint>,
    pub exposure: WithUnit,
    pub thickness: WithUnit,
    pub transmission: WithUnit,
    /// 透射率相对不确定度（分数，如 0.02 = 2%）；导入时可缺省 0。
    pub transmission_rel_err: f64,
    /// 仪器响应来源与乘子（1.0 无修正）。
    pub response_source: String,
    pub response: WithUnit,
    /// 遮罩来源（文件名/规则描述），逐点 mask 已在 points 中。
    pub mask_source: String,
    /// 二维积分器与版本，作为规则来源的一部分。
    pub integrator: String,
    /// 探测器计数 → cm⁻¹ 的定标说明（可为相对尺度）。
    pub absolute: ScaleToAbsolute,
    /// 未被解析的原始键值（原样保留，诊断用）。
    pub unparsed: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatasetRole {
    Sample,
    Buffer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawPoint {
    pub line_no: usize,
    pub q_raw: String,
    pub i_raw: String,
    pub sigma_raw: String,
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub masked: bool,
}

impl Dataset {
    /// 从导入 JSON（保留原始字符串）构造材料记录。
    /// 逐点数值无法解析的，以诊断形式保留，不丢弃该行。
    pub fn from_import(v: &serde_json::Value) -> (Self, Vec<String>) {
        let mut diag = Vec::new();
        let role = match v.get("role").and_then(|x| x.as_str()) {
            Some("buffer") => DatasetRole::Buffer,
            _ => DatasetRole::Sample,
        };
        let q_unit_raw = v.get("q_unit").and_then(|x| x.as_str()).unwrap_or("1/A").to_string();
        let mut points = Vec::new();
        if let Some(arr) = v.get("points").and_then(|x| x.as_array()) {
            for (idx, p) in arr.iter().enumerate() {
                let q_raw = p.get("q").map(|x| x.to_string().trim_matches('"').to_string())
                    .unwrap_or_default();
                let i_raw = p.get("I").map(|x| x.to_string().trim_matches('"').to_string())
                    .unwrap_or_default();
                let sigma_raw = p.get("sigma").map(|x| x.to_string().trim_matches('"').to_string())
                    .unwrap_or_default();
                let qn = p.get("q").and_then(|x| x.as_f64());
                let in_ = p.get("I").and_then(|x| x.as_f64());
                let sn = p.get("sigma").and_then(|x| x.as_f64());
                if qn.is_none() || in_.is_none() || sn.is_none() {
                    diag.push(format!("第 {} 行存在无法解析的数值，已原位保留原始串", idx + 1));
                }
                points.push(RawPoint {
                    line_no: idx + 1,
                    q_raw,
                    i_raw,
                    sigma_raw,
                    q: qn.unwrap_or(f64::NAN),
                    i: in_.unwrap_or(f64::NAN),
                    sigma: sn.unwrap_or(f64::NAN),
                    masked: p.get("masked").and_then(|x| x.as_bool()).unwrap_or(false),
                });
            }
        }
        let g = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let exposure = convert_exposure("exposure", &g("exposure"));
        let thickness = convert_thickness("thickness", &g("thickness"));
        let transmission = convert_transmission("transmission", &g("transmission"));
        if exposure.value.is_none() {
            diag.push("曝光时间无法转换，已保留原始表示".into());
        }
        if thickness.value.is_none() {
            diag.push("样品厚度无法转换，已保留原始表示".into());
        }
        if transmission.value.is_none() {
            diag.push("透射率无法转换，已保留原始表示".into());
        }
        let mut absolute = ScaleToAbsolute::default();
        if let Some(abs) = v.get("absolute_scale") {
            if let Some(u) = abs.get("raw_unit").and_then(|x| x.as_str()) {
                absolute.raw_unit = u.to_string();
            }
            absolute.factor = abs.get("factor").and_then(|x| x.as_f64());
            if let Some(n) = abs.get("note").and_then(|x| x.as_str()) {
                absolute.note = n.to_string();
            }
        }
        let response_raw = g("response");
        let response = if response_raw.is_empty() {
            WithUnit::ok("response", "1.0", crate::units::CanonicalUnit::ResponseFactor, 1.0)
        } else {
            convert_response(&response_raw)
        };
        let mut unparsed = Vec::new();
        if let Some(obj) = v.as_object() {
            const KNOWN: &[&str] = &[
                "role", "name", "q_unit", "points", "exposure", "thickness", "transmission",
                "transmission_rel_err", "response", "response_source", "mask_source",
                "integrator", "absolute_scale",
            ];
            for (k, val) in obj {
                if !KNOWN.contains(&k.as_str()) {
                    unparsed.push((k.clone(), val.to_string()));
                }
            }
        }
        let ds = Dataset {
            role,
            name: g("name"),
            q_unit_raw,
            i_unit_raw: g("i_unit"),
            points,
            exposure,
            thickness,
            transmission,
            transmission_rel_err: v
                .get("transmission_rel_err")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0),
            response_source: g("response_source"),
            response,
            mask_source: g("mask_source"),
            integrator: g("integrator"),
            absolute,
            unparsed,
        };
        // q 坐标基准在派生层规范化（nm^-1 → 1/A）。
        let _ = convert_q;
        (ds, diag)
    }
}

/// 通用曲线（派生层统一坐标：q [1/A]，I [cm^-1 或相对尺度]）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub q: Vec<f64>,
    pub i: Vec<f64>,
    pub sigma: Vec<f64>,
    /// 与 q/i/sigma 对齐；被遮罩点保留但不参与拟合。
    pub masked: Vec<bool>,
    pub i_unit: String,
    /// 每个点在源数据中的行号，保证可逐点追回。
    pub source_line: Vec<usize>,
}

/// 归一化结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Normalized {
    pub curve: Curve,
    pub monitor: f64,
    pub formula: String,
    pub norm_factor: f64,
    /// 归一化（曝光/厚度/透射/响应）引入的相对不确定度。
    pub norm_rel_sigma: f64,
}

/// 背景缩放候选。多候选时全部保留，审阅者裁定。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScaleCandidate {
    pub id: String,
    pub method: String,
    pub q_min: f64,
    pub q_max: f64,
    pub scale: f64,
    pub scale_sigma: f64,
    pub chi2: f64,
    pub dof: usize,
    pub n_points: usize,
}

/// 扣背景后的单点：记录来自样品/缓冲液的哪两个点（可点击追回）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPoint {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
    pub masked: bool,
    pub sample_line: usize,
    /// 缓冲液通过插值得到，记录插值两端行号；None 表示该点不在重叠范围。
    pub buffer_lines: Option<(usize, usize)>,
    pub interpolated: bool,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subtraction {
    pub scale_candidate_id: String,
    pub scale: f64,
    pub scale_sigma: f64,
    pub points: Vec<SubPoint>,
    /// 重叠 q 范围（插值只在此范围生效）。
    pub overlap: Option<(f64, f64)>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelKind {
    Guinier,
    Porod,
}

/// 多区间模型候选。Guinier 与 Porod 永不自动拼接。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCandidate {
    pub id: String,
    pub kind: ModelKind,
    pub q_min: f64,
    pub q_max: f64,
    pub n_points: usize,
    pub chi2: f64,
    pub dof: usize,
    /// Guinier: Rg [A], I0；Porod: 指数 p（lnI vs lnq 斜率取负），前置因子。
    pub param_a: f64,
    pub param_b: f64,
    pub param_a_sigma: f64,
    pub param_b_sigma: f64,
    /// Guinier q_max*Rg 上限 / Porod 线性度诊断等。
    pub checks: Vec<String>,
    /// 与其它候选重叠的候选 id。
    pub overlaps_with: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelFit {
    pub candidates: Vec<ModelCandidate>,
    pub diagnostics: Vec<Diagnostic>,
}
