use turbo_pix::cache::*;

#[test]
fn test_thumbnail_size_conversions() {
    assert_eq!(ThumbnailSize::Small.to_pixels(), 200);
    assert_eq!(ThumbnailSize::Medium.to_pixels(), 400);
    assert_eq!(ThumbnailSize::Large.to_pixels(), 800);

    assert_eq!(ThumbnailSize::Small.as_str(), "small");
    assert_eq!(ThumbnailSize::Medium.as_str(), "medium");
    assert_eq!(ThumbnailSize::Large.as_str(), "large");

    assert_eq!("small".parse::<ThumbnailSize>(), Ok(ThumbnailSize::Small));
    assert_eq!("medium".parse::<ThumbnailSize>(), Ok(ThumbnailSize::Medium));
    assert_eq!("large".parse::<ThumbnailSize>(), Ok(ThumbnailSize::Large));
    assert_eq!("invalid".parse::<ThumbnailSize>(), Err(()));
}

#[test]
fn test_thumbnail_size_display() {
    assert_eq!(format!("{}", ThumbnailSize::Small), "small");
    assert_eq!(format!("{}", ThumbnailSize::Medium), "medium");
    assert_eq!(format!("{}", ThumbnailSize::Large), "large");
}

#[test]
fn test_cache_key() {
    let key = CacheKey::new(123, ThumbnailSize::Medium);
    assert_eq!(key.photo_id, 123);
    assert_eq!(key.size, ThumbnailSize::Medium);
    assert_eq!(format!("{}", key), "123_medium");
}

#[test]
fn test_cache_key_equality() {
    let key1 = CacheKey::new(1, ThumbnailSize::Small);
    let key2 = CacheKey::new(1, ThumbnailSize::Small);
    let key3 = CacheKey::new(1, ThumbnailSize::Medium);
    let key4 = CacheKey::new(2, ThumbnailSize::Small);

    assert_eq!(key1, key2);
    assert_ne!(key1, key3);
    assert_ne!(key1, key4);
}

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
