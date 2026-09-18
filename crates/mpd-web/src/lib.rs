//! mpd-web: serves the embedded frontend (see `web/` at the workspace root).
//!
//! Release builds embed the files in the binary; debug builds read them from
//! disk so frontend edits need no rebuild.

use axum::body::Bytes;
use axum::extract::Request;
use axum::http::{header, HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// The embedded frontend. Paths are relative to this crate's manifest dir.
#[derive(RustEmbed)]
#[folder = "../../web"]
struct Asset;

/// Serve a static asset from the embedded frontend, falling back to
/// `index.html` for unknown paths so the app works without a web server.
pub async fn static_handler(req: Request) -> Response {
    let path = req.uri().path();
    let file_path = path.trim_start_matches('/');
    let file_path = if file_path.is_empty() {
        "index.html"
    } else {
        file_path
    };

    match Asset::get(file_path) {
        Some(file) => serve(file, file_path),
        None => match Asset::get("index.html") {
            Some(file) => serve(file, "index.html"),
            None => (StatusCode::INTERNAL_SERVER_ERROR, "index.html missing").into_response(),
        },
    }
}

fn serve(file: rust_embed::EmbeddedFile, path: &str) -> Response {
    let data: Bytes = Bytes::from(file.data.to_vec());
    let headers: [(HeaderName, String); 2] = [
        (header::CONTENT_TYPE, mime_for(path).to_string()),
        (header::CACHE_CONTROL, cache_for(path).to_string()),
    ];
    (StatusCode::OK, headers, data).into_response()
}

/// The shell is revalidated on every load; assets are cached briefly.
fn cache_for(path: &str) -> &'static str {
    if path == "index.html" {
        "no-cache"
    } else {
        "public, max-age=600"
    }
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("txt" | "md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_expected_files() {
        let files: Vec<String> = Asset::iter().map(|p| p.into_owned()).collect();
        for expected in ["index.html", "css/styles.css", "js/app.js", "js/api.js"] {
            assert!(
                files.iter().any(|f| f == expected),
                "expected {expected} in embedded files: {files:?}"
            );
        }
    }

    #[test]
    fn mime_for_maps_common_types() {
        assert_eq!(mime_for("index.html"), "text/html; charset=utf-8");
        assert_eq!(mime_for("css/styles.css"), "text/css; charset=utf-8");
        assert_eq!(mime_for("js/app.js"), "text/javascript; charset=utf-8");
        assert_eq!(mime_for("img/cover.svg"), "image/svg+xml");
        assert_eq!(mime_for("img/cover.webp"), "image/webp");
        assert_eq!(mime_for("noext"), "application/octet-stream");
    }

    #[test]
    fn cache_for_is_stricter_for_shell() {
        assert_eq!(cache_for("index.html"), "no-cache");
        assert_eq!(cache_for("css/styles.css"), "public, max-age=600");
    }
}
