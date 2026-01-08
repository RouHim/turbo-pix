use serde::Deserialize;
use std::str::FromStr;
use warp::{reject, Filter, Rejection, Reply};

use crate::db::{DbPool, Photo};
use crate::thumbnail_generator::ThumbnailGenerator;
use crate::thumbnail_types::{ThumbnailFormat, ThumbnailSize};
use crate::warp_helpers::{with_db, with_thumbnail_generator, DatabaseError, NotFoundError};

#[derive(Debug, Deserialize)]
pub struct ThumbnailQuery {
    pub size: Option<String>,
    pub format: Option<String>,
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

    let photo = match Photo::find_by_hash(&db_pool, &photo_hash).await {
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

    let format = ThumbnailFormat::from_str(&query.format.unwrap_or_else(|| "jpeg".to_string()))
        .unwrap_or(ThumbnailFormat::Jpeg);

    match thumbnail_generator
        .get_or_generate(&photo, size, format)
        .await
    {
        Ok(thumbnail_data) => {
            let reply =
                warp::reply::with_header(thumbnail_data, "content-type", format.content_type());
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

pub fn build_thumbnail_routes(
    db_pool: DbPool,
    thumbnail_generator: ThumbnailGenerator,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("thumbnail"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<ThumbnailQuery>())
        .and(with_db(db_pool))
        .and(with_thumbnail_generator(thumbnail_generator))
        .and_then(get_photo_thumbnail)
}
