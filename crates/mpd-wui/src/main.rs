//! mpd-wui: single-binary MPD web UI.
//!
//! Wires the MPD client, the HTTP/WS API and the embedded frontend together
//! and serves them on one port.

use std::env;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use anyhow::Context;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use mpd_api::{AppState, router, spawn_cache_invalidation};
use mpd_client::{MpdClient, MpdConfig};
use mpd_web::WebConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mpd_host = env::var("MPD_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let mpd_port: u16 = env::var("MPD_PORT")
        .unwrap_or_else(|_| "6600".into())
        .parse()
        .context("MPD_PORT must be a valid port")?;
    let mpd_password = match env::var("MPD_PASSWORD") {
        Ok(value) if !value.is_empty() => Some(value),
        _ => None,
    };

    let bind_addr: IpAddr = env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0".into())
        .parse()
        .context("BIND_ADDR must be a valid IP address")?;
    let port: u16 = env::var("PORT")
        .unwrap_or_else(|_| "8080".into())
        .parse()
        .context("PORT must be a valid port")?;
    let cache_ttl: u64 = env::var("CACHE_TTL")
        .unwrap_or_else(|_| "300".into())
        .parse()
        .context("CACHE_TTL must be a whole number of seconds")?;

    // Display name for the wordmark and tab title. Defaults to the MPD host
    // (no port) so the tab identifies which server is being controlled.
    let app_title = env::var("APP_TITLE").unwrap_or_else(|_| format!("MPD: {mpd_host}"));

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = MpdConfig::new(mpd_host.clone(), mpd_port, mpd_password);
    tracing::info!(mpd_host = %mpd_host, mpd_port, "connecting to MPD");
    let client = MpdClient::connect(config).await;
    let state = AppState::new(client, Duration::from_secs(cache_ttl));
    spawn_cache_invalidation(state.clone());

    // The API router and the static-frontend fallback carry different state,
    // so the fallback gets its own state and is merged in (only it has a
    // fallback, so the merged router keeps the static handler as its own).
    let web_config = WebConfig { title: app_title };
    let app = router(state).merge(Router::new().fallback(fallback).with_state(web_config));
    let addr = SocketAddr::from((bind_addr, port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "serving MPD web UI");

    tokio::select! {
        result = axum::serve(listener, app) => result?,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutdown signal received");
        }
    }

    Ok(())
}

/// Static assets for the frontend; unknown `/api/` paths get a JSON 404
/// instead of the index.html fallback (the SPA would swallow them silently).
async fn fallback(State(config): State<WebConfig>, req: Request) -> Response {
    if req.uri().path().starts_with("/api/") {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"error":"not found"}"#,
        )
            .into_response();
    }
    mpd_web::static_handler(&config.title, req).await
}
