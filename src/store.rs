//! SQLite storage with immutable version chains.
//! Every derivation commits as a new version; the head pointer switches only
//! after the whole batch succeeds. A crash mid-write leaves the old head.
//! History is a chain: versions(seq, prev). The visible state is "latest row
//! per id whose version is reachable from head", so checking out an old event
//! restores exactly the dependency versions and conclusions of that moment.

use anyhow::Context;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

use crate::core::{Curve, DerivedPoint, PointStatus};

pub const -> placeholder
