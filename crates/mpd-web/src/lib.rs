//! mpd-web: serves the embedded frontend (see `web/` at the workspace root).
//!
//! Release builds embed the files in the binary; debug builds read them from
//! disk so frontend edits need no rebuild.

#[cfg(not(debug_assertions))]
use std::collections::HashMap;
#[cfg(not(debug_assertions))]
use std::sync::{LazyLock, Mutex};

use axum::body::Bytes;
use axum::extract::Request;
use axum::http::{HeaderName, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// The embedded frontend. Paths are relative to this crate's manifest dir.
#[derive(RustEmbed)]
#[folder = "../../web"]
struct Asset;

/// Frontend configuration held as axum state. `title` replaces the
/// `__MPD_WUI_TITLE__` placeholder in `index.html` on every response, so the
/// tab title, the `app-title` meta tag and the wordmark all share one value.
#[derive(Clone)]
pub struct WebConfig {
    pub title: String,
}

/// Serve a static asset from the embedded frontend, falling back to
/// `index.html` for unknown paths so the app works without a web server.
pub async fn static_handler(title: &str, req: Request) -> Response {
    let path = req.uri().path();
    let file_path = path.trim_start_matches('/');
    let file_path = if file_path.is_empty() {
        "index.html"
    } else {
        file_path
    };

    match Asset::get(file_path) {
        Some(file) => serve(file, file_path, title),
        None => match Asset::get("index.html") {
            Some(file) => serve(file, "index.html", title),
            None => (StatusCode::INTERNAL_SERVER_ERROR, "index.html missing").into_response(),
        },
    }
}

/// Release builds embed assets as `&'static [u8]`, so each file is wrapped
/// exactly once (zero-copy) and shared across every request. Debug builds skip
/// the cache on purpose: `web/` is read from disk on each request, so frontend
/// edits are picked up without a rebuild.
#[cfg(not(debug_assertions))]
static ASSET_BYTES: LazyLock<Mutex<HashMap<String, Bytes>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn serve(file: rust_embed::EmbeddedFile, path: &str, title: &str) -> Response {
    let data = asset_bytes(&file, path);
    // Only the shell is templated; other assets are served verbatim.
    let data = if path == "index.html" {
        apply_title(&data, title).into()
    } else {
        data
    };
    let headers: [(HeaderName, &str); 3] = [
        (header::CONTENT_TYPE, mime_for(path)),
        (header::CACHE_CONTROL, cache_for(path)),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    ];
    (StatusCode::OK, headers, data).into_response()
}

/// The token the shell uses where the display name belongs.
const TITLE_PLACEHOLDER: &str = "__MPD_WUI_TITLE__";

/// Replace every `TITLE_PLACEHOLDER` in the shell with the configured title.
fn apply_title(html: &Bytes, title: &str) -> String {
    let text = std::str::from_utf8(html).unwrap_or("");
    text.replace(TITLE_PLACEHOLDER, &html_escape(title))
}

/// Escape the characters that are meaningful in HTML text or a double-quoted
/// attribute value (the title is emitted in both).
fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn asset_bytes(file: &rust_embed::EmbeddedFile, path: &str) -> Bytes {
    #[cfg(not(debug_assertions))]
    {
        let mut cache = ASSET_BYTES.lock().expect("asset cache lock poisoned");
        if let Some(bytes) = cache.get(path) {
            return bytes.clone();
        }
        let bytes = match &file.data {
            // Release embeds `&'static [u8]`; wrap it without copying.
            std::borrow::Cow::Borrowed(static_data) => Bytes::from_static(static_data),
            // Defensive fallback (not expected in release builds).
            std::borrow::Cow::Owned(data) => Bytes::copy_from_slice(data),
        };
        cache.insert(path.to_string(), bytes.clone());
        bytes
    }
    #[cfg(debug_assertions)]
    {
        let _ = path;
        Bytes::copy_from_slice(&file.data)
    }
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
        for expected in [
            "index.html",
            "css/styles.css",
            "js/app.js",
            "js/api.js",
            "img/icon.svg",
        ] {
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

    #[test]
    fn html_escape_escapes_special_characters() {
        assert_eq!(html_escape("a&b<c>d\"e"), "a&amp;b&lt;c&gt;d&quot;e");
        assert_eq!(html_escape("plain"), "plain");
    }

    #[test]
    fn apply_title_replaces_all_placeholders() {
        let html = Bytes::from("<title>__MPD_WUI_TITLE__</title>x__MPD_WUI_TITLE__");
        assert_eq!(
            apply_title(&html, "MPD: host"),
            "<title>MPD: host</title>xMPD: host"
        );
    }
}
