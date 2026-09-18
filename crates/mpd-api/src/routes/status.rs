use axum::Json;
use axum::extract::State;

use mpd_client::{Capabilities, Snapshot, Song};

use crate::AppState;
use crate::error::ApiError;

pub async fn status(State(state): State<AppState>) -> Result<Json<Snapshot>, ApiError> {
    let status = state.client.status().await?;
    let song = state.client.currentsong().await?;
    Ok(Json(Snapshot { status, song }))
}

pub async fn playlist(State(state): State<AppState>) -> Result<Json<Vec<Song>>, ApiError> {
    Ok(Json(state.client.playlist().await?))
}

pub async fn capabilities(State(state): State<AppState>) -> Result<Json<Capabilities>, ApiError> {
    Ok(Json(state.client.capabilities().await))
}
