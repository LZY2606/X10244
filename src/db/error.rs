use rusqlite::ffi;
use rusqlite::{Error, ErrorCode};

/// Build a rusqlite error carrying an application-level SQLITE_CONSTRAINT
/// message. Used for conflict/validation failures so callers match on the
/// same Error type as ordinary database failures.
pub fn app_err(msg: impl Into<String>) -> Error {
    Error::SqliteFailure(
        ffi::Error::new(ffi::SQLITE_CONSTRAINT),
        Some(msg.into()),
    )
}

pub fn is_conflict(e: &Error) -> bool {
    matches!(e, Error::SqliteFailure(inner, _) if inner.code == ErrorCode::ConstraintViolation)
}
