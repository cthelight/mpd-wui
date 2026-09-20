use std::collections::HashMap;

use axum::Json;
use axum::extract::{Query, State};

use mpd_client::Browse;

use crate::AppState;
use crate::error::ApiError;
use crate::search::{self, Field, SearchHit};

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

/// Enabled result kinds for a free-text search, from `?kinds=` (a comma list of
/// `artist`, `album`, `track`). Absent means all enabled; present means exactly
/// the listed tokens are enabled (so an empty value enables none). The frontend
/// omits the param for the default and skips the request entirely when nothing
/// is enabled, so a present value is always an explicit narrowing.
fn parse_kinds(params: &HashMap<String, String>) -> search::HitKinds {
    let Some(raw) = params.get("kinds") else {
        return search::HitKinds::ALL;
    };
    let mut kinds = search::HitKinds::NONE;
    for token in raw.split(',') {
        match token.trim() {
            "artist" => kinds.artist = true,
            "album" => kinds.album = true,
            "track" => kinds.track = true,
            _ => {}
        }
    }
    kinds
}

/// Local search: pull the whole library into the process (cached by TTL) and
/// rank it here. MPD's filter grammar has no `OR`, so broad "match any tag"
/// search can't be done server-side; doing it locally keeps the two paths
/// uniform and avoids the fragile filter-syntax edge cases.
///
/// A `q` term fuzzy-matches artists, albums and tracks together (one ordered
/// list, capped by `?limit=`). Without `q`, the tag params are exact
/// constraints that return every matching track (the collection drill-down
/// "show me the whole item" path).
pub async fn search(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<SearchHit>>, ApiError> {
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
    query.kinds = parse_kinds(&params);
    if query.is_empty() || query.kinds == search::HitKinds::NONE {
        return Ok(Json(Vec::new()));
    }
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|l| *l > 0)
        .unwrap_or(search::DEFAULT_LIMIT);
    let library = state.library.get(&state.client).await?;
    Ok(Json(search::search(&library, &query, limit)))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{normalize_browse_path, parse_kinds};
    use crate::search::HitKinds;

    #[test]
    fn normalize_strips_leading_slash() {
        assert_eq!(normalize_browse_path(""), "");
        assert_eq!(normalize_browse_path("/"), "");
        assert_eq!(normalize_browse_path("/Band"), "Band");
        assert_eq!(normalize_browse_path("///Band"), "Band");
        assert_eq!(normalize_browse_path("Band/Album"), "Band/Album");
    }

    #[test]
    fn kinds_absent_means_all() {
        let params: HashMap<String, String> = HashMap::new();
        assert_eq!(parse_kinds(&params), HitKinds::ALL);
    }

    #[test]
    fn kinds_reads_enabled_tokens() {
        let mut params = HashMap::new();
        params.insert("kinds".into(), "album,track".into());
        let kinds = parse_kinds(&params);
        assert!(!kinds.artist);
        assert!(kinds.album);
        assert!(kinds.track);
    }

    #[test]
    fn kinds_present_but_empty_means_none() {
        let mut params = HashMap::new();
        params.insert("kinds".into(), String::new());
        assert_eq!(parse_kinds(&params), HitKinds::NONE);
    }
}
