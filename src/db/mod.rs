//! SQLite-backed versioned store. A single guarded connection is shared with
//! the web layer; every write that represents a recomputation goes through a
//! transaction that either publishes a complete derivation version or leaves
//! the previous current pointer untouched.

pub mod conn;
pub mod error;
pub mod import;
pub mod graph;
pub mod model;
pub mod repo;

pub use conn::Store;
