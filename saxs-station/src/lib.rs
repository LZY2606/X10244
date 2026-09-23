//! 散射曲线台：SAXS 归一化、背景扣除与多区间模型证据
//!
//! 分层：
//! - [`units`] / [`core`]：领域量（保留原始表示）与纯计算规则
//! - [`store`]：SQLite 事件日志、版本指针、迁移与并发控制
//! - [`app`]：离线案例包与可重放叙事
//! - [`web`]：Axum HTTP 接口与静态页面

pub mod app;
pub mod core;
pub mod store;
pub mod units;
pub mod web;

pub const RULE_VERSION: &str = "rules-v1.0.0";
