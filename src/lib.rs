pub mod ids;
pub mod units;
pub mod parser;
pub mod model;
pub mod engine;
pub mod store;
pub mod service;
pub mod case;
pub mod api;

pub const RULE_VERSION: &str = "saxs-rules-2026.09";

pub const RULE_GRID_INTERP: &str =
    "RULE_GRID_INTERP: q grids differ -> never subtract point-to-point; interpolate buffer inside the physical overlap only";
pub const RULE_ERROR_PROP: &str =
    "RULE_ERROR_PROP: propagated uncertainty includes normalization error and background scale uncertainty";
pub const RULE_NO_SINGLE_MODEL: &str =
    "RULE_NO_SINGLE_MODEL: Guinier and Porod regions are separate candidates and are never auto-joined into one model";
