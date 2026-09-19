use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;

use mpd_client::MpdClient;

use crate::AppState;
use crate::dto::{OptionsReq, PauseReq, PlayReq, SeekReq, VolumeReq};
use crate::error::ApiError;

use super::json_body;

pub async fn play(
    State(state): State<AppState>,
    body: Result<Json<PlayReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    if req.clear {
        play_only(&state.client, req.position, req.id).await?;
    } else {
        match (req.position, req.id) {
            // A stable playlist id wins: it cannot be invalidated by a
            // concurrent queue change the way a positional index can.
            (_, Some(id)) => state.client.play_id(id).await?,
            (position, None) => state.client.play(position).await?,
        }
    }
    let _ = state.client.refresh().await;
    Ok(StatusCode::NO_CONTENT)
}

/// Replace the queue with a single song and start playback.
///
/// The song is re-appended at the end, then the old queue is deleted — the
/// append-first pattern of `add_and_play`: a failure mid-flight leaves the
/// caller's queue intact (plus the appended song), never an empty queue.
async fn play_only(
    client: &MpdClient,
    position: Option<u32>,
    id: Option<u32>,
) -> Result<(), ApiError> {
    // Resolve the song's file while it is still in the queue: once the old
    // entries are gone its id/position can no longer be referenced.
    let songs = client.playlist().await?;
    let file = match (id, position) {
        (Some(id), _) => songs.iter().find(|song| song.id == Some(id)),
        (None, Some(position)) => songs.get(position as usize),
        _ => None,
    }
    .map(|song| song.file.clone())
    .ok_or_else(|| ApiError::not_found("no such song in queue"))?;

    let old_len = client.playlist_count().await?;
    client.add(&file).await?;
    let new_len = client.playlist_count().await?;
    if new_len.saturating_sub(old_len) == 0 {
        return Err(ApiError::bad_request("nothing to add"));
    }
    // Drop the old prefix; the re-appended song is now the only entry. If
    // this fails the queue holds old+song, which the user can recover by hand.
    if old_len > 0 {
        client.delete_range(0, old_len - 1).await?;
    }
    client.play(Some(0)).await?;
    Ok(())
}

pub async fn pause(
    State(state): State<AppState>,
    body: Result<Json<PauseReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    state.client.pause(req.state).await?;
    let _ = state.client.refresh().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn stop(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state.client.stop().await?;
    let _ = state.client.refresh().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn next(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state.client.next().await?;
    let _ = state.client.refresh().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn previous(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state.client.previous().await?;
    let _ = state.client.refresh().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn seek(
    State(state): State<AppState>,
    body: Result<Json<SeekReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    state.client.seek(req.time).await?;
    let _ = state.client.refresh().await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn volume(
    State(state): State<AppState>,
    body: Result<Json<VolumeReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    state.client.set_volume(req.value).await?;
    let _ = state.client.refresh().await;
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
    // MPD's `changed: options` notification can be missed by the idle
    // connection (see MpdClient::refresh); push the new state ourselves.
    let _ = state.client.refresh().await;
    Ok(StatusCode::NO_CONTENT)
}
