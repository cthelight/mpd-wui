//! Request bodies for the JSON API.

use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct PlayReq {
    pub position: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PauseReq {
    pub state: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SeekReq {
    pub time: f32,
}

#[derive(Debug, Deserialize)]
pub struct VolumeReq {
    pub value: u32,
}

#[derive(Debug, Deserialize, Default)]
pub struct OptionsReq {
    pub random: Option<bool>,
    pub repeat: Option<bool>,
    pub single: Option<bool>,
    pub consume: Option<bool>,
}

/// A queue add target. The frontend sends exactly one key per object:
/// `{"path": ...}` for files/folders, or a collection tag such as
/// `{"artist": ...}`, `{"album": ...}`, etc.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum QueueTarget {
    Path { path: String },
    Artist { artist: String },
    Album { album: String },
    AlbumArtist { albumartist: String },
    Genre { genre: String },
    Date { date: String },
}

#[derive(Debug, Deserialize)]
pub struct AddReq {
    pub targets: Vec<QueueTarget>,
    #[serde(default)]
    pub play: bool,
}

#[derive(Debug, Deserialize)]
pub struct RemoveReq {
    pub ids: Vec<u32>,
}

#[derive(Debug, Deserialize)]
pub struct MoveReq {
    pub id: u32,
    pub to: u32,
}
