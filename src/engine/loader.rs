//! Parse the bundled offline case package. Header lines document units and
//! coordinate basis; raw text of every numeric cell survives into storage.

use crate::db::model::{InstrumentIn, MaskIn, PointIn, RecordIn, Scalar};
use crate::domain::units::{Measured, UnitKind};

pub struct ParsedFixture {
    pub record: RecordIn,
}

fn scalar(v: &serde_json::Value, default_unit: &str, kind: UnitKind) -> Scalar {
    let value = v.get("value").and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
    let unit = v.get("unit").and_then(|x| x.as_str()).unwrap_or(default_unit);
    let m = Measured::parse(&format!("{value}"), unit, kind);
    Scalar {
        value: m.value,
        raw: m.raw,
        unit: m.unit,
        canonical: m.canonical,
        canonical_unit: m.canonical_unit,
        convertible: m.convertible,
        diagnostic: m.diagnostic,
    }
}

pub fn load_fixture(txt: &str, meta_json: &serde_json::Value) -> ParsedFixture {
    let mut points = Vec::new();
    let mut ordinal = 1u32;
    for line in txt.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 3 {
            continue;
        }
        let q = Measured::parse(cols[0], "A^-1", UnitKind::Q);
        let i = Measured::parse(cols[1], "a.u.", UnitKind::Intensity);
        let s = Measured::parse(cols[2], "a.u.", UnitKind::Intensity);
        points.push(PointIn {
            ordinal,
            q: to_scalar(q),
            i: to_scalar(i),
            sig: to_scalar(s),
            note: String::new(),
        });
        ordinal += 1;
    }
    let exposure = scalar(&meta_json["exposure"], "s", UnitKind::Time);
    let transmission = scalar(&meta_json["transmission"], "-", UnitKind::Dimensionless);
    let thickness = meta_json.get("thickness").map(|t| scalar(t, "mm", UnitKind::Length));
    let record = RecordIn {
        role: meta_json["role"].as_str().unwrap_or("sample").to_string(),
        name: meta_json["name"].as_str().unwrap_or("unnamed").to_string(),
        points,
        exposure,
        transmission,
        thickness,
        transmission_sigma: meta_json["transmission"].get("sigma").and_then(|v| v.as_f64()).unwrap_or(0.0),
        thickness_sigma: meta_json.get("thickness").and_then(|t| t.get("sigma")).and_then(|v| v.as_f64()).unwrap_or(0.0),
        source_file: meta_json["source_file"].as_str().unwrap_or("").to_string(),
        instrument: InstrumentIn {
            name: meta_json["instrument"]["name"].as_str().unwrap_or("unknown").to_string(),
            spec: meta_json["instrument"].clone(),
        },
        mask: MaskIn {
            name: meta_json["mask"]["name"].as_str().unwrap_or("unknown").to_string(),
            origin: meta_json["mask"]["origin"].as_str().unwrap_or("").to_string(),
            locator: meta_json["mask"]["locator"].as_str().unwrap_or("").to_string(),
            checksum: meta_json["mask"]["checksum"].as_str().unwrap_or("").to_string(),
        },
        revision_of: None,
    };
    ParsedFixture { record }
}

fn to_scalar(m: Measured) -> Scalar {
    Scalar {
        value: m.value,
        raw: m.raw,
        unit: m.unit,
        canonical: m.canonical,
        canonical_unit: m.canonical_unit,
        convertible: m.convertible,
        diagnostic: m.diagnostic,
    }
}
