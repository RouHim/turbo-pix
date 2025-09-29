use crate::db::DbPool;
use serde::{Deserialize, Serialize};
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
pub struct NotFoundError;
impl reject::Reject for NotFoundError {}

#[derive(Debug)]
pub struct ValidationError {
    pub message: String,
}

impl reject::Reject for ValidationError {}

pub fn with_db(
    db_pool: DbPool,
) -> impl Filter<Extract = (DbPool,), Error = Infallible> + Clone {
    warp::any().map(move || db_pool.clone())
}

pub fn with_thumbnail_service<T: Clone + Send + Sync>(
    service: Arc<T>,
) -> impl Filter<Extract = (Arc<T>,), Error = Infallible> + Clone {
    warp::any().map(move || service.clone())
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
        message = database_error.message.clone();
    } else if let Some(_) = err.find::<NotFoundError>() {
        code = warp::http::StatusCode::NOT_FOUND;
        message = "Photo not found".to_string();
    } else if let Some(validation_error) = err.find::<ValidationError>() {
        code = warp::http::StatusCode::BAD_REQUEST;
        message = validation_error.message.clone();
    } else if let Some(_) = err.find::<warp::reject::PayloadTooLarge>() {
        code = warp::http::StatusCode::PAYLOAD_TOO_LARGE;
        message = "Payload too large".to_string();
    } else if let Some(_) = err.find::<warp::reject::UnsupportedMediaType>() {
        code = warp::http::StatusCode::UNSUPPORTED_MEDIA_TYPE;
        message = "Unsupported media type".to_string();
    } else if let Some(_) = err.find::<warp::reject::MethodNotAllowed>() {
        code = warp::http::StatusCode::METHOD_NOT_ALLOWED;
        message = "Method not allowed".to_string();
    } else {
        tracing::error!("Unhandled rejection: {:?}", err);
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

pub fn json_body<T: for<'de> Deserialize<'de> + Send>(
) -> impl Filter<Extract = (T,), Error = Rejection> + Clone {
    warp::body::content_length_limit(1024 * 16).and(warp::body::json())
}

pub fn cors() -> warp::cors::Builder {
    warp::cors()
        .allow_any_origin()
        .allow_headers(vec!["content-type", "authorization"])
        .allow_methods(vec!["GET", "POST", "PUT", "DELETE", "OPTIONS"])
}