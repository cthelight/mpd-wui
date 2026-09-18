//! Small in-memory TTL cache for API responses and proxied album art.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::body::Bytes;

/// A cached value plus its validator.
#[derive(Debug, Clone)]
pub struct CacheHit {
    pub value: Bytes,
    pub etag: String,
    pub mime: Option<String>,
}

#[derive(Debug)]
struct Entry {
    value: Bytes,
    etag: String,
    mime: Option<String>,
    expires: Instant,
}

pub struct Cache {
    library_ttl: Duration,
    art_ttl: Duration,
    entries: Mutex<HashMap<String, Entry>>,
}

impl Cache {
    pub fn new(library_ttl: Duration) -> Self {
        Self {
            library_ttl,
            art_ttl: Duration::from_secs(3600),
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn get_library(&self, key: &str) -> Option<CacheHit> {
        self.get(key)
    }

    pub fn get_art(&self, key: &str) -> Option<CacheHit> {
        self.get(key)
    }

    pub fn store_library(&self, key: &str, value: Bytes) -> CacheHit {
        self.store(key, value, None, self.library_ttl)
    }

    pub fn store_art(&self, key: &str, value: Bytes, mime: &str) -> CacheHit {
        self.store(key, value, Some(mime.to_string()), self.art_ttl)
    }

    /// Drop every entry (used when the MPD database changes).
    pub fn clear(&self) {
        self.entries.lock().expect("cache lock poisoned").clear();
    }

    fn get(&self, key: &str) -> Option<CacheHit> {
        let mut entries = self.entries.lock().expect("cache lock poisoned");
        let entry = entries.get(key)?;
        if Instant::now() >= entry.expires {
            entries.remove(key);
            return None;
        }
        Some(CacheHit {
            value: entry.value.clone(),
            etag: entry.etag.clone(),
            mime: entry.mime.clone(),
        })
    }

    fn store(&self, key: &str, value: Bytes, mime: Option<String>, ttl: Duration) -> CacheHit {
        let etag = etag_for(&value);
        let hit = CacheHit {
            value: value.clone(),
            etag: etag.clone(),
            mime: mime.clone(),
        };
        self.entries.lock().expect("cache lock poisoned").insert(
            key.to_string(),
            Entry {
                value,
                etag,
                mime,
                expires: Instant::now() + ttl,
            },
        );
        hit
    }
}

/// A stable, cheap 64-bit FNV-1a digest formatted as a strong HTTP entity tag.
fn etag_for(bytes: &[u8]) -> String {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("\"{hash:016x}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_returns_library_hit() {
        let cache = Cache::new(Duration::from_secs(60));
        let hit = cache.store_library("k", Bytes::from("v"));
        assert_eq!(hit.value, Bytes::from("v"));
        let got = cache.get_library("k").expect("hit");
        assert_eq!(got.value, Bytes::from("v"));
        assert_eq!(got.etag, hit.etag);
        assert!(got.mime.is_none());
    }

    #[test]
    fn stores_and_returns_art_hit_with_mime() {
        let cache = Cache::new(Duration::from_secs(60));
        let hit = cache.store_art("art", Bytes::copy_from_slice(&[1, 2, 3]), "image/png");
        assert_eq!(hit.mime.as_deref(), Some("image/png"));
        let got = cache.get_art("art").expect("hit");
        assert_eq!(got.mime.as_deref(), Some("image/png"));
    }

    #[test]
    fn expires_entries_after_ttl() {
        let cache = Cache::new(Duration::from_millis(1));
        cache.store_library("k", Bytes::from("v"));
        std::thread::sleep(Duration::from_millis(5));
        assert!(cache.get_library("k").is_none());
    }

    #[test]
    fn clear_drops_all_entries() {
        let cache = Cache::new(Duration::from_secs(60));
        cache.store_library("a", Bytes::from("1"));
        cache.store_art("b", Bytes::copy_from_slice(&[1]), "image/png");
        cache.clear();
        assert!(cache.get_library("a").is_none());
        assert!(cache.get_art("b").is_none());
    }

    #[test]
    fn etag_is_stable_and_quoted() {
        let a = etag_for(b"hello");
        let b = etag_for(b"hello");
        let c = etag_for(b"world");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with('"'));
        assert!(a.ends_with('"'));
    }
}
