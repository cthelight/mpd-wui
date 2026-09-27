//! Audio outputs and partitions: server-level settings surfaced on the
//! Settings page.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;

use mpd_client::AudioOutput;

use crate::AppState;
use crate::dto::{OutputReq, PartitionReq, PartitionsResp};
use crate::error::ApiError;

use super::json_body;

/// All audio outputs of the client's current partition.
///
/// Output ids may change between MPD executions; clients must treat this as
/// the single source of truth before toggling anything.
pub async fn outputs(State(state): State<AppState>) -> Result<Json<Vec<AudioOutput>>, ApiError> {
    Ok(Json(state.client.outputs().await?))
}

/// Enable or disable an audio output by id.
pub async fn set_output(
    State(state): State<AppState>,
    body: Result<Json<OutputReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    // `enableoutput`/`disableoutput` need admin permission; a permission-less
    // MPD user gets an ACK error, which surfaces as a 502 with the message.
    state.client.set_output(req.id, req.enabled).await?;
    let _ = state.client.refresh().await;
    Ok(StatusCode::NO_CONTENT)
}

/// The partition list plus the client's current partition.
pub async fn partitions(State(state): State<AppState>) -> Result<Json<PartitionsResp>, ApiError> {
    // A fresh `status` (not the cached snapshot): it carries the partition
    // this client is connected to, which `listpartitions` does not.
    let status = state.client.status().await?;
    let partitions = state.client.list_partitions().await?;
    Ok(Json(PartitionsResp {
        current: status.partition,
        partitions,
    }))
}

/// Switch the client — and with it the whole UI, since the bridge is a single
/// connection — to another partition.
pub async fn set_partition(
    State(state): State<AppState>,
    body: Result<Json<PartitionReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = json_body(body)?;
    if req.name.trim().is_empty() {
        return Err(ApiError::bad_request("partition name must not be empty"));
    }
    state.client.set_partition(&req.name).await?;
    let _ = state.client.refresh().await;
    Ok(StatusCode::NO_CONTENT)
}
