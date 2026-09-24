use super::types::*;
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
}

fn rows(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split(',').map(|c| c.trim().to_string()).collect())
        .collect()
}

pub fn parse_curve_csv(id: &str, label: &str, q_unit: &str, i_unit: &str, text: &str) -> Result<Curve, ParseError> {
    let data = rows(text);
    if data.len() < 2 {
        return Err(ParseError {
            message: "曲线 CSV 至少需要表头和一行数据".to_string(),
        });
    }
    let header: Vec<String> = data[0].iter().map(|h| h.to_lowercase()).collect();
    let qi = header
        .iter()
        .position(|h| h == "q" || h.starts_with("q(") || h.starts_with("q "))
        .ok_or_else(|| ParseError { message: "缺少 q 列".to_string() })?;
    let ii = header
        .iter()
        .position(|h| h == "i" || h == "i(q)" || h.starts_with("i(") || h.starts_with("i "))
        .ok_or_else(|| ParseError { message: "缺少 I 列".to_string() })?;
    let si = header.iter().position(|h| h.starts_with("sigma") || h.contains("error") || h == "err");

    let mut curve = Curve::new(id, label, q_unit, i_unit);
    for row in &data[1..] {
        let q: f64 = row
            .get(qi)
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| ParseError { message: format!("无法解析 q: {row:?}") })?;
        let i: f64 = row
            .get(ii)
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| ParseError { message: format!("无法解析 I: {row:?}") })?;
        let sigma = si
            .and_then(|idx| row.get(idx))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| (0.03 * i.abs()).max(1e-4));
        curve.push(q, i, sigma);
    }
    Ok(curve)
}

#[derive(Default)]
pub struct HeaderFields {
    pub fields: BTreeMap<String, String>,
}

pub fn parse_header(text: &str) -> HeaderFields {
    let mut fields = BTreeMap::new();
    for line in text.lines().map(|l| l.trim()).filter(|l| l.starts_with('#')) {
        if let Some((k, v)) = line.trim_start_matches('#').split_once('=') {
            fields.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    HeaderFields { fields }
}
