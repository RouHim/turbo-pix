use std::env;

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub thumbnail_cache_path: String,
    pub max_cache_size_mb: u64,
}

#[derive(Debug, Clone)]
pub struct CollageConfig {
    pub width: u32,
    pub height: u32,
    pub max_photos: usize,
}

impl Default for CollageConfig {
    fn default() -> Self {
        Self {
            width: 3840,
            height: 2160,
            max_photos: 6,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub photo_paths: Vec<String>,
    pub data_path: String,
    pub db_path: String,
    pub cache: CacheConfig,
    pub locale: String,
    pub collage: CollageConfig,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let data_path = env::var("TURBO_PIX_DATA_PATH").unwrap_or_else(|_| "./data".to_string());
        let db_path = format!("{}/database/turbo-pix.db", data_path);
        let thumbnail_cache_path = format!("{}/cache/thumbnails", data_path);

        let max_cache_size_mb = env::var("TURBO_PIX_MAX_CACHE_SIZE_MB")
            .unwrap_or_else(|_| "1024".to_string())
            .parse()?;

        let port = env::var("TURBO_PIX_PORT")
            .unwrap_or_else(|_| "18473".to_string())
            .parse()?;

        let photo_paths = env::var("TURBO_PIX_PHOTO_PATHS")
            .unwrap_or_else(|_| "./photos".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        let locale =
            parse_locale(env::var("TURBO_PIX_LOCALE").unwrap_or_else(|_| "en".to_string()));

        let cache = CacheConfig {
            thumbnail_cache_path,
            max_cache_size_mb,
        };

        let collage = CollageConfig {
            width: env::var("TURBO_PIX_COLLAGE_WIDTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3840),
            height: env::var("TURBO_PIX_COLLAGE_HEIGHT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2160),
            max_photos: env::var("TURBO_PIX_COLLAGE_MAX_PHOTOS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(6),
        };

        Ok(Config {
            port,
            photo_paths,
            data_path,
            db_path,
            cache,
            locale,
            collage,
        })
    }
}

fn parse_locale(value: String) -> String {
    let normalized = value.trim().to_lowercase();
    if normalized == "de" || normalized == "en" {
        normalized
    } else {
        "en".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn with_env_lock<T>(f: impl FnOnce() -> T) -> T {
        let lock = ENV_LOCK.get_or_init(|| Mutex::new(()));
        let _guard = lock.lock().unwrap();
        f()
    }

    #[test]
    fn parses_locale_from_env() {
        with_env_lock(|| {
            let original = env::var("TURBO_PIX_LOCALE").ok();
            env::set_var("TURBO_PIX_LOCALE", "de");

            let config = Config::from_env().unwrap();
            assert_eq!(config.locale, "de");

            if let Some(value) = original {
                env::set_var("TURBO_PIX_LOCALE", value);
            } else {
                env::remove_var("TURBO_PIX_LOCALE");
            }
        });
    }

    #[test]
    fn falls_back_to_english_for_invalid_locale() {
        with_env_lock(|| {
            let original = env::var("TURBO_PIX_LOCALE").ok();
            env::set_var("TURBO_PIX_LOCALE", "fr");

            let config = Config::from_env().unwrap();
            assert_eq!(config.locale, "en");

            if let Some(value) = original {
                env::set_var("TURBO_PIX_LOCALE", value);
            } else {
                env::remove_var("TURBO_PIX_LOCALE");
            }
        });
    }
}
