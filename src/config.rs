use std::env;

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub thumbnail_cache_path: String,
    pub max_cache_size_mb: u64,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Bind address. Defaults to loopback: the API is unauthenticated and
    /// exposes destructive operations, so it must not listen on all
    /// interfaces unless the operator explicitly opts in (Docker sets
    /// TURBO_PIX_HOST=0.0.0.0).
    pub host: String,
    /// Comma-separated hostnames the Host header may carry (DNS-rebinding
    /// hardening, see require_same_origin). Empty = accept any hostname that
    /// matches the Origin (default, suitable for personal LAN use).
    pub allowed_hosts: Vec<String>,
    pub port: u16,
    pub photo_paths: Vec<String>,
    pub data_path: String,
    pub db_path: String,
    pub cache: CacheConfig,
    /// Bound on concurrent transcode ffmpeg jobs (0 = disabled). Default 2.
    pub max_transcodes: usize,
    /// Per-transcode timeout in seconds. Default 300.
    pub transcode_timeout_secs: u64,
    pub locale: String,
    pub nominatim_url: String,
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

        let host = env::var("TURBO_PIX_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());

        // DNS-rebinding hardening (see require_same_origin): when no explicit
        // allowlist is configured, pin the Host header to loopback names for
        // loopback binds. Non-loopback binds (0.0.0.0 for LAN/Docker) must
        // set TURBO_PIX_ALLOWED_HOSTS explicitly — otherwise every LAN host
        // would be rejected.
        let allowed_hosts = if let Ok(raw) = env::var("TURBO_PIX_ALLOWED_HOSTS") {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else if host == "127.0.0.1" || host == "::1" || host == "localhost" {
            vec![
                "127.0.0.1".to_string(),
                "localhost".to_string(),
                "::1".to_string(),
            ]
        } else {
            Vec::new()
        };

        let photo_paths = env::var("TURBO_PIX_PHOTO_PATHS")
            .unwrap_or_else(|_| "./photos".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        let locale =
            parse_locale(env::var("TURBO_PIX_LOCALE").unwrap_or_else(|_| "en".to_string()));

        let nominatim_url = env::var("TURBO_PIX_NOMINATIM_URL")
            .unwrap_or_else(|_| "https://nominatim.openstreetmap.org".to_string());

        let max_transcodes = env::var("TURBO_PIX_MAX_TRANSCODES")
            .unwrap_or_else(|_| "2".to_string())
            .parse()?;
        let transcode_timeout_secs = env::var("TURBO_PIX_TRANSCODE_TIMEOUT_SECS")
            .unwrap_or_else(|_| "300".to_string())
            .parse()?;

        let cache = CacheConfig {
            thumbnail_cache_path,
            max_cache_size_mb,
        };

        Ok(Config {
            host,
            allowed_hosts,
            port,
            photo_paths,
            data_path,
            db_path,
            cache,
            max_transcodes,
            transcode_timeout_secs,
            locale,
            nominatim_url,
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
    fn uses_loopback_host_by_default() {
        with_env_lock(|| {
            let original = env::var("TURBO_PIX_HOST").ok();
            let original_allowed = env::var("TURBO_PIX_ALLOWED_HOSTS").ok();
            env::remove_var("TURBO_PIX_HOST");
            env::remove_var("TURBO_PIX_ALLOWED_HOSTS");

            let config = Config::from_env().unwrap();
            assert_eq!(config.host, "127.0.0.1");
            // Loopback binds default the DNS-rebinding Host pin ON.
            assert_eq!(config.allowed_hosts, vec!["127.0.0.1", "localhost", "::1"]);

            if let Some(value) = original {
                env::set_var("TURBO_PIX_HOST", value);
            }
            if let Some(value) = original_allowed {
                env::set_var("TURBO_PIX_ALLOWED_HOSTS", value);
            }
        });
    }

    #[test]
    fn reads_custom_host_from_env() {
        with_env_lock(|| {
            let original = env::var("TURBO_PIX_HOST").ok();
            let original_allowed = env::var("TURBO_PIX_ALLOWED_HOSTS").ok();
            env::set_var("TURBO_PIX_HOST", "0.0.0.0");
            env::remove_var("TURBO_PIX_ALLOWED_HOSTS");

            let config = Config::from_env().unwrap();
            assert_eq!(config.host, "0.0.0.0");
            // Non-loopback binds leave the pin OFF (operator opt-in).
            assert!(config.allowed_hosts.is_empty());

            if let Some(value) = original {
                env::set_var("TURBO_PIX_HOST", value);
            } else {
                env::remove_var("TURBO_PIX_HOST");
            }
            if let Some(value) = original_allowed {
                env::set_var("TURBO_PIX_ALLOWED_HOSTS", value);
            }
        });
    }

    #[test]
    fn parses_allowed_hosts_from_env() {
        with_env_lock(|| {
            let original = env::var("TURBO_PIX_ALLOWED_HOSTS").ok();
            env::set_var(
                "TURBO_PIX_ALLOWED_HOSTS",
                "photos.example.com,  LAN-PIX  ,,photos.example.com",
            );

            let config = Config::from_env().unwrap();
            assert_eq!(
                config.allowed_hosts,
                vec!["photos.example.com", "LAN-PIX", "photos.example.com"]
            );

            if let Some(value) = original {
                env::set_var("TURBO_PIX_ALLOWED_HOSTS", value);
            } else {
                env::remove_var("TURBO_PIX_ALLOWED_HOSTS");
            }
        });
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
    fn uses_default_nominatim_url_when_env_var_is_missing() {
        with_env_lock(|| {
            let original = env::var("TURBO_PIX_NOMINATIM_URL").ok();
            env::remove_var("TURBO_PIX_NOMINATIM_URL");

            let config = Config::from_env().unwrap();
            assert_eq!(config.nominatim_url, "https://nominatim.openstreetmap.org");

            if let Some(value) = original {
                env::set_var("TURBO_PIX_NOMINATIM_URL", value);
            }
        });
    }

    #[test]
    fn reads_custom_nominatim_url_from_env() {
        with_env_lock(|| {
            let original = env::var("TURBO_PIX_NOMINATIM_URL").ok();
            env::set_var("TURBO_PIX_NOMINATIM_URL", "http://my-nominatim:8080");

            let config = Config::from_env().unwrap();
            assert_eq!(config.nominatim_url, "http://my-nominatim:8080");

            if let Some(value) = original {
                env::set_var("TURBO_PIX_NOMINATIM_URL", value);
            } else {
                env::remove_var("TURBO_PIX_NOMINATIM_URL");
            }
        });
    }

    #[test]
    fn reads_transcode_env_defaults() {
        with_env_lock(|| {
            let orig_max = env::var("TURBO_PIX_MAX_TRANSCODES").ok();
            let orig_timeout = env::var("TURBO_PIX_TRANSCODE_TIMEOUT_SECS").ok();
            env::remove_var("TURBO_PIX_MAX_TRANSCODES");
            env::remove_var("TURBO_PIX_TRANSCODE_TIMEOUT_SECS");

            let config = Config::from_env().unwrap();
            assert_eq!(config.max_transcodes, 2);
            assert_eq!(config.transcode_timeout_secs, 300);

            env::set_var("TURBO_PIX_MAX_TRANSCODES", "4");
            env::set_var("TURBO_PIX_TRANSCODE_TIMEOUT_SECS", "420");
            let config = Config::from_env().unwrap();
            assert_eq!(config.max_transcodes, 4);
            assert_eq!(config.transcode_timeout_secs, 420);

            restore_env_var("TURBO_PIX_MAX_TRANSCODES", orig_max);
            restore_env_var("TURBO_PIX_TRANSCODE_TIMEOUT_SECS", orig_timeout);
        });
    }

    #[test]
    fn rejects_invalid_transcode_env() {
        with_env_lock(|| {
            let orig_max = env::var("TURBO_PIX_MAX_TRANSCODES").ok();
            let orig_timeout = env::var("TURBO_PIX_TRANSCODE_TIMEOUT_SECS").ok();
            env::set_var("TURBO_PIX_MAX_TRANSCODES", "not-a-number");

            assert!(
                Config::from_env().is_err(),
                "invalid max_transcodes must error"
            );

            env::remove_var("TURBO_PIX_MAX_TRANSCODES");
            env::set_var("TURBO_PIX_TRANSCODE_TIMEOUT_SECS", "also-bad");
            assert!(
                Config::from_env().is_err(),
                "invalid transcode_timeout_secs must error"
            );

            restore_env_var("TURBO_PIX_MAX_TRANSCODES", orig_max);
            restore_env_var("TURBO_PIX_TRANSCODE_TIMEOUT_SECS", orig_timeout);
        });
    }

    fn restore_env_var(key: &str, value: Option<String>) {
        match value {
            Some(v) => env::set_var(key, v),
            None => env::remove_var(key),
        }
    }
}
