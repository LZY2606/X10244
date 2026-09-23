//! 散射曲线台：SAXS 归一化、背景扣除与多区间模型证据。
//!
//! 分层：
//! - [`domain`]：纯领域规则（单位、归一化、背景扣除、模型证据），不依赖存储。
//! - [`store`]：SQLite 事件存储、版本指针、原子提交、冲突与导出导入。
//! - [`engine`]：事件折叠为当前状态的依赖图传播器（只重算受影响派生）。
//! - [`case`]：仓库内置离线案例包与可重放场景。
//! - [`server`]：Axum HTTP / JSON API 与静态页面。

pub mod domain;
pub mod engine;
pub mod store;
pub mod case;
pub mod server;

pub const RULES_VERSION: &str = "saxs-rules-2026.09";
