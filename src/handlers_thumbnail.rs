use serde::Deserialize;
use std::str::FromStr;
use warp::{reject, Rejection, Reply};

use crate::cache::{ThumbnailGenerator, ThumbnailSize};
use crate::db::{DbPool, Photo};
use crate::warp_helpers::{DatabaseError, NotFoundError};

#[derive(Debug, Deserialize)]
pub struct ThumbnailQuery {
    pub size: Option<String>,
}

pub async fn get_photo_thumbnail(
    photo_hash: String,
    query: ThumbnailQuery,
    db_pool: DbPool,
    thumbnail_generator: ThumbnailGenerator,
) -> Result<Box<dyn Reply>, Rejection> {
    log::debug!(
        "Thumbnail requested for photo {}, size: {:?}",
        photo_hash,
        query.size
    );

    let photo = match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    let size = ThumbnailSize::from_str(&query.size.unwrap_or_else(|| "medium".to_string()))
        .unwrap_or(ThumbnailSize::Medium);

    match thumbnail_generator.get_or_generate(&photo, size).await {
        Ok(thumbnail_data) => {
            let reply = warp::reply::with_header(thumbnail_data, "content-type", "image/jpeg");
            let reply = warp::reply::with_header(
                reply,
                "cache-control",
                "public, max-age=86400", // 24 hours cache for thumbnails
            );

            Ok(Box::new(reply))
        }
        Err(e) => {
            log::error!("Failed to generate thumbnail: {}", e);
            Err(reject::custom(NotFoundError))
        }
    }
}

pub async fn get_thumbnail_by_hash(
    hash: String,
    size: String,
    db_pool: DbPool,
    thumbnail_generator: ThumbnailGenerator,
) -> Result<Box<dyn Reply>, Rejection> {
    log::debug!("Thumbnail by hash requested: {}, size: {}", hash, size);

    let photo = match Photo::find_by_hash(&db_pool, &hash) {
        Ok(Some(photo)) => photo,
        Ok(None) => {
            log::warn!("Photo not found by hash: {}", hash);
            return Err(reject::custom(NotFoundError));
        }
        Err(e) => {
            log::error!("Database error looking up photo by hash {}: {}", hash, e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    let thumbnail_size = ThumbnailSize::from_str(&size).unwrap_or(ThumbnailSize::Medium);

    match thumbnail_generator
        .get_or_generate(&photo, thumbnail_size)
        .await
    {
        Ok(thumbnail_data) => {
            let reply = warp::reply::with_header(thumbnail_data, "content-type", "image/jpeg");
            let reply = warp::reply::with_header(
                reply,
                "cache-control",
                "public, max-age=86400", // 24 hours cache for thumbnails
            );

            Ok(Box::new(reply))
        }
        Err(e) => {
            log::error!("Failed to generate thumbnail for {}: {}", hash, e);
            Err(reject::custom(NotFoundError))
        }
    }
}
