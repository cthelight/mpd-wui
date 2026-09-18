//! Local library search.
//!
//! The whole database is pulled into the process (see [`crate::store::LibraryStore`])
//! and matched here. MPD's filter grammar cannot express "match *any* tag"
//! (there is no `OR`), so broad search is done in-process instead of relying on
//! the server.
//!
//! A snapshot is indexed once by [`LibraryIndex`] — track haystacks, distinct
//! artists/albums, and per-field inverted maps — and reused for every query.
//!
//! A free-text query is ranked with the [`nucleo_matcher`] fuzzy engine (the
//! same scorer Helix uses for its picker): every word in the query must appear
//! as a fuzzy substring of an entity's haystack, and entities are ranked by
//! their total score. Artists, albums and tracks are each scored against the
//! same query, so a single search returns all three in one relevance-ordered
//! list — like typing into `fzf`.

use std::cmp::Ordering;
use std::collections::HashMap;

use mpd_client::Song;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde::Serialize;

/// Default cap on free-text (fuzzy) results.
pub const DEFAULT_LIMIT: usize = 100;

/// A searchable song field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Track,
    Name,
    Composer,
    Date,
    File,
}

impl Field {
    fn value<'a>(&self, song: &'a Song) -> Option<&'a str> {
        match self {
            Field::Title => song.title.as_deref(),
            Field::Artist => song.artist.as_deref(),
            Field::Album => song.album.as_deref(),
            Field::AlbumArtist => song.albumartist.as_deref(),
            Field::Genre => song.genre.as_deref(),
            Field::Track => song.track.as_deref(),
            Field::Name => song.name.as_deref(),
            Field::Composer => song.composer.as_deref(),
            Field::Date => song.date.as_deref(),
            Field::File => Some(song.file.as_str()),
        }
    }
}

/// Fields a free-text query is fuzzy-matched against, in the order they are
/// joined into a track's haystack (so a multi-word query can span several tags).
const TRACK_FIELDS: [Field; 10] = [
    Field::Title,
    Field::Artist,
    Field::Album,
    Field::AlbumArtist,
    Field::Composer,
    Field::Name,
    Field::Genre,
    Field::Track,
    Field::Date,
    Field::File,
];

/// A single search result: an artist, an album, or one track. All three are
/// ranked against the same query so they can share one ordered list.
///
/// The `Track` variant holds a full [`Song`]; the enum is sized to fit it.
/// Boxing the song would shrink the enum but add a heap allocation per track
/// hit for a payload that is only ever serialized to JSON, so it is not worth it.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SearchHit {
    /// A distinct artist / album-artist value.
    Artist {
        /// The artist name to display.
        name: String,
        /// Tag to drill on: `"artist"` or `"albumartist"`.
        key: String,
        /// Number of library tracks attributed to this artist.
        count: u32,
        /// Fuzzy relevance score (higher is a closer match).
        score: u32,
    },
    /// A distinct (album, album-artist) pair.
    Album {
        /// The album name to display.
        name: String,
        /// The album artist, when known.
        artist: Option<String>,
        /// Number of tracks on this album.
        count: u32,
        /// Fuzzy relevance score (higher is a closer match).
        score: u32,
    },
    /// A single matching track.
    Track {
        song: Song,
        /// Fuzzy relevance score (higher is a closer match).
        score: u32,
    },
}

impl SearchHit {
    /// The relevance score, regardless of kind.
    pub fn score(&self) -> u32 {
        match self {
            SearchHit::Artist { score, .. }
            | SearchHit::Album { score, .. }
            | SearchHit::Track { score, .. } => *score,
        }
    }

    /// A cheap, stable string used only to order hits that tie on score.
    fn label(&self) -> &str {
        match self {
            SearchHit::Artist { name, .. } => name,
            SearchHit::Album { name, .. } => name,
            SearchHit::Track { song, .. } => &song.file,
        }
    }
}

/// A search query: an optional free-text term (fuzzy, multi-word) combined with
/// zero or more exact field constraints (ANDed).
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// Free-text term; its words are matched independently, order-insensitive.
    pub free_text: Option<String>,
    /// Exact field constraints, all of which must hold.
    pub exact: Vec<(Field, String)>,
}

impl Query {
    /// True when the query would match nothing (no term and no non-empty exact).
    pub fn is_empty(&self) -> bool {
        self.free_text
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
            && self.exact.iter().all(|(_, v)| v.is_empty())
    }
}

/// A precomputed index over a library snapshot, built once (in a single pass)
/// and reused for every search:
///
/// * track haystacks are precomputed, so fuzzy scoring never rebuilds strings;
/// * distinct artists and albums (with their song lists) are precomputed, so
///   aggregation is O(distinct) instead of O(songs) per query;
/// * exact field constraints are resolved through an inverted map
///   (field → lowercased value → song indices) instead of scanning every song.
pub struct LibraryIndex {
    songs: Vec<Song>,
    track_haystacks: Vec<String>,
    artists: Vec<ArtistEntry>,
    albums: Vec<AlbumEntry>,
    exact: HashMap<Field, HashMap<String, Vec<usize>>>,
}

/// A distinct artist / album-artist name and the songs it appears on.
#[derive(Debug)]
struct ArtistEntry {
    name: String,
    /// Drill tag: `"artist"` or `"albumartist"`.
    key: &'static str,
    /// Song indices, in library order.
    songs: Vec<usize>,
}

/// A distinct (album, album artist) pair, its precomputed haystack and the
/// songs it appears on.
#[derive(Debug)]
struct AlbumEntry {
    name: String,
    artist: Option<String>,
    /// `"{album} {artist}"` (or just the album name), precomputed for scoring.
    haystack: String,
    /// Song indices, in library order.
    songs: Vec<usize>,
}

impl LibraryIndex {
    /// One pass over the snapshot: haystacks, distinct artists/albums and the
    /// per-field inverted maps.
    pub fn build(songs: Vec<Song>) -> Self {
        let track_haystacks: Vec<String> = songs.iter().map(track_haystack).collect();

        // name -> (seen as an album artist, song indices)
        let mut artists: HashMap<String, (bool, Vec<usize>)> = HashMap::new();
        for (i, song) in songs.iter().enumerate() {
            let mut names: Vec<&str> = Vec::new();
            if let Some(a) = song.artist.as_deref().filter(|v| !v.is_empty()) {
                names.push(a);
            }
            if let Some(aa) = song.albumartist.as_deref().filter(|v| !v.is_empty()) {
                if !names.contains(&aa) {
                    names.push(aa);
                }
            }
            for name in &names {
                artists.entry((*name).to_string()).or_default().1.push(i);
            }
            if let Some(aa) = song.albumartist.as_deref().filter(|v| !v.is_empty()) {
                if let Some(entry) = artists.get_mut(aa) {
                    entry.0 = true;
                }
            }
        }
        let artists = artists
            .into_iter()
            .map(|(name, (is_album_artist, song_indices))| ArtistEntry {
                name,
                key: if is_album_artist {
                    "albumartist"
                } else {
                    "artist"
                },
                songs: song_indices,
            })
            .collect();

        // (album, album artist) -> song indices
        let mut albums: HashMap<(String, Option<String>), Vec<usize>> = HashMap::new();
        for (i, song) in songs.iter().enumerate() {
            if let Some(album) = song.album.as_deref().filter(|v| !v.is_empty()) {
                let artist = song
                    .albumartist
                    .as_deref()
                    .filter(|v| !v.is_empty())
                    .map(str::to_string);
                albums
                    .entry((album.to_string(), artist))
                    .or_default()
                    .push(i);
            }
        }
        let albums = albums
            .into_iter()
            .map(|((name, artist), song_indices)| {
                let haystack = match &artist {
                    Some(a) => format!("{name} {a}"),
                    None => name.clone(),
                };
                AlbumEntry {
                    name,
                    artist,
                    haystack,
                    songs: song_indices,
                }
            })
            .collect();

        // Field -> lowercased value -> song indices (in library order).
        let mut exact: HashMap<Field, HashMap<String, Vec<usize>>> = HashMap::new();
        for (i, song) in songs.iter().enumerate() {
            for field in TRACK_FIELDS {
                if let Some(v) = field.value(song).filter(|v| !v.is_empty()) {
                    exact
                        .entry(field)
                        .or_default()
                        .entry(v.to_ascii_lowercase())
                        .or_default()
                        .push(i);
                }
            }
        }

        Self {
            songs,
            track_haystacks,
            artists,
            albums,
            exact,
        }
    }

    /// Number of songs in the indexed snapshot.
    pub fn len(&self) -> usize {
        self.songs.len()
    }

    /// True when the indexed snapshot holds no songs.
    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    /// Indices of the songs satisfying every exact constraint, in library
    /// order.
    fn exact_candidates(&self, query: &Query) -> Vec<usize> {
        let mut lists: Vec<&Vec<usize>> = Vec::new();
        for (field, want) in &query.exact {
            if want.is_empty() {
                continue;
            }
            match self
                .exact
                .get(field)
                .and_then(|map| map.get(&want.to_ascii_lowercase()))
            {
                Some(list) => lists.push(list),
                None => return Vec::new(),
            }
        }
        if lists.is_empty() {
            return (0..self.songs.len()).collect();
        }
        // Start from the smallest list; merge-intersect the rest (all lists
        // are strictly ascending, built in song order).
        let mut out = Vec::new();
        let mut smallest = &lists[0];
        for list in &lists[1..] {
            if list.len() < smallest.len() {
                smallest = list;
            }
        }
        out.extend_from_slice(smallest);
        for list in &lists {
            if std::ptr::eq(list, smallest) {
                continue;
            }
            out = intersect(&out, list);
        }
        out
    }

    /// Fuzzy-score artists, albums and tracks against `term`, interleaving the
    /// kinds best-first and capping at `limit`. When `subset` is `Some`, only
    /// songs in it are considered (the exact-constrained pool).
    fn fuzzy_rank(&self, subset: Option<&[usize]>, term: &str, limit: usize) -> Vec<SearchHit> {
        // Always case-insensitive: an all-caps query must still match Title-Case
        // metadata, so `Smart` (fzf's "caps = case-sensitive") would surprise users.
        let pattern = Pattern::new(
            term,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let mut buf: Vec<char> = Vec::new();
        let mut hits: Vec<SearchHit> = Vec::new();

        // Artists: precomputed distinct names; when constrained, the count is
        // restricted to the candidate pool.
        for artist in &self.artists {
            let count = match subset {
                Some(subset) => count_in(&artist.songs, subset),
                None => artist.songs.len() as u32,
            };
            if count == 0 {
                continue;
            }
            if let Some(score) = score_haystack(&pattern, &mut matcher, &mut buf, &artist.name) {
                hits.push(SearchHit::Artist {
                    key: artist.key.to_string(),
                    name: artist.name.clone(),
                    count,
                    score,
                });
            }
        }

        // Albums: precomputed distinct (album, album artist) pairs.
        for album in &self.albums {
            let count = match subset {
                Some(subset) => count_in(&album.songs, subset),
                None => album.songs.len() as u32,
            };
            if count == 0 {
                continue;
            }
            if let Some(score) = score_haystack(&pattern, &mut matcher, &mut buf, &album.haystack) {
                hits.push(SearchHit::Album {
                    name: album.name.clone(),
                    artist: album.artist.clone(),
                    count,
                    score,
                });
            }
        }

        // Tracks: precomputed haystacks, no per-query string building.
        let mut score_track = |i: usize| {
            if let Some(score) =
                score_haystack(&pattern, &mut matcher, &mut buf, &self.track_haystacks[i])
            {
                hits.push(SearchHit::Track {
                    song: self.songs[i].clone(),
                    score,
                });
            }
        };
        if let Some(subset) = subset {
            for &i in subset {
                score_track(i);
            }
        } else {
            for i in 0..self.songs.len() {
                score_track(i);
            }
        }

        hits.sort_by(|a, b| {
            b.score()
                .cmp(&a.score())
                .then_with(|| a.label().cmp(b.label()))
        });
        hits.truncate(limit);
        hits
    }
}

/// Size of the intersection of two strictly ascending index lists.
fn count_in(a: &[usize], b: &[usize]) -> u32 {
    let (mut i, mut j) = (0usize, 0usize);
    let mut count = 0u32;
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                count += 1;
                i += 1;
                j += 1;
            }
        }
    }
    count
}

/// The intersection of two strictly ascending index lists, in order.
fn intersect(a: &[usize], b: &[usize]) -> Vec<usize> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

/// Rank artists, albums and tracks in `index` against `query` and return them
/// in a single relevance-ordered list.
///
/// * **Free-text** (a non-empty `free_text`): every entity is fuzzy-scored and
///   the three kinds are interleaved best-first, capped at `limit`.
/// * **Exact-only** (no free text): every track satisfying the exact constraints
///   is returned in library order — the "show me the whole item" path used by
///   collection drill-downs. Uncapped and unscored.
pub fn search(index: &LibraryIndex, query: &Query, limit: usize) -> Vec<SearchHit> {
    if query.is_empty() {
        return Vec::new();
    }

    // Candidates are the songs satisfying every exact constraint (`None` = all
    // songs). For exact-only queries these *are* the result; for free-text
    // queries they are the pool the fuzzy scorer ranks (so a constrained query
    // only surfaces that subset).
    let subset: Option<Vec<usize>> = if query.exact.is_empty() {
        None
    } else {
        Some(index.exact_candidates(query))
    };

    match query
        .free_text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        Some(term) => index.fuzzy_rank(subset.as_deref(), term, limit),
        None => subset
            .unwrap_or_else(|| (0..index.len()).collect())
            .into_iter()
            .map(|i| SearchHit::Track {
                song: index.songs[i].clone(),
                score: 0,
            })
            .collect(),
    }
}

/// Score one haystack against the pattern (returns `None` when no word matches).
fn score_haystack(
    pattern: &Pattern,
    matcher: &mut Matcher,
    buf: &mut Vec<char>,
    haystack: &str,
) -> Option<u32> {
    pattern.score(Utf32Str::new(haystack, buf), matcher)
}

/// Join every non-empty searchable field of a song into one haystack, so a
/// multi-word query can span several tags (e.g. "title artist").
fn track_haystack(song: &Song) -> String {
    let mut out = String::new();
    for field in TRACK_FIELDS {
        if let Some(v) = field.value(song).filter(|v| !v.is_empty()) {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(v);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn song(
        file: &str,
        artist: Option<&str>,
        album: Option<&str>,
        albumartist: Option<&str>,
        title: Option<&str>,
        genre: Option<&str>,
    ) -> Song {
        Song {
            file: file.to_string(),
            artist: artist.map(str::to_string),
            album: album.map(str::to_string),
            albumartist: albumartist.map(str::to_string),
            title: title.map(str::to_string),
            genre: genre.map(str::to_string),
            ..Default::default()
        }
    }

    fn library() -> Vec<Song> {
        vec![
            song(
                "alpha/one.flac",
                Some("Alpha"),
                Some("Alpha Debut"),
                Some("Alpha"),
                Some("One"),
                Some("Jazz"),
            ),
            song(
                "alpha/two.flac",
                Some("Alpha"),
                Some("Alpha Debut"),
                Some("Alpha"),
                Some("Two"),
                Some("Jazz"),
            ),
            song(
                "beta/three.flac",
                Some("Beta"),
                Some("Beta Live"),
                Some("Beta"),
                Some("Three"),
                Some("Rock"),
            ),
            // Track artist differs from the album artist.
            song(
                "gamma/four.flac",
                Some("Gamma feat. Delta"),
                Some("Gamma Compilation"),
                Some("Gamma"),
                Some("Four"),
                Some("Folk"),
            ),
        ]
    }

    fn indexed(lib: Vec<Song>) -> LibraryIndex {
        LibraryIndex::build(lib)
    }

    fn kinds(hits: &[SearchHit]) -> impl Iterator<Item = &SearchHit> {
        hits.iter()
    }

    #[test]
    fn free_text_returns_tracks_artists_and_albums() {
        let index = indexed(library());
        let hits = search(
            &index,
            &Query {
                free_text: Some("alpha".into()),
                exact: vec![],
            },
            DEFAULT_LIMIT,
        );
        // 1 artist + 1 album + 2 tracks.
        assert_eq!(hits.len(), 4);
        // Relevance-ordered: scores are non-increasing.
        for pair in hits.windows(2) {
            assert!(pair[0].score() >= pair[1].score());
        }
        assert!(
            kinds(&hits).any(|h| matches!(h, SearchHit::Artist { name, .. } if name == "Alpha"))
        );
        assert!(
            kinds(&hits)
                .any(|h| matches!(h, SearchHit::Album { name, .. } if name == "Alpha Debut"))
        );
        let tracks: Vec<_> = kinds(&hits)
            .filter_map(|h| match h {
                SearchHit::Track { song, .. } => Some(song.file.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tracks, vec!["alpha/one.flac", "alpha/two.flac"]);
    }

    #[test]
    fn free_text_is_case_insensitive() {
        let index = indexed(library());
        let hits = search(
            &index,
            &Query {
                free_text: Some("BETA".into()),
                exact: vec![],
            },
            DEFAULT_LIMIT,
        );
        // Matches the artist "Beta", the album "Beta Live", and the track.
        assert!(
            hits.iter()
                .any(|h| matches!(h, SearchHit::Artist { name, .. } if name == "Beta"))
        );
        assert!(
            hits.iter().any(
                |h| matches!(h, SearchHit::Track { song, .. } if song.file == "beta/three.flac")
            )
        );
    }

    #[test]
    fn free_text_no_match() {
        let index = indexed(library());
        let hits = search(
            &index,
            &Query {
                free_text: Some("zzz".into()),
                exact: vec![],
            },
            DEFAULT_LIMIT,
        );
        assert!(hits.is_empty());
    }

    #[test]
    fn exact_only_returns_all_tracks_in_order() {
        let index = indexed(library());
        let hits = search(
            &index,
            &Query {
                free_text: None,
                exact: vec![(Field::Artist, "alpha".into())],
            },
            DEFAULT_LIMIT,
        );
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| matches!(h, SearchHit::Track { .. })));
        // Library order is preserved (no re-ranking for exact-only).
        assert_eq!(hits[0].label(), "alpha/one.flac");
        assert_eq!(hits[1].label(), "alpha/two.flac");
    }

    #[test]
    fn exact_constraints_are_anded() {
        let index = indexed(library());
        let one = search(
            &index,
            &Query {
                free_text: None,
                exact: vec![(Field::Genre, "jazz".into()), (Field::Title, "one".into())],
            },
            DEFAULT_LIMIT,
        );
        assert_eq!(one.len(), 1);

        let none = search(
            &index,
            &Query {
                free_text: None,
                exact: vec![
                    (Field::Genre, "jazz".into()),
                    (Field::Title, "three".into()),
                ],
            },
            DEFAULT_LIMIT,
        );
        assert!(none.is_empty());
    }

    #[test]
    fn free_text_and_exact_combine() {
        let index = indexed(library());
        let hit = search(
            &index,
            &Query {
                free_text: Some("two".into()),
                exact: vec![(Field::Artist, "alpha".into())],
            },
            DEFAULT_LIMIT,
        );
        assert_eq!(hit.len(), 1);
        assert!(matches!(&hit[0], SearchHit::Track { song, .. } if song.file == "alpha/two.flac"));

        // Term matches a different song than the exact constraint allows.
        let none = search(
            &index,
            &Query {
                free_text: Some("two".into()),
                exact: vec![(Field::Artist, "beta".into())],
            },
            DEFAULT_LIMIT,
        );
        assert!(none.is_empty());
    }

    #[test]
    fn constrained_fuzzy_counts_are_restricted_to_the_pool() {
        let index = indexed(library());
        // "alpha" fuzzy-matches the artist, but the genre constraint restricts
        // the pool to the two Jazz tracks.
        let hits = search(
            &index,
            &Query {
                free_text: Some("alpha".into()),
                exact: vec![(Field::Genre, "jazz".into())],
            },
            DEFAULT_LIMIT,
        );
        assert!(hits.iter().any(
            |h| matches!(h, SearchHit::Artist { name, count, .. } if name == "Alpha" && *count == 2)
        ));
        // The Beta tracks are excluded from the pool entirely.
        assert!(
            !hits.iter().any(
                |h| matches!(h, SearchHit::Track { song, .. } if song.file == "beta/three.flac")
            )
        );
    }

    #[test]
    fn artist_drill_key_prefers_albumartist() {
        let index = indexed(library());
        let hits = search(
            &index,
            &Query {
                free_text: Some("gamma".into()),
                exact: vec![],
            },
            DEFAULT_LIMIT,
        );
        // "Gamma" only appears as an albumartist -> drill on albumartist.
        assert!(hits.iter().any(
            |h| matches!(h, SearchHit::Artist { name, key, .. } if name == "Gamma" && key == "albumartist")
        ));
        // "Gamma feat. Delta" only appears as a track artist -> drill on artist.
        assert!(hits.iter().any(
            |h| matches!(h, SearchHit::Artist { name, key, .. } if name == "Gamma feat. Delta" && key == "artist")
        ));
    }

    #[test]
    fn limit_caps_fuzzy_results() {
        let index = indexed(library());
        // "a" appears in nearly every name/field, so there are plenty of hits.
        let hits = search(
            &index,
            &Query {
                free_text: Some("a".into()),
                exact: vec![],
            },
            2,
        );
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn hit_serializes_with_kind_discriminator() {
        let song = Song {
            file: "a/b.flac".into(),
            ..Default::default()
        };
        let hits = vec![
            SearchHit::Artist {
                name: "A".into(),
                key: "artist".into(),
                count: 3,
                score: 10,
            },
            SearchHit::Album {
                name: "B".into(),
                artist: Some("A".into()),
                count: 2,
                score: 9,
            },
            SearchHit::Track { song, score: 8 },
        ];
        let json = serde_json::to_value(&hits).expect("hits serialize");
        assert_eq!(json[0]["kind"], "artist");
        assert_eq!(json[0]["name"], "A");
        assert_eq!(json[0]["key"], "artist");
        assert_eq!(json[0]["count"], 3);
        assert_eq!(json[0]["score"], 10);
        assert_eq!(json[1]["kind"], "album");
        assert_eq!(json[1]["name"], "B");
        assert_eq!(json[1]["artist"], "A");
        assert_eq!(json[2]["kind"], "track");
        assert_eq!(json[2]["song"]["file"], "a/b.flac");
    }

    #[test]
    fn empty_query_returns_nothing() {
        let index = indexed(library());
        assert!(search(&index, &Query::default(), DEFAULT_LIMIT).is_empty());
        assert!(
            search(
                &index,
                &Query {
                    free_text: Some("   ".into()),
                    exact: vec![]
                },
                DEFAULT_LIMIT
            )
            .is_empty()
        );
    }

    #[test]
    fn index_len_tracks_snapshot_size() {
        let index = indexed(library());
        assert_eq!(index.len(), 4);
        assert!(!index.is_empty());
        let empty = LibraryIndex::build(Vec::new());
        assert!(empty.is_empty());
    }
}
