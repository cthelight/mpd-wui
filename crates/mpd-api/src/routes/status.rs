use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::Json;

use mpd_client::{Capabilities, Snapshot, Song};

use crate::error::ApiError;
use crate::AppState;

use super::parse_u32;

pub async fn status(State(state): State<AppState>) -> Result<Json<Snapshot>, ApiError> {
    let status = state.client.status().await?;
    let song = state.client.currentsong().await?;
    Ok(Json(Snapshot { status, song }))
}

pub async fn playlist(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<Song>>, ApiError> {
    let start = parse_u32(&params, "start")?;
    let end = parse_u32(&params, "end")?;
    Ok(Json(state.client.playlist(start, end).await?))
}

pub async fn capabilities(State(state): State<AppState>) -> Result<Json<Capabilities>, ApiError> {
    Ok(Json(state.client.capabilities().await))
}
