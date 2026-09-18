//! mpd-wui: single-binary MPD web UI.
//!
//! Wires the MPD client, the HTTP/WS API and the embedded frontend together
//! and serves them on one port.

use std::env;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use anyhow::Context;
use mpd_api::{router, spawn_cache_invalidation, AppState};
use mpd_client::{MpdClient, MpdConfig};

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

    let app = router(state).fallback(mpd_web::static_handler);
    let addr = SocketAddr::from((bind_addr, port));
    tracing::info!(%addr, "serving MPD web UI");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;

    Ok(())
}
