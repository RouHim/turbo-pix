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

/// Hostname part of an `Origin` (`http://localhost:5173` → `localhost`) or
/// `Host` (`localhost:18473` → `localhost`) header value.
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
    let hostname = after_scheme
        .split([':', '/'])
        .next()
        .unwrap_or(after_scheme)
        .trim();
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
pub fn require_same_origin() -> impl Filter<Extract = (), Error = Rejection> + Clone {
    warp::header::optional::<String>("origin")
        .and(warp::header::optional::<String>("host"))
        .and_then(|origin: Option<String>, host: Option<String>| async move {
            let Some(origin) = origin else {
                return Ok(());
            };
            if origin == "null" {
                return Err(reject::custom(CrossOriginRequest));
            }
            let Some(origin_host) = header_hostname(&origin) else {
                return Err(reject::custom(CrossOriginRequest));
            };
            match host.and_then(|h| header_hostname(&h)) {
                Some(host_host) if host_host == origin_host => Ok(()),
                _ => Err(reject::custom(CrossOriginRequest)),
            }
        })
        .untuple_one()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_routes(
    ) -> impl Filter<Extract = impl warp::Reply, Error = std::convert::Infallible> + Clone {
        require_same_origin()
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
}
