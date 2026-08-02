use crate::cache_manager::CacheManager;
use crate::db::DbPool;
use crate::semantic_search::SemanticSearch;
use crate::thumbnail_generator::ThumbnailGenerator;
use serde::Serialize;
use std::convert::Infallible;
use std::sync::Arc;

use warp::{reject, Filter, Rejection, Reply};

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: u16,
    pub timestamp: String,
}

#[derive(Debug)]
pub struct DatabaseError {
    pub message: String,
}

impl reject::Reject for DatabaseError {}

#[derive(Debug)]
pub struct PermissionError {
    pub message: String,
}

impl reject::Reject for PermissionError {}

/// Rejection produced when a browser request carries an Origin header naming
/// a different host than the Host header (CSRF / cross-origin read attempt).
#[derive(Debug)]
pub struct CrossOriginRequest;

impl reject::Reject for CrossOriginRequest {}

#[derive(Debug)]
pub struct NotFoundError;
impl reject::Reject for NotFoundError {}

#[derive(Debug)]
pub struct ValidationError {
    pub message: String,
}

impl reject::Reject for ValidationError {}

pub fn with_db(db_pool: DbPool) -> impl Filter<Extract = (DbPool,), Error = Infallible> + Clone {
    warp::any().map(move || db_pool.clone())
}

pub fn with_thumbnail_generator(
    thumbnail_generator: ThumbnailGenerator,
) -> impl Filter<Extract = (ThumbnailGenerator,), Error = Infallible> + Clone {
    warp::any().map(move || thumbnail_generator.clone())
}

pub fn with_semantic_search(
    semantic_search: Arc<dyn SemanticSearch>,
) -> impl Filter<Extract = (Arc<dyn SemanticSearch>,), Error = Infallible> + Clone {
    warp::any().map(move || semantic_search.clone())
}

pub fn with_cache(
    cache_manager: CacheManager,
) -> impl Filter<Extract = (CacheManager,), Error = Infallible> + Clone {
    warp::any().map(move || cache_manager.clone())
}

pub async fn handle_rejection(err: Rejection) -> Result<impl Reply, Infallible> {
    let code;
    let message;
    let timestamp = chrono::Utc::now().to_rfc3339();

    if err.is_not_found() {
        code = warp::http::StatusCode::NOT_FOUND;
        message = "Not Found".to_string();
    } else if let Some(database_error) = err.find::<DatabaseError>() {
        code = warp::http::StatusCode::INTERNAL_SERVER_ERROR;
        // sqlx error text can embed SQL statements and server paths; log it
        // server-side but keep the client response generic (the frontend
        // already maps "Database error" to a translated toast).
        log::error!("Database error (sanitized): {}", database_error.message);
        message = "Database error".to_string();
    } else if let Some(permission_error) = err.find::<PermissionError>() {
        code = warp::http::StatusCode::FORBIDDEN;
        message = permission_error.message.clone();
    } else if err.find::<NotFoundError>().is_some() {
        code = warp::http::StatusCode::NOT_FOUND;
        message = "Photo not found".to_string();
    } else if err.find::<CrossOriginRequest>().is_some() {
        code = warp::http::StatusCode::FORBIDDEN;
        message = "Cross-origin request rejected".to_string();
    } else if let Some(validation_error) = err.find::<ValidationError>() {
        code = warp::http::StatusCode::BAD_REQUEST;
        message = validation_error.message.clone();
    } else if err.find::<warp::reject::PayloadTooLarge>().is_some() {
        code = warp::http::StatusCode::PAYLOAD_TOO_LARGE;
        message = "Payload too large".to_string();
    } else if err.find::<warp::reject::LengthRequired>().is_some() {
        // warp's content_length_limit rejects chunked/missing-Content-Length
        // bodies with 411 BEFORE reading them; map it explicitly so the
        // rejection does not fall through to a misleading 500.
        code = warp::http::StatusCode::LENGTH_REQUIRED;
        message = "Content-Length required".to_string();
    } else if err.find::<warp::reject::UnsupportedMediaType>().is_some() {
        code = warp::http::StatusCode::UNSUPPORTED_MEDIA_TYPE;
        message = "Unsupported media type".to_string();
    } else if err.find::<warp::reject::MethodNotAllowed>().is_some() {
        code = warp::http::StatusCode::METHOD_NOT_ALLOWED;
        message = "Method not allowed".to_string();
    } else if err.find::<warp::reject::InvalidQuery>().is_some() {
        code = warp::http::StatusCode::BAD_REQUEST;
        message = "Invalid query parameters".to_string();
    } else if err
        .find::<warp::filters::body::BodyDeserializeError>()
        .is_some()
    {
        code = warp::http::StatusCode::BAD_REQUEST;
        message = "Invalid request body".to_string();
    } else {
        log::error!("Unhandled rejection: {:?}", err);
        code = warp::http::StatusCode::INTERNAL_SERVER_ERROR;
        message = "Internal server error".to_string();
    }

    let error_response = ErrorResponse {
        error: message,
        code: code.as_u16(),
        timestamp,
    };

    Ok(warp::reply::with_status(
        warp::reply::json(&error_response),
        code,
    ))
}

/// Hostname part of an `Origin` (`http://localhost:5173` → `localhost`),
/// `Host` (`localhost:18473` → `localhost`) or bracketed IPv6
/// (`[::1]:18473` → `::1`) header value.
fn header_hostname(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    // Strip the scheme for Origin values, then take the part before any
    // port separator or path.
    let after_scheme = value
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(value);
    // Bracketed IPv6 literal: take the part between the brackets. Splitting
    // on ':' naively would yield "[" for both sides, making ANY two IPv6
    // hosts compare equal (the same-origin check would be vacuous).
    let hostname = if let Some(rest) = after_scheme.strip_prefix('[') {
        let (host, _) = rest.split_once(']')?;
        host
    } else {
        after_scheme
            .split([':', '/'])
            .next()
            .unwrap_or(after_scheme)
    };
    let hostname = hostname.trim();
    if hostname.is_empty() {
        None
    } else {
        Some(hostname.to_lowercase())
    }
}

/// Rejects browser requests whose `Origin` names a different host than the
/// `Host` header. This replaces the previous allow-any-origin CORS policy,
/// which let any website read the API (and, with the permissive method list,
/// issue destructive requests).
///
/// - Requests without an `Origin` (curl, scripts, same-origin GET/HEAD
///   navigations) pass — non-browser clients are unaffected.
/// - The comparison is hostname-only so the Vite dev-server proxy
///   (`Origin: http://localhost:5173`, `Host: localhost:18473`) still works.
/// - `Origin: null` (sandboxed contexts) is always rejected.
/// - When `allowed_hosts` is non-empty (TURBO_PIX_ALLOWED_HOSTS), the Host
///   header itself must match one of them: DNS-rebinding attacks make
///   Origin == Host trivially satisfiable by pointing the attacker's own
///   domain at the server, so hostname equality alone is not a CSRF proof.
pub fn require_same_origin(
    allowed_hosts: &[String],
) -> impl Filter<Extract = (), Error = Rejection> + Clone {
    let allowed_hosts: Vec<String> = allowed_hosts
        .iter()
        .map(|h| h.trim().to_lowercase())
        .filter(|h| !h.is_empty())
        .collect();
    warp::header::optional::<String>("origin")
        .and(warp::header::optional::<String>("host"))
        .and_then(move |origin: Option<String>, host: Option<String>| {
            let allowed_hosts = allowed_hosts.clone();
            async move {
                let Some(origin) = origin else {
                    return Ok(());
                };
                if origin == "null" {
                    return Err(reject::custom(CrossOriginRequest));
                }
                let Some(origin_host) = header_hostname(&origin) else {
                    return Err(reject::custom(CrossOriginRequest));
                };
                let Some(host_host) = host.and_then(|h| header_hostname(&h)) else {
                    return Err(reject::custom(CrossOriginRequest));
                };
                if host_host != origin_host {
                    return Err(reject::custom(CrossOriginRequest));
                }
                if !allowed_hosts.is_empty() && !allowed_hosts.contains(&host_host) {
                    return Err(reject::custom(CrossOriginRequest));
                }
                Ok(())
            }
        })
        .untuple_one()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_routes(
    ) -> impl Filter<Extract = impl warp::Reply, Error = std::convert::Infallible> + Clone {
        require_same_origin(&[])
            .and(warp::any().map(|| "ok"))
            .recover(handle_rejection)
    }

    fn test_routes_with_allowed(
        hosts: &[String],
    ) -> impl Filter<Extract = impl warp::Reply, Error = std::convert::Infallible> + Clone {
        require_same_origin(hosts)
            .and(warp::any().map(|| "ok"))
            .recover(handle_rejection)
    }

    #[tokio::test]
    async fn allows_requests_without_origin_header() {
        let resp = warp::test::request().path("/").reply(&test_routes()).await;
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn allows_same_hostname_origin() {
        let resp = warp::test::request()
            .path("/")
            .header("origin", "http://localhost:5173")
            .header("host", "localhost:18473")
            .reply(&test_routes())
            .await;
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn rejects_cross_hostname_origin() {
        let resp = warp::test::request()
            .path("/")
            .header("origin", "http://evil.example")
            .header("host", "localhost:18473")
            .reply(&test_routes())
            .await;
        assert_eq!(resp.status(), 403);
    }

    #[tokio::test]
    async fn rejects_null_origin() {
        let resp = warp::test::request()
            .path("/")
            .header("origin", "null")
            .header("host", "localhost:18473")
            .reply(&test_routes())
            .await;
        assert_eq!(resp.status(), 403);
    }

    #[tokio::test]
    async fn rejects_origin_without_host_header() {
        let resp = warp::test::request()
            .path("/")
            .header("origin", "http://localhost:5173")
            .reply(&test_routes())
            .await;
        assert_eq!(resp.status(), 403);
    }

    #[tokio::test]
    async fn parses_bracketed_ipv6_hostnames() {
        // GIVEN the same IPv6 literal on both sides (different ports)
        let routes = test_routes();
        let resp = warp::test::request()
            .path("/")
            .header("origin", "http://[::1]:5173")
            .header("host", "[::1]:18473")
            .reply(&routes)
            .await;
        // THEN same-host IPv6 passes
        assert_eq!(resp.status(), 200);

        // GIVEN DIFFERENT IPv6 literals — naive ':' splitting would collapse
        // both to "[" and let any IPv6 host pass
        let resp = warp::test::request()
            .path("/")
            .header("origin", "http://[2606:4700::6810:8445]")
            .header("host", "[::1]:18473")
            .reply(&routes)
            .await;
        assert_eq!(resp.status(), 403);

        // Malformed bracket (no closing ']') fails closed
        let resp = warp::test::request()
            .path("/")
            .header("origin", "http://[::1")
            .header("host", "[::1]:18473")
            .reply(&routes)
            .await;
        assert_eq!(resp.status(), 403);
    }

    #[tokio::test]
    async fn allowed_hosts_pins_the_host_header() {
        let routes = test_routes_with_allowed(&["photos.example.com".to_string()]);
        // Host not in the allowlist → rejected even though Origin == Host
        let resp = warp::test::request()
            .path("/")
            .header("origin", "http://attacker.com")
            .header("host", "attacker.com")
            .reply(&routes)
            .await;
        assert_eq!(resp.status(), 403);
        // Allowed hostname (with port) passes
        let resp = warp::test::request()
            .path("/")
            .header("origin", "http://photos.example.com:18473")
            .header("host", "photos.example.com:18473")
            .reply(&routes)
            .await;
        assert_eq!(resp.status(), 200);
    }
}
