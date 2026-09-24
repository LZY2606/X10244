//! Pure SAXS domain logic: quantities, curves, normalization, buffer
//! subtraction and multi-interval model evidence. No database or I/O lives
//! here so the rules can be unit tested directly.

pub mod curves;
pub mod models;
pub mod subtract;
pub mod units;

pub use curves::*;
pub use models::*;
pub use subtract::*;
pub use units::*;
