use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use axum::body::Bytes;

use crate::AppState;
use crate::error::ApiError;

use super::param;

const ART_CACHE_CONTROL: &str = "public, max-age=3600";

pub async fn albumart(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let uri = param(&params, "uri").ok_or_else(|| ApiError::bad_request("missing ?uri="))?;
    let key = format!("art:{uri}");
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());

    if let Some(hit) = state.cache.get_art(&key) {
        if etag_matches(if_none_match, &hit.etag) {
            return Ok(not_modified(&hit.etag));
        }
        let mime = hit
            .mime
            .unwrap_or_else(|| "application/octet-stream".to_string());
        return Ok(art_response(hit.value, &hit.etag, &mime));
    }

    let (bytes, mime) = match state.client.read_picture(uri).await {
        Some(art) => art,
        None => return Err(ApiError::not_found("album art not found")),
    };
    let hit = state.cache.store_art(&key, Bytes::from(bytes), &mime);
    if etag_matches(if_none_match, &hit.etag) {
        return Ok(not_modified(&hit.etag));
    }
    Ok(art_response(hit.value, &hit.etag, &mime))
}

fn art_response(value: Bytes, etag: &str, mime: &str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, mime.parse().expect("valid mime"));
    headers.insert(
        header::CACHE_CONTROL,
        ART_CACHE_CONTROL.parse().expect("valid header"),
    );
    headers.insert(header::ETAG, etag.parse().expect("valid etag"));
    (headers, value).into_response()
}

fn not_modified(etag: &str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::ETAG, etag.parse().expect("valid etag"));
    (StatusCode::NOT_MODIFIED, headers).into_response()
}

fn etag_matches(if_none_match: Option<&str>, etag: &str) -> bool {
    let value = match if_none_match {
        Some(value) => value,
        None => return false,
    };
    let value = value.trim();
    if value == "*" {
        return true;
    }
    value.split(',').any(|candidate| candidate.trim() == etag)
}
