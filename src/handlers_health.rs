use serde_json::json;
use std::convert::Infallible;
use warp::{reject, Filter, Rejection, Reply};

use crate::db::DbPool;
use crate::warp_helpers::{with_db, DatabaseError};

pub async fn health_check() -> Result<impl Reply, Infallible> {
    Ok(warp::reply::json(&json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

pub async fn ready_check(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    // Test database connection by acquiring a connection
    // acquire() checks if the pool can provide a connection
    match db_pool.acquire().await {
        Ok(_) => Ok(warp::reply::json(&json!({
            "status": "ready",
            "database": "connected",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))),
        Err(e) => {
            log::error!("Database connection failed: {}", e);
            Err(reject::custom(DatabaseError {
                message: "Database connection failed".to_string(),
            }))
        }
    }
}

pub fn build_health_routes(
    db_pool: DbPool,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let health = warp::path("health").and(warp::get()).and_then(health_check);

    let ready = warp::path("ready")
        .and(warp::get())
        .and(with_db(db_pool))
        .and_then(ready_check);

    health.or(ready)
}