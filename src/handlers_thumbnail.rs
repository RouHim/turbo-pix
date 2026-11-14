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

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;
    use warp::http::StatusCode;

    use super::*;
    use crate::config::{CacheConfig, Config};
    use crate::db::create_in_memory_pool;
    use crate::warp_helpers::handle_rejection;
    use image::GenericImageView;

    /// 64-character stand-in for a SHA-256 the indexer would compute.
    const HASH: &str = "4d3f0a1c5b2e6d7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f";

    fn test_config(temp_dir: &TempDir) -> Config {
        Config {
            host: "127.0.0.1".to_string(),
            allowed_hosts: vec![],
            port: 8080,
            photo_paths: vec![],
            data_path: temp_dir.path().to_string_lossy().to_string(),
            db_path: temp_dir
                .path()
                .join("database/turbo-pix.db")
                .to_string_lossy()
                .to_string(),
            cache: CacheConfig {
                thumbnail_cache_path: temp_dir
                    .path()
                    .join("cache/thumbnails")
                    .to_string_lossy()
                    .to_string(),
                max_cache_size_mb: 1024,
            },
            transcode_timeout_secs: 300,
            locale: "en".to_string(),
            nominatim_url: "https://nominatim.openstreetmap.org".to_string(),
            tile_url: "https://tile.openstreetmap.org/{z}/{x}/{y}.png".to_string(),
        }
    }

    /// A 1000×600 JPEG: every supported size shrinks it without letterboxing,
    /// so the served dimensions name the size that was actually used
    /// (1000×600 → large 800×480, medium 400×240, small 200×120).
    fn write_source_image(path: &Path) {
        let img: image::ImageBuffer<image::Rgb<u8>, Vec<u8>> =
            image::ImageBuffer::from_fn(1000, 600, |x, _y| image::Rgb([0, (x % 251) as u8, 0]));
        img.save(path).unwrap();
    }

    /// A migrated in-memory database holding one photo whose file is on disk,
    /// plus the routes under test and the temp dir that owns both.
    async fn setup() -> (
        TempDir,
        PathBuf,
        Photo,
        impl Filter<Extract = impl warp::Reply, Error = Infallible> + Clone,
    ) {
        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("source.jpg");
        write_source_image(&image_path);

        let db_pool = create_in_memory_pool().await.unwrap();
        let generator = ThumbnailGenerator::new(&test_config(&temp_dir), db_pool.clone()).unwrap();

        let mut photo =
            crate::db::tests::create_test_photo("source.jpg".to_string(), HASH.to_string());
        photo.file_path = image_path.to_string_lossy().to_string();
        photo.create(&db_pool).await.unwrap();

        let routes = build_thumbnail_routes(db_pool, generator).recover(handle_rejection);
        (temp_dir, image_path, photo, routes)
    }

    fn dimensions(body: &[u8]) -> (u32, u32) {
        image::load_from_memory(body).unwrap().dimensions()
    }

    // GIVEN no photo with the requested hash
    // WHEN the thumbnail route is called
    // THEN it answers 404 with the handler's not-found body shape
    #[tokio::test]
    async fn an_unknown_hash_is_a_404_with_the_error_body() {
        let (_temp_dir, _image_path, _photo, routes) = setup().await;
        let unknown = "0".repeat(64);

        let response = warp::test::request()
            .path(&format!("/api/photos/{unknown}/thumbnail?size=medium"))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["error"], "Photo not found");
        assert_eq!(body["code"], 404);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap(),
            "application/json"
        );
    }

    // GIVEN a photo whose thumbnail has never been cached
    // WHEN it is requested with an unsupported size value
    // THEN the size falls back to medium: a 400×240 JPEG, not the 200×120 small
    //      and not the 800×480 large
    #[tokio::test]
    async fn an_unsupported_size_falls_back_to_medium() {
        let (_temp_dir, _image_path, photo, routes) = setup().await;

        let response = warp::test::request()
            .path(&format!(
                "/api/photos/{}/thumbnail?size=gigantic",
                photo.hash_sha256
            ))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap(),
            "image/jpeg"
        );
        assert_eq!(dimensions(response.body()), (400, 240));
    }

    // GIVEN a photo whose thumbnail has never been cached
    // WHEN it is requested with an unsupported format value at size=small
    // THEN the format falls back to jpeg (image/jpeg, 200×120), never webp
    #[tokio::test]
    async fn an_unsupported_format_falls_back_to_jpeg() {
        let (_temp_dir, _image_path, photo, routes) = setup().await;

        let response = warp::test::request()
            .path(&format!(
                "/api/photos/{}/thumbnail?size=small&format=bmp",
                photo.hash_sha256
            ))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap(),
            "image/jpeg"
        );
        assert_eq!(dimensions(response.body()), (200, 120));
    }

    // GIVEN a photo whose thumbnail has never been cached
    // WHEN format=webp is requested
    // THEN the supported format is honoured: image/webp bytes (RIFF....WEBP)
    #[tokio::test]
    async fn a_supported_webp_request_is_served_as_webp() {
        let (_temp_dir, _image_path, photo, routes) = setup().await;

        let response = warp::test::request()
            .path(&format!(
                "/api/photos/{}/thumbnail?size=small&format=webp",
                photo.hash_sha256
            ))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap(),
            "image/webp"
        );
        assert!(response.body().len() > 12);
        assert_eq!(&response.body()[0..4], b"RIFF");
        assert_eq!(&response.body()[8..12], b"WEBP");
    }

    // GIVEN one thumbnail already generated and cached for a photo
    // WHEN the source file is deleted and the same thumbnail is requested again
    // THEN the cached bytes are served (identical body, 200): the hit path does
    //      not go back to the source file, which would now 404
    #[tokio::test]
    async fn a_cache_hit_is_served_without_the_source_file() {
        let (temp_dir, image_path, photo, routes) = setup().await;
        let path = format!(
            "/api/photos/{}/thumbnail?size=medium&format=jpeg",
            photo.hash_sha256
        );

        let first = warp::test::request().path(&path).reply(&routes).await;
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(
            first
                .headers()
                .get("cache-control")
                .unwrap()
                .to_str()
                .unwrap(),
            "public, max-age=86400"
        );

        // The cache lives under the temp dir, the source file goes away: only a
        // cache hit can answer from here on.
        std::fs::remove_file(&image_path).unwrap();
        assert!(!image_path.exists());

        let second = warp::test::request().path(&path).reply(&routes).await;
        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(second.body(), first.body());
        assert_eq!(dimensions(second.body()), (400, 240));
        assert!(temp_dir.path().join("cache/thumbnails").exists());
    }

    // GIVEN a photo row whose file vanished before its thumbnail was cached
    // WHEN the thumbnail is requested
    // THEN the generation failure is the documented 404, not a 500
    #[tokio::test]
    async fn a_photo_whose_file_vanished_is_a_404() {
        let (_temp_dir, image_path, photo, routes) = setup().await;
        std::fs::remove_file(&image_path).unwrap();

        let response = warp::test::request()
            .path(&format!(
                "/api/photos/{}/thumbnail?size=medium",
                photo.hash_sha256
            ))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["error"], "Photo not found");
        assert_eq!(body["code"], 404);
    }
}
