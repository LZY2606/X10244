use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    Db(rusqlite::Error),
    Json(serde_json::Error),
    Conflict(String),
    Unprocessable(String),
    NotFound(String),
    BadRequest(String),
    Io(std::io::Error),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Db(e) => write!(f, "database error: {e}"),
            AppError::Json(e) => write!(f, "json error: {e}"),
            AppError::Conflict(m) => write!(f, "conflict: {m}"),
            AppError::Unprocessable(m) => write!(f, "unprocessable: {m}"),
            AppError::NotFound(m) => write!(f, "not found: {m}"),
            AppError::BadRequest(m) => write!(f, "bad request: {m}"),
            AppError::Io(e) => write!(f, "io error: {e}"),
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

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m),
            AppError::Unprocessable(m) => (StatusCode::UNPROCESSABLE_ENTITY, m),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::Db(m) => (StatusCode::INTERNAL_SERVER_ERROR, m.to_string()),
            AppError::Json(m) => (StatusCode::BAD_REQUEST, m.to_string()),
            AppError::Io(m) => (StatusCode::INTERNAL_SERVER_ERROR, m.to_string()),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
