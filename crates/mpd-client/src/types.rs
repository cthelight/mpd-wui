//! Typed data for the MPD protocol, serializable for the API layer.

use serde::{Deserialize, Serialize};

/// Playback state of the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PlayState {
    #[default]
    Stop,
    Pause,
    Play,
}

impl From<&str> for PlayState {
    fn from(s: &str) -> Self {
        match s {
            "play" => PlayState::Play,
            "pause" => PlayState::Pause,
            _ => PlayState::Stop,
        }
    }
}

/// Player status (the `status` command, minus the current song).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Status {
    pub state: PlayState,
    /// Total track length in seconds.
    pub time: u32,
    /// Position in the current track, seconds (float from MPD).
    pub elapsed: f32,
    /// Volume 0..100 (may exceed with MPD > 100).
    pub volume: u32,
    pub random: bool,
    pub repeat: bool,
    pub single: bool,
    pub consume: bool,
    pub crossfade: u32,
    pub playlist_version: u32,
    /// Number of songs currently in the playlist.
    pub songs: u32,
    /// Index of the current song in the playlist (0-based). MPD omits the
    /// field when nothing is playing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<u32>,
    /// True while the database is updating.
    pub updating: bool,
}

/// A song / track, whether from the database, the playlist or `currentsong`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Song {
    /// Path relative to MPD's music root (the `file` field).
    pub file: String,
    /// Playlist id, present when the song is part of the playlist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub albumartist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Track length in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<u32>,
}

impl Song {
    /// Best human-readable title (falls back to the file name).
    pub fn display_title(&self) -> String {
        if let Some(t) = &self.title {
            if !t.is_empty() {
                return t.clone();
            }
        }
        self.file
            .rsplit('/')
            .next()
            .unwrap_or(&self.file)
            .to_string()
    }

    /// Best human-readable artist (falls back to "Unknown Artist").
    pub fn display_artist(&self) -> String {
        self.artist
            .clone()
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| "Unknown Artist".to_string())
    }
}

/// An entry in an `lsinfo` listing: either a directory or a file (song).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    /// Path relative to the music root.
    pub path: String,
    pub is_dir: bool,
    /// Present when `is_dir` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<Song>,
    /// Song count under this directory (present when `is_dir`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub songcount: Option<u32>,
    /// Total playtime in seconds under this directory (present when `is_dir`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playtime: Option<u32>,
}

/// Result of an `lsinfo` browse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Browse {
    pub directories: Vec<DirEntry>,
    pub files: Vec<DirEntry>,
    /// Total songs under the browsed path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub songcount: Option<u32>,
    /// Total playtime (seconds) under the browsed path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playtime: Option<u32>,
}

/// Database statistics (the `stats` command).
///
/// Field names track MPD ≥ 0.23's `stats` reply: `db_playtime`, `songs`,
/// `albums`, `artists`, and `db_update`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbStats {
    /// Total playtime of the database, in seconds (wire: `db_playtime`).
    pub db_playtime: u64,
    /// Number of songs in the database (wire: `songs`).
    pub songs: u64,
    /// Number of albums in the database (wire: `albums`).
    pub albums: u64,
    /// Number of artists in the database (wire: `artists`).
    pub artists: u64,
    /// Last time the database was updated, as a Unix timestamp (wire: `db_update`).
    pub db_update: u64,
}

/// A point-in-time snapshot of the player, pushed over the WebSocket.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub status: Status,
    pub song: Option<Song>,
}

/// Events emitted by the client for the API / WebSocket layer to consume.
#[derive(Debug, Clone)]
pub enum MpdEvent {
    /// Player status / current song changed (a fresh snapshot follows).
    Snapshot(Box<Snapshot>),
    /// The database changed (library cache should be invalidated).
    DatabaseChanged,
    /// Lost the connection to MPD.
    Disconnected,
    /// (Re)established the connection to MPD.
    Reconnected,
}

/// The set of MPD commands the server reported (from the `commands` command).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// MPD version string from the greeting, e.g. "0.23.15".
    pub version: String,
    /// Commands the server supports.
    pub commands: Vec<String>,
    /// Commands the server explicitly does not support.
    pub not_commands: Vec<String>,
}

impl Capabilities {
    pub fn has(&self, cmd: &str) -> bool {
        self.commands.iter().any(|c| c == cmd)
    }
}
