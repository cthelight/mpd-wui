use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use mpd_client::MpdClient;

use crate::dto::{AddReq, MoveReq, QueueTarget, RemoveReq};
use crate::error::ApiError;
use crate::AppState;

use super::json_body;

pub async fn add(
    State(state): State<AppState>,
    body: Result<Json<AddReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    if req.play {
        state.client.clear().await?;
    }
    for target in &req.targets {
        add_target(&state.client, target).await?;
    }
    if req.play {
        state.client.play(Some(0)).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove(
    State(state): State<AppState>,
    body: Result<Json<RemoveReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    state.client.delete_ids(&req.ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn clear(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state.client.clear().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn move_(
    State(state): State<AppState>,
    body: Result<Json<MoveReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    state.client.move_id(req.id, req.to).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn shuffle(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state.client.shuffle().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn add_target(client: &MpdClient, target: &QueueTarget) -> Result<(), ApiError> {
    match target {
        QueueTarget::Path { path } => client.add(path).await?,
        QueueTarget::Artist { artist } => {
            client
                .searchadd(&[("Artist", artist.as_str())], "==")
                .await?
        }
        QueueTarget::Album { album } => {
            client.searchadd(&[("Album", album.as_str())], "==").await?
        }
        QueueTarget::AlbumArtist { albumartist } => {
            client
                .searchadd(&[("AlbumArtist", albumartist.as_str())], "==")
                .await?
        }
        QueueTarget::Genre { genre } => {
            client.searchadd(&[("Genre", genre.as_str())], "==").await?
        }
    }
    Ok(())
}
