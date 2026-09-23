//! saxs-core: SAXS normalization, buffer subtraction and multi-region model
//! evidence rules. Pure domain logic, no I/O.

pub mod ids;
pub mod model;
pub mod units;
pub mod calc;
pub mod project;
pub mod scenario;
pub mod view;
pub mod replay;

pub use model::*;
pub use project::{Event, Intent, Project, RuleParams};
