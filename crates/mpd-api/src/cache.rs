//! Small in-memory TTL cache for proxied album art.
//!
//! Entries are evicted on TTL expiry or, once the entry cap is reached, least
//! recently used first — a library's distinct album count is small, so the cap
//! mostly guards against pathological churn.

use std::collections::{HashMap, VecDeque};
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

/// Entry cap; beyond it the least recently used entries are evicted.
const MAX_ENTRIES: usize = 512;
const ART_TTL: Duration = Duration::from_secs(3600);

pub struct Cache {
    entries: Mutex<Entries>,
}

/// LRU state: the entries plus their access order (back = most recently used).
#[derive(Default)]
struct Entries {
    map: HashMap<String, Entry>,
    order: VecDeque<String>,
}

impl Entries {
    /// Move `key` to the most-recently-used end.
    fn touch(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key.to_string());
    }

    /// Remove `key` from both the map and the order list.
    fn remove(&mut self, key: &str) {
        if self.map.remove(key).is_some() {
            if let Some(pos) = self.order.iter().position(|k| k == key) {
                self.order.remove(pos);
            }
        }
    }

    /// Evict least recently used entries until the cap is met.
    fn evict_to_cap(&mut self) {
        while self.order.len() > MAX_ENTRIES {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(Entries::default()),
        }
    }
}

impl Cache {
    pub fn new() -> Self {
        Self::default()
    }

    /// A cached art hit, or `None` when absent or expired.
    pub fn get_art(&self, key: &str) -> Option<CacheHit> {
        let mut entries = self.entries.lock().expect("cache lock poisoned");
        let entry = entries.map.get(key)?;
        if Instant::now() >= entry.expires {
            entries.remove(key);
            return None;
        }
        let hit = CacheHit {
            value: entry.value.clone(),
            etag: entry.etag.clone(),
            mime: entry.mime.clone(),
        };
        entries.touch(key);
        Some(hit)
    }

    /// Store art under `key`, evicting least recently used entries past the
    /// cap when the key is new.
    pub fn store_art(&self, key: &str, value: Bytes, mime: &str) -> CacheHit {
        let etag = etag_for(&value);
        let hit = CacheHit {
            value: value.clone(),
            etag: etag.clone(),
            mime: Some(mime.to_string()),
        };
        let mut entries = self.entries.lock().expect("cache lock poisoned");
        if entries.map.contains_key(key) {
            entries.touch(key);
        } else {
            entries.order.push_back(key.to_string());
            entries.evict_to_cap();
        }
        entries.map.insert(
            key.to_string(),
            Entry {
                value,
                etag,
                mime: Some(mime.to_string()),
                expires: Instant::now() + ART_TTL,
            },
        );
        hit
    }

    /// Drop every entry (used when the MPD database changes).
    pub fn clear(&self) {
        let mut entries = self.entries.lock().expect("cache lock poisoned");
        entries.map.clear();
        entries.order.clear();
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_returns_art_hit_with_mime() {
        let cache = Cache::new();
        let hit = cache.store_art("art", Bytes::copy_from_slice(&[1, 2, 3]), "image/png");
        assert_eq!(hit.mime.as_deref(), Some("image/png"));
        let got = cache.get_art("art").expect("hit");
        assert_eq!(got.value, Bytes::copy_from_slice(&[1, 2, 3]));
        assert_eq!(got.etag, hit.etag);
        assert_eq!(got.mime.as_deref(), Some("image/png"));
    }

    #[test]
    fn expires_entries_after_ttl() {
        let cache = Cache::new();
        cache.store_art("k", Bytes::copy_from_slice(&[1]), "image/png");
        // Force expiry by rewriting the entry with a past deadline.
        let mut entries = cache.entries.lock().expect("cache lock poisoned");
        let entry = entries.map.get_mut("k").expect("entry present");
        entry.expires = Instant::now() - Duration::from_secs(1);
        drop(entries);
        assert!(cache.get_art("k").is_none());
    }

    #[test]
    fn clear_drops_all_entries() {
        let cache = Cache::new();
        cache.store_art("a", Bytes::copy_from_slice(&[1]), "image/png");
        cache.store_art("b", Bytes::copy_from_slice(&[2]), "image/jpeg");
        cache.clear();
        assert!(cache.get_art("a").is_none());
        assert!(cache.get_art("b").is_none());
    }

    #[test]
    fn evicts_least_recently_used_past_the_cap() {
        let cache = Cache::new();
        for i in 0..MAX_ENTRIES {
            cache.store_art(&i.to_string(), Bytes::from("v"), "image/png");
        }
        // Touch the first entry so "1" becomes the least recently used.
        assert!(cache.get_art("0").is_some());
        cache.store_art(&MAX_ENTRIES.to_string(), Bytes::from("v"), "image/png");
        assert!(cache.get_art("1").is_none(), "LRU entry must be evicted");
        assert!(cache.get_art("0").is_some());
        assert!(cache.get_art(&MAX_ENTRIES.to_string()).is_some());
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
