use std::env;

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub thumbnail_cache_path: String,
    pub max_cache_size_mb: u64,
}

/// Fixed thumbnail-cache ceiling. It used to be operator-configurable, but the
/// eviction only sees files touched in the current session, so a larger value
/// bought little beyond disk usage; 2 GiB covers tens of thousands of thumbnails.
pub const THUMBNAIL_CACHE_LIMIT_MB: u64 = 2048;

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub photo_paths: Vec<String>,
    pub data_path: String,
    pub db_path: String,
    pub cache: CacheConfig,
    pub locale: String,
    pub tile_url: String,
    /// Face detection/recognition is on by default, like CLIP. The kill
    /// switch skips the face phase and hides the People surface.
    pub face_recognition_enabled: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let data_path = env::var("TURBO_PIX_DATA_PATH").unwrap_or_else(|_| "./data".to_string());
        let db_path = format!("{}/database/turbo-pix.db", data_path);
        let thumbnail_cache_path = format!("{}/cache/thumbnails", data_path);

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

        // Operator-configurable raster tile endpoint (FR-002). The default is
        // the public OSM service; self-hosted/alternative OSM-compatible
        // endpoints need this env var only — no code change or rebuild.
        let tile_url = env::var("TURBO_PIX_TILE_URL")
            .unwrap_or_else(|_| "https://tile.openstreetmap.org/{z}/{x}/{y}.png".to_string());

        let face_recognition_enabled =
            parse_bool_env(env::var("TURBO_PIX_FACE_RECOGNITION").ok().as_deref(), true);

        let cache = CacheConfig {
            thumbnail_cache_path,
            max_cache_size_mb: THUMBNAIL_CACHE_LIMIT_MB,
        };

        Ok(Config {
            port,
            photo_paths,
            data_path,
            db_path,
            cache,
            locale,
            tile_url,
            face_recognition_enabled,
        })
    }
}

fn parse_bool_env(value: Option<&str>, default: bool) -> bool {
    match value
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("1" | "true" | "yes" | "on") => true,
        Some("0" | "false" | "no" | "off") => false,
        _ => default,
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

    #[test]
    fn uses_default_tile_url_when_env_var_is_missing() {
        with_env_lock(|| {
            let original = std::env::var("TURBO_PIX_TILE_URL").ok();
            restore_env_var("TURBO_PIX_TILE_URL", None);
            let config = Config::from_env().expect("config should parse");
            assert_eq!(
                config.tile_url,
                "https://tile.openstreetmap.org/{z}/{x}/{y}.png"
            );
            restore_env_var("TURBO_PIX_TILE_URL", original);
        });
    }

    #[test]
    fn reads_custom_tile_url_from_env() {
        with_env_lock(|| {
            let original = std::env::var("TURBO_PIX_TILE_URL").ok();
            std::env::set_var(
                "TURBO_PIX_TILE_URL",
                "http://tiles.example.lan/{z}/{x}/{y}.png",
            );
            let config = Config::from_env().expect("config should parse");
            assert_eq!(config.tile_url, "http://tiles.example.lan/{z}/{x}/{y}.png");
            restore_env_var("TURBO_PIX_TILE_URL", original);
        });
    }

    #[test]
    fn ignores_removed_tuning_env_vars_and_fixes_the_cache_ceiling() {
        with_env_lock(|| {
            let removed = [
                ("TURBO_PIX_HOST", "not-an-ip"),
                ("TURBO_PIX_MAX_CACHE_SIZE_MB", "not-a-number"),
                ("TURBO_PIX_TRANSCODE_TIMEOUT_SECS", "not-a-number"),
                ("TURBO_PIX_MAX_TRANSCODES", "0"),
                ("TURBO_PIX_STREAM_QUEUE_WAIT_SECS", "not-a-number"),
                ("TRANSCODE_CACHE_DIR", "/tmp/must-not-be-read"),
            ];
            let originals: Vec<_> = removed
                .iter()
                .map(|(key, _)| (*key, env::var(key).ok()))
                .collect();
            for (key, value) in removed {
                env::set_var(key, value);
            }

            let config = Config::from_env()
                .expect("removed variables must not make configuration parsing fail");
            assert_eq!(
                config.cache.max_cache_size_mb, 2048,
                "the thumbnail cache ceiling is fixed, not read from the environment"
            );

            for (key, value) in originals {
                restore_env_var(key, value);
            }
        });
    }

    #[test]
    fn face_recognition_is_on_by_default_and_has_a_kill_switch() {
        with_env_lock(|| {
            let original = env::var("TURBO_PIX_FACE_RECOGNITION").ok();
            env::remove_var("TURBO_PIX_FACE_RECOGNITION");
            assert!(Config::from_env().unwrap().face_recognition_enabled);

            env::set_var("TURBO_PIX_FACE_RECOGNITION", "false");
            assert!(!Config::from_env().unwrap().face_recognition_enabled);

            env::set_var("TURBO_PIX_FACE_RECOGNITION", "0");
            assert!(!Config::from_env().unwrap().face_recognition_enabled);

            env::set_var("TURBO_PIX_FACE_RECOGNITION", "true");
            assert!(Config::from_env().unwrap().face_recognition_enabled);

            restore_env_var("TURBO_PIX_FACE_RECOGNITION", original);
        });
    }

    fn restore_env_var(key: &str, value: Option<String>) {
        match value {
            Some(v) => env::set_var(key, v),
            None => env::remove_var(key),
        }
    }
}
