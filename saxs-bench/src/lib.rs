//! 散射曲线台 — SAXS 归一化、背景扣除与多区间模型证据.
//!
//! Pure domain core (`units`, `model`, `id`, `saxs`, `events`, `case`,
//! `replay`) plus a SQLite event store (`store`) and an offline Axum web UI
//! (`server`).

pub mod case;
pub mod events;
pub mod id;
pub mod model;
pub mod replay;
pub mod saxs;
pub mod server;
pub mod store;
pub mod units;
