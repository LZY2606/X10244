//! Orchestration: turns stored raw material into published derivation
//! versions through the pure domain rules, recording machine-readable
//! judgment events for every rule boundary.

pub mod compute;
pub mod loader;
pub mod replay;

pub use compute::*;
pub use replay::*;
