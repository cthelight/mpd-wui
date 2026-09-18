//! API error type: maps internal and extractor failures to JSON responses.

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn upstream(error: anyhow::Error) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, error.to_string())
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        Self::upstream(err)
    }
}

impl From<JsonRejection> for ApiError {
    fn from(_rejection: JsonRejection) -> Self {
        Self::bad_request("invalid JSON body")
    }
}

impl From<QueryRejection> for ApiError {
    fn from(_rejection: QueryRejection) -> Self {
        Self::bad_request("invalid query string")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            serde_json::json!({ "error": self.message }).to_string(),
        )
            .into_response()
    }
}
