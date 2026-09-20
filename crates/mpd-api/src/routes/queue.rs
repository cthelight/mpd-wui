use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;

use mpd_client::MpdClient;

use crate::AppState;
use crate::dto::{AddPosition, AddReq, MoveReq, QueueTarget, RemoveReq};
use crate::error::ApiError;

use super::json_body;

pub async fn add(
    State(state): State<AppState>,
    body: Result<Json<AddReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    tracing::debug!(
        targets = ?req.targets,
        play = req.play,
        position = ?req.position,
        "queue add request"
    );
    if req.play {
        add_and_play(&state, &req.targets).await
    } else if req.position == AddPosition::AfterCurrent {
        add_after_current(&state.client, &req.targets).await
    } else {
        for target in &req.targets {
            add_target(&state.client, target).await?;
        }
        Ok(StatusCode::NO_CONTENT)
    }
}

/// Replace the queue with `targets` and start playback.
///
/// The new songs are appended after the existing queue, then the old prefix
/// is deleted. Appending first (instead of clearing first) means a failure
/// mid-add only requires deleting the songs this request appended, leaving
/// the caller's old queue intact.
async fn add_and_play(state: &AppState, targets: &[QueueTarget]) -> Result<StatusCode, ApiError> {
    let client = &state.client;
    if targets.is_empty() {
        return Err(ApiError::bad_request("nothing to add"));
    }
    // Validate every target before touching the queue: a target that would
    // add nothing is a client error and must not disturb the queue.
    for target in targets {
        validate_target(client, target).await?;
    }

    let old_len = client.playlist_count().await?;

    // Append each target, tracking how many songs were added so far.
    let mut added: u32 = 0;
    for target in targets {
        let before = client.playlist_count().await?;
        if let Err(e) = add_target(client, target).await {
            if added > 0 {
                // Compensation: drop only what this request appended.
                let _ = client.delete_range(old_len, old_len + added).await;
            }
            return Err(e);
        }
        let after = client.playlist_count().await?;
        added += after.saturating_sub(before);
    }
    if added == 0 {
        return Err(ApiError::bad_request("nothing to add"));
    }

    // Drop the old prefix, then start at the top. If this fails the queue
    // holds old+new, which the user can still recover by hand.
    if old_len > 0 {
        client.delete_range(0, old_len).await?;
    }
    client.play(Some(0)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Append `targets` and move them right after the currently playing song so
/// they play before the rest of the queue. With nothing playing, the songs go
/// to the front of the queue.
///
/// `add`/`searchadd` do not report the new playlist ids, so the songs are
/// placed by position: they are appended (at `old_len..new_len`) and then
/// moved back to `song + 1`. The move is always backward, which MPD's
/// `move start:end pos` defines unambiguously.
async fn add_after_current(
    client: &MpdClient,
    targets: &[QueueTarget],
) -> Result<StatusCode, ApiError> {
    if targets.is_empty() {
        return Err(ApiError::bad_request("nothing to add"));
    }
    for target in targets {
        validate_target(client, target).await?;
    }

    let status = client.status().await?;
    let at = status.song.map(|p| p.saturating_add(1)).unwrap_or(0);
    let old_len = status.songs;
    tracing::debug!(
        state = ?status.state,
        song = ?status.song,
        old_len,
        at,
        targets = targets.len(),
        "enqueue-next: status before add"
    );

    for target in targets {
        add_target(client, target).await?;
    }
    let new_len = client.playlist_count().await?;
    let added = new_len.saturating_sub(old_len);
    if added == 0 {
        return Err(ApiError::bad_request("nothing to add"));
    }
    // Nothing to move when the range already starts at `at` (queue was empty).
    if at < old_len {
        tracing::debug!(
            old_len,
            new_len,
            added,
            at,
            range_start = old_len,
            range_end = new_len,
            "enqueue-next: moving appended range to `at`"
        );
        client.move_range(old_len, new_len, at).await?;
    } else {
        tracing::debug!(
            old_len,
            new_len,
            added,
            at,
            "enqueue-next: no move (appended range already at/after `at`)"
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Check that a target would actually add songs, before the queue is touched.
async fn validate_target(client: &MpdClient, target: &QueueTarget) -> Result<(), ApiError> {
    match target {
        QueueTarget::Path { path } => {
            client
                .lsinfo(path)
                .await
                .map_err(|_| ApiError::not_found(format!("no such path: {path}")))?;
            Ok(())
        }
        QueueTarget::Artist { artist } => count_target(client, "Artist", artist).await,
        QueueTarget::Album { album } => count_target(client, "Album", album).await,
        QueueTarget::AlbumArtist { albumartist } => {
            count_target(client, "AlbumArtist", albumartist).await
        }
        QueueTarget::Genre { genre } => count_target(client, "Genre", genre).await,
        QueueTarget::Date { date } => count_target(client, "Date", date).await,
    }
}

async fn count_target(client: &MpdClient, tag: &str, value: &str) -> Result<(), ApiError> {
    let (songs, _) = client.count(&[(tag, value)], "==").await?;
    if songs == 0 {
        Err(ApiError::bad_request("no matching tracks"))
    } else {
        Ok(())
    }
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
        QueueTarget::Date { date } => client.searchadd(&[("Date", date.as_str())], "==").await?,
    }
    Ok(())
}
