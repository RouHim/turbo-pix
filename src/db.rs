use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, NaiveDateTime, Utc};
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, Row};

use crate::media_facts::MediaFactsIndex;

pub use crate::db_pool::{create_db_pool, delete_orphaned_photos, vacuum_database, DbPool};
pub use crate::db_types::{SearchQuery, TimelineData, TimelineDensity};

/// `skip(offset).take(limit)` over an in-memory result set. Both positions are
/// client-supplied: a negative value is treated as "no rows skipped" rather
/// than panicking on the cast.
fn paginate(photos: Vec<Photo>, offset: i64, limit: i64) -> Vec<Photo> {
    let offset = usize::try_from(offset).unwrap_or(0);
    let limit = usize::try_from(limit).unwrap_or(0);
    photos.into_iter().skip(offset).take(limit).collect()
}

/// Photo entity with metadata stored as JSON
/// Breaking change: All EXIF/camera/location/video metadata moved to `metadata` JSON field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Photo {
    // === CORE IDENTIFICATION ===
    pub hash_sha256: String,
    pub file_path: String,
    pub filename: String,
    pub file_size: i64,
    pub mime_type: Option<String>,

    // === COMPUTATIONAL (used in application logic) ===
    /// Capture date. TRANSIENT: the DB stores no date any more, so `from_row`
    /// always yields `None`; only `MediaFactsIndex::enrich` fills it in (from
    /// the file) on the way out to a response.
    #[serde(deserialize_with = "deserialize_optional_datetime")]
    pub taken_at: Option<DateTime<Utc>>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub orientation: Option<i32>,
    pub duration: Option<f64>, // Video duration in seconds

    // === UI STATE ===
    pub thumbnail_path: Option<String>,
    pub has_thumbnail: Option<bool>,
    pub blurhash: Option<String>,
    pub is_favorite: Option<bool>,
    pub semantic_vector_indexed: Option<bool>,

    // === METADATA (JSON blob) ===
    /// Contains: camera{make,model,lens_make,lens_model}, settings{iso,aperture,...},
    /// location{city,...}, video{codec,audio_codec,bitrate,frame_rate}. The
    /// coordinates in `location` are response-only: [`MediaFactsIndex::enrich`]
    /// merges them in from the file, and [`stored_metadata`] strips them again
    /// before any write.
    #[serde(deserialize_with = "deserialize_json_value")]
    pub metadata: serde_json::Value,

    // === SYSTEM TIMESTAMPS ===
    #[serde(deserialize_with = "deserialize_datetime", rename = "file_modified")]
    pub date_modified: DateTime<Utc>,
    #[serde(deserialize_with = "deserialize_optional_datetime")]
    pub date_indexed: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "deserialize_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "deserialize_datetime")]
    pub updated_at: DateTime<Utc>,
}

// Custom deserializers for handling SQLite TEXT -> Rust DateTime conversion
fn deserialize_datetime<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    parse_datetime(&s).ok_or_else(|| serde::de::Error::custom("invalid datetime format"))
}

fn deserialize_optional_datetime<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Deserialize::deserialize(deserializer)?;
    Ok(s.and_then(|s| parse_datetime(&s)))
}

fn deserialize_json_value<'de, D>(deserializer: D) -> Result<serde_json::Value, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    s.parse()
        .map_err(|_| {
            log::warn!("Failed to parse metadata JSON, using empty object");
            D::Error::custom("invalid JSON")
        })
        .or(Ok(json!({})))
}

/// The single serializer for `photos.metadata`: the value to store in a row.
///
/// Capture coordinates are file facts (the DB stores none), so the two
/// coordinate keys are stripped from `location` on every write. `location`
/// itself is kept — city and any other key survive, and an emptied
/// `location` object is deliberately not collapsed (removing a key is not a
/// reason to change the shape other code reads).
fn stored_metadata(metadata: &serde_json::Value) -> String {
    let mut sanitized = metadata.clone();
    if let Some(location) = sanitized
        .get_mut("location")
        .and_then(serde_json::Value::as_object_mut)
    {
        location.remove("latitude");
        location.remove("longitude");
    }
    sanitized.to_string()
}

/// Parses a timestamp, accepting both RFC3339 ("2026-01-04T16:17:10Z")
/// and SQLite's format ("2026-01-04 16:17:10", produced by `datetime('now')`
/// and `CURRENT_TIMESTAMP`).
pub(crate) fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
    // Try RFC3339 first (e.g., "2026-01-04T16:17:10Z")
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| {
            // Try SQLite datetime format (e.g., "2026-01-04 16:17:10")
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|ndt| DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc))
        })
}

fn build_general_search_condition() -> &'static str {
    "(filename LIKE ? ESCAPE '\\' OR json_extract(metadata, '$.camera.make') LIKE ? ESCAPE '\\' OR json_extract(metadata, '$.camera.model') LIKE ? ESCAPE '\\' OR json_extract(metadata, '$.location.city') LIKE ? ESCAPE '\\')"
}

/// Escape LIKE wildcards (`%`, `_`) and the escape character itself so user
/// input matches literally. Without this, a query like `IMG_2024` also matches
/// `IMGX2024` (`_` = any single char) and a `%` in the query matches every row
/// (LIKE '%%'). Backslash must be escaped FIRST or the other escapes would
/// produce `\%` sequences that the ESCAPE clause then consumes.
fn escape_like(token: &str) -> String {
    token
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn add_general_search_params(params: &mut Vec<String>, query: &str) {
    let pattern = format!("%{}%", escape_like(query));
    params.push(pattern.clone());
    params.push(pattern.clone());
    params.push(pattern.clone());
    params.push(pattern);
}

/// Builds a validated ORDER BY clause (`<field> <order>, hash_sha256 <order>`)
/// for photo listings. `sort`/`order` are client-supplied but whitelisted by
/// the match; the `hash_sha256` tiebreak keeps pagination deterministic.
///
/// Date ordering is NOT in here: `taken_at` lives in the files, so the DB
/// cannot order by it. [`sort_photos`] owns every date-ordered (and the
/// default) listing; this clause is only used on the SQL fast path, which
/// [`is_sql_sort`] gates to the explicit column sorts below.
pub(crate) fn build_order_clause(sort: Option<&str>, order: Option<&str>) -> String {
    let sort_field = match sort {
        Some("filename") | Some("name") => "filename",
        Some("file_size") | Some("size") => "file_size",
        _ => "created_at", // default (the only remaining column sort)
    };
    let sort_order = match order {
        Some("asc") => "ASC",
        _ => "DESC", // default
    };
    format!("{sort_field} {sort_order}, hash_sha256 {sort_order}")
}

/// Whether `sort` names an explicit non-date column sort — the only case a
/// listing may let SQLite order and page the result itself. Everything else
/// (the default listing and `sort=date`) orders on the file-derived date,
/// which only exists in the in-memory index.
pub(crate) fn is_sql_sort(sort: Option<&str>) -> bool {
    matches!(
        sort,
        Some("filename") | Some("name") | Some("file_size") | Some("size") | Some("created_at")
    )
}

/// Orders `photos` in memory by the listing sort contract, on the
/// file-derived `taken_at` an enriching index put on each row (the DB column
/// is gone). The comparator mirrors SQLite exactly so pagination stays
/// deterministic:
///
/// - an unknown date is `None`, which orders *before* every `Some` — SQLite's
///   `NULL` first for `ASC`; reversing for `DESC` puts it last;
/// - equal primary values tie-break on `hash_sha256` in the same direction as
///   the sort, exactly like `..., hash_sha256 ASC|DESC`.
pub(crate) fn sort_photos(photos: &mut [Photo], sort: Option<&str>, order: Option<&str>) {
    let ascending = match order {
        Some("asc") => true,
        _ => false, // default
    };
    photos.sort_by(|a, b| {
        let primary = match sort {
            Some("filename") | Some("name") => a.filename.cmp(&b.filename),
            Some("file_size") | Some("size") => a.file_size.cmp(&b.file_size),
            Some("created_at") => a.created_at.cmp(&b.created_at),
            // `date` and the default both order by the capture date.
            _ => a.taken_at.cmp(&b.taken_at),
        };
        let ordering = primary.then_with(|| a.hash_sha256.cmp(&b.hash_sha256));
        if ascending {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

impl FromRow<'_, sqlx::sqlite::SqliteRow> for Photo {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;

        Ok(Photo {
            hash_sha256: row.try_get("hash_sha256")?,
            file_path: row.try_get("file_path")?,
            filename: row.try_get("filename")?,
            file_size: row.try_get("file_size")?,
            mime_type: row.try_get("mime_type")?,
            // The DB keeps no capture date (the column is dropped in the
            // schema that follows this change); the file-derived value is
            // attached by `MediaFactsIndex::enrich` before any response.
            taken_at: None,
            width: row.try_get("width")?,
            height: row.try_get("height")?,
            orientation: row.try_get("orientation")?,
            duration: row.try_get("duration")?,
            thumbnail_path: row.try_get("thumbnail_path")?,
            has_thumbnail: row.try_get("has_thumbnail")?,
            blurhash: row.try_get("blurhash")?,
            is_favorite: row.try_get("is_favorite")?,
            semantic_vector_indexed: row.try_get("semantic_vector_indexed")?,
            metadata: row
                .try_get::<String, _>("metadata")?
                .parse()
                .unwrap_or_else(|e| {
                    log::warn!("Failed to parse metadata JSON for photo: {}", e);
                    json!({})
                }),
            date_modified: parse_datetime(&row.try_get::<String, _>("file_modified")?).ok_or_else(
                || sqlx::Error::ColumnDecode {
                    index: "file_modified".to_string(),
                    source: Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid datetime",
                    )),
                },
            )?,
            date_indexed: row
                .try_get::<Option<String>, _>("date_indexed")?
                .and_then(|s| parse_datetime(&s)),
            created_at: parse_datetime(&row.try_get::<String, _>("created_at")?).ok_or_else(
                || sqlx::Error::ColumnDecode {
                    index: "created_at".to_string(),
                    source: Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid datetime",
                    )),
                },
            )?,
            updated_at: parse_datetime(&row.try_get::<String, _>("updated_at")?).ok_or_else(
                || sqlx::Error::ColumnDecode {
                    index: "updated_at".to_string(),
                    source: Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid datetime",
                    )),
                },
            )?,
        })
    }
}

/// Inclusive month-granular filter bounds as zero-padded `YYYY-MM` strings.
///
/// `year`/`month` is the start bound, `to_year`/`to_month` the end bound; an
/// absent end bound makes the filter a single period. A missing month means
/// January for a start bound and December for an end bound, so `year=2012`
/// still means "all of 2012" and `from=2012-03,to=2015` stops at December
/// 2015. A month outside `1..=12` is not formattable, and formatting it as a
/// literal `YYYY-MM` would silently turn it into a valid *ordering point*
/// (`2012-13` sorts after every 2012 row but before `2013-01`, so
/// `month=13&to_year=2015` would widen the range to 2013-01…2015-12); such a
/// bound yields an inverted interval that no row satisfies, keeping the old
/// equality filter's "month = 13 matches nothing" result. `to_year` without
/// `year` filters nothing, because a bound is only formed from `year`.
fn month_range_bounds(
    year: Option<i32>,
    month: Option<i32>,
    to_year: Option<i32>,
    to_month: Option<i32>,
) -> Option<(String, String)> {
    let year = year?;
    let from_month = month.unwrap_or(1);
    let end_month = match to_year {
        Some(_) => to_month.unwrap_or(12),
        None => month.unwrap_or(12),
    };
    if !(1..=12).contains(&from_month) || !(1..=12).contains(&end_month) {
        return Some(("9999-99".to_string(), "0000-00".to_string()));
    }
    let from = format!("{:04}-{:02}", year, from_month);
    let to = format!("{:04}-{:02}", to_year.unwrap_or(year), end_month);
    Some((from, to))
}

/// Whether `photo`'s file-derived capture date lies inside the inclusive
/// month-granular `bounds` from [`month_range_bounds`].
///
/// Compares the zero-padded `YYYY-MM` key lexicographically, which sorts
/// chronologically — the exact comparison the removed
/// `strftime('%Y-%m', taken_at)` SQL made. A photo whose date is unknown (no
/// facts entry, so `taken_at` stayed `None`) never matches, just as a NULL
/// `taken_at` failed the old SQL comparison.
pub(crate) fn photo_in_month_range(photo: &Photo, bounds: &(String, String)) -> bool {
    let Some(taken_at) = photo.taken_at else {
        return false;
    };
    let key = taken_at.format("%Y-%m").to_string();
    key.as_str() >= bounds.0.as_str() && key.as_str() <= bounds.1.as_str()
}

/// Builds the reusable WHERE clause for photo searches: the `q` token grammar
/// (`type:` / `is_favorite:` / `location:` / general LIKE). Returns the clause
/// (starting with `" WHERE 1=1"`) plus its string parameters in placeholder
/// order. Shared by `Photo::search_photos` and `Photo::list_all_filtered` so
/// both honor identical filter semantics. The month range is NOT here: it
/// filters a file-derived date, see [`photo_in_month_range`].
fn build_search_where(query: &SearchQuery) -> (String, Vec<String>) {
    let mut where_clause = String::from(" WHERE 1=1");
    let mut params: Vec<String> = Vec::new();

    if let Some(q) = &query.q {
        // Split on whitespace and AND per-token conditions so combined
        // queries like "sunset is_favorite:true" work. A token with an
        // unknown type:/is_favorite: value falls back to general search.
        let tokens: Vec<&str> = q.split_whitespace().collect();
        let mut i = 0;
        while i < tokens.len() {
            let token = tokens[i];
            if let Some(media_type) = token.strip_prefix("type:") {
                match media_type {
                    "video" => where_clause.push_str(" AND mime_type LIKE 'video/%'"),
                    "image" => where_clause.push_str(" AND mime_type LIKE 'image/%'"),
                    _ => {
                        // Unknown type, fall back to general search
                        where_clause.push_str(" AND ");
                        where_clause.push_str(build_general_search_condition());
                        add_general_search_params(&mut params, token);
                    }
                }
            } else if let Some(favorite_value) = token.strip_prefix("is_favorite:") {
                match favorite_value {
                    "true" => where_clause.push_str(" AND is_favorite = 1"),
                    "false" => {
                        where_clause.push_str(" AND (is_favorite = 0 OR is_favorite IS NULL)");
                    }
                    _ => {
                        // Unknown value, fall back to general search
                        where_clause.push_str(" AND ");
                        where_clause.push_str(build_general_search_condition());
                        add_general_search_params(&mut params, token);
                    }
                }
            } else if token.starts_with("location:") {
                // Absorb following words until the next prefix token or
                // end, so multi-word cities ("location:New York") keep
                // working.
                let mut city = token.strip_prefix("location:").unwrap_or("").to_string();
                while i + 1 < tokens.len()
                    && !tokens[i + 1].starts_with("type:")
                    && !tokens[i + 1].starts_with("is_favorite:")
                    && !tokens[i + 1].starts_with("location:")
                {
                    i += 1;
                    city.push(' ');
                    city.push_str(tokens[i]);
                }
                if city.trim().is_empty() {
                    // Bare "location:" token — no city to match, skip it
                    // (LIKE '%%' would match every row with a city).
                } else {
                    // Trim once: "location: New York" accumulates a leading
                    // space during absorption that would break the LIKE
                    // pattern ('% New York%' matches nothing).
                    let city = city.trim();
                    where_clause.push_str(
                        " AND json_extract(metadata, '$.location.city') LIKE ? ESCAPE '\\'",
                    );
                    params.push(format!("%{}%", escape_like(city)));
                }
            } else {
                // General search across multiple fields (filename + JSON metadata)
                where_clause.push_str(" AND ");
                where_clause.push_str(build_general_search_condition());
                add_general_search_params(&mut params, token);
            }
            i += 1;
        }
    }

    // The month-granular range is NOT part of the SQL: it filters a
    // file-derived date the database does not store. `month_range_bounds` is
    // applied in Rust by the callers, via `photo_in_month_range`.
    (where_clause, params)
}

impl Photo {
    // ===== METADATA ACCESSORS (for Rust code) =====
    // Frontend reads metadata.* directly from JSON
    // Used by photo_processor and handlers_video to build responses.

    // Camera
    pub fn camera_make(&self) -> Option<&str> {
        self.metadata.get("camera")?.get("make")?.as_str()
    }

    pub fn camera_model(&self) -> Option<&str> {
        self.metadata.get("camera")?.get("model")?.as_str()
    }

    pub fn lens_make(&self) -> Option<&str> {
        self.metadata.get("camera")?.get("lens_make")?.as_str()
    }

    pub fn lens_model(&self) -> Option<&str> {
        self.metadata.get("camera")?.get("lens_model")?.as_str()
    }

    pub fn aperture(&self) -> Option<f64> {
        self.metadata.get("settings")?.get("aperture")?.as_f64()
    }

    pub fn shutter_speed(&self) -> Option<&str> {
        self.metadata
            .get("settings")?
            .get("shutter_speed")?
            .as_str()
    }

    pub fn focal_length(&self) -> Option<f64> {
        self.metadata.get("settings")?.get("focal_length")?.as_f64()
    }

    pub fn iso(&self) -> Option<i32> {
        self.metadata
            .get("settings")?
            .get("iso")?
            .as_i64()?
            .try_into()
            .ok()
    }

    pub fn exposure_mode(&self) -> Option<&str> {
        self.metadata
            .get("settings")?
            .get("exposure_mode")?
            .as_str()
    }

    pub fn metering_mode(&self) -> Option<&str> {
        self.metadata
            .get("settings")?
            .get("metering_mode")?
            .as_str()
    }

    pub fn white_balance(&self) -> Option<&str> {
        self.metadata
            .get("settings")?
            .get("white_balance")?
            .as_str()
    }

    pub fn color_space(&self) -> Option<&str> {
        self.metadata.get("settings")?.get("color_space")?.as_str()
    }

    pub fn flash_used(&self) -> Option<bool> {
        self.metadata.get("settings")?.get("flash_used")?.as_bool()
    }

    // Video
    pub fn video_codec(&self) -> Option<&str> {
        self.metadata.get("video")?.get("codec")?.as_str()
    }

    pub fn audio_codec(&self) -> Option<&str> {
        self.metadata.get("video")?.get("audio_codec")?.as_str()
    }

    pub fn bitrate(&self) -> Option<i32> {
        self.metadata
            .get("video")?
            .get("bitrate")?
            .as_i64()?
            .try_into()
            .ok()
    }

    pub fn frame_rate(&self) -> Option<f64> {
        self.metadata.get("video")?.get("frame_rate")?.as_f64()
    }

    pub fn video_profile(&self) -> Option<&str> {
        self.metadata.get("video")?.get("profile")?.as_str()
    }

    pub fn bit_depth(&self) -> Option<u32> {
        self.metadata
            .get("video")?
            .get("bit_depth")?
            .as_u64()
            .map(|v| v as u32)
    }

    pub fn container(&self) -> Option<&str> {
        self.metadata.get("video")?.get("container")?.as_str()
    }

    /// Whether the MOOV atom is at the start of the container. Defaults to
    /// `true` when absent (absence means the record never wrote a false value).
    pub fn moov_at_start(&self) -> bool {
        self.metadata
            .get("video")
            .and_then(|v| v.get("moov_at_start"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    // ===== DATABASE OPERATIONS =====

    /// Update photo (convenience wrapper)
    pub async fn update(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let mut tx = pool.begin().await?;
        self.update_with_transaction(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Create or update photo (convenience wrapper)
    /// Use `batch_write_photos` in production for better performance
    #[cfg(test)]
    pub async fn create_or_update(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let mut tx = pool.begin().await?;
        self.create_or_update_with_transaction(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_with_pagination(
        pool: &DbPool,
        facts: &MediaFactsIndex,
        limit: i64,
        offset: i64,
        sort: Option<&str>,
        order: Option<&str>,
    ) -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>> {
        // SQL fast path: an explicit column sort with no date filter can be
        // ordered and paged by SQLite itself.
        if is_sql_sort(sort) {
            let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
                .fetch_one(pool)
                .await?;

            let query_str = format!(
                "SELECT * FROM photos ORDER BY {} LIMIT ? OFFSET ?",
                build_order_clause(sort, order)
            );

            let mut photos = sqlx::query_as::<_, Photo>(sqlx::AssertSqlSafe(query_str))
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await?;
            for photo in &mut photos {
                facts.enrich(photo);
            }
            return Ok((photos, total));
        }

        // Date-ordered (or default) listing: the DB stores no date, so the
        // whole set is ordered from the index in memory.
        let mut photos: Vec<Photo> = sqlx::query_as::<_, Photo>("SELECT * FROM photos")
            .fetch_all(pool)
            .await?;
        for photo in &mut photos {
            facts.enrich(photo);
        }
        sort_photos(&mut photos, sort, order);
        let total = photos.len() as i64;

        Ok((paginate(photos, offset, limit), total))
    }

    pub async fn find_by_hash(
        pool: &DbPool,
        hash: &str,
    ) -> Result<Option<Photo>, Box<dyn std::error::Error>> {
        let photo = sqlx::query_as::<_, Photo>("SELECT * FROM photos WHERE hash_sha256 = ?")
            .bind(hash)
            .fetch_optional(pool)
            .await?;

        Ok(photo)
    }

    /// Merge-patch the stored `photos.metadata` JSON and fill in a duration the
    /// indexer never captured, in ONE transaction: a capability record is
    /// either fully written or not written at all. Written as two autocommit
    /// statements, a failed duration write (SQLITE_BUSY under concurrent API
    /// writes, disk error, crash between them) left the `capability_version`
    /// marker behind — and that marker alone completes the record for
    /// [`crate::video_probe::record_is_complete`], so the probe would
    /// short-circuit forever after and the NULL duration would be permanent
    /// until the file changed on disk.
    ///
    /// The patch is merge-patched in one statement (SQLite `json_patch`
    /// implements RFC 7396), so concurrent capability writes cannot tear each
    /// other's metadata and unrelated keys survive untouched. RFC 7396 treats
    /// `null` literally: a null member REMOVES the stored key instead of
    /// storing a null — which is how a caller records "asked, there is none"
    /// while dropping a stale value. The capability probe does exactly that for
    /// a source whose probe found no audio track: it always sends the
    /// `audio_codec` member, so a null deletes any stale codec, and the record
    /// still reads as "probed, no audio" because the `capability_version`
    /// marker written by this same call separates it from "never probed". A
    /// fact whose ABSENCE could not be told apart from "never probed" needs a
    /// sentinel instead, the way `no_video_stream` does.
    ///
    /// A duration is written only where the indexer captured none; a positive
    /// stored value is never overwritten.
    pub async fn persist_capability_and_duration(
        pool: &DbPool,
        hash: &str,
        patch: &serde_json::Value,
        duration_secs: Option<f64>,
    ) -> Result<(), sqlx::Error> {
        let patch = patch.to_string();
        let mut tx = pool.begin().await?;
        sqlx::query("UPDATE photos SET metadata = json_patch(metadata, ?1) WHERE hash_sha256 = ?2")
            .bind(&patch)
            .bind(hash)
            .execute(&mut *tx)
            .await?;
        if let Some(duration_secs) = duration_secs {
            sqlx::query(
                "UPDATE photos SET duration = ?1 \
                 WHERE hash_sha256 = ?2 AND (duration IS NULL OR duration <= 0)",
            )
            .bind(duration_secs)
            .bind(hash)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Check if a photo exists with matching path, size, and modification time
    /// Returns the full Photo if unchanged, None if new/modified
    pub async fn find_unchanged_photo(
        pool: &DbPool,
        file_path: &str,
        file_size: i64,
        date_modified: DateTime<Utc>,
    ) -> Result<Option<Photo>, Box<dyn std::error::Error>> {
        let photo = sqlx::query_as::<_, Photo>(
            "SELECT * FROM photos WHERE file_path = ? AND file_size = ? AND file_modified = ?",
        )
        .bind(file_path)
        .bind(file_size)
        .bind(date_modified.to_rfc3339())
        .fetch_optional(pool)
        .await?;

        Ok(photo)
    }

    /// Create photo using an existing transaction (for batch operations)
    #[cfg(test)]
    pub async fn create_with_transaction(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        sqlx::query(
            r#"
            INSERT INTO photos (
                hash_sha256, file_path, filename, file_size, mime_type,
                width, height, orientation, duration,
                thumbnail_path, has_thumbnail, blurhash, is_favorite, semantic_vector_indexed,
                metadata,
                file_modified, date_indexed, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
            )
            "#,
        )
        .bind(&self.hash_sha256)
        .bind(&self.file_path)
        .bind(&self.filename)
        .bind(self.file_size)
        .bind(&self.mime_type)
        .bind(self.width)
        .bind(self.height)
        .bind(self.orientation)
        .bind(self.duration)
        .bind(&self.thumbnail_path)
        .bind(self.has_thumbnail)
        .bind(&self.blurhash)
        .bind(self.is_favorite.unwrap_or(false))
        .bind(self.semantic_vector_indexed.unwrap_or(false))
        .bind(stored_metadata(&self.metadata))
        .bind(self.date_modified.to_rfc3339())
        .bind(self.date_indexed.map(|dt| dt.to_rfc3339()))
        .bind(Utc::now().to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    /// Create photo (test helper - use create_with_transaction for production)
    #[cfg(test)]
    pub async fn create(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let mut tx = pool.begin().await?;
        self.create_with_transaction(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Update photo using an existing transaction (for batch operations)
    pub async fn update_with_transaction(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        sqlx::query(
            r#"
            UPDATE photos SET
                file_path = ?, filename = ?, file_size = ?, mime_type = ?,
                width = ?, height = ?, orientation = ?, duration = ?,
                thumbnail_path = ?, has_thumbnail = ?, blurhash = ?, is_favorite = ?, semantic_vector_indexed = ?,
                metadata = ?,
                file_modified = ?, updated_at = ?
            WHERE hash_sha256 = ?
            "#,
        )
        .bind(&self.file_path)
        .bind(&self.filename)
        .bind(self.file_size)
        .bind(&self.mime_type)
        .bind(self.width)
        .bind(self.height)
        .bind(self.orientation)
        .bind(self.duration)
        .bind(&self.thumbnail_path)
        .bind(self.has_thumbnail)
        .bind(&self.blurhash)
        .bind(self.is_favorite.unwrap_or(false))
        .bind(self.semantic_vector_indexed.unwrap_or(false))
        .bind(stored_metadata(&self.metadata))
        .bind(self.date_modified.to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .bind(&self.hash_sha256)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    /// Update photo using old hash in WHERE clause (for operations that change the hash)
    ///
    /// # Transaction Requirement
    ///
    /// Caller must provide an active transaction: rewriting `hash_sha256` (the parent
    /// key referenced by `housekeeping_candidates.photo_hash` via `ON DELETE CASCADE`
    /// with no `ON UPDATE`) requires deleting the stale candidate rows inside the
    /// same transaction (see `image_editor::rotate_image`). Album memberships
    /// cascade automatically on migrated databases (the `create_manual_albums`
    /// migration defines `ON UPDATE CASCADE`) and are repointed explicitly
    /// below for the rest.
    pub async fn update_with_old_hash(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        old_hash: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        sqlx::query(
            r#"
            UPDATE photos SET
                hash_sha256 = ?,
                file_path = ?, filename = ?, file_size = ?, mime_type = ?,
                width = ?, height = ?, orientation = ?, duration = ?,
                thumbnail_path = ?, has_thumbnail = ?, blurhash = ?, is_favorite = ?, semantic_vector_indexed = ?,
                metadata = ?,
                file_modified = ?, updated_at = ?
            WHERE hash_sha256 = ?
            "#,
        )
        .bind(&self.hash_sha256)
        .bind(&self.file_path)
        .bind(&self.filename)
        .bind(self.file_size)
        .bind(&self.mime_type)
        .bind(self.width)
        .bind(self.height)
        .bind(self.orientation)
        .bind(self.duration)
        .bind(&self.thumbnail_path)
        .bind(self.has_thumbnail)
        .bind(&self.blurhash)
        .bind(self.is_favorite.unwrap_or(false))
        .bind(self.semantic_vector_indexed.unwrap_or(false))
        .bind(stored_metadata(&self.metadata))
        .bind(self.date_modified.to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .bind(old_hash)
        .execute(&mut **tx)
        .await?;
        // A stale-snapshot write (the row's hash changed while we were
        // rotating — e.g. a second overlapping rotate request) matches 0
        // rows; committing silently would leave the DB divergent from the
        // file on disk. Fail loudly so the caller aborts the transaction.
        let affected = sqlx::query("SELECT changes()")
            .fetch_one(&mut **tx)
            .await?
            .try_get::<i64, _>(0)?;
        if affected == 0 {
            return Err(format!(
                "Photo with hash {} no longer exists (stale snapshot?) — update matched 0 rows",
                old_hash
            )
            .into());
        }
        // Repoint album memberships to the new hash. On migrated databases
        // the ON UPDATE CASCADE already moved these rows (both statements
        // below match 0 rows); on older schemas the explicit repoint keeps
        // the photo in its albums instead of failing the parent-key update.
        // Copy-then-delete (rather than a bare UPDATE) also survives the
        // degenerate case where the new hash is already a member.
        // Byte-identical rotate output (e.g. a solid-color PNG) yields
        // new hash == old hash: the photos UPDATE above is then a harmless
        // self-rewrite, but copy-then-delete below would be destructive —
        // INSERT OR IGNORE no-ops (the rows already match) while the DELETE
        // removes every membership. Skip the repoint: nothing moved.
        if self.hash_sha256 != old_hash {
            sqlx::query(
                "INSERT OR IGNORE INTO album_members (album_id, photo_hash, added_at)
                 SELECT album_id, ?1, added_at FROM album_members WHERE photo_hash = ?2",
            )
            .bind(&self.hash_sha256)
            .bind(old_hash)
            .execute(&mut **tx)
            .await?;
            sqlx::query("DELETE FROM album_members WHERE photo_hash = ?")
                .bind(old_hash)
                .execute(&mut **tx)
                .await?;
        }
        Ok(())
    }

    /// Create or update photo using an existing transaction (for batch operations)
    ///
    /// # Transaction Requirement
    ///
    /// **IMPORTANT**: This method MUST be called within an active database transaction.
    /// The operation consists of two separate SQL statements (DELETE + UPSERT) that must
    /// execute atomically to prevent race conditions when the same file_path is processed
    /// concurrently or a file's hash changes between operations.
    ///
    /// # Behavior
    ///
    /// 1. Deletes any existing photo with the same `file_path` but different `hash_sha256`
    ///    (the hash is derived from the file PATH, so an in-app rotation — which
    ///    rewrites the bytes — produces a different content hash than the rescan's
    ///    path hash; content changes at the same path keep the hash, so cache
    ///    invalidation is handled by the size+mtime content version in the
    ///    thumbnail/transcode/collage keys, NOT by this branch)
    /// 2. Uses UPSERT to insert if new, or update if `hash_sha256` already exists
    ///
    /// # Safety
    ///
    /// Caller must ensure this is called within a transaction. The `batch_write_photos`
    /// function in `scheduler.rs` demonstrates correct usage.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut tx = pool.begin().await?;
    /// sqlx::query("BEGIN IMMEDIATE").execute(&mut *tx).await?;
    /// for photo in photos {
    ///     photo.create_or_update_with_transaction(&mut tx).await?;
    /// }
    /// tx.commit().await?;
    /// ```
    pub async fn create_or_update_with_transaction(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // First, delete any existing photo with same file_path but different hash
        // This handles the case where a file was modified (hash changed)
        //
        // Capture user state from the row being replaced: the UPSERT below can
        // only preserve is_favorite via COALESCE when the row still exists
        // (same hash). After an in-app rotation the row is keyed by a content
        // hash while the next rescan re-derives the path hash, so the row is
        // deleted before the upsert — without this carry the favorite flag
        // would silently reset on every rotated photo.
        let replaced_favorite: Option<Option<bool>> = sqlx::query_scalar(
            "SELECT is_favorite FROM photos WHERE file_path = ? AND hash_sha256 != ?",
        )
        .bind(&self.file_path)
        .bind(&self.hash_sha256)
        .fetch_optional(&mut **tx)
        .await?;
        // Carry album memberships across the re-key: the DELETE below
        // cascades the replaced rows' album_members entries, so snapshot
        // them first and re-attach to the new hash after the upsert
        // (mirrors the is_favorite carry above). Gated on a replaced row
        // existing so the common path costs no extra query.
        let carried_members: Vec<(i64, String)> = if replaced_favorite.is_some() {
            sqlx::query_as(
                "SELECT album_id, COALESCE(added_at, datetime('now')) FROM album_members
                 WHERE photo_hash IN (
                     SELECT hash_sha256 FROM photos WHERE file_path = ? AND hash_sha256 != ?
                 )",
            )
            .bind(&self.file_path)
            .bind(&self.hash_sha256)
            .fetch_all(&mut **tx)
            .await?
        } else {
            Vec::new()
        };

        sqlx::query("DELETE FROM photos WHERE file_path = ? AND hash_sha256 != ?")
            .bind(&self.file_path)
            .bind(&self.hash_sha256)
            .execute(&mut **tx)
            .await?;

        // Then use UPSERT to insert or update by hash
        sqlx::query(
            r#"
            INSERT INTO photos (
                hash_sha256, file_path, filename, file_size, mime_type,
                width, height, orientation, duration,
                thumbnail_path, has_thumbnail, blurhash, is_favorite, semantic_vector_indexed,
                metadata,
                file_modified, date_indexed, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
            )
            ON CONFLICT(hash_sha256) DO UPDATE SET
                file_path = excluded.file_path,
                filename = excluded.filename,
                file_size = excluded.file_size,
                mime_type = excluded.mime_type,
                width = excluded.width,
                height = excluded.height,
                orientation = excluded.orientation,
                duration = excluded.duration,
                thumbnail_path = excluded.thumbnail_path,
                has_thumbnail = excluded.has_thumbnail,
                blurhash = excluded.blurhash,
                is_favorite = COALESCE(photos.is_favorite, excluded.is_favorite),
                semantic_vector_indexed = excluded.semantic_vector_indexed,
                -- RFC 7396 merge, not a wholesale replace: the fresh extraction
                -- owns only the keys it emits, so keys written by other paths
                -- (location.city, video.capability_version, video.no_video_stream)
                -- survive the rescan. An explicit null in the fresh extraction
                -- still deletes the key, so coordinates the file no longer has
                -- are cleared. A non-object stored value (NULL) merges from {}.
                metadata = json_patch(
                    CASE WHEN json_type(photos.metadata) = 'object' THEN photos.metadata ELSE '{}' END,
                    excluded.metadata),
                file_modified = excluded.file_modified,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&self.hash_sha256)
        .bind(&self.file_path)
        .bind(&self.filename)
        .bind(self.file_size)
        .bind(&self.mime_type)
        .bind(self.width)
        .bind(self.height)
        .bind(self.orientation)
        .bind(self.duration)
        .bind(&self.thumbnail_path)
        .bind(self.has_thumbnail)
        .bind(&self.blurhash)
        .bind(
            replaced_favorite
                .flatten()
                .or(self.is_favorite)
                .unwrap_or(false),
        )
        .bind(self.semantic_vector_indexed.unwrap_or(false))
        .bind(stored_metadata(&self.metadata))
        .bind(self.date_modified.to_rfc3339())
        .bind(self.date_indexed.map(|dt| dt.to_rfc3339()))
        .bind(Utc::now().to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .execute(&mut **tx)
        .await?;

        // Re-attach the memberships snapshotted above to the new hash. The
        // parent row now exists again, so the FK is satisfied with or
        // without ON UPDATE CASCADE; OR IGNORE absorbs degenerate
        // double-memberships from merging duplicate paths.
        for (album_id, added_at) in &carried_members {
            sqlx::query(
                "INSERT OR IGNORE INTO album_members (album_id, photo_hash, added_at)
                 VALUES (?, ?, ?)",
            )
            .bind(album_id)
            .bind(&self.hash_sha256)
            .bind(added_at)
            .execute(&mut **tx)
            .await?;
        }

        Ok(())
    }

    pub async fn search_photos(
        pool: &DbPool,
        facts: &MediaFactsIndex,
        query: &SearchQuery,
        limit: i64,
        offset: i64,
        sort: Option<&str>,
        order: Option<&str>,
    ) -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>> {
        let (where_clause, params) = build_search_where(query);
        let bounds = month_range_bounds(query.year, query.month, query.to_year, query.to_month);

        // SQL fast path: no date filter and an explicit column sort.
        if bounds.is_none() && is_sql_sort(sort) {
            // Get total count
            let count_sql = format!("SELECT COUNT(*) FROM photos{}", where_clause);
            let mut count_query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql));
            for param in &params {
                count_query = count_query.bind(param);
            }
            let total = count_query.fetch_one(pool).await?;

            let data_sql = format!(
                "SELECT * FROM photos{} ORDER BY {} LIMIT ? OFFSET ?",
                where_clause,
                build_order_clause(sort, order)
            );

            let mut data_query = sqlx::query_as::<_, Photo>(sqlx::AssertSqlSafe(data_sql));
            for param in &params {
                data_query = data_query.bind(param);
            }
            data_query = data_query.bind(limit).bind(offset);

            let mut photos = data_query.fetch_all(pool).await?;
            for photo in &mut photos {
                facts.enrich(photo);
            }
            return Ok((photos, total));
        }

        // The month filter is on a file-derived date and a date-ordered sort
        // has nothing to order by in SQL: fetch every row the filters select,
        // then filter, order and page with the index.
        let data_sql = format!("SELECT * FROM photos{}", where_clause);
        let mut data_query = sqlx::query_as::<_, Photo>(sqlx::AssertSqlSafe(data_sql));
        for param in &params {
            data_query = data_query.bind(param);
        }
        let mut photos = data_query.fetch_all(pool).await?;

        for photo in &mut photos {
            facts.enrich(photo);
        }
        if let Some(bounds) = &bounds {
            photos.retain(|photo| photo_in_month_range(photo, bounds));
        }
        sort_photos(&mut photos, sort, order);
        let total = photos.len() as i64;

        Ok((paginate(photos, offset, limit), total))
    }

    /// Returns every photo matching `query` (optionally scoped to one album)
    /// with no pagination. The Map view needs the complete filtered set: it
    /// plots the geo-located subset and hands the same sorted array to the
    /// viewer so next/previous span exactly the grid's result set
    /// (FR-005, FR-011). Album scoping mirrors the grid's album detail view,
    /// which ignores q/year/month.
    pub async fn list_all_filtered(
        pool: &DbPool,
        facts: &MediaFactsIndex,
        query: &SearchQuery,
        sort: Option<&str>,
        order: Option<&str>,
        album: Option<i64>,
    ) -> Result<Vec<Photo>, Box<dyn std::error::Error>> {
        let (mut where_clause, params) = build_search_where(query);

        if album.is_some() {
            where_clause.push_str(
                " AND hash_sha256 IN (SELECT photo_hash FROM album_members WHERE album_id = ?)",
            );
        }
        let bounds = month_range_bounds(query.year, query.month, query.to_year, query.to_month);

        // SQL fast path: no date filter and an explicit column sort. The rows
        // still get enriched — the map hands this array straight to the
        // response, coordinates included.
        if bounds.is_none() && is_sql_sort(sort) {
            let sql = format!(
                "SELECT * FROM photos{} ORDER BY {}",
                where_clause,
                build_order_clause(sort, order)
            );

            let mut data_query = sqlx::query_as::<_, Photo>(sqlx::AssertSqlSafe(sql));
            for param in &params {
                data_query = data_query.bind(param);
            }
            if let Some(album_id) = album {
                data_query = data_query.bind(album_id);
            }

            let mut photos = data_query.fetch_all(pool).await?;
            for photo in &mut photos {
                facts.enrich(photo);
            }
            return Ok(photos);
        }

        // No pagination here (the Map needs the complete filtered set), but a
        // month filter or a date sort still has to run on the file-derived
        // dates.
        let sql = format!("SELECT * FROM photos{}", where_clause);
        let mut data_query = sqlx::query_as::<_, Photo>(sqlx::AssertSqlSafe(sql));
        for param in &params {
            data_query = data_query.bind(param);
        }
        if let Some(album_id) = album {
            data_query = data_query.bind(album_id);
        }

        let mut photos = data_query.fetch_all(pool).await?;
        for photo in &mut photos {
            facts.enrich(photo);
        }
        if let Some(bounds) = &bounds {
            photos.retain(|photo| photo_in_month_range(photo, bounds));
        }
        sort_photos(&mut photos, sort, order);

        Ok(photos)
    }

    pub async fn get_timeline_data(
        pool: &DbPool,
        facts: &MediaFactsIndex,
    ) -> Result<TimelineData, Box<dyn std::error::Error>> {
        // The dates live in the files; the DB only supplies the paths to look
        // up in the index.
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT hash_sha256, file_path FROM photos")
                .fetch_all(pool)
                .await?;

        let mut min_date: Option<DateTime<Utc>> = None;
        let mut max_date: Option<DateTime<Utc>> = None;
        let mut buckets: BTreeMap<(i32, i32), i64> = BTreeMap::new();

        for (_, file_path) in rows {
            let Some(taken_at) = facts.get(&file_path).and_then(|facts| facts.taken_at) else {
                continue;
            };
            min_date = Some(min_date.map_or(taken_at, |current| current.min(taken_at)));
            max_date = Some(max_date.map_or(taken_at, |current| current.max(taken_at)));
            *buckets
                .entry((taken_at.year(), taken_at.month() as i32))
                .or_default() += 1;
        }

        // `BTreeMap<(year, month), _>` iterates ordered by year then month.
        let density = buckets
            .into_iter()
            .map(|((year, month), count)| TimelineDensity { year, month, count })
            .collect();

        Ok(TimelineData {
            min_date: min_date.map(|dt| dt.to_rfc3339()),
            max_date: max_date.map(|dt| dt.to_rfc3339()),
            density,
        })
    }
}

impl From<crate::indexer::ProcessedPhoto> for Photo {
    fn from(processed: crate::indexer::ProcessedPhoto) -> Self {
        // Build metadata JSON from ProcessedPhoto fields
        let mut camera = serde_json::Map::new();
        if let Some(make) = processed.camera_make {
            camera.insert("make".to_string(), json!(make));
        }
        if let Some(model) = processed.camera_model {
            camera.insert("model".to_string(), json!(model));
        }
        if let Some(lens_make) = processed.lens_make {
            camera.insert("lens_make".to_string(), json!(lens_make));
        }
        if let Some(lens_model) = processed.lens_model {
            camera.insert("lens_model".to_string(), json!(lens_model));
        }

        let mut settings = serde_json::Map::new();
        if let Some(iso) = processed.iso {
            settings.insert("iso".to_string(), json!(iso));
        }
        if let Some(aperture) = processed.aperture {
            settings.insert("aperture".to_string(), json!(aperture));
        }
        if let Some(shutter_speed) = processed.shutter_speed {
            settings.insert("shutter_speed".to_string(), json!(shutter_speed));
        }
        if let Some(focal_length) = processed.focal_length {
            settings.insert("focal_length".to_string(), json!(focal_length));
        }
        if let Some(exposure_mode) = processed.exposure_mode {
            settings.insert("exposure_mode".to_string(), json!(exposure_mode));
        }
        if let Some(metering_mode) = processed.metering_mode {
            settings.insert("metering_mode".to_string(), json!(metering_mode));
        }
        if let Some(white_balance) = processed.white_balance {
            settings.insert("white_balance".to_string(), json!(white_balance));
        }
        if let Some(color_space) = processed.color_space {
            settings.insert("color_space".to_string(), json!(color_space));
        }
        if let Some(flash_used) = processed.flash_used {
            settings.insert("flash_used".to_string(), json!(flash_used));
        }


        let mut video = serde_json::Map::new();
        if let Some(codec) = processed.video_codec {
            video.insert("codec".to_string(), json!(codec));
        }
        if let Some(audio_codec) = processed.audio_codec {
            video.insert("audio_codec".to_string(), json!(audio_codec));
        }
        if let Some(bitrate) = processed.bitrate {
            video.insert("bitrate".to_string(), json!(bitrate));
        }
        if let Some(frame_rate) = processed.frame_rate {
            video.insert("frame_rate".to_string(), json!(frame_rate));
        }
        if let Some(profile) = processed.video_profile {
            video.insert("profile".to_string(), json!(profile));
        }
        if let Some(bit_depth) = processed.bit_depth {
            video.insert("bit_depth".to_string(), json!(bit_depth));
        }
        if let Some(container) = processed.container {
            video.insert("container".to_string(), json!(container));
        }
        // Only record moov_at_start when false — absence means true, which
        // avoids bloating every record.
        if !processed.moov_at_start {
            video.insert("moov_at_start".to_string(), json!(false));
        }

        let mut metadata = serde_json::Map::new();
        if !camera.is_empty() {
            metadata.insert("camera".to_string(), json!(camera));
        }
        if !settings.is_empty() {
            metadata.insert("settings".to_string(), json!(settings));
        }

        if !video.is_empty() {
            metadata.insert("video".to_string(), json!(video));
        }

        Photo {
            hash_sha256: processed
                .hash_sha256
                .expect("ProcessedPhoto must have hash_sha256"),
            file_path: processed.file_path,
            filename: processed.filename,
            file_size: processed.file_size,
            mime_type: processed.mime_type,
            // Transient: the DB stores no date, the scan publishes the file's
            // facts to the index instead.
            taken_at: None,
            width: processed.width,
            height: processed.height,
            orientation: processed.orientation,
            duration: processed.duration,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: processed.blurhash,
            is_favorite: None,
            semantic_vector_indexed: processed.semantic_vector_indexed,
            metadata: json!(metadata),
            date_modified: processed.date_modified,
            date_indexed: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

#[cfg(test)]
pub async fn create_test_db_pool() -> Result<DbPool, Box<dyn std::error::Error>> {
    crate::db_pool::create_in_memory_pool().await
}

#[cfg(test)]
pub async fn create_in_memory_pool() -> Result<DbPool, Box<dyn std::error::Error>> {
    crate::db_pool::create_in_memory_pool().await
}

/// Get all photo file paths from the database
#[cfg(test)]
pub async fn get_all_photo_paths(pool: &DbPool) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let paths: Vec<String> = sqlx::query_scalar("SELECT file_path FROM photos ORDER BY file_path")
        .fetch_all(pool)
        .await?;
    Ok(paths)
}

/// Get file paths of photos that need semantic vector indexing (Phase 2)
pub async fn get_paths_needing_semantic_indexing(
    pool: &DbPool,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let paths: Vec<String> = sqlx::query_scalar(
        "SELECT file_path FROM photos WHERE semantic_vector_indexed = 0 OR semantic_vector_indexed IS NULL ORDER BY file_path"
    )
    .fetch_all(pool)
    .await?;
    Ok(paths)
}

/// Photos still needing a location label, with the coordinates the file
/// carries. Coordinates live in the index (the DB stores none), so the query
/// narrows to unresolved rows and the index decides which of them have a
/// usable coordinate pair.
pub async fn get_photos_needing_geo_resolution(
    pool: &DbPool,
    facts: &MediaFactsIndex,
) -> Result<Vec<(String, f64, f64)>, Box<dyn std::error::Error>> {
    let paths: Vec<String> = sqlx::query_scalar(
        "SELECT file_path FROM photos
         WHERE geo_location_resolved IS NULL OR geo_location_resolved = 0
         ORDER BY file_path",
    )
    .fetch_all(pool)
    .await?;

    let mut photos = Vec::new();
    for file_path in paths {
        let Some(entry) = facts.get(&file_path) else {
            continue;
        };
        if let (Some(latitude), Some(longitude)) = (entry.latitude, entry.longitude) {
            photos.push((file_path, latitude, longitude));
        }
    }

    Ok(photos)
}

pub async fn mark_photo_geo_resolved(
    pool: &DbPool,
    file_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query("UPDATE photos SET geo_location_resolved = 1 WHERE file_path = ?")
        .bind(file_path)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_photo_city(
    pool: &DbPool,
    file_path: &str,
    city: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(city) = city {
        sqlx::query("UPDATE photos SET metadata = json_set(metadata, '$.location.city', ?) WHERE file_path = ?")
            .bind(city)
            .bind(file_path)
            .execute(pool)
            .await?;
    }

    mark_photo_geo_resolved(pool, file_path).await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::Datelike;
    use sqlx::Row;

    use crate::media_facts::{test_facts, test_facts_with_coords, MediaFacts, MediaFactsIndex};

    pub(crate) fn create_test_photo(filename: String, hash: String) -> Photo {
        // Ensure hash is 64 characters for SHA256
        let hash_64 = if hash.len() < 64 {
            format!("{:0<64}", hash)
        } else {
            hash
        };
        // No `taken_at`: the DB stores no date any more; tests that need one
        // seed the facts index (see `create_photo_with_facts`).
        Photo {
            hash_sha256: hash_64,
            file_path: format!("./test/{}", filename),
            filename,
            file_size: 1024,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: None,
            width: Some(1920),
            height: Some(1080),
            orientation: None,
            duration: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: None,
            is_favorite: None,
            semantic_vector_indexed: Some(false),
            metadata: json!({}), // Empty metadata for tests
            date_modified: Utc::now(),
            date_indexed: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Builds the row and seeds its file-derived date, keeping the DB date-free.
    async fn create_photo_with_facts(
        pool: &DbPool,
        facts: &MediaFactsIndex,
        hash: &str,
        filename: &str,
        taken_at: &str,
    ) -> Photo {
        let photo = create_test_photo(filename.to_string(), hash.to_string());
        facts.set(
            &photo.file_path,
            MediaFacts {
                taken_at: Some(
                    DateTime::parse_from_rfc3339(taken_at)
                        .expect("invalid RFC3339 date")
                        .with_timezone(&Utc),
                ),
                ..MediaFacts::default()
            },
        );
        photo.create(pool).await.unwrap();
        photo
    }

    fn create_test_photo_with_metadata(
        filename: &str,
        hash: &str,
        metadata: serde_json::Value,
    ) -> Photo {
        let mut photo = create_test_photo(filename.to_string(), hash.to_string());
        photo.metadata = metadata;
        photo
    }

    /// The row a scan writes, built through the production JSON builder
    /// (`ProcessedPhoto` → `Photo`) rather than hand-made JSON: that builder is
    /// the only definition of the stored metadata shape, so tests that bypass
    /// it cannot catch a shape change. `latitude`/`longitude` stand for what
    /// the extraction found — `None` means the file carries no position.
    fn scanned_photo(
        filename: &str,
        hash: &str,
        mime_type: &str,
        latitude: Option<f64>,
        longitude: Option<f64>,
    ) -> crate::indexer::ProcessedPhoto {
        crate::indexer::ProcessedPhoto {
            file_path: format!("./test/{}", filename),
            filename: filename.to_string(),
            file_size: 1024,
            mime_type: Some(mime_type.to_string()),
            taken_at: Some(Utc::now()),
            date_modified: Utc::now(),
            camera_make: None,
            camera_model: None,
            lens_make: None,
            lens_model: None,
            iso: None,
            aperture: None,
            shutter_speed: None,
            focal_length: None,
            width: Some(1920),
            height: Some(1080),
            color_space: None,
            white_balance: None,
            exposure_mode: None,
            metering_mode: None,
            orientation: None,
            flash_used: None,
            latitude,
            longitude,
            hash_sha256: Some(hash.to_string()),
            blurhash: None,
            duration: None,
            video_codec: None,
            audio_codec: None,
            bitrate: None,
            frame_rate: None,
            video_profile: None,
            bit_depth: None,
            container: None,
            moov_at_start: true,
            semantic_vector_indexed: Some(false),
        }
    }

    async fn read_photo_metadata(pool: &DbPool, file_path: &str) -> serde_json::Value {
        let metadata: String =
            sqlx::query_scalar("SELECT metadata FROM photos WHERE file_path = ?")
                .bind(file_path)
                .fetch_one(pool)
                .await
                .unwrap();

        serde_json::from_str(&metadata).unwrap()
    }

    fn create_search_query(query: &str) -> SearchQuery {
        SearchQuery {
            q: Some(query.to_string()),
            year: None,
            month: None,
            to_year: None,
            to_month: None,
        }
    }

    fn photo_names(photos: &[Photo]) -> Vec<&str> {
        photos.iter().map(|p| p.filename.as_str()).collect()
    }

    /// An index with no entries, for listings whose assertions are about the
    /// filters and not about any date.
    fn no_facts() -> MediaFactsIndex {
        test_facts(&[])
    }

    #[test]
    fn test_sort_photos_orders_unknown_dates_like_sql() {
        // GIVEN: three photos, two of them with a file-derived date and one
        // with no facts entry at all
        let index = test_facts(&[
            ("./test/old.jpg", "2012-03-15T10:00:00Z"),
            ("./test/new.jpg", "2015-08-31T10:00:00Z"),
        ]);
        let mut unknown = create_test_photo("unknown.jpg".to_string(), "c".repeat(64));
        let mut old = create_test_photo("old.jpg".to_string(), "a".repeat(64));
        let mut new = create_test_photo("new.jpg".to_string(), "b".repeat(64));
        for photo in [&mut unknown, &mut old, &mut new] {
            index.enrich(photo);
        }

        // WHEN: sorting ascending (no explicit sort is the date default)
        let mut photos = vec![unknown.clone(), new.clone(), old.clone()];
        sort_photos(&mut photos, None, Some("asc"));

        // THEN: the unknown date sorts first, exactly like SQLite's NULL-first
        // `ORDER BY taken_at ASC`
        assert_eq!(photo_names(&photos), ["unknown.jpg", "old.jpg", "new.jpg"]);

        // AND: an explicit `sort=date` is that same ordering
        let mut photos = vec![unknown.clone(), new.clone(), old.clone()];
        sort_photos(&mut photos, Some("date"), Some("asc"));
        assert_eq!(photo_names(&photos), ["unknown.jpg", "old.jpg", "new.jpg"]);

        // WHEN: sorting descending
        let mut photos = vec![unknown.clone(), old.clone(), new.clone()];
        sort_photos(&mut photos, Some("date"), None);

        // THEN: the unknown date sorts last, exactly like SQLite's NULL-last
        // `ORDER BY taken_at DESC`
        assert_eq!(photo_names(&photos), ["new.jpg", "old.jpg", "unknown.jpg"]);
    }

    #[test]
    fn test_sort_photos_tiebreaks_by_hash_like_sql() {
        // GIVEN: two photos sharing one date, hashes "aa…" < "bb…"
        let index = test_facts(&[
            ("./test/aa.jpg", "2012-03-15T10:00:00Z"),
            ("./test/bb.jpg", "2012-03-15T10:00:00Z"),
        ]);
        let mut aa = create_test_photo("aa.jpg".to_string(), "aa".to_string());
        let mut bb = create_test_photo("bb.jpg".to_string(), "bb".to_string());
        for photo in [&mut aa, &mut bb] {
            index.enrich(photo);
        }

        // WHEN: sorting ascending
        let mut photos = vec![bb.clone(), aa.clone()];
        sort_photos(&mut photos, Some("date"), Some("asc"));

        // THEN: the hash tiebreak runs in the same direction as the sort
        assert_eq!(photo_names(&photos), ["aa.jpg", "bb.jpg"]);

        // AND: descending reverses both the date and the tiebreak
        let mut photos = vec![aa.clone(), bb.clone()];
        sort_photos(&mut photos, Some("date"), Some("desc"));
        assert_eq!(photo_names(&photos), ["bb.jpg", "aa.jpg"]);
    }

    #[test]
    fn test_sort_photos_supports_filename_and_size() {
        let mut a = create_test_photo("a.jpg".to_string(), "1".repeat(64));
        a.file_size = 300;
        let mut b = create_test_photo("b.jpg".to_string(), "2".repeat(64));
        b.file_size = 100;
        let mut c = create_test_photo("c.jpg".to_string(), "3".repeat(64));
        c.file_size = 200;

        // WHEN: sorting by filename
        let mut photos = vec![c.clone(), a.clone(), b.clone()];
        sort_photos(&mut photos, Some("filename"), Some("asc"));
        assert_eq!(photo_names(&photos), ["a.jpg", "b.jpg", "c.jpg"]);

        sort_photos(&mut photos, Some("name"), None);
        assert_eq!(photo_names(&photos), ["c.jpg", "b.jpg", "a.jpg"]);

        // AND: sorting by file size (`size` is the same field)
        let mut photos = vec![a.clone(), b.clone(), c.clone()];
        sort_photos(&mut photos, Some("file_size"), Some("asc"));
        assert_eq!(
            photos.iter().map(|p| p.file_size).collect::<Vec<_>>(),
            [100, 200, 300]
        );

        let mut photos = vec![a.clone(), b.clone(), c.clone()];
        sort_photos(&mut photos, Some("size"), None);
        assert_eq!(
            photos.iter().map(|p| p.file_size).collect::<Vec<_>>(),
            [300, 200, 100]
        );
    }

    #[test]
    fn test_photo_in_month_range_uses_lexicographic_year_month() {
        // GIVEN: the inclusive bounds of `year=2012&month=3&to_year=2015`
        let bounds = month_range_bounds(Some(2012), Some(3), Some(2015), None)
            .expect("year=2012 forms bounds");
        assert_eq!(bounds, ("2012-03".to_string(), "2015-12".to_string()));

        let index = test_facts(&[
            ("./test/mar2012.jpg", "2012-03-15T10:00:00Z"),
            ("./test/feb2012.jpg", "2012-02-29T10:00:00Z"),
            ("./test/dec2015.jpg", "2015-12-31T23:59:59Z"),
        ]);
        let mut mar = create_test_photo("mar2012.jpg".to_string(), "a".repeat(64));
        let mut feb = create_test_photo("feb2012.jpg".to_string(), "b".repeat(64));
        let mut dec = create_test_photo("dec2015.jpg".to_string(), "c".repeat(64));
        let mut unknown = create_test_photo("unknown.jpg".to_string(), "d".repeat(64));
        for photo in [&mut mar, &mut feb, &mut dec, &mut unknown] {
            index.enrich(photo);
        }

        // THEN: both bounds are inclusive
        assert!(photo_in_month_range(&mar, &bounds));
        assert!(photo_in_month_range(&dec, &bounds));
        // AND: the month before the start bound does not match
        assert!(!photo_in_month_range(&feb, &bounds));
        // AND: a photo with no facts entry never matches
        assert!(!photo_in_month_range(&unknown, &bounds));
    }

    #[tokio::test]
    async fn test_get_timeline_data() {
        let pool = create_test_db_pool().await.unwrap();
        // The dates live in the files; the index is their in-memory stand-in.
        let facts = test_facts(&[
            ("./test/photo1.jpg", "2010-05-25T10:00:00Z"),
            ("./test/photo2.jpg", "2010-05-26T10:00:00Z"),
            ("./test/photo3.jpg", "2011-12-01T10:00:00Z"),
            ("./test/photo4.jpg", "2024-01-15T10:00:00Z"),
        ]);

        create_photo_with_facts(
            &pool,
            &facts,
            &"a".repeat(64),
            "photo1.jpg",
            "2010-05-25T10:00:00Z",
        )
        .await;
        create_photo_with_facts(
            &pool,
            &facts,
            &"b".repeat(64),
            "photo2.jpg",
            "2010-05-26T10:00:00Z",
        )
        .await;
        create_photo_with_facts(
            &pool,
            &facts,
            &"c".repeat(64),
            "photo3.jpg",
            "2011-12-01T10:00:00Z",
        )
        .await;
        create_photo_with_facts(
            &pool,
            &facts,
            &"d".repeat(64),
            "photo4.jpg",
            "2024-01-15T10:00:00Z",
        )
        .await;

        // Get timeline data
        let timeline = Photo::get_timeline_data(&pool, &facts).await.unwrap();

        // Verify min/max dates
        assert_eq!(
            timeline.min_date,
            Some("2010-05-25T10:00:00+00:00".to_string())
        );
        assert_eq!(
            timeline.max_date,
            Some("2024-01-15T10:00:00+00:00".to_string())
        );

        // Verify density data
        assert_eq!(timeline.density.len(), 3); // 3 unique year-month combinations

        // The buckets are ordered by year, then month.
        let buckets: Vec<(i32, i32)> = timeline.density.iter().map(|d| (d.year, d.month)).collect();
        assert_eq!(buckets, [(2010, 5), (2011, 12), (2024, 1)]);

        // Check May 2010 (2 photos)
        let may_2010 = timeline
            .density
            .iter()
            .find(|d| d.year == 2010 && d.month == 5)
            .unwrap();
        assert_eq!(may_2010.count, 2);

        // Check December 2011 (1 photo)
        let dec_2011 = timeline
            .density
            .iter()
            .find(|d| d.year == 2011 && d.month == 12)
            .unwrap();
        assert_eq!(dec_2011.count, 1);

        // Check January 2024 (1 photo)
        let jan_2024 = timeline
            .density
            .iter()
            .find(|d| d.year == 2024 && d.month == 1)
            .unwrap();
        assert_eq!(jan_2024.count, 1);
    }

    #[tokio::test]
    async fn test_rotate_db_update_removes_housekeeping_candidate() {
        let pool = create_test_db_pool().await.unwrap();

        // Create a photo and a stale housekeeping candidate referencing its hash
        let photo = create_test_photo("rotate.jpg".to_string(), "a".repeat(64));
        photo.create(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO housekeeping_candidates (photo_hash, reason, score) VALUES (?, 'test', 0.5)",
        )
        .bind(&photo.hash_sha256)
        .execute(&pool)
        .await
        .unwrap();

        // Simulate rotate_image's DB sequence: delete the stale candidate row and
        // rewrite the PK inside one transaction (AGENTS.md known bug: FK has no
        // ON UPDATE, so a bare PK rewrite fails)
        let mut updated = photo.clone();
        updated.hash_sha256 = "b".repeat(64);
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("DELETE FROM housekeeping_candidates WHERE photo_hash = ?")
            .bind(&photo.hash_sha256)
            .execute(&mut *tx)
            .await
            .unwrap();
        updated
            .update_with_old_hash(&mut tx, &photo.hash_sha256)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // Old candidate row is gone; photo lives under the new hash
        let old_candidates: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping_candidates WHERE photo_hash = ?")
                .bind(&photo.hash_sha256)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(old_candidates, 0);
        let new_photos: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE hash_sha256 = ?")
                .bind(&updated.hash_sha256)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(new_photos, 1);
    }

    #[tokio::test]
    async fn test_create_or_update_preserves_favorite_across_hash_rekey() {
        let pool = create_test_db_pool().await.unwrap();

        // GIVEN: a favorited photo keyed by hash H1 at path rotate.jpg
        let mut photo = create_test_photo("rotate.jpg".to_string(), "a".repeat(64));
        photo.is_favorite = Some(true);
        photo.create(&pool).await.unwrap();

        // WHEN: the same path is re-keyed under a different hash (the
        // rotate-then-rescan sequence: in-app rotation keys the row by the
        // content hash, the next rescan re-derives the path hash)
        let mut reprocessed = photo.clone();
        reprocessed.hash_sha256 = "b".repeat(64);
        reprocessed.is_favorite = None; // fresh extraction knows nothing about favorites
        let mut tx = pool.begin().await.unwrap();
        reprocessed
            .create_or_update_with_transaction(&mut tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // THEN: the favorite flag survived the re-key
        let stored: Option<bool> =
            sqlx::query_scalar("SELECT is_favorite FROM photos WHERE hash_sha256 = ?")
                .bind("b".repeat(64))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, Some(true), "is_favorite must survive a hash re-key");

        // AND: the old row is gone (the DELETE really ran, not just an UPSERT)
        let old_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE hash_sha256 = ?")
            .bind("a".repeat(64))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            old_rows, 0,
            "re-keyed row must not remain under the old hash"
        );

        // AND: a same-hash rescan keeps the favorite via COALESCE (no re-key)
        let mut rescan = photo.clone();
        rescan.hash_sha256 = "b".repeat(64);
        rescan.is_favorite = None;
        let mut tx = pool.begin().await.unwrap();
        rescan
            .create_or_update_with_transaction(&mut tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let stored: Option<bool> =
            sqlx::query_scalar("SELECT is_favorite FROM photos WHERE hash_sha256 = ?")
                .bind("b".repeat(64))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            stored,
            Some(true),
            "same-hash upsert must keep the favorite"
        );
    }

    #[tokio::test]
    async fn test_update_with_old_hash_repoints_album_members() {
        let pool = create_test_db_pool().await.unwrap();
        let photo = create_test_photo("rotate.jpg".to_string(), "a".repeat(64));
        photo.create(&pool).await.unwrap();
        let album = crate::albums::create(&pool, "Trip").await.unwrap();
        crate::albums::add_members(&pool, album.id, std::slice::from_ref(&photo.hash_sha256))
            .await
            .unwrap();

        // WHEN: the PK is rewritten exactly like rotate_image does
        let mut updated = photo.clone();
        updated.hash_sha256 = "b".repeat(64);
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("DELETE FROM housekeeping_candidates WHERE photo_hash = ?")
            .bind(&photo.hash_sha256)
            .execute(&mut *tx)
            .await
            .unwrap();
        updated
            .update_with_old_hash(&mut tx, &photo.hash_sha256)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // THEN: no FK failure and the membership followed the new hash
        assert_eq!(
            crate::albums::count_members(&pool, album.id).await.unwrap(),
            1
        );
        let member_hash: String =
            sqlx::query_scalar("SELECT photo_hash FROM album_members WHERE album_id = ?")
                .bind(album.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(member_hash, "b".repeat(64));
    }

    #[tokio::test]
    async fn test_update_with_old_hash_same_hash_keeps_album_members() {
        let pool = create_test_db_pool().await.unwrap();
        let photo = create_test_photo("rotate.jpg".to_string(), "a".repeat(64));
        photo.create(&pool).await.unwrap();
        let album = crate::albums::create(&pool, "Trip").await.unwrap();
        crate::albums::add_members(&pool, album.id, std::slice::from_ref(&photo.hash_sha256))
            .await
            .unwrap();

        // WHEN: rotate_image produces byte-identical output (solid-color PNG),
        // so the new hash equals the old hash
        let updated = photo.clone();
        let mut tx = pool.begin().await.unwrap();
        updated
            .update_with_old_hash(&mut tx, &photo.hash_sha256)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // THEN: the membership survives (copy-then-delete must no-op, not
        // INSERT-noop + DELETE-everything)
        assert_eq!(
            crate::albums::count_members(&pool, album.id).await.unwrap(),
            1
        );
        let member_hash: String =
            sqlx::query_scalar("SELECT photo_hash FROM album_members WHERE album_id = ?")
                .bind(album.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(member_hash, "a".repeat(64));
    }

    #[tokio::test]
    async fn test_create_or_update_carries_album_members_across_rekey() {
        let pool = create_test_db_pool().await.unwrap();
        let photo = create_test_photo("rotate.jpg".to_string(), "a".repeat(64));
        photo.create(&pool).await.unwrap();
        let album = crate::albums::create(&pool, "Trip").await.unwrap();
        crate::albums::add_members(&pool, album.id, std::slice::from_ref(&photo.hash_sha256))
            .await
            .unwrap();

        // WHEN: the same path is re-keyed under a different hash (rescan
        // after an in-place edit) — the DELETE would cascade memberships
        let mut reprocessed = photo.clone();
        reprocessed.hash_sha256 = "b".repeat(64);
        let mut tx = pool.begin().await.unwrap();
        reprocessed
            .create_or_update_with_transaction(&mut tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // THEN: the membership survived under the new hash
        assert_eq!(
            crate::albums::count_members(&pool, album.id).await.unwrap(),
            1
        );
        let member_hash: String =
            sqlx::query_scalar("SELECT photo_hash FROM album_members WHERE album_id = ?")
                .bind(album.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(member_hash, "b".repeat(64));
    }

    #[tokio::test]
    async fn test_create_or_update_upsert_round_trips_timestamps() {
        let pool = create_test_db_pool().await.unwrap();
        let photo = create_test_photo("scan.jpg".to_string(), "c".repeat(64));
        let mut tx = pool.begin().await.unwrap();
        photo
            .create_or_update_with_transaction(&mut tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // Raw check: every bound column must be non-null. A dropped bind
        // shifts the trailing values so ?20=updated_at binds NULL.
        let row: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT date_indexed, created_at, updated_at FROM photos WHERE hash_sha256 = ?",
        )
        .bind(&photo.hash_sha256)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(row.0.is_some(), "upsert must store date_indexed");
        assert!(row.1.is_some(), "upsert must store created_at");
        assert!(row.2.is_some(), "upsert must store updated_at");

        // Read path: Photo::from_row decodes updated_at as a non-optional
        // String, so a NULL updated_at fails the query — the round-trip
        // must succeed.
        let found = Photo::find_by_hash(&pool, &photo.hash_sha256)
            .await
            .unwrap()
            .expect("upserted photo must be readable");
        assert_eq!(found.hash_sha256, photo.hash_sha256);

        // Same-hash rescan (ON CONFLICT DO UPDATE path) must keep
        // updated_at non-null too.
        let mut tx = pool.begin().await.unwrap();
        photo
            .create_or_update_with_transaction(&mut tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let updated_at: Option<String> =
            sqlx::query_scalar("SELECT updated_at FROM photos WHERE hash_sha256 = ?")
                .bind(&photo.hash_sha256)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(updated_at.is_some(), "rescan upsert must keep updated_at");
    }

    /// The schema contract: no stored capture date and no index for one.
    /// `taken_at` survives on `Photo` only as a response-side field that
    /// `MediaFactsIndex::enrich` fills from the file.
    #[tokio::test]
    async fn test_photos_schema_has_no_taken_at_column() {
        let pool = create_test_db_pool().await.unwrap();

        let columns: Vec<String> = sqlx::query("PRAGMA table_info(photos)")
            .map(|row: sqlx::sqlite::SqliteRow| row.get::<String, _>("name"))
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(
            !columns.iter().any(|name| name == "taken_at"),
            "photos must not store a capture date, found columns {columns:?}"
        );

        let taken_at_index: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_photos_taken_at'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(
            taken_at_index.is_none(),
            "idx_photos_taken_at must be dropped"
        );
    }

    /// Coordinates are file facts too: `location` keeps its city (search and
    /// the UI need it) but never the coordinate pair, on any write path.
    #[tokio::test]
    async fn test_db_writes_never_store_coordinates() {
        let pool = create_test_db_pool().await.unwrap();
        let mut photo = create_test_photo("coords.jpg".to_string(), "d".repeat(64));
        photo.metadata = json!({
            "location": { "latitude": 48.1, "longitude": 11.5, "city": "Munich" }
        });
        photo.create(&pool).await.unwrap();

        let stored = read_photo_metadata(&pool, &photo.file_path).await;
        assert!(
            stored["location"].get("latitude").is_none(),
            "create stored latitude: {stored}"
        );
        assert!(
            stored["location"].get("longitude").is_none(),
            "create stored longitude: {stored}"
        );
        assert_eq!(stored["location"]["city"], "Munich");

        // An enriched copy (the response-side shape, coordinates merged in
        // from the file) must not smuggle them back in on update.
        let facts =
            test_facts_with_coords(&[(&photo.file_path, "2024-05-25T10:00:00Z", 48.1, 11.5)]);
        let mut enriched = photo.clone();
        facts.enrich(&mut enriched);
        assert!(
            enriched.metadata["location"].get("latitude").is_some(),
            "enrich must have merged the file's coordinates into the copy"
        );
        enriched.update(&pool).await.unwrap();

        let stored = read_photo_metadata(&pool, &photo.file_path).await;
        assert!(
            stored["location"].get("latitude").is_none(),
            "update stored latitude: {stored}"
        );
        assert!(
            stored["location"].get("longitude").is_none(),
            "update stored longitude: {stored}"
        );
        assert_eq!(stored["location"]["city"], "Munich");

        // The scan path (`batch_write_photos` -> the UPSERT) is the other
        // production writer; it must sanitize too.
        let mut rescanned = photo.clone();
        rescanned.metadata = enriched.metadata.clone();
        rescanned.create_or_update(&pool).await.unwrap();

        let stored = read_photo_metadata(&pool, &photo.file_path).await;
        assert!(
            stored["location"].get("latitude").is_none(),
            "upsert stored latitude: {stored}"
        );
        assert!(
            stored["location"].get("longitude").is_none(),
            "upsert stored longitude: {stored}"
        );
        assert_eq!(stored["location"]["city"], "Munich");
    }

    /// The migration must clean existing rows, not just future writes: a
    /// legacy database carrying a date column and coordinate keys comes out of
    /// it without either. This is the only test that pins the row cleanup —
    /// every other pool is built by the migrator from an already-current
    /// schema.
    #[tokio::test]
    async fn test_migration_drops_stored_dates_and_coordinates() {
        // Migration 2 creates a `vec0` virtual table and registration is
        // process-global and applies at connection-open time, so load the
        // extension here instead of hoping another test ran first.
        crate::db_pool::register_vector_extension();

        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("legacy.db");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&db_path)
                    .create_if_missing(true),
            )
            .await
            .expect("legacy pool");

        // The ten migrations that shipped before this one, in order (plain DDL).
        for migration in [
            include_str!("../migrations/20250101000001_create_photos_table.sql"),
            include_str!("../migrations/20250101000002_create_vector_tables.sql"),
            include_str!("../migrations/20250101000003_create_video_metadata_table.sql"),
            include_str!("../migrations/20250101000004_create_collages_table.sql"),
            include_str!("../migrations/20250101000005_create_indexes.sql"),
            include_str!("../migrations/20250101000006_create_housekeeping_candidates_table.sql"),
            include_str!("../migrations/20250101000007_add_geo_location_resolved.sql"),
            include_str!("../migrations/20250101000008_create_saved_searches_table.sql"),
            include_str!("../migrations/20250101000009_saved_search_range.sql"),
            include_str!("../migrations/20250101000010_create_manual_albums.sql"),
        ] {
            sqlx::raw_sql(migration).execute(&pool).await.unwrap();
        }

        sqlx::query(
            "INSERT INTO photos (hash_sha256, file_path, filename, file_size, taken_at, metadata, file_modified)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("a".repeat(64))
        .bind("/legacy/photo.jpg")
        .bind("photo.jpg")
        .bind(1024_i64)
        .bind("2012-03-15T10:00:00Z")
        .bind(
            json!({
                "location": { "latitude": 48.1, "longitude": 11.5, "city": "Munich" },
                "camera": { "make": "Canon" }
            })
            .to_string(),
        )
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!(
            "../migrations/20260928000001_drop_taken_at_and_location_coordinates.sql"
        ))
        .execute(&pool)
        .await
        .expect("the new migration must apply to a legacy database");

        let columns: Vec<String> = sqlx::query("PRAGMA table_info(photos)")
            .map(|row: sqlx::sqlite::SqliteRow| row.get::<String, _>("name"))
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(
            !columns.iter().any(|name| name == "taken_at"),
            "migration left the column: {columns:?}"
        );

        let taken_at_index: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_photos_taken_at'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(
            taken_at_index.is_none(),
            "idx_photos_taken_at survived the migration"
        );

        let metadata: String =
            sqlx::query_scalar("SELECT metadata FROM photos WHERE file_path = ?")
                .bind("/legacy/photo.jpg")
                .fetch_one(&pool)
                .await
                .unwrap();
        let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
        assert!(
            metadata["location"].get("latitude").is_none(),
            "migration left latitude: {metadata}"
        );
        assert!(
            metadata["location"].get("longitude").is_none(),
            "migration left longitude: {metadata}"
        );
        assert_eq!(metadata["location"]["city"], "Munich");
        assert_eq!(metadata["camera"]["make"], "Canon");
    }

    /// The builder must state the position explicitly (`latitude: null`) when
    /// the extraction found none — otherwise `json_patch` would preserve the
    /// stored pair and the row would keep serving coordinates the video file
    /// no longer carries, where the old wholesale replace cleared them.
    #[tokio::test]
    async fn scan_upsert_clears_video_coordinates_the_file_no_longer_carries() {
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-builder-video");
        // GIVEN: a video row that still holds the file's former position and a
        // resolved city
        let mut existing = create_test_photo_with_date(&hash, "builder_video.mp4", Utc::now());
        existing.metadata = json!({
            "location": { "latitude": 48.2082, "longitude": 16.3737, "city": "Vienna" },
            "video": { "codec": "h264", "capability_version": 1 }
        });
        existing.create(&pool).await.expect("create");

        // WHEN: the next scan writes what a file without a position produces,
        // through the real builder
        let mut extracted = scanned_photo("builder_video.mp4", &hash, "video/mp4", None, None);
        extracted.video_codec = Some("h264".to_string());
        extracted.audio_codec = Some("aac".to_string());
        extracted.container = Some("mp4".to_string());
        let fresh: Photo = extracted.into();
        assert!(
            fresh.metadata["location"]["latitude"].is_null(),
            "the builder must state the absent position, got {}",
            fresh.metadata
        );
        let mut tx = pool.begin().await.expect("tx");
        fresh
            .create_or_update_with_transaction(&mut tx)
            .await
            .expect("upsert");
        tx.commit().await.expect("commit");

        // THEN: the stale pair is gone, the city and the capability record stay
        let stored = Photo::find_by_hash(&pool, &hash)
            .await
            .expect("read")
            .expect("row");
        assert!(
            stored.metadata["location"].get("latitude").is_none(),
            "the file no longer has a position: {}",
            stored.metadata
        );
        assert!(
            stored.metadata["location"].get("longitude").is_none(),
            "the file no longer has a position: {}",
            stored.metadata
        );
        assert!(stored.latitude().is_none());
        assert_eq!(stored.metadata["location"]["city"], "Vienna");
        assert_eq!(stored.metadata["video"]["capability_version"], 1);
        assert!(crate::video_probe::record_is_complete(&stored));
    }

    /// Same guarantee for the photo path (FR-011: photo behaviour stays as it
    /// was) — a photo whose EXIF lost its GPS must not keep the stored pair,
    /// while the resolved city survives, and a position that reappears is
    /// written again.
    #[tokio::test]
    async fn scan_upsert_clears_photo_coordinates_the_file_no_longer_carries() {
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-builder-photo");
        let mut existing = create_test_photo_with_date(&hash, "builder_photo.jpg", Utc::now());
        existing.metadata = json!({
            "location": { "latitude": 52.52, "longitude": 13.405, "city": "Berlin" }
        });
        existing.create(&pool).await.expect("create");

        // WHEN: a scan runs over the same file with no GPS left in its EXIF
        let fresh: Photo =
            scanned_photo("builder_photo.jpg", &hash, "image/jpeg", None, None).into();
        let mut tx = pool.begin().await.expect("tx");
        fresh
            .create_or_update_with_transaction(&mut tx)
            .await
            .expect("upsert");
        tx.commit().await.expect("commit");

        // THEN: the coordinates are cleared, the city is not
        let stored = Photo::find_by_hash(&pool, &hash)
            .await
            .expect("read")
            .expect("row");
        assert!(stored.latitude().is_none(), "{}", stored.metadata);
        assert!(stored.longitude().is_none(), "{}", stored.metadata);
        assert_eq!(stored.metadata["location"]["city"], "Berlin");

        // AND: a position that is in the file again lands in the row
        let mut extracted = scanned_photo("builder_photo.jpg", &hash, "image/jpeg", None, None);
        extracted.latitude = Some(48.2082);
        extracted.longitude = Some(16.3737);
        let fresh: Photo = extracted.into();
        let mut tx = pool.begin().await.expect("tx");
        fresh
            .create_or_update_with_transaction(&mut tx)
            .await
            .expect("upsert");
        tx.commit().await.expect("commit");

        let stored = Photo::find_by_hash(&pool, &hash)
            .await
            .expect("read")
            .expect("row");
        assert_eq!(stored.latitude(), Some(48.2082));
        assert_eq!(stored.longitude(), Some(16.3737));
        assert_eq!(stored.metadata["location"]["city"], "Berlin");
    }

    #[tokio::test]
    async fn test_get_timeline_data_empty() {
        let pool = create_test_db_pool().await.unwrap();

        // Get timeline data from empty database
        let timeline = Photo::get_timeline_data(&pool, &no_facts()).await.unwrap();

        // Should return None for dates and empty density
        assert_eq!(timeline.min_date, None);
        assert_eq!(timeline.max_date, None);
        assert_eq!(timeline.density.len(), 0);
    }

    #[tokio::test]
    async fn test_transaction_rollback_on_constraint_violation() {
        let pool = create_test_db_pool().await.unwrap();

        // Create first photo
        let photo1 = create_test_photo("test1.jpg".to_string(), "abc123".to_string());
        photo1.create(&pool).await.unwrap();

        // Verify photo exists
        let found = Photo::find_by_hash(&pool, &photo1.hash_sha256)
            .await
            .unwrap();
        assert!(found.is_some());

        // Attempt to create photo with duplicate hash in a transaction
        let mut tx = pool.begin().await.unwrap();
        let photo2 = create_test_photo("test2.jpg".to_string(), "abc123".to_string()); // Same hash
        let result = photo2.create_with_transaction(&mut tx).await;

        // Should fail due to PRIMARY KEY constraint
        assert!(result.is_err());

        // Rollback transaction (or let it drop)
        drop(tx);

        // Verify database is still consistent - only one photo exists
        let all_photos = sqlx::query("SELECT COUNT(*) as count FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        let count: i64 = all_photos.get("count");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_transaction_atomicity() {
        let pool = create_test_db_pool().await.unwrap();

        // Create multiple photos in a transaction
        let photos = vec![
            create_test_photo("test1.jpg".to_string(), "hash1".to_string()),
            create_test_photo("test2.jpg".to_string(), "hash2".to_string()),
            create_test_photo("test3.jpg".to_string(), "hash3".to_string()),
        ];

        // Test 1: Successful transaction - all photos committed
        let mut tx = pool.begin().await.unwrap();
        for photo in &photos {
            photo.create_with_transaction(&mut tx).await.unwrap();
        }
        tx.commit().await.unwrap();

        // Verify all photos were committed
        let count = sqlx::query("SELECT COUNT(*) as count FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        let count: i64 = count.get("count");
        assert_eq!(count, 3, "All photos should be visible after commit");

        // Test 2: Failed transaction - no photos should be added
        let more_photos = vec![
            create_test_photo("test4.jpg".to_string(), "hash4".to_string()),
            create_test_photo("test5.jpg".to_string(), "hash1".to_string()), // Duplicate hash - will fail
        ];

        let mut tx2 = pool.begin().await.unwrap();
        let result = async {
            for photo in &more_photos {
                photo.create_with_transaction(&mut tx2).await?;
            }
            tx2.commit().await?;
            Ok::<(), Box<dyn std::error::Error>>(())
        }
        .await;

        // Transaction should fail due to duplicate hash
        assert!(result.is_err());

        // Verify count is still 3 (rollback worked)
        let final_count = sqlx::query("SELECT COUNT(*) as count FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        let final_count: i64 = final_count.get("count");
        assert_eq!(
            final_count, 3,
            "Count should remain 3 after failed transaction"
        );
    }

    #[tokio::test]
    async fn test_transaction_update_and_rollback() {
        let pool = create_test_db_pool().await.unwrap();

        // Create initial photo
        let mut photo = create_test_photo("test.jpg".to_string(), "hash123".to_string());
        photo.create(&pool).await.unwrap();

        // Verify initial state
        let original_filename = photo.filename.clone();

        // Start transaction and update photo
        let mut tx = pool.begin().await.unwrap();
        photo.filename = "updated.jpg".to_string();
        photo.update_with_transaction(&mut tx).await.unwrap();

        // Rollback transaction
        drop(tx);

        // Verify photo was NOT updated (rollback worked)
        let found = Photo::find_by_hash(&pool, &photo.hash_sha256)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            found.filename, original_filename,
            "Photo should not be updated after rollback"
        );
    }

    #[tokio::test]
    async fn test_concurrent_writes_consistency() {
        let pool = create_test_db_pool().await.unwrap();

        // Create two photos concurrently
        let photo1 = create_test_photo("test1.jpg".to_string(), "hash1".to_string());
        let photo2 = create_test_photo("test2.jpg".to_string(), "hash2".to_string());

        // Both should succeed since they have different hashes
        let result1 = photo1.create(&pool).await;
        let result2 = photo2.create(&pool).await;

        assert!(result1.is_ok());
        assert!(result2.is_ok());

        // Verify both photos exist
        let count = sqlx::query("SELECT COUNT(*) as count FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        let count: i64 = count.get("count");
        assert_eq!(count, 2, "Both photos should be created");
    }

    #[tokio::test]
    async fn test_batch_transaction_consistency() {
        let pool = create_test_db_pool().await.unwrap();

        // Create 100 photos in a single transaction to test batch performance
        let mut tx = pool.begin().await.unwrap();

        for i in 0..100 {
            let photo = create_test_photo(
                format!("test_{}.jpg", i),
                format!("{:064}", i), // Generate unique 64-char hash by padding number
            );
            photo.create_with_transaction(&mut tx).await.unwrap();
        }

        // Commit all at once
        tx.commit().await.unwrap();

        // Verify all 100 photos were created
        let count = sqlx::query("SELECT COUNT(*) as count FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        let count: i64 = count.get("count");
        assert_eq!(count, 100, "All 100 photos should be created");
    }

    #[tokio::test]
    async fn test_geo_location_resolved_defaults_to_false_and_persists_true() {
        let pool = create_test_db_pool().await.unwrap();
        let photo = create_test_photo(
            "geo-default.jpg".to_string(),
            "geo-default-hash".to_string(),
        );

        photo.create(&pool).await.unwrap();

        let initial_value: i64 =
            sqlx::query_scalar("SELECT geo_location_resolved FROM photos WHERE hash_sha256 = ?")
                .bind(&photo.hash_sha256)
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(initial_value, 0);

        sqlx::query("UPDATE photos SET geo_location_resolved = 1 WHERE hash_sha256 = ?")
            .bind(&photo.hash_sha256)
            .execute(&pool)
            .await
            .unwrap();

        let updated_value: i64 =
            sqlx::query_scalar("SELECT geo_location_resolved FROM photos WHERE hash_sha256 = ?")
                .bind(&photo.hash_sha256)
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(updated_value, 1);
    }

    #[tokio::test]
    async fn test_geo_location_resolved_defaults_false_for_multiple_rows() {
        let pool = create_test_db_pool().await.unwrap();

        for index in 0..3 {
            let photo = create_test_photo(
                format!("geo-default-{}.jpg", index),
                format!("geo-default-hash-{}", index),
            );
            photo.create(&pool).await.unwrap();
        }

        let resolved_values: Vec<i64> =
            sqlx::query_scalar("SELECT geo_location_resolved FROM photos ORDER BY file_path")
                .fetch_all(&pool)
                .await
                .unwrap();

        assert_eq!(resolved_values, [0, 0, 0]);
    }

    #[tokio::test]
    async fn test_get_photos_needing_geo_resolution() {
        let pool = create_test_db_pool().await.unwrap();
        // Coordinates live in the index (the DB stores none).
        let facts = test_facts_with_coords(&[
            (
                "./test/needs-geo.jpg",
                "2024-01-01T00:00:00Z",
                52.52,
                13.405,
            ),
            (
                "./test/resolved-geo.jpg",
                "2024-01-01T00:00:00Z",
                48.137,
                11.575,
            ),
        ]);
        let unresolved_photo =
            create_test_photo("needs-geo.jpg".to_string(), "needs-geo-hash".to_string());
        let resolved_photo = create_test_photo(
            "resolved-geo.jpg".to_string(),
            "resolved-geo-hash".to_string(),
        );
        let no_gps_photo = create_test_photo("no-gps.jpg".to_string(), "no-gps-hash".to_string());

        unresolved_photo.create(&pool).await.unwrap();
        resolved_photo.create(&pool).await.unwrap();
        no_gps_photo.create(&pool).await.unwrap();

        sqlx::query("UPDATE photos SET geo_location_resolved = 1 WHERE file_path = ?")
            .bind(&resolved_photo.file_path)
            .execute(&pool)
            .await
            .unwrap();

        let photos = get_photos_needing_geo_resolution(&pool, &facts)
            .await
            .unwrap();

        assert_eq!(
            photos,
            vec![(unresolved_photo.file_path.clone(), 52.52, 13.405)]
        );
    }

    #[tokio::test]
    async fn test_mark_photo_geo_resolved() {
        let pool = create_test_db_pool().await.unwrap();
        let facts = test_facts_with_coords(&[(
            "./test/mark-resolved.jpg",
            "2024-01-01T00:00:00Z",
            52.52,
            13.405,
        )]);
        let photo = create_test_photo(
            "mark-resolved.jpg".to_string(),
            "mark-resolved-hash".to_string(),
        );

        photo.create(&pool).await.unwrap();
        mark_photo_geo_resolved(&pool, &photo.file_path)
            .await
            .unwrap();

        let photos = get_photos_needing_geo_resolution(&pool, &facts)
            .await
            .unwrap();

        assert!(photos.is_empty());
    }

    #[tokio::test]
    async fn test_update_photo_city() {
        let pool = create_test_db_pool().await.unwrap();
        let facts = test_facts_with_coords(&[(
            "./test/city-update.jpg",
            "2024-01-01T00:00:00Z",
            52.52,
            13.405,
        )]);
        let photo = create_test_photo(
            "city-update.jpg".to_string(),
            "city-update-hash".to_string(),
        );

        photo.create(&pool).await.unwrap();
        update_photo_city(&pool, &photo.file_path, Some("Berlin"))
            .await
            .unwrap();

        let metadata = read_photo_metadata(&pool, &photo.file_path).await;
        let resolved_value: i64 =
            sqlx::query_scalar("SELECT geo_location_resolved FROM photos WHERE file_path = ?")
                .bind(&photo.file_path)
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(metadata["location"]["city"], json!("Berlin"));
        assert_eq!(resolved_value, 1);
        // A labelled photo is no longer a geo-resolution candidate.
        assert!(get_photos_needing_geo_resolution(&pool, &facts)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn test_update_photo_city_null() {
        let pool = create_test_db_pool().await.unwrap();
        let photo = create_test_photo(
            "city-update-null.jpg".to_string(),
            "city-update-null-hash".to_string(),
        );

        photo.create(&pool).await.unwrap();
        update_photo_city(&pool, &photo.file_path, None)
            .await
            .unwrap();

        let metadata = read_photo_metadata(&pool, &photo.file_path).await;
        let resolved_value: i64 =
            sqlx::query_scalar("SELECT geo_location_resolved FROM photos WHERE file_path = ?")
                .bind(&photo.file_path)
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(metadata["location"].get("city"), None);
        assert_eq!(resolved_value, 1);
    }

    #[tokio::test]
    async fn test_search_photos_by_city() {
        let pool = create_test_db_pool().await.unwrap();
        let berlin_photo = create_test_photo_with_metadata(
            "berlin.jpg",
            "berlin-hash",
            json!({
                "location": {
                    "city": "Berlin"
                }
            }),
        );

        berlin_photo.create(&pool).await.unwrap();

        let query = create_search_query("location:Berlin");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, berlin_photo.file_path);
    }

    fn date_filter_query(
        year: Option<i32>,
        month: Option<i32>,
        to_year: Option<i32>,
        to_month: Option<i32>,
    ) -> SearchQuery {
        SearchQuery {
            q: None,
            year,
            month,
            to_year,
            to_month,
        }
    }

    #[tokio::test]
    async fn test_search_photos_filters_inclusive_month_range() {
        let pool = create_test_db_pool().await.unwrap();
        let facts = MediaFactsIndex::new();
        for (hash, filename, taken_at) in [
            ("a", "feb2012.jpg", "2012-02-10T10:00:00Z"),
            ("b", "mar2012.jpg", "2012-03-15T10:00:00Z"),
            ("c", "dec2012.jpg", "2012-12-31T23:00:00Z"),
            ("d", "jan2013.jpg", "2013-01-01T00:30:00Z"),
            ("e", "aug2015.jpg", "2015-08-31T23:30:00Z"),
            ("f", "sep2015.jpg", "2015-09-01T00:00:00Z"),
        ] {
            create_photo_with_facts(&pool, &facts, &hash.repeat(64), filename, taken_at).await;
        }

        // Single month (start bound only).
        let query = date_filter_query(Some(2012), Some(3), None, None);
        let (photos, total) = Photo::search_photos(&pool, &facts, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos[0].filename, "mar2012.jpg");

        // Whole year (no month) stays a single-year filter.
        let query = date_filter_query(Some(2012), None, None, None);
        let (_, total) = Photo::search_photos(&pool, &facts, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 3);

        // Month-granular range spanning years, both bounds inclusive.
        let query = date_filter_query(Some(2012), Some(3), Some(2015), Some(8));
        let (photos, total) = Photo::search_photos(&pool, &facts, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 4);
        let names: Vec<&str> = photos.iter().map(|p| p.filename.as_str()).collect();
        assert!(!names.contains(&"feb2012.jpg"));
        assert!(!names.contains(&"sep2015.jpg"));

        // Year-precision end bound stops at December of that year.
        let query = date_filter_query(Some(2012), Some(3), Some(2012), None);
        let (photos, total) = Photo::search_photos(&pool, &facts, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(photos[0].taken_at.unwrap().year(), 2012);

        // Reversed bounds match nothing (the router normalises before it gets here).
        let query = date_filter_query(Some(2015), Some(8), Some(2012), Some(3));
        let (_, total) = Photo::search_photos(&pool, &facts, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 0);

        // A month bound without a year filters nothing.
        let query = date_filter_query(None, Some(3), None, None);
        let (_, total) = Photo::search_photos(&pool, &facts, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 6);

        // An end year without a start year filters nothing either.
        let query = date_filter_query(None, None, Some(2015), None);
        let (_, total) = Photo::search_photos(&pool, &facts, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 6);

        // An out-of-range start month matches nothing, with or without an end
        // bound: `2012-13` must not become a valid ordering point that lets a
        // later end year widen the range to 2013-01…2015-12.
        let query = date_filter_query(Some(2012), Some(13), None, None);
        let (_, total) = Photo::search_photos(&pool, &facts, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 0);

        let query = date_filter_query(Some(2012), Some(13), Some(2015), None);
        let (_, total) = Photo::search_photos(&pool, &facts, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 0);

        // Likewise for an out-of-range end month.
        let query = date_filter_query(Some(2012), None, Some(2015), Some(13));
        let (_, total) = Photo::search_photos(&pool, &facts, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn test_search_photos_range_ignores_unknown_dates() {
        let pool = create_test_db_pool().await.unwrap();
        // The undated photo gets NO index entry: its file yields no date.
        let facts = MediaFactsIndex::new();
        create_photo_with_facts(
            &pool,
            &facts,
            &"c".repeat(64),
            "mar2012.jpg",
            "2012-03-15T10:00:00Z",
        )
        .await;
        create_test_photo("undated.jpg".to_string(), "undated".to_string())
            .create(&pool)
            .await
            .unwrap();

        let query = date_filter_query(Some(2012), Some(1), Some(2012), Some(12));
        let (photos, total) = Photo::search_photos(&pool, &facts, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos[0].filename, "mar2012.jpg");
    }

    #[tokio::test]
    async fn test_search_like_wildcards_are_literal() {
        let pool = create_test_db_pool().await.unwrap();
        // `_` and `%` in user input must match literally: without the ESCAPE
        // clause, `IMG_2024` also matches `IMGX2024` and a `%` matches every
        // row (LIKE '%%').
        let underscore_photo =
            create_test_photo_with_metadata("IMG_2024.jpg", "underscore-hash", json!({}));
        let x_photo = create_test_photo_with_metadata("IMGX2024.jpg", "x-hash", json!({}));
        underscore_photo.create(&pool).await.unwrap();
        x_photo.create(&pool).await.unwrap();

        let query = create_search_query("IMG_2024");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1, "underscore must match literally");
        assert_eq!(photos[0].file_path, underscore_photo.file_path);

        // A literal % must match only rows whose filename contains '%'
        let percent_photo =
            create_test_photo_with_metadata("weird%name.jpg", "percent-hash", json!({}));
        percent_photo.create(&pool).await.unwrap();
        let query = create_search_query("100%");
        let (_, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 0, "'100%' must not match every row");

        let query = create_search_query("weird%name");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos[0].file_path, percent_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_like_wildcards_literal_in_location() {
        let pool = create_test_db_pool().await.unwrap();
        let city_photo = create_test_photo_with_metadata(
            "cologne.jpg",
            "cologne-hash",
            json!({
                "location": {
                    "city": "Cologne"
                }
            }),
        );
        city_photo.create(&pool).await.unwrap();

        // `_` in a location token must not widen to a single-char wildcard
        let query = create_search_query("location:Col_gne");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert!(photos.is_empty());
    }

    #[tokio::test]
    async fn test_search_general_includes_city() {
        let pool = create_test_db_pool().await.unwrap();
        let berlin_photo = create_test_photo_with_metadata(
            "berlin-general.jpg",
            "berlin-general-hash",
            json!({
                "location": {
                    "city": "Berlin"
                }
            }),
        );

        berlin_photo.create(&pool).await.unwrap();

        let query = create_search_query("Berlin");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, berlin_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_photos_by_city_no_match() {
        let pool = create_test_db_pool().await.unwrap();
        let berlin_photo = create_test_photo_with_metadata(
            "berlin-no-match.jpg",
            "berlin-no-match-hash",
            json!({
                "location": {
                    "city": "Berlin"
                }
            }),
        );

        berlin_photo.create(&pool).await.unwrap();

        let query = create_search_query("location:Paris");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 0);
        assert!(photos.is_empty());
    }

    #[tokio::test]
    async fn test_search_combined_general_and_favorite() {
        let pool = create_test_db_pool().await.unwrap();
        let mut fav_photo =
            create_test_photo("sunset-fav.jpg".to_string(), "sunset-fav-hash".to_string());
        fav_photo.is_favorite = Some(true);
        fav_photo.create(&pool).await.unwrap();
        let plain_photo =
            create_test_photo("sunset.jpg".to_string(), "sunset-plain-hash".to_string());
        plain_photo.create(&pool).await.unwrap();

        let query = create_search_query("sunset is_favorite:true");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, fav_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_combined_general_and_type() {
        let pool = create_test_db_pool().await.unwrap();
        let mut video_photo =
            create_test_photo("sunset-vid.mp4".to_string(), "sunset-vid-hash".to_string());
        video_photo.mime_type = Some("video/mp4".to_string());
        video_photo.create(&pool).await.unwrap();
        let image_photo =
            create_test_photo("sunset.jpg".to_string(), "sunset-img-hash".to_string());
        image_photo.create(&pool).await.unwrap();

        let query = create_search_query("sunset type:video");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, video_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_location_multiple_words() {
        let pool = create_test_db_pool().await.unwrap();
        let ny_photo = create_test_photo_with_metadata(
            "new-york.jpg",
            "new-york-hash",
            json!({
                "location": {
                    "city": "New York"
                }
            }),
        );
        ny_photo.create(&pool).await.unwrap();

        let query = create_search_query("location:New York");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, ny_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_location_space_separated_after_colon() {
        let pool = create_test_db_pool().await.unwrap();
        let ny_photo = create_test_photo_with_metadata(
            "ny-space.jpg",
            "ny-space-hash",
            json!({
                "location": {
                    "city": "New York"
                }
            }),
        );
        ny_photo.create(&pool).await.unwrap();

        // "location: New York" (space after the colon) absorbs "New York"
        // with a leading space — the LIKE pattern must be trimmed or it
        // matches nothing.
        let query = create_search_query("location: New York");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos[0].file_path, ny_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_bare_location_token_is_skipped() {
        let pool = create_test_db_pool().await.unwrap();
        let photo = create_test_photo_with_metadata(
            "with-city.jpg",
            "with-city-hash",
            json!({
                "location": {
                    "city": "Berlin"
                }
            }),
        );
        photo.create(&pool).await.unwrap();
        // A second photo with a city that does NOT match the general token:
        // under the old LIKE '%%' behavior the bare location: token would
        // match BOTH rows (total 2); with the skip it contributes nothing.
        let other = create_test_photo_with_metadata(
            "other.jpg",
            "other-hash",
            json!({
                "location": {
                    "city": "Hamburg"
                }
            }),
        );
        other.create(&pool).await.unwrap();

        // A bare "location:" token must not filter (previously it emitted
        // LIKE '%%' which matched every row with a city).
        let query = create_search_query("with-city location:");
        let (_, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
    }

    #[tokio::test]
    async fn test_search_location_combined_with_general() {
        let pool = create_test_db_pool().await.unwrap();
        let berlin_photo = create_test_photo_with_metadata(
            "sunset-berlin.jpg",
            "sunset-berlin-hash",
            json!({
                "location": {
                    "city": "Berlin"
                }
            }),
        );
        berlin_photo.create(&pool).await.unwrap();

        let query = create_search_query("sunset location:Berlin");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, berlin_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_injection_shaped_tokens_are_literal() {
        let pool = create_test_db_pool().await.unwrap();
        let mut fav_photo = create_test_photo("sunset-fav.jpg".to_string(), "fav-hash".to_string());
        fav_photo.is_favorite = Some(true);
        fav_photo.create(&pool).await.unwrap();

        for query in ["is_favorite:true' OR '1'='1", "sunset' OR '1'='1 --"] {
            let (photos, total) = Photo::search_photos(
                &pool,
                &no_facts(),
                &create_search_query(query),
                50,
                0,
                None,
                None,
            )
            .await
            .unwrap();
            assert_eq!(total, 0, "query {query:?} must not match");
            assert!(photos.is_empty(), "query {query:?} must not match");
        }
    }

    #[tokio::test]
    async fn test_search_unknown_type_value_falls_back_to_general() {
        let pool = create_test_db_pool().await.unwrap();
        let raw_photo = create_test_photo("x_type:raw_y.jpg".to_string(), "raw-hash".to_string());
        raw_photo.create(&pool).await.unwrap();
        let plain_photo = create_test_photo("sunset.jpg".to_string(), "plain-hash".to_string());
        plain_photo.create(&pool).await.unwrap();

        let query = create_search_query("type:raw");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, raw_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_location_absorption_stops_at_prefix_token() {
        let pool = create_test_db_pool().await.unwrap();
        let mut ny_fav_photo = create_test_photo_with_metadata(
            "ny-fav.jpg",
            "ny-fav-hash",
            json!({ "location": { "city": "New York" } }),
        );
        ny_fav_photo.is_favorite = Some(true);
        ny_fav_photo.create(&pool).await.unwrap();
        let ny_plain_photo = create_test_photo_with_metadata(
            "ny-plain.jpg",
            "ny-plain-hash",
            json!({ "location": { "city": "New York" } }),
        );
        ny_plain_photo.create(&pool).await.unwrap();
        let mut berlin_fav_photo = create_test_photo_with_metadata(
            "berlin-fav.jpg",
            "berlin-fav-hash",
            json!({ "location": { "city": "Berlin" } }),
        );
        berlin_fav_photo.is_favorite = Some(true);
        berlin_fav_photo.create(&pool).await.unwrap();

        let query = create_search_query("location:New York is_favorite:true");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, ny_fav_photo.file_path);

        let mut ny_video_photo = create_test_photo_with_metadata(
            "ny-video.mp4",
            "ny-video-hash",
            json!({ "location": { "city": "New York" } }),
        );
        ny_video_photo.mime_type = Some("video/mp4".to_string());
        ny_video_photo.create(&pool).await.unwrap();
        let ny_image_photo = create_test_photo_with_metadata(
            "ny-image.jpg",
            "ny-image-hash",
            json!({ "location": { "city": "New York" } }),
        );
        ny_image_photo.create(&pool).await.unwrap();

        let query = create_search_query("location:New York type:video");
        let (photos, total) = Photo::search_photos(&pool, &no_facts(), &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, ny_video_photo.file_path);
    }

    #[tokio::test]
    async fn test_list_all_filtered_returns_every_match_without_pagination() {
        let pool = create_test_db_pool().await.unwrap();
        let facts = MediaFactsIndex::new();

        for index in 0..120 {
            // `create_test_photo` zero-pads short hashes to 64 chars, so
            // "bulk1" and "bulk10" would collapse to the same padded hash
            // (11 collisions across 0..120). Fixed-width digits keep them
            // distinct.
            let photo = create_test_photo(format!("bulk_{index}.jpg"), format!("bulk{index:03}"));
            photo.create(&pool).await.unwrap();
        }

        let photos = Photo::list_all_filtered(
            &pool,
            &facts,
            &SearchQuery {
                q: None,
                year: None,
                month: None,
                to_year: None,
                to_month: None,
            },
            None,
            None,
            None,
        )
        .await
        .unwrap();

        // The paginated endpoint caps at 100; the map listing must not truncate.
        assert_eq!(photos.len(), 120);
    }

    #[tokio::test]
    async fn test_list_all_filtered_applies_search_tokens_and_year() {
        let pool = create_test_db_pool().await.unwrap();
        let facts = MediaFactsIndex::new();

        let mut berlin_2020 = create_photo_with_facts(
            &pool,
            &facts,
            &"1".repeat(64),
            "berlin_2020.jpg",
            "2020-05-25T10:00:00Z",
        )
        .await;
        berlin_2020.metadata = json!({ "location": { "city": "Berlin" } });
        berlin_2020.update(&pool).await.unwrap();

        let mut berlin_2024 = create_photo_with_facts(
            &pool,
            &facts,
            &"2".repeat(64),
            "berlin_2024.jpg",
            "2024-05-25T10:00:00Z",
        )
        .await;
        berlin_2024.metadata = json!({ "location": { "city": "Berlin" } });
        berlin_2024.update(&pool).await.unwrap();

        let mut rome = create_photo_with_facts(
            &pool,
            &facts,
            &"3".repeat(64),
            "rome.jpg",
            "2020-05-25T10:00:00Z",
        )
        .await;
        rome.metadata = json!({ "location": { "city": "Rome" } });
        rome.update(&pool).await.unwrap();

        let photos = Photo::list_all_filtered(
            &pool,
            &facts,
            &SearchQuery {
                q: Some("location:Berlin".to_string()),
                year: Some(2020),
                month: None,
                to_year: None,
                to_month: None,
            },
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].filename, "berlin_2020.jpg");
    }

    #[tokio::test]
    async fn test_list_all_filtered_scopes_to_album() {
        let pool = create_test_db_pool().await.unwrap();

        let mut in_album = create_test_photo("in_album.jpg".to_string(), "inalbum".to_string());
        in_album.metadata = json!({ "location": { "latitude": 1.0, "longitude": 2.0 } });
        in_album.create(&pool).await.unwrap();

        let outside = create_test_photo("outside.jpg".to_string(), "outside".to_string());
        outside.create(&pool).await.unwrap();

        let album =
            crate::albums::create_with_members(&pool, "Trip", &[in_album.hash_sha256.clone()])
                .await
                .unwrap();

        let photos = Photo::list_all_filtered(
            &pool,
            &no_facts(),
            &SearchQuery {
                q: None,
                year: None,
                month: None,
                to_year: None,
                to_month: None,
            },
            None,
            None,
            Some(album.id),
        )
        .await
        .unwrap();

        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].hash_sha256, in_album.hash_sha256);
    }

    #[tokio::test]
    async fn persist_capability_and_duration_merges_without_clobbering() {
        let pool = create_in_memory_pool().await.expect("pool");
        // `photos.hash_sha256` carries a `length(...) = 64` CHECK constraint, so
        // the brief's short literal is padded to a valid hash.
        let hash = format!("{:0<64}", "hash-patch");
        let mut photo = create_test_photo("patch.mp4".to_string(), hash.to_string());
        photo.metadata = json!({ "camera": { "make": "Canon" } });
        photo.create(&pool).await.expect("create");

        Photo::persist_capability_and_duration(
            &pool,
            &hash,
            &json!({ "video": { "capability_version": 1, "codec": "h264" } }),
            Some(12.5),
        )
        .await
        .expect("patch");

        let stored = Photo::find_by_hash(&pool, &hash).await.unwrap().unwrap();
        assert_eq!(stored.metadata["camera"]["make"], "Canon");
        assert_eq!(stored.metadata["video"]["codec"], "h264");
        assert_eq!(stored.metadata["video"]["capability_version"], 1);
        assert_eq!(
            stored.duration,
            Some(12.5),
            "the patch and the derived duration land in the same write"
        );
    }

    #[tokio::test]
    async fn persist_capability_and_duration_never_overwrites_a_known_duration() {
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-duration");
        let mut photo = create_test_photo("duration.mp4".to_string(), hash.to_string());
        photo.duration = Some(9.0);
        photo.create(&pool).await.expect("create");

        Photo::persist_capability_and_duration(
            &pool,
            &hash,
            &json!({ "video": { "capability_version": 1, "codec": "h264" } }),
            Some(30.0),
        )
        .await
        .expect("patch");

        let stored = Photo::find_by_hash(&pool, &hash).await.unwrap().unwrap();
        assert_eq!(stored.duration, Some(9.0));
        assert_eq!(stored.metadata["video"]["capability_version"], 1);
    }

    /// The patch must not survive a failed duration write: `capability_version`
    /// alone completes the record for `video_probe::record_is_complete`, so a
    /// half-written pair would make the NULL duration permanent. The trigger
    /// makes the duration write fail deterministically.
    #[tokio::test]
    async fn persist_capability_and_duration_rolls_back_the_patch_with_the_duration() {
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-atomic");
        let mut photo = create_test_photo("atomic.mp4".to_string(), hash.to_string());
        photo.metadata = json!({ "camera": { "make": "Canon" } });
        photo.create(&pool).await.expect("create");

        sqlx::query(
            "CREATE TRIGGER fail_duration_update BEFORE UPDATE OF duration ON photos \
             BEGIN SELECT RAISE(ABORT, 'duration write refused'); END",
        )
        .execute(&pool)
        .await
        .expect("create failing trigger");

        let result = Photo::persist_capability_and_duration(
            &pool,
            &hash,
            &json!({ "video": { "capability_version": 1, "codec": "h264" } }),
            Some(12.5),
        )
        .await;
        assert!(result.is_err(), "the failing duration write must surface");

        let stored = Photo::find_by_hash(&pool, &hash).await.unwrap().unwrap();
        assert_eq!(stored.metadata["camera"]["make"], "Canon");
        assert!(
            stored.metadata.get("video").is_none(),
            "the patch must roll back with the duration (result: {result:?})"
        );
        assert_eq!(stored.duration, None);
    }
}
