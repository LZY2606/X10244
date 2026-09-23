//! 领域模型：SAXS 源曲线、派生节点、事件。
//! 所有领域量携带单位或坐标基准；导入层保留原始表示，派生层才规范化。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type Id = String;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QUnit {
    InvAngstrom,
    InvNm,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IUnit {
    Arbitrary,
    InvCm,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Sample,
    Buffer,
    InstrumentResponse,
    Mask,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Sample => "sample",
            SourceKind::Buffer => "buffer",
            SourceKind::InstrumentResponse => "instrument_response",
            SourceKind::Mask => "mask",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "sample" => Some(Self::Sample),
            "buffer" => Some(Self::Buffer),
            "instrument_response" => Some(Self::InstrumentResponse),
            "mask" => Some(Self::Mask),
            _ => None,
        }
    }
}

/// 原始数据点：完全保留导入时的字符串表示。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawPoint {
    pub q: String,
    pub i: String,
    pub err: String,
}

/// 规范化后的数据点（q 统一为 1/Å）。
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Point {
    pub q: f64,
    pub i: f64,
    pub err: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurveMeta {
    pub q_unit: QUnit,
    pub i_unit: IUnit,
    pub exposure_s: Option<f64>,
    pub thickness_mm: Option<f64>,
    pub transmission: Option<f64>,
    /// 仪器响应、遮罩来源等附加溯源信息。
    #[serde(default)]
    pub extra: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceCurve {
    pub id: Id,
    pub kind: SourceKind,
    pub name: String,
    /// 原始文件全文，导入时原样保留。
    pub raw_text: String,
    pub meta: CurveMeta,
    pub raw_points: Vec<RawPoint>,
    /// 与 raw_points 等长；无法转换的值留在原位置为 None，并附诊断。
    pub points: Vec<Option<Point>>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Normalize,
    Subtract,
    IntervalSet,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Ok,
    Invalid,
    Multi,
    Stale,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DepRef {
    pub id: Id,
    pub hash: u64,
}

/// 派生曲线上的一个点，带稳定身份 (node_id#idx) 与溯源。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurvePoint {
    pub q: f64,
    pub i: Option<f64>,
    pub err: Option<f64>,
    #[serde(default)]
    pub flags: Vec<String>,
    /// 溯源：(源曲线或上游节点 id, 点下标)。
    #[serde(default)]
    pub src: Vec<(Id, usize)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalCandidate {
    pub model: String, // "guinier" | "porod"
    pub qmin: f64,
    pub qmax: f64,
    pub params: BTreeMap<String, f64>,
    pub residual_rms: f64,
    pub n_points: usize,
    pub note: String,
    /// None = 未裁定；Some(true) = 采纳；Some(false) = 未采纳（保留）。
    #[serde(default)]
    pub accepted: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeResult {
    Curve { points: Vec<CurvePoint> },
    Intervals { candidates: Vec<IntervalCandidate> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: Id,
    pub kind: NodeKind,
    pub deps: Vec<DepRef>,
    pub params: serde_json::Value,
    pub status: NodeStatus,
    pub result: Option<NodeResult>,
    pub diagnostics: Vec<String>,
    /// 为何仍有效 / 为何需要重算。
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub id: Id,
    pub version: Id,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created: f64,
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn source_hash(s: &SourceCurve) -> u64 {
    fnv1a(serde_json::to_string(&(s.kind, &s.meta, &s.raw_points)).unwrap().as_bytes())
}

pub fn node_hash(n: &Node) -> u64 {
    fnv1a(serde_json::to_string(&(n.kind, &n.params, &n.result)).unwrap().as_bytes())
}

pub fn q_to_canonical(v: f64, unit: QUnit) -> f64 {
    match unit {
        QUnit::InvAngstrom => v,
        QUnit::InvNm => v / 10.0,
    }
}

fn parse_num(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("n/a") || t.eq_ignore_ascii_case("nan") {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// 解析 fixture 文本为源曲线；保留原始表示，无法转换的值留在原位置并附诊断。
pub fn parse_source(id: &str, kind: SourceKind, name: &str, text: &str) -> SourceCurve {
    let mut q_unit = QUnit::InvAngstrom;
    let mut i_unit = IUnit::Arbitrary;
    let mut exposure_s = None;
    let mut thickness_mm = None;
    let mut transmission = None;
    let mut extra = BTreeMap::new();
    let mut raw_points = Vec::new();
    let mut points = Vec::new();
    let mut diagnostics = Vec::new();

    for (ln, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(h) = line.strip_prefix('#') {
            let h = h.trim();
            if let Some((k, v)) = h.split_once('=') {
                let k = k.trim();
                let v = v.trim();
                match k {
                    "q_unit" => {
                        q_unit = match v {
                            "1/A" | "1/Angstrom" => QUnit::InvAngstrom,
                            "1/nm" => QUnit::InvNm,
                            _ => {
                                diagnostics.push(format!("未知 q 单位 '{v}'，按 1/Å 处理"));
                                QUnit::InvAngstrom
                            }
                        }
                    }
                    "i_unit" => {
                        i_unit = match v {
                            "1/cm" => IUnit::InvCm,
                            "arb" | "arbitrary" => IUnit::Arbitrary,
                            _ => {
                                diagnostics.push(format!("未知 I 单位 '{v}'，按任意单位处理"));
                                IUnit::Arbitrary
                            }
                        }
                    }
                    "exposure_s" => exposure_s = parse_num(v),
                    "thickness_mm" => thickness_mm = parse_num(v),
                    "transmission" => transmission = parse_num(v),
                    _ => {
                        extra.insert(k.to_string(), v.to_string());
                    }
                }
            }
            continue;
        }
        if line.starts_with("q,") || line.starts_with("q\t") {
            continue; // 表头
        }
        let cols: Vec<&str> = line.split([',', '\t', ' ']).filter(|c| !c.is_empty()).collect();
        if cols.len() < 3 {
            diagnostics.push(format!("第 {} 行列数不足，保留原文不解析", ln + 1));
            raw_points.push(RawPoint {
                q: line.to_string(),
                i: String::new(),
                err: String::new(),
            });
            points.push(None);
            continue;
        }
        let rp = RawPoint {
            q: cols[0].to_string(),
            i: cols[1].to_string(),
            err: cols[2].to_string(),
        };
        let parsed = match (parse_num(&rp.q), parse_num(&rp.i), parse_num(&rp.err)) {
            (Some(q), Some(i), Some(e)) => Some(Point {
                q: q_to_canonical(q, q_unit),
                i,
                err: e.abs(),
            }),
            _ => {
                diagnostics.push(format!(
                    "第 {} 行含无法转换的值 (q='{}', I='{}', err='{}')，保留在原位置",
                    ln + 1,
                    rp.q,
                    rp.i,
                    rp.err
                ));
                None
            }
        };
        raw_points.push(rp);
        points.push(parsed);
    }

    SourceCurve {
        id: id.to_string(),
        kind,
        name: name.to_string(),
        raw_text: text.to_string(),
        meta: CurveMeta {
            q_unit,
            i_unit,
            exposure_s,
            thickness_mm,
            transmission,
            extra,
        },
        raw_points,
        points,
        diagnostics,
    }
}
