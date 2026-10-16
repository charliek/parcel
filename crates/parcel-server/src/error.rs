//! The application error type is `stridelabs_http::AppError`; this module
//! re-exports it and adds the parcel-specific 4xx constructors.

use axum::http::StatusCode;

pub use stridelabs_http::{AppError, AppResult};

/// 413 — an upload exceeded a configured size or count limit.
pub fn too_large(msg: impl Into<String>) -> AppError {
    AppError::custom_client(StatusCode::PAYLOAD_TOO_LARGE, msg)
}

/// 503 is a server-side status, so it can't go through `custom_client`
/// (4xx only); an overloaded upload queue is reported as 429 instead, which
/// is also what tells a client to back off and retry.
pub fn busy(msg: impl Into<String>) -> AppError {
    AppError::TooManyRequests(msg.into())
}

pub fn not_found() -> AppError {
    AppError::NotFound("parcel not found".into())
}
