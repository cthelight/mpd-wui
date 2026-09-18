//! A stale-while-revalidate, in-process copy of the whole MPD library, used
//! for local search.
//!
//! The store keeps a single `Arc<LibraryIndex>` snapshot: a precomputed search
//! index (see [`crate::search::LibraryIndex`]). Reads are cheap `Arc` clones;
//! the expensive part (a full library dump over the wire) happens at most once
//! per TTL window and is single-flighted. After the very first load, an
//! expired snapshot is served *immediately* and refreshed in the background,
//! so a search request is never blocked on the MPD round-trip.

use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use mpd_client::MpdClient;

use crate::search::LibraryIndex;

#[derive(Clone)]
struct Entry {
    index: Arc<LibraryIndex>,
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

    /// Return the library snapshot. A fresh snapshot is served from memory; a
    /// stale one is served *and* refreshed in the background; the first load
    /// (no stale data to serve) waits for the dump.
    pub async fn get(self: &Arc<Self>, client: &MpdClient) -> anyhow::Result<Arc<LibraryIndex>> {
        if let Some(entry) = self.current() {
            if entry.loaded_at.elapsed() < self.ttl {
                return Ok(entry.index);
            }
            self.refresh_in_background(client);
            return Ok(entry.index);
        }
        // First load: no stale data, so wait for the dump (single-flighted).
        let _guard = self.loading.lock().await;
        if let Some(entry) = self.current() {
            // Populated (and its own refresh, if any, kicked off) while we
            // waited for the guard.
            return Ok(entry.index);
        }
        let songs = client.library().await?;
        let index = Arc::new(LibraryIndex::build(songs));
        self.store(index.clone());
        Ok(index)
    }

    /// Drop the cached snapshot (called when the MPD database changes).
    pub fn invalidate(&self) {
        *self.data.write().expect("library store poisoned") = None;
    }

    /// Re-dump the library in the background, single-flighted. Failures keep
    /// the stale snapshot (logging the error) so search stays available.
    fn refresh_in_background(self: &Arc<Self>, client: &MpdClient) {
        let this = self.clone();
        let client = client.clone();
        tokio::spawn(async move {
            let _guard = this.loading.lock().await;
            if let Some(entry) = this.current() {
                if entry.loaded_at.elapsed() < this.ttl {
                    return; // Refreshed while we waited for the guard.
                }
            }
            match client.library().await {
                Ok(songs) => this.store(Arc::new(LibraryIndex::build(songs))),
                Err(error) => {
                    tracing::warn!(%error, "library refresh failed; keeping stale snapshot")
                }
            }
        });
    }

    fn current(&self) -> Option<Entry> {
        self.data.read().expect("library store poisoned").clone()
    }

    fn store(&self, index: Arc<LibraryIndex>) {
        *self.data.write().expect("library store poisoned") = Some(Entry {
            index,
            loaded_at: Instant::now(),
        });
    }
}

#[cfg(test)]
use mpd_client::Song;

#[cfg(test)]
impl LibraryStore {
    /// Seed the snapshot directly (tests only; bypasses the network).
    fn prime(&self, songs: Vec<Song>) {
        self.store(Arc::new(LibraryIndex::build(songs)));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn song(file: &str) -> Song {
        Song {
            file: file.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn snapshot_is_present_and_fresh_when_not_expired() {
        let store = LibraryStore::new(Duration::from_secs(60));
        store.prime(vec![song("a.flac"), song("b.flac")]);
        let entry = store.current().expect("snapshot present");
        assert_eq!(entry.index.len(), 2);
        assert!(entry.loaded_at.elapsed() < store.ttl);
        // A second read sees the same underlying data.
        assert_eq!(store.current().unwrap().index.len(), 2);
    }

    #[test]
    fn snapshot_is_stale_after_the_ttl() {
        let store = LibraryStore::new(Duration::from_millis(1));
        store.prime(vec![song("a.flac")]);
        std::thread::sleep(Duration::from_millis(5));
        let entry = store.current().expect("snapshot still present");
        assert!(entry.loaded_at.elapsed() >= store.ttl);
    }

    #[test]
    fn invalidate_drops_snapshot() {
        let store = LibraryStore::new(Duration::from_secs(60));
        store.prime(vec![song("a.flac")]);
        assert!(store.current().is_some());
        store.invalidate();
        assert!(store.current().is_none());
    }
}
