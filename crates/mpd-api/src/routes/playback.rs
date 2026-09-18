use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::dto::{OptionsReq, PauseReq, PlayReq, SeekReq, VolumeReq};
use crate::error::ApiError;
use crate::AppState;

use super::json_body;

pub async fn play(
    State(state): State<AppState>,
    body: Result<Json<PlayReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    state.client.play(req.position).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pause(
    State(state): State<AppState>,
    body: Result<Json<PauseReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    state.client.pause(req.state).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn stop(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state.client.stop().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn next(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state.client.next().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn previous(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state.client.previous().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn seek(
    State(state): State<AppState>,
    body: Result<Json<SeekReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    state.client.seek(req.time).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn volume(
    State(state): State<AppState>,
    body: Result<Json<VolumeReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    state.client.set_volume(req.value).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn options(
    State(state): State<AppState>,
    body: Result<Json<OptionsReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    state
        .client
        .set_options(req.random, req.repeat, req.single, req.consume)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
