use std::env;

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub thumbnail_cache_path: String,
    pub memory_cache_size: usize,
    pub memory_cache_max_size_mb: usize,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub port: u16,
    pub host: String,
    pub photo_paths: Vec<String>,
    pub db_path: String,
    pub cache_path: String,
    pub cache: CacheConfig,
    pub thumbnail_sizes: Vec<u32>,
    pub workers: usize,
    pub max_connections: u32,
    pub cache_size_mb: usize,
    pub scan_interval: u64,
    pub batch_size: usize,

    pub health_check_path: String,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let cache_path =
            env::var("TURBO_PIX_CACHE_PATH").unwrap_or_else(|_| "./data/cache".to_string());

        Ok(Config {
            port: env::var("TURBO_PIX_PORT")
                .unwrap_or_else(|_| "18473".to_string())
                .parse()?,
            host: env::var("TURBO_PIX_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            photo_paths: env::var("TURBO_PIX_PHOTO_PATHS")
                .unwrap_or_else(|_| "./photos".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            db_path: env::var("TURBO_PIX_DB_PATH")
                .unwrap_or_else(|_| "./data/database/turbo-pix.db".to_string()),
            cache_path: cache_path.clone(),
            cache: CacheConfig {
                thumbnail_cache_path: format!("{}/thumbnails", cache_path),
                memory_cache_size: env::var("TURBO_PIX_MEMORY_CACHE_SIZE")
                    .unwrap_or_else(|_| "1000".to_string())
                    .parse()?,
                memory_cache_max_size_mb: env::var("TURBO_PIX_MEMORY_CACHE_MAX_SIZE_MB")
                    .unwrap_or_else(|_| "100".to_string())
                    .parse()?,
            },
            thumbnail_sizes: env::var("TURBO_PIX_THUMBNAIL_SIZES")
                .unwrap_or_else(|_| "200,400,800".to_string())
                .split(',')
                .map(|s| s.trim().parse())
                .collect::<Result<Vec<_>, _>>()?,
            workers: env::var("TURBO_PIX_WORKERS")
                .unwrap_or_else(|_| "4".to_string())
                .parse()?,
            max_connections: env::var("TURBO_PIX_MAX_CONNECTIONS")
                .unwrap_or_else(|_| "100".to_string())
                .parse()?,
            cache_size_mb: env::var("TURBO_PIX_CACHE_SIZE_MB")
                .unwrap_or_else(|_| "512".to_string())
                .parse()?,
            scan_interval: env::var("TURBO_PIX_SCAN_INTERVAL")
                .unwrap_or_else(|_| "3600".to_string())
                .parse()?,
            batch_size: env::var("TURBO_PIX_BATCH_SIZE")
                .unwrap_or_else(|_| "1000".to_string())
                .parse()?,

            health_check_path: env::var("TURBO_PIX_HEALTH_CHECK_PATH")
                .unwrap_or_else(|_| "/health".to_string()),
        })
    }
}
