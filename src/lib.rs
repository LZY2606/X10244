//! SAXS station core library.
//!
//! Semantics that must never be violated:
//! 1. Differently spaced q grids are never subtracted directly; interpolation
//!    is applied only inside the overlapping q range.
//! 2. Uncertainty bands include both normalization uncertainty and buffer
//!    scale uncertainty.
//! 3. Guinier and Porod intervals are never joined automatically into a single
//!    piecewise model.
//! 4. Multiple legal candidates and unresolved uncertainty are retained rather
//!    than collapsed into one forced answer.

pub mod casepack;
pub mod ids;
pub mod model;
pub mod project;
pub mod quantity;
pub mod rules;
pub mod store;
pub mod web;

pub const RULE_VERSION: &str = "rules-1.0.0";
