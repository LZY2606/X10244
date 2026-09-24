//! Quantities that carry their original textual representation, unit and
//! coordinate basis through import. Normalization happens only in the derived
//! layer; values that cannot be converted stay in place with a diagnostic.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit {
    /// Canonical unit name (e.g. "mm", "1/cm", "counts/s", "1/(cm*sr)").
    pub name: String,
    /// Dimension marker used by conversions: "length", "inverse-length", ...
    pub dimension: String,
}

impl Unit {
    pub fn new(name: &str, dimension: &str) -> Self {
        Unit { name: name.to_string(), dimension: dimension.to_string() }
    }
    pub fn dimensionless() -> Self {
        Unit::new("1", "dimensionless")
    }
}

/// A scalar exactly as imported plus, when possible, its SI/canonical value.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawValue {
    pub original: String,
    pub unit: Unit,
    /// Parsed value expressed in `unit`. None when unparseable.
    pub parsed: Option<f64>,
    /// Value converted to the derived layer's canonical unit, when feasible.
    pub canonical_value: Option<f64>,
    pub canonical_unit: Option<String>,
    pub diagnostics: Vec<String>,
}

impl RawValue {
    pub fn parse(text: &str, unit: Unit) -> Self {
        let trimmed = text.trim();
        let parsed = trimmed.parse::<f64>().ok().filter(|v| v.is_finite());
        let mut out = RawValue {
            original: trimmed.to_string(),
            unit,
            parsed,
            canonical_value: None,
            canonical_unit: None,
            diagnostics: vec![],
        };
        if parsed.is_none() {
            out.diagnostics
                .push(format!("cannot parse '{}' as a number; original representation retained", trimmed));
        } else {
            match convert(&out.unit, parsed.unwrap()) {
                Ok((v, u)) => {
                    out.canonical_value = Some(v);
                    out.canonical_unit = Some(u);
                }
                Err(d) => out.diagnostics.push(d),
            }
        }
        out
    }

    pub fn scalar(value: f64, unit: Unit) -> Self {
        RawValue::parse(&format!("{}", value), unit)
    }

    pub fn ok(&self) -> bool {
        self.canonical_value.is_some()
    }
}

/// Convert a parsed value in `unit` to the canonical unit used by the derived
/// layer. Lengths -> cm, inverse lengths -> 1/nm, transmission -> dimensionless.
pub fn convert(unit: &Unit, value: f64) -> Result<(f64, String), String> {
    let factor = match unit.dimension.as_str() {
        "dimensionless" => 1.0,
        "length" => match unit.name.as_str() {
            "cm" => 1.0,
            "mm" => 0.1,
            "um" => 1e-4,
            "m" => 100.0,
            other => return Err(format!("no conversion for length unit '{}'", other)),
        },
        "inverse-length" => match unit.name.as_str() {
            "1/nm" => 1.0,
            "1/A" | "1/angstrom" => 10.0,
            "1/cm" => 1e-7,
            other => return Err(format!("no conversion for inverse-length unit '{}'", other)),
        },
        "time" => match unit.name.as_str() {
            "s" | "sec" => 1.0,
            "ms" => 1e-3,
            "min" => 60.0,
            other => return Err(format!("no conversion for time unit '{}'", other)),
        },
        "intensity-rate" => match unit.name.as_str() {
            "counts/s" | "a.u./s" => 1.0,
            other => return Err(format!("no conversion for intensity unit '{}'", other)),
        },
        "intensity" => match unit.name.as_str() {
            "counts" | "a.u." => 1.0,
            other => return Err(format!("no conversion for intensity unit '{}'", other)),
        },
        other => return Err(format!("unknown dimension '{}'", other)),
    };
    let canonical = match unit.dimension.as_str() {
        "dimensionless" => "1",
        "length" => "cm",
        "inverse-length" => "1/nm",
        "time" => "s",
        "intensity-rate" => "counts/s",
        "intensity" => "counts",
        _ => unreachable!(),
    };
    Ok((value * factor, canonical.to_string()))
}
