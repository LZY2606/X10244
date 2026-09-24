//! Offline case-pack import. The case pack is domain material (SAXS curves,
//! transmissions, exposures, masks, instrument response), not a generic table.
//!
//! Everything is imported verbatim: original q/I/sigma text stays attached to
//! every row, and rows that cannot be converted remain in place with
//! diagnostics instead of being dropped.

use crate::domain::models::{CurveRole, InstrumentResponse, MaskSource, RawCurve, RawPoint};
use crate::domain::quantity::{RawValue, Unit};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CasePack {
    pub case_id: String,
    pub title: String,
    pub version: String,
    pub schema_version: u32,
    pub narrative: Vec<String>,
    pub materials: Vec<MaterialSpec>,
    #[serde(default)]
    pub scenarios: Vec<ScenarioSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterialSpec {
    pub stable_key: String,
    pub label: String,
    pub role: String,
    pub source_file: String,
    pub q_unit: String,
    pub i_unit: String,
    pub sigma_unit: String,
    pub exposure_s: String,
    pub thickness_mm: String,
    pub transmission: String,
    pub instrument_response: ResponseSpec,
    pub mask: MaskSpec,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResponseSpec {
    pub response_id: String,
    pub origin: String,
    pub c: String,
    pub unit_target: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaskSpec {
    pub mask_id: String,
    pub origin: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScenarioSpec {
    pub key: String,
    pub title: String,
    pub expect: Vec<String>,
}

fn q_unit(name: &str) -> Unit {
    match name {
        "1/A" | "1/angstrom" => Unit::new("1/A", "inverse-length"),
        "1/nm" => Unit::new("1/nm", "inverse-length"),
        "1/cm" => Unit::new("1/cm", "inverse-length"),
        other => Unit::new(other, "inverse-length"),
    }
}

fn i_unit(name: &str) -> Unit {
    match name {
        "counts" => Unit::new("counts", "intensity"),
        "counts/s" | "a.u./s" => Unit::new(name, "intensity-rate"),
        "a.u." => Unit::new("a.u.", "intensity"),
        other => Unit::new(other, "intensity"),
    }
}

/// Parse a .dat-style integrated curve file. Columns: q I sigma. Comment lines
/// start with `#`; malformed data lines are retained as diagnostic rows.
pub fn parse_curve_file(contents: &str) -> Vec<(usize, String, String, String, Vec<String>)> {
    let mut rows = Vec::new();
    for (idx, raw_line) in contents.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        let mut diag = Vec::new();
        let (q, i, s) = if cols.len() >= 3 {
            (cols[0].to_string(), cols[1].to_string(), cols[2].to_string())
        } else if cols.len() == 2 {
            diag.push(format!("line {}: missing sigma column", idx + 1));
            (cols[0].to_string(), cols[1].to_string(), String::new())
        } else {
            diag.push(format!("line {}: expected q I sigma, got '{}'", idx + 1, line));
            (line.to_string(), String::new(), String::new())
        };
        rows.push((idx, q, i, s, diag));
    }
    rows
}

/// Parse a mask file: `#` comments, otherwise first token is a masked raw row.
pub fn parse_mask_file(contents: &str) -> (Vec<usize>, Vec<String>) {
    let mut rows = Vec::new();
    let mut diag = Vec::new();
    for (idx, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let token = line.split_whitespace().next().unwrap_or("");
        match token.parse::<usize>() {
            Ok(r) => rows.push(r),
            Err(_) => diag.push(format!("mask line {}: cannot read row index from '{}'", idx + 1, line)),
        }
    }
    rows.sort_unstable();
    rows.dedup();
    (rows, diag)
}

pub fn load_case_pack(dir: &Path) -> Result<CasePack, String> {
    let manifest = dir.join("case.json");
    let text = std::fs::read_to_string(&manifest)
        .map(|t| strip_jsonc(&t))
        .map_err(|e| format!("cannot read {}: {}", manifest.display(), e))?;
    let pack: CasePack = serde_json::from_str(&text).map_err(|e| format!("invalid case.json: {}", e))?;
    if pack.schema_version != 1 {
        return Err(format!("unsupported case-pack schema_version {}", pack.schema_version));
    }
    Ok(pack)
}

/// Minimal JSON-with-comments stripping (the case manifest uses `#` narrative
/// lines? no — JSONC `//` and `/* */`) used only for fixture ergonomics.
fn strip_jsonc(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

pub struct ImportedMaterial {
    pub curve: RawCurve,
    pub data_path: PathBuf,
}

pub fn import_material(dir: &Path, spec: &MaterialSpec) -> Result<ImportedMaterial, String> {
    let data_path = dir.join(&spec.source_file);
    let text = std::fs::read_to_string(&data_path)
        .map_err(|e| format!("cannot read {}: {}", spec.source_file, e))?;
    let raw_rows = parse_curve_file(&text);

    let mask_path = dir.join(&spec.mask.origin);
    let (masked_rows, mask_diag) = if let Ok(mt) = std::fs::read_to_string(&mask_path) {
        parse_mask_file(&mt)
    } else {
        (Vec::new(), vec![format!("mask file '{}' not found; treating as empty mask", spec.mask.origin)])
    };

    let role = match spec.role.as_str() {
        "sample" => CurveRole::Sample,
        "buffer" => CurveRole::Buffer,
        other => return Err(format!("material {}: unknown role '{}'", spec.stable_key, other)),
    };

    let q_unit = q_unit(&spec.q_unit);
    let i_unit = i_unit(&spec.i_unit);
    let sigma_unit = i_unit(&spec.sigma_unit);

    let mut points = Vec::new();
    let mut file_diagnostics = Vec::new();
    for (row, qt, it, st, diag) in raw_rows {
        let q = RawValue::parse(&qt, q_unit.clone());
        let i = RawValue::parse(&it, i_unit.clone());
        let sigma = RawValue::parse(&st, sigma_unit.clone());
        let mut row_diag = diag.clone();
        row_diag.extend(q.diagnostics.clone());
        row_diag.extend(i.diagnostics.clone());
        row_diag.extend(sigma.diagnostics.clone());
        if !row_diag.is_empty() {
            file_diagnostics.push(format!("row {}: {}", row, row_diag.join("; ")));
        }
        points.push(RawPoint {
            row,
            q_text: qt,
            i_text: it,
            sigma_text: st,
            q,
            i,
            sigma,
        });
    }
    file_diagnostics.extend(mask_diag);

    let exposure = RawValue::parse(&spec.exposure_s, Unit::new("s", "time"));
    let thickness = RawValue::parse(&spec.thickness_mm, Unit::new("mm", "length"));
    let transmission = RawValue::parse(&spec.transmission, Unit::dimensionless());
    let c = RawValue::parse(&spec.instrument_response.c, Unit::dimensionless());

    let mut diagnostics = file_diagnostics;
    for (name, v) in [
        ("exposure", &exposure),
        ("thickness", &thickness),
        ("transmission", &transmission),
        ("instrument response C", &c),
    ] {
        for d in &v.diagnostics {
            diagnostics.push(format!("{}: {}", name, d));
        }
    }
    if points.iter().any(|p| p.i.canonical_value.is_some() && p.i.canonical_value.unwrap_or(0.0) < 0.0) {
        diagnostics.push("raw file contains negative intensity rows (retained, not clipped)".to_string());
    }

    Ok(ImportedMaterial {
        curve: RawCurve {
            stable_key: spec.stable_key.clone(),
            label: spec.label.clone(),
            role,
            source_file: spec.source_file.clone(),
            exposure_s: exposure,
            thickness,
            transmission,
            response: InstrumentResponse {
                response_id: spec.instrument_response.response_id.clone(),
                c_text: spec.instrument_response.c.clone(),
                c,
                unit_target: spec.instrument_response.unit_target.clone(),
            },
            mask: MaskSource {
                mask_id: spec.mask.mask_id.clone(),
                origin: spec.mask.origin.clone(),
                masked_rows,
                description: spec.mask.description.clone(),
            },
            points,
            diagnostics,
        },
        data_path,
    })
}
