use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use tracing::debug;

use super::{CacheKey, CacheResult};

#[derive(Clone)]
pub struct MemoryCache {
    cache: Arc<Mutex<LruCache<String, Vec<u8>>>>,
    max_size_bytes: usize,
    current_size: Arc<Mutex<usize>>,
}

impl MemoryCache {
    pub fn new(capacity: usize, max_size_mb: usize) -> Self {
        let capacity = NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(1000).unwrap());

        Self {
            cache: Arc::new(Mutex::new(LruCache::new(capacity))),
            max_size_bytes: max_size_mb * 1024 * 1024, // Convert MB to bytes
            current_size: Arc::new(Mutex::new(0)),
        }
    }

    pub fn get(&self, key: &CacheKey) -> Option<Vec<u8>> {
        let key_str = key.to_string();

        if let Ok(mut cache) = self.cache.lock() {
            if let Some(data) = cache.get(&key_str) {
                debug!("Memory cache hit for {}", key_str);
                return Some(data.clone());
            }
        }

        debug!("Memory cache miss for {}", key_str);
        None
    }

    pub fn put(&self, key: &CacheKey, data: Vec<u8>) -> CacheResult<()> {
        let key_str = key.to_string();
        let data_size = data.len();

        // Check if this single item would exceed our size limit
        if data_size > self.max_size_bytes {
            debug!("Item too large for memory cache: {} bytes", data_size);
            return Ok(());
        }

        // Acquire locks
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| std::io::Error::other("Failed to acquire cache lock"))?;

        let mut current_size = self
            .current_size
            .lock()
            .map_err(|_| std::io::Error::other("Failed to acquire size lock"))?;

        // Make room if needed
        while *current_size + data_size > self.max_size_bytes && !cache.is_empty() {
            if let Some((_, removed_data)) = cache.pop_lru() {
                *current_size = current_size.saturating_sub(removed_data.len());
                debug!(
                    "Evicted item from memory cache, new size: {} bytes",
                    *current_size
                );
            } else {
                break;
            }
        }

        // Add the new item
        if let Some(old_data) = cache.put(key_str.clone(), data) {
            // Replace existing item
            *current_size = current_size.saturating_sub(old_data.len()) + data_size;
        } else {
            // New item
            *current_size += data_size;
        }

        debug!(
            "Added {} to memory cache, total size: {} bytes",
            key_str, *current_size
        );
        Ok(())
    }

    #[allow(dead_code)]
    pub fn remove(&self, key: &CacheKey) -> Option<Vec<u8>> {
        let key_str = key.to_string();

        if let (Ok(mut cache), Ok(mut current_size)) = (self.cache.lock(), self.current_size.lock())
        {
            if let Some(data) = cache.pop(&key_str) {
                *current_size = current_size.saturating_sub(data.len());
                debug!("Removed {} from memory cache", key_str);
                return Some(data);
            }
        }

        None
    }

    pub fn clear(&self) {
        if let (Ok(mut cache), Ok(mut current_size)) = (self.cache.lock(), self.current_size.lock())
        {
            cache.clear();
            *current_size = 0;
            debug!("Cleared memory cache");
        }
    }

    pub fn stats(&self) -> (usize, usize, usize) {
        if let (Ok(cache), Ok(current_size)) = (self.cache.lock(), self.current_size.lock()) {
            let len = cache.len();
            let cap = cache.cap().get();
            let size = *current_size;

            (len, cap, size)
        } else {
            (0, 0, 0)
        }
    }

    pub fn hit_rate(&self) -> f64 {
        // This is a simplified implementation
        // In a real system, you'd track hits/misses over time
        if let Ok(cache) = self.cache.lock() {
            if cache.cap().get() > 0 {
                cache.len() as f64 / cache.cap().get() as f64
            } else {
                0.0
            }
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::ThumbnailSize;

    #[test]
    fn test_memory_cache_basic_operations() {
        let cache = MemoryCache::new(10, 1); // 10 items, 1MB
        let key = CacheKey::new(1, ThumbnailSize::Small);
        let data = vec![1, 2, 3, 4, 5];

        // Initially empty
        assert!(cache.get(&key).is_none());

        // Put and get
        cache.put(&key, data.clone()).unwrap();
        assert_eq!(cache.get(&key), Some(data));

        // Stats
        let (len, cap, size) = cache.stats();
        assert_eq!(len, 1);
        assert_eq!(cap, 10);
        assert_eq!(size, 5);
    }

    #[test]
    fn test_memory_cache_eviction() {
        let cache = MemoryCache::new(2, 1); // 2 items max
        let key1 = CacheKey::new(1, ThumbnailSize::Small);
        let key2 = CacheKey::new(2, ThumbnailSize::Small);
        let key3 = CacheKey::new(3, ThumbnailSize::Small);
        let data = vec![0; 100]; // 100 bytes each

        // Fill cache
        cache.put(&key1, data.clone()).unwrap();
        cache.put(&key2, data.clone()).unwrap();

        // Both should be present
        assert!(cache.get(&key1).is_some());
        assert!(cache.get(&key2).is_some());

        // Add third item, should evict first (LRU)
        cache.put(&key3, data.clone()).unwrap();

        // key1 should be evicted, key2 and key3 should be present
        assert!(cache.get(&key1).is_none());
        assert!(cache.get(&key2).is_some());
        assert!(cache.get(&key3).is_some());
    }

    #[test]
    fn test_memory_cache_size_limit() {
        let cache = MemoryCache::new(10, 1); // 1MB = 1,048,576 bytes
        let key = CacheKey::new(1, ThumbnailSize::Large);
        let large_data = vec![0; 2 * 1024 * 1024]; // 2MB data (too large)
        let small_data = vec![0; 1000]; // 1KB data

        // Large data should be rejected
        cache.put(&key, large_data).unwrap();
        assert!(cache.get(&key).is_none());

        // Small data should work
        cache.put(&key, small_data.clone()).unwrap();
        assert_eq!(cache.get(&key), Some(small_data));
    }

    #[test]
    fn test_memory_cache_remove() {
        let cache = MemoryCache::new(10, 1);
        let key = CacheKey::new(1, ThumbnailSize::Medium);
        let data = vec![1, 2, 3];

        cache.put(&key, data.clone()).unwrap();
        assert_eq!(cache.get(&key), Some(data.clone()));

        let removed = cache.remove(&key);
        assert_eq!(removed, Some(data));
        assert!(cache.get(&key).is_none());

        let (len, _, size) = cache.stats();
        assert_eq!(len, 0);
        assert_eq!(size, 0);
    }

    #[test]
    fn test_memory_cache_clear() {
        let cache = MemoryCache::new(10, 1);
        let key1 = CacheKey::new(1, ThumbnailSize::Small);
        let key2 = CacheKey::new(2, ThumbnailSize::Medium);
        let data = vec![1, 2, 3];

        cache.put(&key1, data.clone()).unwrap();
        cache.put(&key2, data.clone()).unwrap();

        let (len, _, size) = cache.stats();
        assert_eq!(len, 2);
        assert!(size > 0);

        cache.clear();

        let (len, _, size) = cache.stats();
        assert_eq!(len, 0);
        assert_eq!(size, 0);
        assert!(cache.get(&key1).is_none());
        assert!(cache.get(&key2).is_none());
    }

    #[test]
    fn test_memory_cache_replace_existing() {
        let cache = MemoryCache::new(10, 1);
        let key = CacheKey::new(1, ThumbnailSize::Small);
        let data1 = vec![1, 2, 3];
        let data2 = vec![4, 5, 6, 7];

        cache.put(&key, data1).unwrap();
        cache.put(&key, data2.clone()).unwrap();

        assert_eq!(cache.get(&key), Some(data2));

        let (len, _, size) = cache.stats();
        assert_eq!(len, 1);
        assert_eq!(size, 4); // Should be size of second data
    }
}
