//! HTTP route handlers, grouped by concern.

pub mod albumart;
pub mod library;
pub mod playback;
pub mod queue;
pub mod status;

use axum::extract::rejection::JsonRejection;
use axum::Json;

/// Unwrap an optional JSON body extractor, mapping every rejection to a 400.
pub(crate) fn json_body<T>(
    body: Result<Json<T>, JsonRejection>,
) -> Result<T, crate::error::ApiError> {
    body.map(|Json(value)| value)
        .map_err(|_| crate::error::ApiError::bad_request("invalid JSON body"))
}

/// Parse an optional non-negative integer query parameter.
pub(crate) fn parse_u32(
    params: &std::collections::HashMap<String, String>,
    key: &str,
) -> Result<Option<u32>, crate::error::ApiError> {
    match params.get(key) {
        Some(value) if !value.is_empty() => value
            .parse::<u32>()
            .map(Some)
            .map_err(|_| crate::error::ApiError::bad_request(format!("{key} must be an integer"))),
        _ => Ok(None),
    }
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
