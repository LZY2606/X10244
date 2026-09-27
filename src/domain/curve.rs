//! 1-D integrated curves `I(q)` with explicit coordinate basis and strict
//! overlap-aware interpolation.

use super::*;

/// A single sampled point with statistical error. `i` is its stable ordinal
/// index in the source (column order on import is never reordered).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Point {
    pub i: usize,
    /// q in canonical 1/A.
    pub q: f64,
    pub intensity: f64,
    pub sigma: f64,
    /// Whether this point was masked out of fitting (still stored/displayed).
    #[serde(default)]
    pub masked: bool,
}

/// An integrated curve on a single q grid.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Curve {
    pub q_unit: QUnit,
    pub i_unit: IUnit,
    pub points: Vec<Point>,
}

impl Curve {
    pub fn qs(&self) -> impl Iterator<Item = f64> + '_ {
        self.points.iter().map(|p| p.q)
    }

    /// Strictly increasing finite q grid with non-negative finite errors.
    pub fn validate_grid(&self) -> Vec<Diagnostic> {
        let mut diags = Vec::new();
        if self.points.is_empty() {
            diags.push(Diagnostic::err("EMPTY_CURVE", "curve contains no points"));
            return diags;
        }
        for p in &self.points {
            if !p.q.is_finite() || p.q <= 0.0 {
                diags.push(Diagnostic::err("Q_INVALID", "q must be finite and positive").with_target(p.i.to_string()));
            }
            if !p.intensity.is_finite() {
                diags.push(Diagnostic::err("I_INVALID", "intensity must be finite").with_target(p.i.to_string()));
            }
            if !p.sigma.is_finite() || p.sigma < 0.0 {
                diags.push(Diagnostic::err("SIGMA_INVALID", "sigma must be finite and non-negative").with_target(p.i.to_string()));
            }
        }
        for w in self.points.windows(2) {
            if w[1].q <= w[0].q {
                diags.push(Diagnostic::warn(
                    "Q_NOT_INCREASING",
                    "q grid is not strictly increasing; points are kept in import order",
                ));
                break;
            }
        }
        diags
    }
}

/// Half-open closed overlap range `[lo, hi]` of two q grids.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Overlap {
    pub lo: f64,
    pub hi: f64,
}

impl Overlap {
    /// Strict overlap: a point is usable only if its q falls inside (or on)
    /// both grids, so interpolation never extrapolates.
    pub fn of(sample_qs: &[f64], buffer_qs: &[f64]) -> Option<Overlap> {
        let (slo, shi) = (*sample_qs.first()?, *sample_qs.last()?);
        let (blo, bhi) = (*buffer_qs.first()?, *buffer_qs.last()?);
        let lo = slo.max(blo);
        let hi = shi.min(bhi);
        if hi <= lo {
            return None;
        }
        Some(Overlap { lo, hi })
    }

    pub fn contains(&self, q: f64) -> bool {
        q >= self.lo && q <= self.hi
    }
}

/// Result of interpolating the buffer onto one sample q point.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Interp {
    /// Buffer point ordinal of the left bracket.
    pub left_i: usize,
    pub right_i: usize,
    pub weight_left: f64,
    pub value: f64,
    pub sigma: f64,
}

/// Piecewise-linear interpolation of the buffer curve at sample grid points.
///
/// Error propagation for linear interpolation:
/// `sigma^2 = (1-w)^2 sigma_l^2 + w^2 sigma_r^2` (no covariance assumed).
///
/// Points outside the strict overlap are returned as `None`; they are never
/// extrapolated. q grids that differ never get subtracted directly.
pub fn interpolate_buffer_at(
    sample_qs: &[f64],
    buffer: &Curve,
) -> (Vec<Option<Interp>>, Option<Overlap>) {
    let bqs: Vec<f64> = buffer.points.iter().map(|p| p.q).collect();
    let overlap = Overlap::of(sample_qs, &bqs);
    let mut out = Vec::with_capacity(sample_qs.len());
    for &q in sample_qs {
        let usable = overlap.map(|o| o.contains(q)).unwrap_or(false);
        if !usable {
            out.push(None);
            continue;
        }
        // binary search for the bracket
        let mut lo = 0usize;
        let mut hi = bqs.len() - 1;
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if bqs[mid] <= q {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (ql, qr) = (bqs[lo], bqs[hi]);
        let w = if qr > ql { (q - ql) / (qr - ql) } else { 0.0 };
        let (pl, pr) = (&buffer.points[lo], &buffer.points[hi]);
        let value = (1.0 - w) * pl.intensity + w * pr.intensity;
        let sigma = (((1.0 - w) * pl.sigma).powi(2) + (w * pr.sigma).powi(2)).sqrt();
        out.push(Some(Interp {
            left_i: lo,
            right_i: hi,
            weight_left: 1.0 - w,
            value,
            sigma,
        }));
    }
    (out, overlap)
}

/// True when two grids agree within a relative tolerance at every shared point.
pub fn grids_match(a: &[f64], b: &[f64], rel_tol: f64) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).all(|(x, y)| {
        let scale = x.abs().max(y.abs()).max(1e-12);
        (x - y).abs() <= rel_tol * scale
    })
}
