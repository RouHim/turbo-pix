use std::env;

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub thumbnail_cache_path: String,
    pub max_cache_size_mb: u64,
}

#[derive(Debug, Clone)]
pub struct ClipConfig {
    pub enabled: bool,
    pub model_path: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub photo_paths: Vec<String>,
    pub data_path: String,
    pub db_path: String,
    pub cache: CacheConfig,
    pub clip: ClipConfig,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let data_path = env::var("TURBO_PIX_DATA_PATH").unwrap_or_else(|_| "./data".to_string());

        let db_path = format!("{}/database/turbo-pix.db", data_path);
        let thumbnail_cache_path = format!("{}/cache/thumbnails", data_path);
        let max_cache_size_mb = env::var("TURBO_PIX_MAX_CACHE_SIZE_MB")
            .unwrap_or_else(|_| "1024".to_string())
            .parse()?;

        let clip_enabled = env::var("CLIP_ENABLE")
            .unwrap_or_else(|_| "false".to_string())
            .parse()?;
        let clip_model_path = env::var("CLIP_MODEL_PATH")
            .unwrap_or_else(|_| "./models/clip".to_string());

        Ok(Config {
            port: env::var("TURBO_PIX_PORT")
                .unwrap_or_else(|_| "18473".to_string())
                .parse()?,
            photo_paths: env::var("TURBO_PIX_PHOTO_PATHS")
                .unwrap_or_else(|_| "./photos".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            data_path,
            db_path,
            cache: CacheConfig {
                thumbnail_cache_path,
                max_cache_size_mb,
            },
            clip: ClipConfig {
                enabled: clip_enabled,
                model_path: clip_model_path,
            },
        })
    }
}
