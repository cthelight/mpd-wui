//! mpd-api: HTTP + WebSocket API layer over [`mpd_client`].
//!
//! Exposes an axum [`Router`] covering status, playback, queue, library,
//! album art and the real-time WebSocket, plus a small TTL cache.
