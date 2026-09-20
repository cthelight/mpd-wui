//! MPD text-protocol primitives: response parsing, item grouping, filter
//! escaping, and binary (album-art) responses.

use crate::types::{Browse, DirEntry, PlayState, Song, Status};

/// An MPD `ACK` error line: `ACK [code@index] message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ack {
    pub code: i32,
    pub index: i32,
    pub message: String,
}

impl std::fmt::Display for Ack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MPD error [{}@{}]: {}",
            self.code, self.index, self.message
        )
    }
}

/// A single `key: value` field.
pub type Field = (String, String);
/// An ordered list of fields (one item group, or a flat response).
pub type FieldList = Vec<Field>;

/// A parsed MPD text response (everything up to and including `OK`/`ACK`).
#[derive(Debug, Clone, Default)]
pub struct Response {
    /// All `key: value` lines, in order.
    pub fields: FieldList,
    /// Set if the response terminated with `ACK`.
    pub ack: Option<Ack>,
}

impl Response {
    /// Error if the response was an `ACK`.
    pub fn check(&self) -> Result<(), Ack> {
        match &self.ack {
            Some(a) => Err(a.clone()),
            None => Ok(()),
        }
    }

    /// First value for `key`, if present.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// All values for `key`, in order.
    pub fn get_all(&self, key: &str) -> Vec<&str> {
        self.fields
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

/// Parse a text response (raw bytes up to and including `OK`/`ACK`).
pub fn parse_text(raw: &str) -> Response {
    let mut fields = Vec::new();
    let mut ack = None;
    for line in raw.lines() {
        let line = line.trim_end_matches('\r');
        if line == "OK" {
            break;
        }
        if let Some(rest) = line.strip_prefix("ACK ") {
            ack = Some(parse_ack(rest));
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            fields.push((k.to_string(), v.trim_start().to_string()));
        }
    }
    Response { fields, ack }
}

fn parse_ack(s: &str) -> Ack {
    if let Some(br) = s.strip_prefix('[') {
        if let Some(end) = br.find(']') {
            let code_idx = &br[..end];
            let (code, index) = match code_idx.split_once('@') {
                Some((c, i)) => (
                    c.parse::<i32>().unwrap_or(-1),
                    i.parse::<i32>().unwrap_or(-1),
                ),
                None => (-1, -1),
            };
            let message = br[end + 1..].trim().to_string();
            return Ack {
                code,
                index,
                message,
            };
        }
    }
    Ack {
        code: -1,
        index: -1,
        message: s.to_string(),
    }
}

/// Group a flat field list into item groups.
///
/// A new item begins at any line whose key is in `item_start`. Lines whose key
/// is in `global` are collected separately (e.g. trailing `songcount` /
/// `playtime`). Lines that appear before the first item-start and are not
/// global are also collected as flat lines.
///
/// Returns `(items, flat)` where each item is an ordered field list.
pub fn group_items(
    fields: &FieldList,
    item_start: &[&str],
    global: &[&str],
) -> (Vec<FieldList>, FieldList) {
    let mut items: Vec<FieldList> = Vec::new();
    let mut flat: FieldList = Vec::new();
    let mut current: Option<FieldList> = None;

    for (k, v) in fields {
        if global.contains(&k.as_str()) {
            if let Some(c) = current.take() {
                items.push(c);
            }
            flat.push((k.clone(), v.clone()));
        } else if item_start.contains(&k.as_str()) {
            if let Some(c) = current.take() {
                items.push(c);
            }
            current = Some(vec![(k.clone(), v.clone())]);
        } else if let Some(c) = current.as_mut() {
            c.push((k.clone(), v.clone()));
        } else {
            flat.push((k.clone(), v.clone()));
        }
    }
    if let Some(c) = current {
        items.push(c);
    }
    (items, flat)
}

/// Escape a value for use inside an MPD filter expression (single-quoted).
/// Backslash first, then single quote.
pub fn escape_filter_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            _ => out.push(c),
        }
    }
    out
}

/// Quote a value for use as a protocol argument. Backslash, then double quote,
/// then wrap in double quotes.
pub fn quote_arg(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Build a single MPD filter clause: `(Tag op 'value')`.
pub fn filter_clause(tag: &str, op: &str, value: &str) -> String {
    format!("({tag} {op} '{}')", escape_filter_value(value))
}

/// A parsed binary (album-art) response.
#[derive(Debug, Clone)]
pub struct Art {
    /// Total size of the picture in bytes (from the `size:` line).
    pub total: u32,
    /// Declared mime type (from the `type:` line), if any.
    pub mime: Option<String>,
    /// The picture bytes for this chunk.
    pub data: Vec<u8>,
}

/// Parse a binary response. The raw buffer contains the header lines, the
/// `binary: N` line, N bytes of data, then a newline and `OK`.
pub fn parse_art(raw: &[u8]) -> Option<Art> {
    let mut pos = 0usize;
    let mut total: Option<u32> = None;
    let mut mime: Option<String> = None;

    while pos < raw.len() {
        let nl = raw[pos..].iter().position(|b| *b == b'\n')?;
        let line_end = pos + nl;
        let line_bytes = &raw[pos..line_end];
        let line = std::str::from_utf8(line_bytes).ok()?;
        let line = line.trim_end_matches('\r');

        if line == "OK" || line.starts_with("ACK") {
            // No binary section present.
            break;
        }
        if let Some(rest) = line.strip_prefix("binary:") {
            let n: u32 = rest.trim().parse().ok()?;
            pos = line_end + 1;
            let start = pos;
            let end = (start + n as usize).min(raw.len());
            let data = raw[start..end].to_vec();
            return Some(Art {
                total: total.unwrap_or(n),
                mime,
                data,
            });
        }
        if let Some(rest) = line.strip_prefix("size:") {
            total = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("type:") {
            mime = Some(rest.trim().to_string());
        }
        pos = line_end + 1;
    }
    None
}

/// Guess an image mime type from magic bytes.
pub fn sniff_mime(b: &[u8]) -> String {
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp".to_string()
    } else if b.len() >= 8 && &b[0..8] == b"\x89PNG\r\n\x1a\n" {
        "image/png".to_string()
    } else if b.len() >= 3 && b[0] == 0xFF && b[1] == 0xD8 && b[2] == 0xFF {
        "image/jpeg".to_string()
    } else if b.len() >= 6 && (&b[0..6] == b"GIF87a" || &b[0..6] == b"GIF89a") {
        "image/gif".to_string()
    } else if b.len() >= 2 && b[0] == b'B' && b[1] == b'M' {
        "image/bmp".to_string()
    } else {
        "image/jpeg".to_string()
    }
}

// ---------------------------------------------------------------------------
// Typed parsers
// ---------------------------------------------------------------------------

/// Look up an optional string field in an ordered field list.
fn opt_str(fields: &FieldList, key: &str) -> Option<String> {
    fields
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
}

/// Build a [`Song`] from an ordered field list (one item group).
pub fn parse_song(fields: &FieldList) -> Song {
    Song {
        file: opt_str(fields, "file").unwrap_or_default(),
        id: opt_str(fields, "Id").and_then(|v| v.parse().ok()),
        title: opt_str(fields, "Title"),
        artist: opt_str(fields, "Artist"),
        album: opt_str(fields, "Album"),
        albumartist: opt_str(fields, "AlbumArtist"),
        track: opt_str(fields, "Track"),
        genre: opt_str(fields, "Genre"),
        name: opt_str(fields, "Name"),
        composer: opt_str(fields, "Composer"),
        date: opt_str(fields, "Date"),
        time: opt_str(fields, "Time").and_then(|v| v.parse().ok()),
    }
}

/// Parse the `status` command into a [`Status`].
pub fn parse_status(resp: &Response) -> Status {
    let bool_of = |k: &str| resp.get(k) == Some("1");
    let uint_of = |k: &str| resp.get(k).and_then(|v| v.parse().ok()).unwrap_or(0);
    let elapsed: f32 = resp
        .get("elapsed")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let updating = match resp.get("updating_db") {
        Some(v) => v.parse::<u32>().unwrap_or(0) > 0,
        None => false,
    };
    Status {
        state: PlayState::from(resp.get("state").unwrap_or("stop")),
        // MPD reports the track as `time: <elapsed>:<total>` seconds; take the
        // total (the value after the colon).
        time: resp.get("time").map(status_time_total).unwrap_or(0),
        elapsed,
        volume: uint_of("volume"),
        random: bool_of("random"),
        repeat: bool_of("repeat"),
        single: bool_of("single"),
        consume: bool_of("consume"),
        // The wire field is `xfade`, not `crossfade`.
        crossfade: uint_of("xfade"),
        playlist_version: uint_of("playlist"),
        // The playlist length is reported as `playlistlength`, not `songs`.
        songs: uint_of("playlistlength"),
        // The current song's playlist position; MPD omits it when stopped.
        song: resp.get("song").and_then(|v| v.parse().ok()),
        updating,
    }
}

/// Extract the total track length from MPD's `time: <elapsed>:<total>` field.
/// Falls back to the whole value when no colon is present.
fn status_time_total(v: &str) -> u32 {
    v.rsplit_once(':')
        .map(|(_, total)| total)
        .unwrap_or(v)
        .parse()
        .unwrap_or(0)
}

/// Parse `currentsong` into an optional [`Song`] (None when the playlist is empty).
pub fn parse_currentsong(resp: &Response) -> Option<Song> {
    let (items, _) = group_items(&resp.fields, &["file"], &[]);
    items.first().map(parse_song)
}

/// Parse `playlistinfo` / `search` / `find` into a list of songs.
pub fn parse_song_list(resp: &Response) -> Vec<Song> {
    let (items, _) = group_items(&resp.fields, &["file"], &[]);
    items.into_iter().map(|f| parse_song(&f)).collect()
}

/// Parse `lsinfo` into a [`Browse`].
///
/// Each directory entry carries its own `songcount`/`playtime`. The listing
/// totals are computed as the sum over directories plus the loose files.
pub fn parse_lsinfo(resp: &Response) -> Browse {
    let (items, _flat) = group_items(&resp.fields, &["directory", "file"], &[]);
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let mut total_songs: u32 = 0;
    let mut total_playtime: u32 = 0;
    let mut songs_known = true;
    let mut playtime_known = true;

    for item in items {
        if item.iter().any(|(k, _)| k == "directory") {
            let songcount = opt_str(&item, "songcount").and_then(|v| v.parse().ok());
            let playtime = opt_str(&item, "playtime").and_then(|v| v.parse().ok());
            match songcount {
                Some(c) => total_songs += c,
                None => songs_known = false,
            }
            match playtime {
                Some(p) => total_playtime += p,
                None => playtime_known = false,
            }
            directories.push(DirEntry {
                path: opt_str(&item, "directory").unwrap_or_default(),
                is_dir: true,
                song: None,
                songcount,
                playtime,
            });
        } else if item.iter().any(|(k, _)| k == "file") {
            let song = Some(parse_song(&item));
            total_songs += 1;
            match song.as_ref().and_then(|s| s.time) {
                Some(t) => total_playtime += t,
                None => playtime_known = false,
            }
            files.push(DirEntry {
                path: song.as_ref().map(|s| s.file.clone()).unwrap_or_default(),
                is_dir: false,
                song,
                songcount: None,
                playtime: None,
            });
        }
    }
    Browse {
        directories,
        files,
        // MPD's `lsinfo` only reports per-directory `songcount`/`playtime` when
        // it can; if any directory omits them the true total is unknown, so we
        // report None rather than a misleading partial sum.
        songcount: songs_known.then_some(total_songs),
        playtime: playtime_known.then_some(total_playtime),
    }
}

/// Parse a `list` command into the values for a single tag (e.g. `artist`).
///
/// MPD replies with the canonical (capitalized) tag name, e.g. `Artist: X`,
/// so the lookup key is normalized before reading the response.
pub fn parse_list(resp: &Response, tag: &str) -> Vec<String> {
    resp.get_all(canonical_tag(tag))
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Map a tag name to MPD's canonical (capitalized) response field name.
fn canonical_tag(tag: &str) -> &str {
    let lower = tag.to_ascii_lowercase();
    match lower.as_str() {
        "artist" => "Artist",
        "albumartist" => "AlbumArtist",
        "album" => "Album",
        "title" => "Title",
        "track" => "Track",
        "name" => "Name",
        "genre" => "Genre",
        "date" => "Date",
        "composer" => "Composer",
        _ => tag,
    }
}

/// Build the `searchadd`/`findadd`/`search`/`count` filter argument from
/// tag/value pairs.
///
/// MPD's filter grammar has no top-level operator: a compound `AND` expression
/// must itself be wrapped in parentheses, i.e. `((A) AND (B))` — a bare
/// `(A) AND (B)` is rejected with `ACK ... Unparsed garbage after expression`.
/// A single clause needs no extra wrapping.
pub fn build_search_filters(pairs: &[(&str, &str)], op: &str) -> String {
    let mut clauses: Vec<String> = Vec::new();
    for (tag, value) in pairs {
        if !value.is_empty() {
            clauses.push(filter_clause(tag, op, value));
        }
    }
    match clauses.len() {
        0 => String::new(),
        1 => clauses.pop().expect("single clause"),
        _ => format!("({})", clauses.join(" AND ")),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_status() {
        let raw = "volume: 50\nrandom: 0\nrepeat: 1\nsingle: 0\nconsume: 0\nstate: play\nxfade: 0\nplaylist: 7\nplaylistlength: 3\ntime: 12:245\nelapsed: 12.5\nOK\n";
        let r = parse_text(raw);
        assert!(r.ack.is_none());
        let s = parse_status(&r);
        assert_eq!(s.state, crate::types::PlayState::Play);
        assert_eq!(s.volume, 50);
        assert!(s.repeat);
        assert!(!s.random);
        assert_eq!(s.playlist_version, 7);
        assert_eq!(s.songs, 3);
        assert_eq!(s.time, 245);
        assert!((s.elapsed - 12.5).abs() < 1e-6);
    }

    /// A realistic `status` response from MPD 0.23.x (playing): `time` is
    /// `<elapsed>:<total>`, the playlist length is `playlistlength`, and the
    /// crossfade field is `xfade`.
    #[test]
    fn parse_status_mpd_023_wire_format() {
        let raw = concat!(
            "volume: 100\n",
            "repeat: 0\n",
            "random: 0\n",
            "single: 0\n",
            "consume: 1\n",
            "partition: default\n",
            "playlist: 843\n",
            "playlistlength: 5\n",
            "mixrampdb: 0\n",
            "state: play\n",
            "xfade: 5\n",
            "song: 2\n",
            "songid: 17\n",
            "time: 42:229\n",
            "elapsed: 42.123\n",
            "bitrate: 320\n",
            "OK\n",
        );
        let r = parse_text(raw);
        let s = parse_status(&r);
        assert_eq!(s.state, crate::types::PlayState::Play);
        assert_eq!(s.songs, 5, "playlistlength must map to songs");
        assert_eq!(s.time, 229, "time must be the total (after the colon)");
        assert!((s.elapsed - 42.123).abs() < 1e-3);
        assert_eq!(s.crossfade, 5, "xfade must map to crossfade");
        assert_eq!(s.playlist_version, 843);
        assert!(s.consume);
        assert_eq!(s.song, Some(2), "song must be the current playlist index");
    }

    #[test]
    fn parse_status_time_without_playing_is_zero() {
        // When stopped MPD omits `time`/`elapsed`/`song` entirely.
        let raw = "volume: 100\nstate: stop\nplaylist: 843\nplaylistlength: 0\nOK\n";
        let s = parse_status(&parse_text(raw));
        assert_eq!(s.state, crate::types::PlayState::Stop);
        assert_eq!(s.time, 0);
        assert_eq!(s.elapsed, 0.0);
        assert_eq!(s.songs, 0);
        assert_eq!(s.song, None, "song must be absent when stopped");
    }

    #[test]
    fn parse_list_reads_canonical_capitalized_tag() {
        // MPD `list artist` replies `Artist: X` (capitalized), not `artist: X`.
        let raw = "Artist: A\nArtist: B\nArtist: C\nOK\n";
        let r = parse_text(raw);
        assert_eq!(
            parse_list(&r, "artist"),
            vec!["A".to_string(), "B".to_string(), "C".to_string()]
        );
    }

    #[test]
    fn parse_list_reads_other_canonical_tags() {
        let raw = "Album: X\nAlbum: Y\nOK\n";
        let r = parse_text(raw);
        assert_eq!(
            parse_list(&r, "album"),
            vec!["X".to_string(), "Y".to_string()]
        );
        let raw2 = "Genre: Jazz\nOK\n";
        assert_eq!(
            parse_list(&parse_text(raw2), "genre"),
            vec!["Jazz".to_string()]
        );
    }

    #[test]
    fn parse_ack() {
        let raw = "ACK [50@1] password not set\n";
        let r = parse_text(raw);
        let a = r.ack.expect("should be ack");
        assert_eq!(a.code, 50);
        assert_eq!(a.index, 1);
        assert_eq!(a.message, "password not set");
    }

    #[test]
    fn parse_currentsong_item_group() {
        let raw = "file: music/Band/Album/01 Song.flac\nId: 3\nTitle: Song\nArtist: Band\nAlbum: Album\nAlbumArtist: Band\nTrack: 01\nTime: 200\nOK\n";
        let r = parse_text(raw);
        let song = parse_currentsong(&r).expect("song");
        assert_eq!(song.file, "music/Band/Album/01 Song.flac");
        assert_eq!(song.id, Some(3));
        assert_eq!(song.albumartist.as_deref(), Some("Band"));
        assert_eq!(song.time, Some(200));
    }

    #[test]
    fn parse_playlist_multiple_items() {
        let raw = concat!(
            "file: a/one.flac\nId: 0\nTitle: One\nArtist: A\n",
            "file: a/two.flac\nId: 1\nTitle: Two\nArtist: A\n",
            "file: b/three.flac\nId: 2\nTitle: Three\nArtist: B\n",
            "OK\n",
        );
        let r = parse_text(raw);
        let songs = parse_song_list(&r);
        assert_eq!(songs.len(), 3);
        assert_eq!(songs[0].id, Some(0));
        assert_eq!(songs[2].artist.as_deref(), Some("B"));
    }

    #[test]
    fn parse_lsinfo_per_directory_counts_and_computed_totals() {
        let raw = concat!(
            "directory: /Music/Band\n",
            "songcount: 10\n",
            "playtime: 2400\n",
            "directory: /Music/Other\n",
            "songcount: 5\n",
            "playtime: 1200\n",
            "file: /Music/loose.flac\n",
            "Title: Loose\n",
            "Artist: X\n",
            "Time: 60\n",
            "OK\n",
        );
        let r = parse_text(raw);
        let b = parse_lsinfo(&r);
        assert_eq!(b.directories.len(), 2);
        assert_eq!(b.directories[0].path, "/Music/Band");
        assert_eq!(b.directories[0].songcount, Some(10));
        assert_eq!(b.directories[0].playtime, Some(2400));
        assert_eq!(b.directories[1].path, "/Music/Other");
        assert_eq!(b.files.len(), 1);
        assert_eq!(b.files[0].path, "/Music/loose.flac");
        assert_eq!(
            b.files[0].song.as_ref().unwrap().title.as_deref(),
            Some("Loose")
        );
        assert_eq!(b.songcount, Some(16)); // 10 + 5 + 1 loose file
        assert_eq!(b.playtime, Some(3660)); // 2400 + 1200 + 60
    }

    /// MPD 0.23.x `lsinfo` does not emit `songcount`/`playtime` for
    /// directories (only `directory:` + `Last-Modified:`). The listing totals
    /// must then be `None`, not a misleading partial count of loose files.
    #[test]
    fn parse_lsinfo_totals_none_when_directories_lack_counts() {
        let raw = concat!(
            "directory: Band\n",
            "Last-Modified: 1700000000\n",
            "directory: Other\n",
            "Last-Modified: 1700000001\n",
            "file: loose.flac\n",
            "Title: Loose\n",
            "Artist: X\n",
            "Time: 60\n",
            "OK\n",
        );
        let r = parse_text(raw);
        let b = parse_lsinfo(&r);
        assert_eq!(b.directories.len(), 2);
        assert_eq!(b.directories[0].songcount, None);
        assert_eq!(b.directories[0].playtime, None);
        assert_eq!(b.files.len(), 1);
        assert_eq!(
            b.songcount, None,
            "total songs unknown when dirs lack counts"
        );
        assert_eq!(
            b.playtime, None,
            "total playtime unknown when dirs lack counts"
        );
    }

    #[test]
    fn parse_lsinfo_totals_known_for_only_loose_files() {
        // With no directories, the totals are exactly the loose files.
        let raw = "file: a.flac\nTime: 60\nfile: b.flac\nTime: 30\nOK\n";
        let b = parse_lsinfo(&parse_text(raw));
        assert_eq!(b.directories.len(), 0);
        assert_eq!(b.files.len(), 2);
        assert_eq!(b.songcount, Some(2));
        assert_eq!(b.playtime, Some(90));
    }

    #[test]
    fn escape_filter_value_orders_backslash_first() {
        // A value with both a backslash and a quote must escape the backslash
        // first so we don't double-escape.
        assert_eq!(escape_filter_value("a\\b"), "a\\\\b");
        assert_eq!(escape_filter_value("it's"), "it\\'s");
        assert_eq!(escape_filter_value("a\\'b"), "a\\\\\\'b");
    }

    #[test]
    fn quote_arg_escapes_quotes() {
        assert_eq!(quote_arg("plain"), "\"plain\"");
        assert_eq!(quote_arg("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote_arg("a\\b"), "\"a\\\\b\"");
    }

    #[test]
    fn build_search_filters_wraps_compound_and_in_parens() {
        // A single non-empty clause is sent bare (MPD accepts `(Tag op 'v')`).
        assert_eq!(
            build_search_filters(&[("Artist", "AC/DC"), ("Album", "")], "=="),
            "(Artist == 'AC/DC')"
        );
        // Two or more clauses must be wrapped: `((A) AND (B))`. A bare
        // `(A) AND (B)` is rejected by MPD with "Unparsed garbage".
        assert_eq!(
            build_search_filters(&[("Artist", "A"), ("Album", "B")], "contains"),
            "((Artist contains 'A') AND (Album contains 'B'))"
        );
        // Empty values are skipped entirely.
        assert_eq!(
            build_search_filters(&[("Artist", ""), ("Album", "")], "=="),
            ""
        );
    }

    #[test]
    fn parse_art_extracts_binary() {
        let payload: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4];
        let mut raw = Vec::new();
        raw.extend_from_slice(b"size: 8\ntype: image/jpeg\nbinary: 8\n");
        raw.extend_from_slice(&payload);
        raw.extend_from_slice(b"\nOK\n");
        let art = parse_art(&raw).expect("art");
        assert_eq!(art.total, 8);
        assert_eq!(art.mime.as_deref(), Some("image/jpeg"));
        assert_eq!(art.data, payload);
    }

    #[test]
    fn sniff_mime_detects_png_and_jpeg() {
        assert_eq!(
            sniff_mime(&[0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 0]),
            "image/png"
        );
        assert_eq!(sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE1]), "image/jpeg");
        assert_eq!(sniff_mime(&b"RIFF\x00\x00\x00\x00WEBP"[..]), "image/webp");
    }

    #[test]
    fn group_items_with_no_item_start_is_all_flat() {
        let fields = vec![
            ("artist".to_string(), "A".to_string()),
            ("artist".to_string(), "B".to_string()),
        ];
        let (items, flat) = group_items(&fields, &[], &[]);
        assert!(items.is_empty());
        assert_eq!(flat.len(), 2);
    }
}
