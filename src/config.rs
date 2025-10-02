use std::env;

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub thumbnail_cache_path: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub photo_paths: Vec<String>,
    pub cache: CacheConfig,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Config {
            port: env::var("TURBO_PIX_PORT")
                .unwrap_or_else(|_| "18473".to_string())
                .parse()?,
            photo_paths: env::var("TURBO_PIX_PHOTO_PATHS")
                .unwrap_or_else(|_| "./photos".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            cache: CacheConfig {
                thumbnail_cache_path: "./data/cache/thumbnails".to_string(),
            },
        })
    }
}
