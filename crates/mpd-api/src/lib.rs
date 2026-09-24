//! mpd-api: HTTP + WebSocket API layer over [`mpd_client`].
//!
//! Exposes an axum [`Router`] covering status, playback, queue, library,
//! album art and the real-time WebSocket, plus a small TTL cache.

mod cache;
mod dto;
mod error;
mod routes;
mod search;
mod store;
mod ws;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};

use mpd_client::{MpdClient, MpdEvent};

pub use cache::{Cache, CacheHit};
pub use dto::*;
pub use error::ApiError;
pub use store::LibraryStore;

/// Shared application state for the HTTP/WS layer.
#[derive(Clone)]
pub struct AppState {
    pub client: MpdClient,
    pub cache: Arc<Cache>,
    /// In-process copy of the whole library for local (in-process) search.
    pub library: Arc<LibraryStore>,
}

impl AppState {
    pub fn new(client: MpdClient, library_ttl: Duration) -> Self {
        Self {
            client,
            cache: Arc::new(Cache::new()),
            library: Arc::new(LibraryStore::new(library_ttl)),
        }
    }
}

/// Build the API router. The static-frontend fallback is attached by the
/// `mpd-wui` binary so this crate stays free of web assets.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/status", get(routes::status::status))
        .route("/api/playlist", get(routes::status::playlist))
        .route("/api/capabilities", get(routes::status::capabilities))
        .route("/api/browse", get(routes::library::browse))
        .route("/api/list", get(routes::library::list))
        .route("/api/search", get(routes::library::search))
        .route("/api/play", post(routes::playback::play))
        .route("/api/pause", post(routes::playback::pause))
        .route("/api/stop", post(routes::playback::stop))
        .route("/api/next", post(routes::playback::next))
        .route("/api/previous", post(routes::playback::previous))
        .route("/api/seek", post(routes::playback::seek))
        .route("/api/volume", post(routes::playback::volume))
        .route("/api/options", post(routes::playback::options))
        .route("/api/queue/add", post(routes::queue::add))
        .route("/api/queue/remove", post(routes::queue::remove))
        .route("/api/queue/clear", post(routes::queue::clear))
        .route("/api/queue/move", post(routes::queue::move_))
        .route("/api/queue/shuffle", post(routes::queue::shuffle))
        .route("/api/albumart", get(routes::albumart::albumart))
        .route("/api/database/update", post(routes::database::update))
        .route("/api/database/rescan", post(routes::database::rescan))
        .route("/api/database/stats", get(routes::database::stats))
        .route("/api/cache/clear", post(routes::database::clear_cache))
        .route("/ws", get(ws::ws_handler))
        .with_state(state)
}

/// Spawn a task that clears the library cache when the MPD database changes.
pub fn spawn_cache_invalidation(state: AppState) {
    tokio::spawn(async move {
        let mut events = state.client.events();
        while let Ok(event) = events.recv().await {
            if matches!(event, MpdEvent::DatabaseChanged) {
                state.cache.clear();
                state.library.invalidate();
            }
        }
    });
}
