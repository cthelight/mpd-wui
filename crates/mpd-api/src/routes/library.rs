use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::Json;

use mpd_client::{Browse, Song};

use crate::error::ApiError;
use crate::AppState;

use super::param;

const LIST_TAGS: &[&str] = &[
    "artist",
    "albumartist",
    "album",
    "genre",
    "date",
    "title",
    "composer",
];

pub async fn browse(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Browse>, ApiError> {
    let path = params.get("path").map(String::as_str).unwrap_or("");
    Ok(Json(state.client.lsinfo(normalize_browse_path(path)).await?))
}

/// MPD treats a leading `/` as an absolute filesystem path (which it rejects
/// with `Access denied`); browse paths are relative to the music root, so strip
/// any leading slashes.
fn normalize_browse_path(path: &str) -> &str {
    path.trim_start_matches('/')
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<String>>, ApiError> {
    let tag = param(&params, "type").ok_or_else(|| ApiError::bad_request("missing ?type="))?;
    if !LIST_TAGS.contains(&tag) {
        return Err(ApiError::bad_request(format!(
            "unsupported list type '{tag}'"
        )));
    }
    let artist = param(&params, "artist");
    let albumartist = param(&params, "albumartist");
    Ok(Json(state.client.list(tag, artist, albumartist).await?))
}

pub async fn search(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<Song>>, ApiError> {
    let tags: [(&str, &str); 6] = [
        ("artist", "Artist"),
        ("album", "Album"),
        ("albumartist", "AlbumArtist"),
        ("genre", "Genre"),
        ("title", "Title"),
        ("date", "Date"),
    ];
    let exact = matches!(
        param(&params, "exact")
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true")
    );
    if exact {
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        for (key, tag) in tags {
            if let Some(value) = param(&params, key) {
                pairs.push((tag, value));
            }
        }
        return Ok(Json(state.client.search(&pairs, "==").await?));
    }

    let query = param(&params, "q");
    let mut extra: Vec<(&str, &str)> = Vec::new();
    for (key, tag) in tags {
        if let Some(value) = param(&params, key) {
            extra.push((tag, value));
        }
    }
    Ok(Json(state.client.search_any(query, &extra).await?))
}

#[cfg(test)]
mod tests {
    use super::normalize_browse_path;

    #[test]
    fn normalize_strips_leading_slash() {
        assert_eq!(normalize_browse_path(""), "");
        assert_eq!(normalize_browse_path("/"), "");
        assert_eq!(normalize_browse_path("/Band"), "Band");
        assert_eq!(normalize_browse_path("///Band"), "Band");
        assert_eq!(normalize_browse_path("Band/Album"), "Band/Album");
    }
}
