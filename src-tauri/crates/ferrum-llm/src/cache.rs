use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

pub struct ExactCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
    max_entries: usize,
    ttl_seconds: u64,
    total_lookups: std::sync::atomic::AtomicU64,
}

#[derive(Clone)]
struct CacheEntry {
    response: String,
    created_at: u64,
    hit_count: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CacheStats {
    pub entries: usize,
    pub total_hits: u32,
    pub hit_rate: f64,
}

impl ExactCache {
    pub fn new(max_entries: usize, ttl_seconds: u64) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_entries,
            ttl_seconds,
            total_lookups: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn lookup(&self, prompt_hash: &str) -> Option<String> {
        self.total_lookups.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut entries = self.entries.lock().ok()?;
        let now = now_secs();
        if let Some(entry) = entries.get_mut(prompt_hash) {
            if now - entry.created_at < self.ttl_seconds {
                entry.hit_count += 1;
                return Some(entry.response.clone());
            }
            entries.remove(prompt_hash);
        }
        None
    }

    pub fn store(&self, prompt_hash: String, response: String) {
        if let Ok(mut entries) = self.entries.lock() {
            if entries.len() >= self.max_entries {
                // LRU eviction
                let now = now_secs();
                entries.retain(|_, v| now - v.created_at < self.ttl_seconds);
                if entries.len() >= self.max_entries {
                    if let Some(key) = entries
                        .iter()
                        .min_by_key(|(_, v)| v.created_at)
                        .map(|(k, _)| k.clone())
                    {
                        entries.remove(&key);
                    }
                }
            }
            entries.insert(prompt_hash, CacheEntry {
                response,
                created_at: now_secs(),
                hit_count: 0,
            });
        }
    }

    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }

    pub fn stats(&self) -> CacheStats {
        // Single lock acquisition for consistent snapshot
        let (entries, total_hits) = self.entries.lock()
            .map(|e| (e.len(), e.values().map(|v| v.hit_count).sum::<u32>()))
            .unwrap_or((0, 0));
        let total_lookups = self.total_lookups.load(std::sync::atomic::Ordering::Relaxed);
        let hit_rate = if total_lookups > 0 { total_hits as f64 / total_lookups as f64 } else { 0.0 };
        CacheStats { entries, total_hits, hit_rate }
    }

    pub fn hash_prompt(messages_json: &str, model: &str, temperature: f64) -> String {
        let mut hasher = Sha256::new();
        hasher.update(model.as_bytes());
        hasher.update(format!("{:.2}", temperature).as_bytes());
        hasher.update(messages_json.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_and_lookup() {
        let cache = ExactCache::new(10, 3600);
        cache.store("key1".to_string(), "response1".to_string());
        assert_eq!(cache.lookup("key1"), Some("response1".to_string()));
    }

    #[test]
    fn lookup_missing_key() {
        let cache = ExactCache::new(10, 3600);
        assert_eq!(cache.lookup("nonexistent"), None);
    }

    #[test]
    fn lookup_increments_hit_count() {
        let cache = ExactCache::new(10, 3600);
        cache.store("key1".to_string(), "resp".to_string());
        cache.lookup("key1");
        cache.lookup("key1");
        cache.lookup("key1");
        let stats = cache.stats();
        assert_eq!(stats.total_hits, 3);
    }

    #[test]
    fn clear_removes_all() {
        let cache = ExactCache::new(10, 3600);
        cache.store("a".to_string(), "1".to_string());
        cache.store("b".to_string(), "2".to_string());
        assert_eq!(cache.stats().entries, 2);
        cache.clear();
        assert_eq!(cache.stats().entries, 0);
        assert_eq!(cache.lookup("a"), None);
    }

    #[test]
    fn eviction_when_full() {
        let cache = ExactCache::new(2, 3600);
        cache.store("a".to_string(), "1".to_string());
        cache.store("b".to_string(), "2".to_string());
        // This should evict the oldest entry
        cache.store("c".to_string(), "3".to_string());
        assert_eq!(cache.stats().entries, 2);
        // "a" was oldest, should be evicted
        assert_eq!(cache.lookup("a"), None);
        assert_eq!(cache.lookup("c"), Some("3".to_string()));
    }

    #[test]
    fn stats_empty_cache() {
        let cache = ExactCache::new(10, 3600);
        let stats = cache.stats();
        assert_eq!(stats.entries, 0);
        assert_eq!(stats.total_hits, 0);
        assert!((stats.hit_rate - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn stats_with_entries() {
        let cache = ExactCache::new(10, 3600);
        cache.store("a".to_string(), "1".to_string());
        cache.store("b".to_string(), "2".to_string());
        let stats = cache.stats();
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.total_hits, 0);
    }

    #[test]
    fn hash_prompt_deterministic() {
        let h1 = ExactCache::hash_prompt("[{\"role\":\"user\"}]", "gpt-4", 0.7);
        let h2 = ExactCache::hash_prompt("[{\"role\":\"user\"}]", "gpt-4", 0.7);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_prompt_different_models() {
        let h1 = ExactCache::hash_prompt("msg", "gpt-4", 0.7);
        let h2 = ExactCache::hash_prompt("msg", "gpt-3.5", 0.7);
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_prompt_different_temperature() {
        let h1 = ExactCache::hash_prompt("msg", "gpt-4", 0.7);
        let h2 = ExactCache::hash_prompt("msg", "gpt-4", 0.9);
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_prompt_different_messages() {
        let h1 = ExactCache::hash_prompt("hello", "gpt-4", 0.7);
        let h2 = ExactCache::hash_prompt("world", "gpt-4", 0.7);
        assert_ne!(h1, h2);
    }

    #[test]
    fn overwrite_existing_key() {
        let cache = ExactCache::new(10, 3600);
        cache.store("k".to_string(), "old".to_string());
        cache.store("k".to_string(), "new".to_string());
        assert_eq!(cache.lookup("k"), Some("new".to_string()));
        assert_eq!(cache.stats().entries, 1);
    }
}
