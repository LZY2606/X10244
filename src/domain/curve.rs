//! Normalized curves on canonical coordinates.
//!
//! All derived curves share canonical units: q in 1/A, I and sigma in 1/cm.
//! Grids are strictly ascending and not assumed to match across curves.

use serde::{Deserialize, Serialize};

pub const Q_UNIT: &str = "1/A";
pub const I_UNIT: &str = "1/cm";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub q: f64,
    pub i: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    /// Stable identity of every *point* is derived from
    /// `curve_id:index`, so it follows the value across export/import and
    /// stays stable as long as the curve content is unchanged.
    pub id: super::id::Sid,
    pub q_unit: String,
    pub i_unit: String,
    pub points: Vec<Point>,
}

impl Curve {
    pub fn new(id: super::id::Sid, points: Vec<Point>) -> Self {
        Curve { id, q_unit: Q_UNIT.into(), i_unit: I_UNIT.into(), points }
    }

    pub fn qs(&self) -> impl Iterator<Item = f64> + '_ {
        self.points.iter().map(|p| p.q)
    }

    pub fn q_min(&self) -> f64 {
        self.points.first().map(|p| p.q).unwrap_or(f64::NAN)
    }

    pub fn q_max(&self) -> f64 {
        self.points.last().map(|p| p.q).unwrap_or(f64::NAN)
    }

    pub fn point_id(&self, idx: usize) -> String {
        format!("{}#{}", self.id, idx)
    }
}

/// Inclusive-overlap q window [lo, hi] of two ascending grids.
pub fn overlap_window(a: &[f64], b: &[f64]) -> Option<(f64, f64)> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let lo = a[0].max(b[0]);
    let hi = a[a.len() - 1].min(b[b.len() - 1]);
    if lo <= hi + 1e-12 {
        Some((lo, hi))
    } else {
        None
    }
}

/// Count points of `grid` lying within [lo, hi].
pub fn count_in(grid: &[f64], lo: f64, hi: f64) -> usize {
    grid.iter().filter(|&&q| q >= lo - 1e-12 && q <= hi + 1e-12).count()
}

/// Linear interpolation of an ascending (x, y) tabulation at xq.
/// Returns None outside the tabulated range — interpolation never
/// extrapolates beyond the overlap.
pub fn interp_at(xs: &[f64], ys: &[f64], xq: f64) -> Option<(f64, f64)> {
    if xs.len() < 2 || xq < xs[0] || xq > xs[xs.len() - 1] {
        return None;
    }
    match xs.binary_search_by(|v| v.partial_cmp(&xq).unwrap()) {
        Ok(k) => Some((ys[k], 0.0)),
        Err(k) => {
            let (x0, x1) = (xs[k - 1], xs[k]);
            let t = (xq - x0) / (x1 - x0);
            let y0 = ys[k - 1];
            let y1 = ys[k];
            let value = y0 + t * (y1 - y0);
            Some((value, t))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_and_interp_respect_ranges() {
        let a = [0.01, 0.02, 0.05, 0.1];
        let b = [0.03, 0.04, 0.08];
        assert_eq!(overlap_window(&a, &b), Some((0.03, 0.08)));
        let c = [0.2, 0.3];
        assert_eq!(overlap_window(&a, &c), None);
        let xs = vec![0.0, 1.0, 2.0];
        let ys = vec![0.0, 10.0, 20.0];
        assert_eq!(interp_at(&xs, &ys, 0.5), Some((5.0, 0.5)));
        assert_eq!(interp_at(&xs, &ys, 3.0), None);
    }
}
