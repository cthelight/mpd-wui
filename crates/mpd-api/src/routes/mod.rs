//! HTTP route handlers, grouped by concern.

pub mod albumart;
pub mod database;
pub mod library;
pub mod playback;
pub mod queue;
pub mod status;

use axum::Json;
use axum::extract::rejection::JsonRejection;

/// Unwrap an optional JSON body extractor, mapping every rejection to a 400.
pub(crate) fn json_body<T>(
    body: Result<Json<T>, JsonRejection>,
) -> Result<T, crate::error::ApiError> {
    body.map(|Json(value)| value)
        .map_err(|_| crate::error::ApiError::bad_request("invalid JSON body"))
}

/// Non-empty query parameter, if present.
pub(crate) fn param<'a>(
    params: &'a std::collections::HashMap<String, String>,
    key: &str,
) -> Option<&'a str> {
    params
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}
