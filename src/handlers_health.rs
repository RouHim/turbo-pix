use serde_json::json;
use std::convert::Infallible;
use warp::{reject, Rejection, Reply};

use crate::db::DbPool;
use crate::warp_helpers::DatabaseError;

pub async fn health_check() -> Result<impl Reply, Infallible> {
    Ok(warp::reply::json(&json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

pub async fn ready_check(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    // Test database connection
    match db_pool.get() {
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
