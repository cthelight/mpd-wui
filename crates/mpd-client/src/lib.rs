//! mpd-client: minimal async MPD protocol client.
//!
//! No HTTP knowledge here — just the MPD text protocol, connection management,
//! idle event streaming and binary (album art) reads.

mod client;
mod protocol;
mod types;

pub use client::{MpdClient, MpdConfig};
pub use types::{Browse, Capabilities, DirEntry, MpdEvent, PlayState, Snapshot, Song, Status};
