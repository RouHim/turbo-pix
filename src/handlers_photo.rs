use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use warp::{reject, Rejection, Reply};

use crate::db::{DbPool, Photo, SearchQuery};
use crate::mimetype_detector;
use crate::warp_helpers::{DatabaseError, NotFoundError};

#[derive(Debug, Deserialize)]
pub struct PhotoQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub q: Option<String>,
    pub year: Option<i32>,
    pub month: Option<i32>,
}

#[derive(Debug, serde::Serialize)]
pub struct PhotosResponse {
    pub photos: Vec<Photo>,
    pub total: usize,
    pub page: u32,
    pub limit: u32,
    pub has_next: bool,
    pub has_prev: bool,
}

pub async fn list_photos(query: PhotoQuery, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(50).min(100);
    let offset = (page - 1) * limit;

    // If a query string or year/month filter is provided, use search instead of list
    let result = if query.q.is_some() || query.year.is_some() || query.month.is_some() {
        let search_query = SearchQuery {
            q: query.q.clone(),
            camera_make: None,
            camera_model: None,
            year: query.year,
            month: query.month,
            keywords: None,
            has_location: None,
            country: None,
            limit: Some(limit),
            page: Some(page),
            sort: query.sort.clone(),
            order: query.order.clone(),
        };
        Photo::search_photos(
            &db_pool,
            &search_query,
            limit as i64,
            offset as i64,
            query.sort.as_deref(),
            query.order.as_deref(),
        )
    } else {
        Photo::list_with_pagination(
            &db_pool,
            limit as i64,
            offset as i64,
            query.sort.as_deref(),
            query.order.as_deref(),
        )
    };

    match result {
        Ok((photos, total)) => {
            let has_next = offset + limit < total as u32;
            let has_prev = page > 1;

            Ok(warp::reply::json(&PhotosResponse {
                photos,
                total: total as usize,
                page,
                limit,
                has_next,
                has_prev,
            }))
        }
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

pub async fn get_photo(photo_hash: String, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => Ok(warp::reply::json(&photo)),
        Ok(None) => Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

pub async fn get_photo_file(
    photo_hash: String,
    db_pool: DbPool,
) -> Result<Box<dyn Reply>, Rejection> {
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

    match std::fs::read(&photo.file_path) {
        Ok(file_data) => {
            let content_type = photo.mime_type.unwrap_or_else(|| {
                mimetype_detector::from_path(Path::new(&photo.file_path))
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "application/octet-stream".to_string())
            });

            let reply = warp::reply::with_header(file_data, "content-type", content_type);
            let reply =
                warp::reply::with_header(reply, "cache-control", "public, max-age=31536000");

            Ok(Box::new(reply))
        }
        Err(_) => Err(reject::custom(NotFoundError)),
    }
}

#[derive(Debug, Deserialize)]
pub struct PhotoUpdateRequest {
    pub filename: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub iso: Option<i32>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub focal_length: Option<f64>,
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
    pub location_name: Option<String>,
    pub is_favorite: Option<bool>,
}

#[allow(dead_code)]
pub async fn update_photo(
    photo_hash: String,
    update_req: PhotoUpdateRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let mut photo = match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    // Update fields if provided
    if let Some(filename) = &update_req.filename {
        photo.filename = filename.clone();
    }
    if let Some(camera_make) = &update_req.camera_make {
        photo.camera_make = Some(camera_make.clone());
    }
    if let Some(camera_model) = &update_req.camera_model {
        photo.camera_model = Some(camera_model.clone());
    }
    if let Some(iso) = update_req.iso {
        photo.iso = Some(iso);
    }
    if let Some(aperture) = update_req.aperture {
        photo.aperture = Some(aperture);
    }
    if let Some(shutter_speed) = &update_req.shutter_speed {
        photo.shutter_speed = Some(shutter_speed.clone());
    }
    if let Some(focal_length) = update_req.focal_length {
        photo.focal_length = Some(focal_length);
    }
    if let Some(gps_latitude) = update_req.gps_latitude {
        photo.latitude = Some(gps_latitude);
    }
    if let Some(gps_longitude) = update_req.gps_longitude {
        photo.longitude = Some(gps_longitude);
    }
    if let Some(location_name) = &update_req.location_name {
        photo.location_name = Some(location_name.clone());
    }
    if let Some(is_favorite) = update_req.is_favorite {
        photo.is_favorite = Some(is_favorite);
    }

    match photo.update(&db_pool) {
        Ok(_) => Ok(warp::reply::json(&photo)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

#[allow(dead_code)]
pub async fn delete_photo(photo_hash: String, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(_)) => match Photo::delete(&db_pool, &photo_hash) {
            Ok(true) => Ok(warp::reply::with_status(
                "",
                warp::http::StatusCode::NO_CONTENT,
            )),
            Ok(false) => Err(reject::custom(NotFoundError)),
            Err(e) => {
                log::error!("Database error: {}", e);
                Err(reject::custom(DatabaseError {
                    message: format!("Database error: {}", e),
                }))
            }
        },
        Ok(None) => Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct FavoriteRequest {
    pub is_favorite: bool,
}

pub async fn toggle_favorite(
    photo_hash: String,
    favorite_req: FavoriteRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let mut photo = match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    photo.is_favorite = Some(favorite_req.is_favorite);

    match photo.update(&db_pool) {
        Ok(_) => Ok(warp::reply::json(&photo)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

#[allow(dead_code)]
pub async fn get_photo_metadata(
    photo_hash: String,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => {
            let metadata = json!({
                "hash_sha256": photo.hash_sha256,
                "filename": photo.filename,
                "file_size": photo.file_size,
                "mime_type": photo.mime_type,
                "taken_at": photo.taken_at.map(|dt| dt.to_rfc3339()),
                "date_modified": photo.date_modified.to_rfc3339(),
                "camera_make": photo.camera_make,
                "camera_model": photo.camera_model,
                "iso": photo.iso,
                "aperture": photo.aperture,
                "shutter_speed": photo.shutter_speed,
                "focal_length": photo.focal_length,
                "width": photo.width,
                "height": photo.height,
                "orientation": photo.orientation,
                "gps_latitude": photo.latitude,
                "gps_longitude": photo.longitude,
                "location_name": photo.location_name,
            });
            Ok(warp::reply::json(&metadata))
        }
        Ok(None) => Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

pub async fn get_stats(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Photo::get_stats(&db_pool) {
        Ok(stats) => Ok(warp::reply::json(&stats)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

pub async fn get_timeline(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Photo::get_timeline_data(&db_pool) {
        Ok(timeline) => Ok(warp::reply::json(&timeline)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

pub async fn get_photo_exif(
    photo_hash: String,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    use exif::Reader;
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::io::BufReader;

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

    let file = match File::open(&photo.file_path) {
        Ok(f) => f,
        Err(e) => {
            log::error!("Failed to open file {}: {}", photo.file_path, e);
            return Err(reject::custom(NotFoundError));
        }
    };

    let mut reader = BufReader::new(file);
    let exif_reader = match Reader::new().read_from_container(&mut reader) {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to read EXIF from {}: {}", photo.file_path, e);
            return Ok(warp::reply::json(&json!({
                "error": "No EXIF data found",
                "message": format!("{}", e)
            })));
        }
    };

    let mut exif_data: BTreeMap<String, serde_json::Value> = BTreeMap::new();

    for field in exif_reader.fields() {
        let tag_name = format!("{}", field.tag);
        let value = field.display_value().to_string();

        exif_data.insert(
            tag_name,
            json!({
                "value": value,
                "ifd": format!("{:?}", field.ifd_num)
            })
        );
    }

    Ok(warp::reply::json(&json!({
        "hash": photo_hash,
        "filename": photo.filename,
        "exif": exif_data
    })))
}
