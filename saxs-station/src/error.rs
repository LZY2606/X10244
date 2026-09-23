//! 统一错误类型：领域层返回带诊断码的错误，存储层区分冲突/崩溃注入。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Domain(String),
    #[error("并发冲突: {0}")]
    Conflict(String),
    #[error("未找到: {0}")]
    NotFound(String),
    #[error("存储错误: {0}")]
    Storage(String),
    #[error("请求错误: {0}")]
    BadRequest(String),
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Storage(e.to_string())
    }
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Domain(_) => "DOMAIN_ERROR",
            AppError::Conflict(_) => "CONFLICT",
            AppError::NotFound(_) => "NOT_FOUND",
            AppError::Storage(_) => "STORAGE_ERROR",
            AppError::BadRequest(_) => "BAD_REQUEST",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status(),
            Json(json!({ "error": self.code(), "message": self.to_string() })),
        )
            .into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
