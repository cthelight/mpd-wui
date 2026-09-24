use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;

use mpd_client::DbStats;

use crate::AppState;
use crate::dto::{DbPathReq, RescanResp, UpdateResp};
use crate::error::ApiError;

use super::json_body;

/// MPD paths are relative to the music root; accept a leading slash too.
fn normalize_path(path: Option<String>) -> Result<Option<String>, ApiError> {
    let mut path = match path {
        Some(path) => path,
        None => return Ok(None),
    };
    while let Some(stripped) = path.strip_prefix('/') {
        path = stripped.to_string();
    }
    if path.is_empty() {
        Ok(None)
    } else {
        Ok(Some(path))
    }
}

pub async fn update(
    State(state): State<AppState>,
    body: Result<Json<DbPathReq>, JsonRejection>,
) -> Result<(StatusCode, Json<UpdateResp>), ApiError> {
    let req = json_body(body)?;
    let path = normalize_path(req.path)?;
    let updating = state.client.update(path.as_deref()).await?;
    let _ = state.client.refresh().await;
    Ok((StatusCode::ACCEPTED, Json(UpdateResp { updating })))
}

pub async fn rescan(
    State(state): State<AppState>,
    body: Result<Json<DbPathReq>, JsonRejection>,
) -> Result<(StatusCode, Json<RescanResp>), ApiError> {
    let req = json_body(body)?;
    let path = normalize_path(req.path)?;
    let caps = state.client.capabilities().await;
    if !caps.has("rescan") {
        return Err(ApiError::bad_request(
            "rescan is not supported by this MPD server",
        ));
    }
    let scanning = state.client.rescan(path.as_deref()).await?;
    let _ = state.client.refresh().await;
    Ok((StatusCode::ACCEPTED, Json(RescanResp { scanning })))
}

pub async fn stats(State(state): State<AppState>) -> Result<Json<DbStats>, ApiError> {
    Ok(Json(state.client.stats().await?))
}

pub async fn clear_cache(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state.cache.clear();
    state.library.invalidate();
    Ok(StatusCode::NO_CONTENT)
}
