//! Local library search.
//!
//! The whole database is pulled into the process (see [`crate::store::LibraryStore`])
//! and filtered here. MPD's filter grammar cannot express "match *any* tag"
//! (there is no `OR`), so broad free-text search is done in-process instead of
//! relying on the server.

use mpd_client::Song;

/// A searchable song field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Fields a free-text term is matched against (case-insensitive substring).
/// This is the "OR across tags" that MPD itself cannot do.
const FREE_TEXT_FIELDS: [Field; 10] = [
    Field::Title,
    Field::Artist,
    Field::Album,
    Field::AlbumArtist,
    Field::Genre,
    Field::Track,
    Field::Name,
    Field::Composer,
    Field::Date,
    Field::File,
];

/// A search query: an optional free-text term (OR across all fields) combined
/// with zero or more exact field constraints (AND).
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub free_text: Option<String>,
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

    /// True when the song satisfies every part of the query.
    pub fn matches(&self, song: &Song) -> bool {
        if let Some(needle) = self
            .free_text
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(|q| q.to_ascii_lowercase())
        {
            let hit = FREE_TEXT_FIELDS.iter().any(|f| {
                f.value(song)
                    .map(|v| v.to_ascii_lowercase().contains(&needle))
                    .unwrap_or(false)
            });
            if !hit {
                return false;
            }
        }
        for (field, want) in &self.exact {
            if want.is_empty() {
                continue;
            }
            match field.value(song) {
                Some(have) if have.eq_ignore_ascii_case(want) => {}
                _ => return false,
            }
        }
        true
    }
}

/// Return the songs in `songs` matching `query`, preserving order.
pub fn filter(songs: &[Song], query: &Query) -> Vec<Song> {
    if query.is_empty() {
        return Vec::new();
    }
    songs.iter().filter(|s| query.matches(s)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(file: &str, artist: Option<&str>, title: Option<&str>, genre: Option<&str>) -> Song {
        Song {
            file: file.to_string(),
            artist: artist.map(str::to_string),
            title: title.map(str::to_string),
            genre: genre.map(str::to_string),
            ..Default::default()
        }
    }

    fn library() -> Vec<Song> {
        vec![
            song("alpha/one.flac", Some("Alpha"), Some("One"), Some("Jazz")),
            song("beta/two.flac", Some("Beta"), Some("Two"), Some("Rock")),
            song(
                "gamma/blue.flac",
                Some("Gamma"),
                Some("Blue Album"),
                Some("Folk"),
            ),
        ]
    }

    #[test]
    fn free_text_matches_any_field_case_insensitive() {
        let lib = library();
        // Matches the artist "Beta".
        assert_eq!(
            filter(
                &lib,
                &Query {
                    free_text: Some("beta".into()),
                    exact: vec![]
                }
            )
            .len(),
            1
        );
        // Matches the title "One" even though the query is uppercase.
        assert_eq!(
            filter(
                &lib,
                &Query {
                    free_text: Some("ONE".into()),
                    exact: vec![]
                }
            )
            .len(),
            1
        );
        // Matches a genre.
        assert_eq!(
            filter(
                &lib,
                &Query {
                    free_text: Some("folK".into()),
                    exact: vec![]
                }
            )
            .len(),
            1
        );
        // Matches the file path.
        assert_eq!(
            filter(
                &lib,
                &Query {
                    free_text: Some("blue.flac".into()),
                    exact: vec![]
                }
            )
            .len(),
            1
        );
    }

    #[test]
    fn free_text_no_match() {
        let lib = library();
        assert_eq!(
            filter(
                &lib,
                &Query {
                    free_text: Some("zzz".into()),
                    exact: vec![]
                }
            )
            .len(),
            0
        );
    }

    #[test]
    fn exact_single_and_multi_tag() {
        let lib = library();
        // Single exact constraint.
        let one = Query {
            free_text: None,
            exact: vec![(Field::Genre, "rock".into())],
        };
        assert_eq!(filter(&lib, &one).len(), 1);

        // Multiple exact constraints are ANDed (the case MPD's grammar broke on).
        let two = Query {
            free_text: None,
            exact: vec![(Field::Genre, "rock".into()), (Field::Title, "two".into())],
        };
        assert_eq!(filter(&lib, &two).len(), 1);

        // Contradictory exact constraints match nothing.
        let none = Query {
            free_text: None,
            exact: vec![(Field::Genre, "rock".into()), (Field::Title, "one".into())],
        };
        assert_eq!(filter(&lib, &none).len(), 0);
    }

    #[test]
    fn free_text_and_exact_combine() {
        let lib = library();
        // Term must match some field AND the exact constraint must hold.
        let q = Query {
            free_text: Some("two".into()),
            exact: vec![(Field::Artist, "beta".into())],
        };
        assert_eq!(filter(&lib, &q).len(), 1);

        // Term matches a different song than the exact constraint.
        let q = Query {
            free_text: Some("two".into()),
            exact: vec![(Field::Artist, "alpha".into())],
        };
        assert_eq!(filter(&lib, &q).len(), 0);
    }

    #[test]
    fn empty_query_matches_nothing() {
        let lib = library();
        assert_eq!(filter(&lib, &Query::default()).len(), 0);
        assert_eq!(
            filter(
                &lib,
                &Query {
                    free_text: Some("   ".into()),
                    exact: vec![]
                }
            )
            .len(),
            0
        );
    }
}
