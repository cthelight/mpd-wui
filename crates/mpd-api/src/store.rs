//! A TTL-cached, in-process copy of the whole MPD library, used for local
//! search.
//!
//! The store keeps a single `Arc<Vec<Song>>` snapshot. Reads are cheap clones
//! of the `Arc`; the expensive part (a full `search (File contains '')` dump
//! over the wire) happens at most once per TTL window and is single-flighted so
//! concurrent misses don't stampede the server.

use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use mpd_client::{MpdClient, Song};

struct Entry {
    songs: Arc<Vec<Song>>,
    loaded_at: Instant,
}

/// In-process library cache.
pub struct LibraryStore {
    ttl: Duration,
    /// Never held across an `.await`: the snapshot is swapped in only after the
    /// network round-trip completes.
    data: RwLock<Option<Entry>>,
    /// Single-flight guard so only one task performs the dump at a time.
    loading: tokio::sync::Mutex<()>,
}

impl LibraryStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            data: RwLock::new(None),
            loading: tokio::sync::Mutex::new(()),
        }
    }

    /// Return the library snapshot, fetching (and refreshing) it if the cached
    /// copy is missing or older than the TTL.
    pub async fn get(&self, client: &MpdClient) -> anyhow::Result<Arc<Vec<Song>>> {
        if let Some(fresh) = self.fresh() {
            return Ok(fresh);
        }
        let _guard = self.loading.lock().await;
        // Re-check under the guard: another task may have just populated it.
        if let Some(fresh) = self.fresh() {
            return Ok(fresh);
        }
        let songs = client.library().await?;
        let snapshot = Arc::new(songs);
        *self.data.write().expect("library store poisoned") = Some(Entry {
            songs: snapshot.clone(),
            loaded_at: Instant::now(),
        });
        Ok(snapshot)
    }

    /// Drop the cached snapshot (called when the MPD database changes).
    pub fn invalidate(&self) {
        *self.data.write().expect("library store poisoned") = None;
    }

    fn fresh(&self) -> Option<Arc<Vec<Song>>> {
        let guard = self.data.read().expect("library store poisoned");
        let entry = guard.as_ref()?;
        if entry.loaded_at.elapsed() < self.ttl {
            Some(entry.songs.clone())
        } else {
            None
        }
    }
}

#[cfg(test)]
impl LibraryStore {
    /// Seed the snapshot directly (tests only; bypasses the network).
    fn prime(&self, songs: Vec<Song>) {
        *self.data.write().expect("library store poisoned") = Some(Entry {
            songs: Arc::new(songs),
            loaded_at: Instant::now(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(file: &str) -> Song {
        Song {
            file: file.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn fresh_returns_cloned_snapshot_when_not_expired() {
        let store = LibraryStore::new(Duration::from_secs(60));
        store.prime(vec![song("a.flac"), song("b.flac")]);
        let got = store.fresh().expect("should be fresh");
        assert_eq!(got.len(), 2);
        // A fresh read returns the same underlying data.
        assert_eq!(store.fresh().unwrap()[0].file, "a.flac");
    }

    #[test]
    fn fresh_is_none_when_expired() {
        let store = LibraryStore::new(Duration::from_millis(1));
        store.prime(vec![song("a.flac")]);
        std::thread::sleep(Duration::from_millis(5));
        assert!(store.fresh().is_none());
    }

    #[test]
    fn invalidate_drops_snapshot() {
        let store = LibraryStore::new(Duration::from_secs(60));
        store.prime(vec![song("a.flac")]);
        assert!(store.fresh().is_some());
        store.invalidate();
        assert!(store.fresh().is_none());
    }
}
