use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::Json;

use mpd_client::{Browse, Song};

use crate::error::ApiError;
use crate::search::{self, Field};
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
    Ok(Json(
        state.client.lsinfo(normalize_browse_path(path)).await?,
    ))
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

/// Query-string tag params mapped to local search fields (exact-match
/// constraints, ANDed together).
const SEARCH_FIELDS: &[(&str, Field)] = &[
    ("artist", Field::Artist),
    ("album", Field::Album),
    ("albumartist", Field::AlbumArtist),
    ("genre", Field::Genre),
    ("title", Field::Title),
    ("composer", Field::Composer),
    ("date", Field::Date),
];

/// Local search: pull the whole library into the process (cached by TTL) and
/// filter it here. MPD's filter grammar has no `OR`, so broad "match any tag"
/// search can't be done server-side; exact multi-tag search can, but doing
/// everything locally keeps the two paths uniform and avoids the fragile
/// filter-syntax edge cases.
pub async fn search(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<Song>>, ApiError> {
    let mut query = search::Query::default();
    if let Some(q) = param(&params, "q") {
        if !q.trim().is_empty() {
            query.free_text = Some(q.to_string());
        }
    }
    for (key, field) in SEARCH_FIELDS {
        if let Some(value) = param(&params, key) {
            query.exact.push((*field, value.to_string()));
        }
    }
    if query.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let library = state.library.get(&state.client).await?;
    Ok(Json(search::filter(&library, &query)))
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
