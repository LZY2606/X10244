//! Unit handling. Raw representations are preserved on import; conversion to
//! canonical units (q in 1/angstrom, I in arb.) happens only in the derived
//! layer. Unconvertible values stay in place and carry a diagnostic.

fn normalize_unit(u: &str) -> String {
    u.trim()
        .to_lowercase()
        .replace(' ', "")
        .replace('\u{00e5}', "a") // å
        .replace("angstrom", "a")
}

/// Multiplicative factor converting a q value in `unit` to 1/angstrom.
pub fn q_factor_to_angstrom(unit: &str) -> Option<f64> {
    match normalize_unit(unit).as_str() {
        "1/a" | "a^-1" | "1/a." => Some(1.0),
        "1/nm" | "nm^-1" => Some(0.1),
        _ => None,
    }
}

/// Multiplicative factor converting an intensity value in `unit` to the
/// canonical "arb." basis. Absolute-scale units (1/cm) are kept numerically
/// but the basis change is recorded in provenance by the caller.
pub fn i_factor_to_arb(unit: &str) -> Option<f64> {
    match normalize_unit(unit).as_str() {
        "arb" | "arb." | "a.u." | "au" | "counts" => Some(1.0),
        "1/cm" | "cm^-1" => Some(1.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn q_units() {
        assert_eq!(q_factor_to_angstrom("1/nm"), Some(0.1));
        assert_eq!(q_factor_to_angstrom("A^-1"), Some(1.0));
        assert_eq!(q_factor_to_angstrom("1/angstrom"), Some(1.0));
        assert_eq!(q_factor_to_angstrom("1/furlong"), None);
    }
}
