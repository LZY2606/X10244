//! Unified error type for SAXS station.
use std::fmt;

#[derive(Debug)]
pub enum AppError {
    Db(rusqlite::Error),
    Json(serde_json::Error),
    Io(std::io::Error),
    Conflict { msg: String, current_seq: i64 },
    Domain(String),
    NotFound(String),
    Corrupt(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Db(e) => write!(f, "database error: {e}"),
            AppError::Json(e) => write!(f, "json error: {e}"),
            AppError::Io(e) => write!(f, "io error: {e}"),
            AppError::Conflict { msg, current_seq } => {
                write!(f, "optimistic conflict at seq {current_seq}: {msg}")
            }
            AppError::Domain(s) => write!(f, "{s}"),
            AppError::NotFound(s) => write!(f, "not found: {s}"),
            AppError::Corrupt(s) => write!(f, "corrupt state: {s}"),
        }
    }
}
impl std::error::Error for AppError {}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Db(e)
    }
}
impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Json(e)
    }
}
impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e)
    }
}

pub type AppResult<T> = Result<T, AppError>;
