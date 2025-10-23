use image::DynamicImage;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use warp::{reject, Filter, Rejection, Reply};

use crate::db::{DbPool, Photo, SearchQuery};
use crate::handlers_video::{get_video_file, VideoQuery};
use crate::mimetype_detector;
use crate::warp_helpers::{with_db, DatabaseError, NotFoundError};

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
            year: query.year,
            month: query.month,
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

/// Apply EXIF orientation transformation to an image
/// Matches the orientation values from EXIF specification
fn apply_orientation(img: DynamicImage, orientation: Option<i32>) -> DynamicImage {
    match orientation {
        Some(2) => img.fliph(),
        Some(3) => img.rotate180(),
        Some(4) => img.flipv(),
        Some(5) => img.fliph().rotate90(),
        Some(6) => img.rotate90(),
        Some(7) => img.fliph().rotate270(),
        Some(8) => img.rotate270(),
        _ => img, // 1 or None = no transformation needed
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

    let file_path = Path::new(&photo.file_path);

    // Check if this is a RAW file that needs conversion
    if crate::raw_processor::is_raw_file(file_path) {
        log::debug!(
            "Converting RAW file to JPEG for detail view: {}",
            photo.file_path
        );

        match crate::raw_processor::decode_raw_to_dynamic_image(file_path) {
            Ok(img) => {
                // Apply orientation correction
                let img = apply_orientation(img, photo.orientation);

                // Encode as JPEG with high quality
                let mut jpeg_data = Vec::new();
                let mut cursor = std::io::Cursor::new(&mut jpeg_data);

                match img.write_to(&mut cursor, image::ImageFormat::Jpeg) {
                    Ok(_) => {
                        let reply =
                            warp::reply::with_header(jpeg_data, "content-type", "image/jpeg");
                        let reply = warp::reply::with_header(
                            reply,
                            "cache-control",
                            "public, max-age=31536000",
                        );
                        return Ok(Box::new(reply));
                    }
                    Err(e) => {
                        log::error!("Failed to encode RAW as JPEG: {}", e);
                        return Err(reject::custom(DatabaseError {
                            message: format!("Failed to encode RAW as JPEG: {}", e),
                        }));
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to decode RAW file {}: {}", photo.file_path, e);
                return Err(reject::custom(DatabaseError {
                    message: format!("Failed to decode RAW file: {}", e),
                }));
            }
        }
    }

    // For non-RAW files, serve directly
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

pub async fn get_photo_exif(photo_hash: String, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    use little_exif::metadata::Metadata;
    use std::collections::BTreeMap;
    use std::path::Path;

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

    let exif_metadata = match Metadata::new_from_path(Path::new(&photo.file_path)) {
        Ok(m) => m,
        Err(e) => {
            log::error!("Failed to read EXIF from {}: {}", photo.file_path, e);
            return Ok(warp::reply::json(&json!({
                "error": "No EXIF data found",
                "message": format!("{}", e)
            })));
        }
    };

    let mut exif_data: BTreeMap<String, serde_json::Value> = BTreeMap::new();

    // Iterate through all IFDs and their tags
    for ifd in exif_metadata.get_ifds() {
        for tag in ifd.get_tags() {
            let tag_hex = tag.as_u16();
            // Use discriminant-based tag name extraction for robustness
            let tag_name = get_exif_tag_name(tag);
            let value = format!("{:?}", tag);

            exif_data.insert(
                format!("0x{:04X}_{}", tag_hex, tag_name),
                json!({
                    "value": value,
                    "tag": tag_name
                }),
            );
        }
    }

    Ok(warp::reply::json(&json!({
        "hash": photo_hash,
        "filename": photo.filename,
        "exif": exif_data
    })))
}

/// Extract tag name from ExifTag enum variant
/// More robust than string parsing of Debug output
fn get_exif_tag_name(tag: &little_exif::exif_tag::ExifTag) -> String {
    use little_exif::exif_tag::ExifTag;

    match tag {
        ExifTag::Make(_) => "Make",
        ExifTag::Model(_) => "Model",
        ExifTag::Orientation(_) => "Orientation",
        ExifTag::XResolution(_) => "XResolution",
        ExifTag::YResolution(_) => "YResolution",
        ExifTag::ResolutionUnit(_) => "ResolutionUnit",
        ExifTag::Software(_) => "Software",
        ExifTag::ModifyDate(_) => "ModifyDate",
        ExifTag::Artist(_) => "Artist",
        ExifTag::YCbCrPositioning(_) => "YCbCrPositioning",
        ExifTag::Copyright(_) => "Copyright",
        ExifTag::ExifOffset(_) => "ExifOffset",
        ExifTag::ExposureTime(_) => "ExposureTime",
        ExifTag::FNumber(_) => "FNumber",
        ExifTag::ExposureProgram(_) => "ExposureProgram",
        ExifTag::ISO(_) => "ISO",
        ExifTag::SensitivityType(_) => "SensitivityType",
        ExifTag::ExifVersion(_) => "ExifVersion",
        ExifTag::DateTimeOriginal(_) => "DateTimeOriginal",
        ExifTag::CreateDate(_) => "CreateDate",
        ExifTag::OffsetTime(_) => "OffsetTime",
        ExifTag::OffsetTimeOriginal(_) => "OffsetTimeOriginal",
        ExifTag::OffsetTimeDigitized(_) => "OffsetTimeDigitized",
        ExifTag::ShutterSpeedValue(_) => "ShutterSpeedValue",
        ExifTag::ApertureValue(_) => "ApertureValue",
        ExifTag::BrightnessValue(_) => "BrightnessValue",
        ExifTag::ExposureCompensation(_) => "ExposureCompensation",
        ExifTag::MaxApertureValue(_) => "MaxApertureValue",
        ExifTag::MeteringMode(_) => "MeteringMode",
        ExifTag::LightSource(_) => "LightSource",
        ExifTag::Flash(_) => "Flash",
        ExifTag::FocalLength(_) => "FocalLength",
        ExifTag::SubjectArea(_) => "SubjectArea",
        ExifTag::SubSecTime(_) => "SubSecTime",
        ExifTag::SubSecTimeOriginal(_) => "SubSecTimeOriginal",
        ExifTag::SubSecTimeDigitized(_) => "SubSecTimeDigitized",
        ExifTag::ColorSpace(_) => "ColorSpace",
        ExifTag::ExifImageWidth(_) => "ExifImageWidth",
        ExifTag::ExifImageHeight(_) => "ExifImageHeight",
        ExifTag::SensingMethod(_) => "SensingMethod",
        ExifTag::SceneType(_) => "SceneType",
        ExifTag::ExposureMode(_) => "ExposureMode",
        ExifTag::WhiteBalance(_) => "WhiteBalance",
        ExifTag::FocalLengthIn35mmFormat(_) => "FocalLengthIn35mmFormat",
        ExifTag::SceneCaptureType(_) => "SceneCaptureType",
        ExifTag::LensInfo(_) => "LensInfo",
        ExifTag::LensMake(_) => "LensMake",
        ExifTag::LensModel(_) => "LensModel",
        ExifTag::GPSLatitudeRef(_) => "GPSLatitudeRef",
        ExifTag::GPSLatitude(_) => "GPSLatitude",
        ExifTag::GPSLongitudeRef(_) => "GPSLongitudeRef",
        ExifTag::GPSLongitude(_) => "GPSLongitude",
        ExifTag::GPSAltitudeRef(_) => "GPSAltitudeRef",
        ExifTag::GPSAltitude(_) => "GPSAltitude",
        ExifTag::GPSSpeedRef(_) => "GPSSpeedRef",
        ExifTag::GPSSpeed(_) => "GPSSpeed",
        ExifTag::GPSImgDirectionRef(_) => "GPSImgDirectionRef",
        ExifTag::GPSImgDirection(_) => "GPSImgDirection",
        ExifTag::GPSDestBearingRef(_) => "GPSDestBearingRef",
        ExifTag::GPSDestBearing(_) => "GPSDestBearing",
        ExifTag::GPSDateStamp(_) => "GPSDateStamp",
        ExifTag::GPSHPositioningError(_) => "GPSHPositioningError",
        ExifTag::ImageWidth(_) => "ImageWidth",
        ExifTag::ImageHeight(_) => "ImageHeight",
        ExifTag::BitsPerSample(_) => "BitsPerSample",
        ExifTag::Compression(_) => "Compression",
        ExifTag::PhotometricInterpretation(_) => "PhotometricInterpretation",
        ExifTag::ImageDescription(_) => "ImageDescription",
        ExifTag::StripOffsets(..) => "StripOffsets",
        ExifTag::SamplesPerPixel(_) => "SamplesPerPixel",
        ExifTag::RowsPerStrip(_) => "RowsPerStrip",
        ExifTag::StripByteCounts(..) => "StripByteCounts",
        ExifTag::PlanarConfiguration(_) => "PlanarConfiguration",
        ExifTag::ISOSpeed(_) => "ISOSpeed",
        ExifTag::GPSTimeStamp(_) => "GPSTimeStamp",
        _ => "Unknown",
    }
    .to_string()
}

pub fn build_photo_routes(
    db_pool: DbPool,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let api_photos_list = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<PhotoQuery>())
        .and(with_db(db_pool.clone()))
        .and_then(list_photos);

    let api_photo_get = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(get_photo);

    let api_photo_file = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("file"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(get_photo_file);

    let api_photo_video = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("video"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<VideoQuery>())
        .and(warp::header::headers_cloned())
        .and(with_db(db_pool.clone()))
        .and_then(get_video_file);

    let api_photo_favorite = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("favorite"))
        .and(warp::path::end())
        .and(warp::put())
        .and(warp::body::json::<FavoriteRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(toggle_favorite);

    let api_photo_timeline = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path("timeline"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(get_timeline);

    let api_photo_exif = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("exif"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool))
        .and_then(get_photo_exif);

    api_photos_list
        .or(api_photo_get)
        .or(api_photo_file)
        .or(api_photo_video)
        .or(api_photo_favorite)
        .or(api_photo_timeline)
        .or(api_photo_exif)
}
