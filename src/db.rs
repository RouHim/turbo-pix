use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, NaiveDateTime, Utc};
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, Row};

use crate::media_facts::MediaFacts;

pub use crate::db_pool::{create_db_pool, delete_orphaned_photos, vacuum_database, DbPool};
pub use crate::db_types::{SearchQuery, TimelineData, TimelineDensity};

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

    // === FILE-DERIVED FACTS ===
    /// The capture date as the file carries it.
    ///
    /// Read-only in the database sense: only the scan's upsert, the two
    /// metadata saves (`Photo::update` from a fresh file read, and
    /// `Photo::record_video_edit` from a container readback), and the rotation
    /// rewrite (`Photo::update_with_old_hash`, which restates the facts it
    /// preserves) write it, and each derives it from the file *before* the row
    /// write. No API path may write it from client input, and the scan
    /// reconciles it on every pass, so a divergence heals at the next scan
    /// instead of ageing in place. The favorite paths therefore own only
    /// `is_favorite`, through [`Photo::set_favorite`].
    #[serde(deserialize_with = "deserialize_optional_datetime")]
    pub taken_at: Option<DateTime<Utc>>,
    /// The file's coordinates, with the same ownership as `taken_at`. Never
    /// serialized at the top level: [`crate::photo_reply::PhotoReply`] nests
    /// them under `metadata.location`, which is the API's shape.
    #[serde(skip)]
    pub taken_latitude: Option<f64>,
    /// See [`Photo::taken_latitude`].
    #[serde(skip)]
    pub taken_longitude: Option<f64>,
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
    /// location{city,...}, video{codec,audio_codec,bitrate,frame_rate}. Stored
    /// documents hold no coordinates: [`stored_metadata`] strips the pair from
    /// every write, and [`crate::photo_reply::PhotoReply`] nests the row's
    /// derived pair into the response.
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
/// Capture coordinates live in the photo's derived columns, so the two
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
/// The default (and `date`) sort is the derived capture date itself: an
/// unknown date is `NULL`, which SQLite orders first ascending / last
/// descending — exactly the "unknown" contract.
pub(crate) fn build_order_clause(sort: Option<&str>, order: Option<&str>) -> String {
    let sort_field = match sort {
        Some("filename") | Some("name") => "filename",
        Some("file_size") | Some("size") => "file_size",
        Some("created_at") => "created_at",
        _ => "taken_at",
    };
    let sort_order = match order {
        Some("asc") => "ASC",
        _ => "DESC", // default
    };
    format!("{sort_field} {sort_order}, hash_sha256 {sort_order}")
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
            taken_at: row
                .try_get::<Option<String>, _>("taken_at")?
                .and_then(|value| parse_datetime(&value)),
            taken_latitude: row.try_get("taken_latitude")?,
            taken_longitude: row.try_get("taken_longitude")?,
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

/// The SQL bounds of an inclusive month-granular range, as the `YYYY-MM`
/// prefixes `taken_at` starts with: `from` is the range's first month,
/// `to_exclusive` the month after its last, so callers compare
/// `taken_at >= from AND taken_at < to_exclusive`. RFC3339 UTC text compares
/// chronologically (`'2012-03-15T…' >= '2012-03'` and `< '2012-04'`), and a
/// `NULL` date never matches either comparison.
///
/// `year`/`month` is the start bound, `to_year`/`to_month` the end bound; an
/// absent end bound makes the filter a single period. A missing month means
/// January for a start bound and December for an end bound, so `year=2012`
/// still means "all of 2012" and `from=2012-03,to=2015` stops at December
/// 2015. A month outside `1..=12` is not formattable and would otherwise
/// become a valid ordering point (`2012-13` sorts after every 2012 row), so
/// such a bound yields an inverted interval that no row satisfies — the old
/// equality filter's "month = 13 matches nothing". A year outside `1..=9999`
/// gets the same treatment: the prefix is four digits, so `10000-01` sorts
/// before every real date and would silently match everything instead. An end
/// bound whose month after it would exceed 9999 (December 9999) saturates to
/// `"9999-99"`, which orders after every four-digit date, so a range that ends
/// there still reaches the end of the timeline. `to_year` without `year`
/// filters nothing, because a bound is only formed from `year`.
pub(crate) fn month_range_sql_bounds(
    year: Option<i32>,
    month: Option<i32>,
    to_year: Option<i32>,
    to_month: Option<i32>,
) -> Option<(String, String)> {
    let year = year?;
    let from_month = month.unwrap_or(1);
    let end_year = to_year.unwrap_or(year);
    let end_month = match to_year {
        Some(_) => to_month.unwrap_or(12),
        None => month.unwrap_or(12),
    };
    // An unrepresentable year is as unformattable as an impossible month: the
    // five-digit `10000-01` prefix sorts before `2012-…`, so it would turn a
    // bound meant to match nothing into one that matches everything.
    if !(1..=9999).contains(&year) || !(1..=9999).contains(&end_year) {
        return Some(("9999-99".to_string(), "0000-00".to_string()));
    }
    if !(1..=12).contains(&from_month) || !(1..=12).contains(&end_month) {
        return Some(("9999-99".to_string(), "0000-00".to_string()));
    }
    // December 9999 has no four-digit month after it; `"10000-01"` would sort
    // before every real date, so the exclusive end saturates instead.
    let to_exclusive = if end_month == 12 && end_year == 9999 {
        "9999-99".to_string()
    } else if end_month == 12 {
        format!("{:04}-{:02}", end_year + 1, 1)
    } else {
        format!("{:04}-{:02}", end_year, end_month + 1)
    };
    Some((format!("{:04}-{:02}", year, from_month), to_exclusive))
}

/// Builds the reusable WHERE clause for photo searches: the `q` token grammar
/// (`type:` / `is_favorite:` / `location:` / general LIKE) and the inclusive
/// month range on the derived `taken_at`. Returns the clause (starting with
/// `" WHERE 1=1"`) plus its string parameters in placeholder order. Used by the
/// photo listing module so all listings honor identical filter semantics.
pub(crate) fn build_search_where(query: &SearchQuery) -> (String, Vec<String>) {
    let mut where_clause = String::from(" WHERE 1=1");
    let mut params: Vec<String> = Vec::new();

    if let Some((from, to_exclusive)) =
        month_range_sql_bounds(query.year, query.month, query.to_year, query.to_month)
    {
        where_clause.push_str(" AND taken_at >= ? AND taken_at < ?");
        params.push(from);
        params.push(to_exclusive);
    }

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

    // ===== DATABASE OPERATIONS =====

    /// Records the facts a fresh read of the file yielded. The only writers are
    /// the file-reading paths (scan upsert, EXIF save, container save): a
    /// client value must never reach these columns, which is why the signature
    /// takes [`MediaFacts`] and not three loose options.
    pub fn set_capture_facts(&mut self, facts: MediaFacts) {
        self.taken_at = facts.taken_at;
        self.taken_latitude = facts.latitude;
        self.taken_longitude = facts.longitude;
    }

    /// Update photo (convenience wrapper)
    pub async fn update(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let mut tx = pool.begin().await?;
        self.update_with_transaction(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Writes what a video metadata save owns into the row — and nothing else.
    ///
    /// A save re-reads its row, spends a container write and a readback on the
    /// file, and only then records what the container ended up holding, so
    /// flushing the struct it read reverts whatever was committed in that
    /// window — and a resolved place name is lost for good that way, because
    /// its `geo_location_resolved` flag is already 1, so nothing ever derives
    /// the name again. This statement therefore names the save's own members
    /// and merges them into the row AS IT IS NOW (RFC 7396, like
    /// [`Self::persist_capability_and_duration`]).
    ///
    /// `fingerprint` is optional because a save owns only what it actually
    /// wrote: a row that does not describe the file being patched must keep its
    /// stale fingerprint — that staleness is what makes the next scan
    /// re-extract the file, see [`Self::find_unchanged_photo`].
    ///
    /// `rearm_geo_resolution` is the caller's statement that this save moved
    /// the photo's coordinates: the place name was geocoded from the position
    /// that is GONE, so it is dropped with an explicit null and
    /// `geo_location_resolved` goes back to 0. The flag matters twice over —
    /// `get_photos_needing_geo_resolution` selects exactly the rows with it
    /// clear, and a save restates the file's fingerprint, so nothing downstream
    /// of the scan would correct a stale name left next to the new pin. A save
    /// that did not move the photo passes `false` and leaves both alone.
    ///
    /// `updated_at` is the instant the caller stamps into the row. It is taken
    /// once, by the caller, so the row and the response it hands back carry the
    /// same instant: stamping a second `Utc::now()` here would leave the reply
    /// behind the committed row, which says the save happened at a moment it
    /// did not.
    ///
    /// Fails when no row matched: `UPDATE ... WHERE hash_sha256 = ?` reports
    /// success even when it touched nothing, and a caller that already rewrote
    /// the container must not answer 200 while no row describes the file it
    /// patched. The caller's rollback path then puts the file back.
    pub async fn record_video_edit(
        pool: &DbPool,
        hash_sha256: &str,
        rearm_geo_resolution: bool,
        fingerprint: Option<(i64, DateTime<Utc>)>,
        facts: MediaFacts,
        updated_at: DateTime<Utc>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (file_size, file_modified) = fingerprint
            .map(|(size, modified)| (size, modified.to_rfc3339()))
            .unzip();
        let mut tx = pool.begin().await?;
        sqlx::query(
            r#"
            UPDATE photos SET
                -- The save's date and coordinates land in their columns (bound
                -- from the container readback), and `stored_metadata` strips the
                -- coordinate pair from this document on every write anyway. The
                -- only DOCUMENT member the save invalidates is the place name
                -- geocoded from the position it replaced.
                metadata = CASE WHEN ? THEN json_patch(
                                    CASE WHEN json_type(metadata) = 'object' THEN metadata ELSE '{}' END,
                                    '{"location":{"city":null}}')
                               ELSE metadata END,
                geo_location_resolved = CASE WHEN ? THEN 0 ELSE geo_location_resolved END,
                file_size = COALESCE(?, file_size),
                file_modified = COALESCE(?, file_modified),
                -- The container was rewritten before this row write; these are
                -- the values the readback proved it now holds (the save's own
                -- members, restated per carrier precision).
                taken_at = ?,
                taken_latitude = ?,
                taken_longitude = ?,
                updated_at = ?
            WHERE hash_sha256 = ?
            "#,
        )
        .bind(rearm_geo_resolution)
        .bind(rearm_geo_resolution)
        .bind(file_size)
        .bind(&file_modified)
        .bind(facts.taken_at.map(|dt| dt.to_rfc3339()))
        .bind(facts.latitude)
        .bind(facts.longitude)
        .bind(updated_at.to_rfc3339())
        .bind(hash_sha256)
        .execute(&mut *tx)
        .await?;
        let affected = sqlx::query("SELECT changes()")
            .fetch_one(&mut *tx)
            .await?
            .try_get::<i64, _>(0)?;
        if affected == 0 {
            return Err(format!(
                "Photo with hash {} no longer exists (deleted while it was being edited?) — update matched 0 rows",
                hash_sha256
            )
            .into());
        }
        tx.commit().await?;
        Ok(())
    }

    /// Record a favorite change and nothing else.
    ///
    /// The favorite paths own `is_favorite`, not the file-derived facts: they
    /// never read the file, so rewriting the whole row from their snapshot
    /// would revert a scan reconciliation (or a metadata save) that landed in
    /// between. This statement therefore touches only the column the UI owns
    /// plus `updated_at`, which the caller stamps so its reply carries the
    /// instant the row committed.
    ///
    /// Fails when no row matched, like [`Photo::record_video_edit`]: an
    /// `UPDATE` reports success even when it touched nothing, and the caller
    /// must not answer 200 for a photo that vanished mid-request.
    pub async fn set_favorite(
        pool: &DbPool,
        hash_sha256: &str,
        is_favorite: bool,
        updated_at: DateTime<Utc>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut tx = pool.begin().await?;
        sqlx::query("UPDATE photos SET is_favorite = ?, updated_at = ? WHERE hash_sha256 = ?")
            .bind(is_favorite)
            .bind(updated_at.to_rfc3339())
            .bind(hash_sha256)
            .execute(&mut *tx)
            .await?;
        let affected = sqlx::query("SELECT changes()")
            .fetch_one(&mut *tx)
            .await?
            .try_get::<i64, _>(0)?;
        if affected == 0 {
            return Err(format!(
                "Photo with hash {} no longer exists (deleted mid-request?) — favorite update matched 0 rows",
                hash_sha256
            )
            .into());
        }
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
    /// [`crate::video_capability::record_is_complete`], so the probe would
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
                file_modified, date_indexed, created_at, updated_at,
                taken_at, taken_latitude, taken_longitude
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                ?20, ?21, ?22
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
        .bind(self.taken_at.map(|dt| dt.to_rfc3339()))
        .bind(self.taken_latitude)
        .bind(self.taken_longitude)
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
    ///
    /// This is the row writer for the paths that own the FILE's facts: the
    /// image metadata save (from its fresh EXIF readback). It restates
    /// `taken_at`/`taken_latitude`/`taken_longitude` from the snapshot, so a
    /// caller that does not derive them from the file must use a targeted
    /// statement instead — [`Photo::set_favorite`] for the favorite paths — or
    /// it reverts a concurrent scan reconciliation. (The rotation rewrite uses
    /// [`Photo::update_with_old_hash`], which restates the same facts while
    /// rekeying the row.)
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
                taken_at = ?, taken_latitude = ?, taken_longitude = ?,
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
        .bind(self.taken_at.map(|dt| dt.to_rfc3339()))
        .bind(self.taken_latitude)
        .bind(self.taken_longitude)
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
                taken_at = ?, taken_latitude = ?, taken_longitude = ?,
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
        .bind(self.taken_at.map(|dt| dt.to_rfc3339()))
        .bind(self.taken_latitude)
        .bind(self.taken_longitude)
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
    /// 2. Uses UPSERT to insert if new, or update if `hash_sha256` already exists,
    ///    merging the scan's metadata document into the row's (RFC 7396) rather
    ///    than replacing it: the scan owns the members it writes, and the
    ///    resolved place name and the video probe record survive a rescan. A row
    ///    whose fingerprint is not the file's describes other bytes, so its
    ///    place name and probe record are cleared with the same statement.
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
                file_modified, date_indexed, created_at, updated_at,
                taken_at, taken_latitude, taken_longitude
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                ?20, ?21, ?22
            )
            ON CONFLICT(hash_sha256) DO UPDATE SET
                file_path = excluded.file_path,
                filename = excluded.filename,
                file_size = excluded.file_size,
                mime_type = excluded.mime_type,
                -- The scan's statement of what the file carries is the whole
                -- reconciliation: every pass re-states it, so an edit made
                -- outside the app heals here and no stale copy survives a scan.
                taken_at = excluded.taken_at,
                taken_latitude = excluded.taken_latitude,
                taken_longitude = excluded.taken_longitude,

                width = excluded.width,
                height = excluded.height,
                orientation = excluded.orientation,
                duration = excluded.duration,
                thumbnail_path = excluded.thumbnail_path,
                has_thumbnail = excluded.has_thumbnail,
                blurhash = excluded.blurhash,
                is_favorite = COALESCE(photos.is_favorite, excluded.is_favorite),
                semantic_vector_indexed = excluded.semantic_vector_indexed,
                -- RFC 7396 (SQLite `json_patch`), not a replacement: the scan's
                -- document names the facts it owns and every other member the
                -- row holds survives it — the resolved place name
                -- (`$.location.city`) and the video probe record
                -- (`$.video.capability_version`) among them. Replacing the
                -- document wholesale dropped both on every pass, and every
                -- resume re-upserts even the rows the unchanged-file fast path
                -- built; neither member is ever re-derived, because
                -- `get_photos_needing_geo_resolution` selects only the rows with
                -- `geo_location_resolved = 0` (the name is lost for good when
                -- that flag is already 1) and a dropped `capability_version`
                -- makes `video_probe::record_is_complete` force a re-probe on
                -- the next playback.
                --
                -- A row whose fingerprint is not the file's holds facts about
                -- DIFFERENT bytes, so the groups the scan owns are dropped out
                -- of it first (the same wholesale clearing, narrowed to the rows
                -- it is right for): the place name was geocoded from a position
                -- the changed file may no longer carry, and the probe record is
                -- a verdict on the old layout, which would otherwise keep
                -- `record_is_complete` true while `plan` reads its dropped
                -- `moov_at_start` as "never established". `json_remove` rather
                -- than null members, so a row that carried none of these keys
                -- does not acquire emptied ones (RFC 7396 creates the member for
                -- a nested patch).
                metadata = json_remove(
                    CASE
                        WHEN photos.file_size = excluded.file_size
                             AND photos.file_modified = excluded.file_modified
                        THEN json_patch(
                                 CASE WHEN json_type(photos.metadata) = 'object'
                                      THEN photos.metadata
                                      ELSE '{}' END,
                                 excluded.metadata)
                        ELSE json_patch(
                                 json_remove(
                                     CASE WHEN json_type(photos.metadata) = 'object'
                                          THEN photos.metadata
                                          ELSE '{}' END,
                                     '$.camera', '$.settings', '$.video', '$.location.city'),
                                 excluded.metadata)
                    END,
                    -- Stored coordinate keys are legacy residue: the pair lives
                    -- in `taken_latitude`/`taken_longitude`, `stored_metadata`
                    -- strips it out of what the scan writes, and this clears one
                    -- a pre-migration row kept — the file is authoritative for
                    -- its own position.
                    '$.location.latitude',
                    '$.location.longitude'),
                -- The place name that branch drops was geocoded from the
                -- position the changed file may no longer carry, so the gate
                -- that makes the resolver look at the row again goes back to 0
                -- with it: `get_photos_needing_geo_resolution` selects exactly
                -- the rows with it clear, and a row left at 1 would keep no
                -- name and never be handed to `geo_location::resolve_batch` —
                -- losing the map label and the `location:` search hits for
                -- good. An unchanged row keeps both the name the merge above
                -- preserves and its flag, deliberately: its coordinates are
                -- still the ones the file carries, which is the same pairing
                -- `record_video_edit` makes for a save (it re-arms only when
                -- the save moved the position).
                geo_location_resolved = CASE
                    WHEN photos.file_size = excluded.file_size
                         AND photos.file_modified = excluded.file_modified
                    THEN geo_location_resolved
                    ELSE 0
                END,
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
        .bind(self.taken_at.map(|dt| dt.to_rfc3339()))
        .bind(self.taken_latitude)
        .bind(self.taken_longitude)
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

    pub async fn get_timeline_data(
        pool: &DbPool,
    ) -> Result<TimelineData, Box<dyn std::error::Error>> {
        // One pass over the derived dates; an undated row contributes no bucket.
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT taken_at FROM photos WHERE taken_at IS NOT NULL")
                .fetch_all(pool)
                .await?;

        let mut min_date: Option<DateTime<Utc>> = None;
        let mut max_date: Option<DateTime<Utc>> = None;
        let mut buckets: BTreeMap<(i32, i32), i64> = BTreeMap::new();

        for raw in rows {
            let Some(taken_at) = parse_datetime(&raw) else {
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

        // Video members are stated only for a video row: a photo must not
        // acquire a `video` object it never had. `codec`, `audio_codec`,
        // `bit_depth` and `container` are what `ResolvedCapabilities::from_record`
        // reads back; `bitrate`, `frame_rate` and `profile` are display facts
        // the scan owns just as much, so they must not outlive the file's own
        // values either.
        let mut video = serde_json::Map::new();
        if processed
            .mime_type
            .as_deref()
            .is_some_and(|mime| mime.starts_with("video/"))
        {
            video.insert("codec".to_string(), json!(processed.video_codec));
            video.insert("audio_codec".to_string(), json!(processed.audio_codec));
            video.insert("bitrate".to_string(), json!(processed.bitrate));
            video.insert("frame_rate".to_string(), json!(processed.frame_rate));
            video.insert("profile".to_string(), json!(processed.video_profile));
            video.insert("bit_depth".to_string(), json!(processed.bit_depth));
            video.insert("container".to_string(), json!(processed.container));
            // `moov_at_start` keeps its "absence means true" idiom: the
            // scanner's own fallback for a failed layout pass is `true`
            // (`photo_processor`), and a persisted `true` is exactly what
            // `stored_moov_at_start` trusts for `Delivery::Direct` — the
            // scenario `video_probe` warns about. Absent stays "never
            // established", which no playback decision reads as progressive.
            // That leaves a stored `false` on bytes that are gone with nothing
            // to replace it, so the upsert drops it (and the capability marker
            // that would keep it trusted) for a row whose file changed — see
            // `create_or_update_with_transaction`.
            if !processed.moov_at_start {
                video.insert("moov_at_start".to_string(), json!(false));
            }
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
            // The file facts the scan just read: this row is the derived copy
            // the listing orders by, and the upsert restates it on every pass.
            taken_at: processed.taken_at,
            taken_latitude: processed.latitude,
            taken_longitude: processed.longitude,
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
/// carries: the row's derived pair, narrowed in SQL to unresolved rows that
/// have one.
pub async fn get_photos_needing_geo_resolution(
    pool: &DbPool,
) -> Result<Vec<(String, f64, f64)>, Box<dyn std::error::Error>> {
    let photos: Vec<(String, f64, f64)> = sqlx::query_as(
        "SELECT file_path, taken_latitude, taken_longitude FROM photos
         WHERE (geo_location_resolved IS NULL OR geo_location_resolved = 0)
           AND taken_latitude IS NOT NULL AND taken_longitude IS NOT NULL
         ORDER BY file_path",
    )
    .fetch_all(pool)
    .await?;

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
    use chrono::{Datelike, TimeZone};
    use sqlx::Row;

    use crate::photo_listing::{ListingPage, ListingRequest};

    async fn search(
        pool: &DbPool,
        query: &SearchQuery,
        limit: i64,
        offset: i64,
        sort: Option<&str>,
        order: Option<&str>,
    ) -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>> {
        let listing = crate::photo_listing::list(
            pool,
            ListingRequest {
                query,
                album: None,
                sort,
                order,
                page: Some(ListingPage { limit, offset }),
            },
        )
        .await?;
        Ok((listing.photos, listing.total as i64))
    }

    async fn list_all_filtered(
        pool: &DbPool,
        query: &SearchQuery,
        sort: Option<&str>,
        order: Option<&str>,
        album: Option<i64>,
    ) -> Result<Vec<Photo>, Box<dyn std::error::Error>> {
        let listing = crate::photo_listing::list(
            pool,
            ListingRequest {
                query,
                album,
                sort,
                order,
                page: None,
            },
        )
        .await?;
        Ok(listing.photos)
    }

    pub(crate) fn create_test_photo(filename: String, hash: String) -> Photo {
        // Ensure hash is 64 characters for SHA256
        let hash_64 = if hash.len() < 64 {
            format!("{:0<64}", hash)
        } else {
            hash
        };
        // No date and no coordinates unless a test states them: they are the
        // file's facts, held in the derived columns (see `create_photo_with_date`).
        Photo {
            hash_sha256: hash_64,
            file_path: format!("./test/{}", filename),
            filename,
            file_size: 1024,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: None,
            taken_latitude: None,
            taken_longitude: None,
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

    /// Builds the row with the derived capture date the scan would write.
    async fn create_photo_with_date(
        pool: &DbPool,
        hash: &str,
        filename: &str,
        taken_at: &str,
    ) -> Photo {
        let mut photo = create_test_photo(filename.to_string(), hash.to_string());
        photo.taken_at = Some(
            DateTime::parse_from_rfc3339(taken_at)
                .expect("invalid RFC3339 date")
                .with_timezone(&Utc),
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

    #[test]
    fn month_range_sql_bounds_are_inclusive_prefixes() {
        // GIVEN: `year=2012&month=3&to_year=2015` (an absent end month is December)
        let (from, to_exclusive) = month_range_sql_bounds(Some(2012), Some(3), Some(2015), None)
            .expect("year forms bounds");
        // THEN: the range starts at March 2012 and stops before January 2016, so
        // December 2015 is still inside it
        assert_eq!(from, "2012-03");
        assert_eq!(to_exclusive, "2016-01");

        // AND: a single year spans January through December
        let (from, to_exclusive) =
            month_range_sql_bounds(Some(2020), None, None, None).expect("year forms bounds");
        assert_eq!(
            (from.as_str(), to_exclusive.as_str()),
            ("2020-01", "2021-01")
        );

        // AND: a single month is that month only
        let (from, to_exclusive) =
            month_range_sql_bounds(Some(2020), Some(12), None, None).expect("year forms bounds");
        assert_eq!(
            (from.as_str(), to_exclusive.as_str()),
            ("2020-12", "2021-01")
        );

        // AND: an impossible month yields an inverted interval no row satisfies
        let (from, to_exclusive) = month_range_sql_bounds(Some(2012), Some(13), Some(2015), None)
            .expect("year forms bounds");
        assert!(from > to_exclusive);

        // AND: an end bound without a start filters nothing
        assert!(month_range_sql_bounds(None, None, Some(2015), None).is_none());

        // AND: an end bound at the schema's ceiling still reaches every later
        // row, because the exclusive end saturates past every four-digit date
        let (from, to_exclusive) =
            month_range_sql_bounds(Some(2012), None, Some(9999), None).expect("year forms bounds");
        assert_eq!(
            (from.as_str(), to_exclusive.as_str()),
            ("2012-01", "9999-99")
        );

        // AND: December 9999 is its own last month, so the end saturates too
        let (from, to_exclusive) =
            month_range_sql_bounds(Some(9999), Some(12), None, None).expect("year forms bounds");
        assert_eq!(
            (from.as_str(), to_exclusive.as_str()),
            ("9999-12", "9999-99")
        );

        // AND: a year beyond four digits is unrepresentable and must filter
        // nothing, never match everything
        let (from, to_exclusive) = month_range_sql_bounds(Some(i32::MAX), Some(12), None, None)
            .expect("year forms bounds");
        assert_eq!(
            (from.as_str(), to_exclusive.as_str()),
            ("9999-99", "0000-00")
        );
    }

    #[tokio::test]
    async fn test_get_timeline_data() {
        let pool = create_test_db_pool().await.unwrap();
        // The dates live in the files; the index is their in-memory stand-in.

        create_photo_with_date(&pool, &"a".repeat(64), "photo1.jpg", "2010-05-25T10:00:00Z").await;
        create_photo_with_date(&pool, &"b".repeat(64), "photo2.jpg", "2010-05-26T10:00:00Z").await;
        create_photo_with_date(&pool, &"c".repeat(64), "photo3.jpg", "2011-12-01T10:00:00Z").await;
        create_photo_with_date(&pool, &"d".repeat(64), "photo4.jpg", "2024-01-15T10:00:00Z").await;

        // Get timeline data
        let timeline = Photo::get_timeline_data(&pool).await.unwrap();

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
        // shifts the trailing values so ?22=updated_at binds NULL.
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

    /// The scan's reconciliation at the row write: the upsert stores the file's
    /// date and both coordinates, and a same-hash rescan that found new facts
    /// replaces the derived copy with them.
    #[tokio::test]
    async fn test_create_or_update_upsert_carries_file_facts() {
        let pool = create_test_db_pool().await.unwrap();
        let mut photo = create_test_photo("scan-facts.jpg".to_string(), "d".repeat(64));
        photo.taken_at = Some(Utc.with_ymd_and_hms(2020, 5, 25, 10, 0, 0).unwrap());
        photo.taken_latitude = Some(48.137);
        photo.taken_longitude = Some(11.575);
        photo.create_or_update(&pool).await.unwrap();

        let found = Photo::find_by_hash(&pool, &photo.hash_sha256)
            .await
            .unwrap()
            .expect("upserted photo must be readable");
        assert_eq!(found.taken_at, photo.taken_at, "the date must survive");
        assert_eq!(
            found.taken_latitude, photo.taken_latitude,
            "the latitude must survive"
        );
        assert_eq!(
            found.taken_longitude, photo.taken_longitude,
            "the longitude must survive"
        );

        // A later scan finds a different date and position for the same file.
        photo.taken_at = Some(Utc.with_ymd_and_hms(2024, 8, 1, 9, 15, 0).unwrap());
        photo.taken_latitude = Some(52.52);
        photo.taken_longitude = Some(13.405);
        photo.create_or_update(&pool).await.unwrap();

        let found = Photo::find_by_hash(&pool, &photo.hash_sha256)
            .await
            .unwrap()
            .expect("rescan must keep the row readable");
        assert_eq!(
            found.taken_at, photo.taken_at,
            "a rescan must overwrite the date the row carried"
        );
        assert_eq!(
            found.taken_latitude, photo.taken_latitude,
            "a rescan must overwrite the latitude the row carried"
        );
        assert_eq!(
            found.taken_longitude, photo.taken_longitude,
            "a rescan must overwrite the longitude the row carried"
        );
    }

    /// The schema contract: the derived copy of the file's date and
    /// coordinates exists as real columns, and the date is indexed so listings
    /// page in SQL. Stored metadata documents stay free of coordinates.
    #[tokio::test]
    async fn test_photos_schema_carries_the_derived_file_facts() {
        let pool = create_test_db_pool().await.unwrap();

        let columns: Vec<String> = sqlx::query("PRAGMA table_info(photos)")
            .map(|row: sqlx::sqlite::SqliteRow| row.get::<String, _>("name"))
            .fetch_all(&pool)
            .await
            .unwrap();
        for expected in ["taken_at", "taken_latitude", "taken_longitude"] {
            assert!(
                columns.iter().any(|name| name == expected),
                "photos must carry the derived {expected}, found columns {columns:?}"
            );
        }

        let taken_at_index: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_photos_taken_at'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(
            taken_at_index.is_some(),
            "idx_photos_taken_at must exist for the date-ordered listing"
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

        // The response shape (coordinates nested from the row's derived pair)
        // must not smuggle them back into the document on update.
        let mut enriched = photo.clone();
        enriched.taken_latitude = Some(48.1);
        enriched.taken_longitude = Some(11.5);
        let enriched = crate::photo_reply::PhotoReply::from_row(enriched).into_inner();
        assert!(
            enriched.metadata["location"].get("latitude").is_some(),
            "the reply must nest the row's coordinates into location"
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

    /// A scan owns the video facts it writes, so the document it stores holds
    /// exactly those facts: a row's identity is the PATH hash, so replacing the
    /// file in place at the same path writes the fresh extraction's document
    /// wholesale and never has to DELETE the row first. An omitted member
    /// therefore clears the stored value, `capability_version` no longer keeps
    /// `record_is_complete` true, and `video_probe::resolve` re-probes instead
    /// of short-circuiting to `ResolvedCapabilities::from_record` — trusting
    /// facts the file no longer has.
    #[tokio::test]
    async fn scan_upsert_clears_stale_video_facts_so_the_record_is_reprobed() {
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-builder-stale");
        // GIVEN: an mp4 once probed as h264 with audio
        let mut existing = create_test_photo("replaced.mp4".to_string(), hash.clone());
        existing.metadata = json!({
            "video": {
                "codec": "h264",
                "audio_codec": "aac",
                "bit_depth": 10,
                "container": "mp4",
                "frame_rate": 30.0,
                "capability_version": 1
            }
        });
        existing.create(&pool).await.expect("create");

        // WHEN: the path now holds an audio-only mp4 and the scan writes what
        // the extraction found — no video-stream facts at all
        let mut extracted = scanned_photo("replaced.mp4", &hash, "video/mp4", None, None);
        extracted.container = Some("mp4".to_string());
        let fresh: Photo = extracted.into();
        let mut tx = pool.begin().await.expect("tx");
        fresh
            .create_or_update_with_transaction(&mut tx)
            .await
            .expect("upsert");
        tx.commit().await.expect("commit");

        // THEN: every stale fact is gone
        let stored = Photo::find_by_hash(&pool, &hash)
            .await
            .expect("read")
            .expect("row");
        assert!(
            stored.video_codec().is_none(),
            "stale codec survived: {}",
            stored.metadata
        );
        assert!(
            stored.audio_codec().is_none(),
            "stale audio codec survived: {}",
            stored.metadata
        );
        assert!(
            stored.bit_depth().is_none(),
            "stale bit depth survived: {}",
            stored.metadata
        );
        assert!(
            stored.frame_rate().is_none(),
            "stale frame rate survived: {}",
            stored.metadata
        );
        // AND: what the file does have is recorded
        assert_eq!(stored.metadata["video"]["container"], "mp4");
        // AND: the record no longer looks probed, so the next request re-probes
        assert!(
            !crate::video_capability::record_is_complete(&stored),
            "a record whose video facts are gone must be re-probed: {}",
            stored.metadata
        );
    }

    /// Same mechanism for the EXIF-owned members: a photo whose EXIF was
    /// stripped must not keep showing the camera and settings it had, and a
    /// photo row must not acquire a `video` object.
    #[tokio::test]
    async fn scan_upsert_clears_camera_and_settings_the_file_no_longer_has() {
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-builder-exif");
        let mut existing = create_test_photo("stripped.jpg".to_string(), hash.clone());
        existing.metadata = json!({
            "camera": { "make": "Canon", "model": "Canon EOS 40D" },
            "settings": { "iso": 400, "aperture": 4.0, "flash_used": true }
        });
        existing.create(&pool).await.expect("create");

        // WHEN: the file is replaced by one whose EXIF carries none of that
        let fresh: Photo = scanned_photo("stripped.jpg", &hash, "image/jpeg", None, None).into();
        let mut tx = pool.begin().await.expect("tx");
        fresh
            .create_or_update_with_transaction(&mut tx)
            .await
            .expect("upsert");
        tx.commit().await.expect("commit");

        // THEN: the stale EXIF members are cleared
        let stored = Photo::find_by_hash(&pool, &hash)
            .await
            .expect("read")
            .expect("row");
        assert!(
            stored.camera_make().is_none(),
            "stale camera make survived: {}",
            stored.metadata
        );
        assert!(
            stored.camera_model().is_none(),
            "stale camera model survived: {}",
            stored.metadata
        );
        assert!(
            stored.iso().is_none(),
            "stale iso survived: {}",
            stored.metadata
        );
        assert!(
            stored.aperture().is_none(),
            "stale aperture survived: {}",
            stored.metadata
        );
        assert!(!stored.flash_used().unwrap_or(false));
        // AND: a photo row still carries no video object
        assert!(
            stored.metadata.get("video").is_none(),
            "{}",
            stored.metadata
        );
    }

    /// A stored `moov` verdict describes the bytes it was taken from. A scan
    /// of a changed file stores the fresh extraction's document wholesale, so
    /// an extraction that states no layout (the scanner's `true` is either an
    /// established progressive layout or its failed-pass fallback, so the
    /// builder cannot state it) leaves the row with no verdict at all — and the
    /// capability marker goes with it, or `video_probe::record_is_complete`
    /// stays true, `resolve` short-circuits and `plan` reads a dropped layout
    /// as "never established" forever instead of probing once and persisting an
    /// honest verdict.
    #[tokio::test]
    async fn scan_upsert_drops_a_stale_moov_verdict_for_a_changed_file() {
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-moov-changed");
        // GIVEN: an mp4 probed as moov-at-end, on bytes that are now gone
        let mut existing = create_test_photo("changed.mp4".to_string(), hash.clone());
        existing.mime_type = Some("video/mp4".to_string());
        existing.metadata = json!({
            "video": {
                "codec": "h264",
                "audio_codec": "aac",
                "capability_version": 1,
                "moov_at_start": false
            }
        });
        existing.create(&pool).await.expect("create");

        // WHEN: the file changed and the fresh extraction states no layout
        // (this is what the builder produces for a faststart file: the layout
        // member is only ever stated in the false direction)
        let mut rescan = existing.clone();
        rescan.file_size += 1;
        rescan.metadata = json!({
            "video": { "codec": "h264", "audio_codec": "aac", "container": "mp4" }
        });
        let mut tx = pool.begin().await.expect("tx");
        rescan
            .create_or_update_with_transaction(&mut tx)
            .await
            .expect("upsert");
        tx.commit().await.expect("commit");

        // THEN: the verdict on the old bytes is gone, and the marker that kept
        // playback from re-probing goes with it
        let stored = Photo::find_by_hash(&pool, &hash)
            .await
            .expect("read")
            .expect("row");
        assert!(
            stored.metadata["video"].get("moov_at_start").is_none(),
            "a verdict on the old bytes must not survive: {}",
            stored.metadata
        );
        assert!(
            stored.metadata["video"].get("capability_version").is_none(),
            "the record must read as unprobed so the layout is probed once: {}",
            stored.metadata
        );
        assert!(
            !crate::video_capability::record_is_complete(&stored),
            "{}",
            stored.metadata
        );
        // AND: the facts the fresh extraction does own landed
        assert_eq!(stored.metadata["video"]["container"], "mp4");
        assert_eq!(stored.metadata["video"]["audio_codec"], "aac");
    }

    /// The `video` object is the extraction's to write: a photo's fresh
    /// extraction carries no video facts, so the document stored for a photo
    /// row never acquires one from a scan (the builder states a `video` object
    /// only for a video mime type).
    #[tokio::test]
    async fn scan_upsert_never_gives_a_row_a_video_object_it_did_not_extract() {
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-moov-photo");
        // GIVEN: a photo row holding unrelated metadata
        let mut existing = create_test_photo("still_a_photo.jpg".to_string(), hash.clone());
        existing.metadata = json!({
            "location": { "latitude": 48.2082, "longitude": 16.3737, "city": "Vienna" }
        });
        existing.create(&pool).await.expect("create");

        // WHEN: the extraction writes what the file has — no video facts
        let mut extracted = scanned_photo("still_a_photo.jpg", &hash, "image/jpeg", None, None);
        extracted.file_size += 1;
        let fresh: Photo = extracted.into();
        let mut tx = pool.begin().await.expect("tx");
        fresh
            .create_or_update_with_transaction(&mut tx)
            .await
            .expect("upsert");
        tx.commit().await.expect("commit");

        // THEN: the fresh extraction landed and no video object appeared
        let stored = Photo::find_by_hash(&pool, &hash)
            .await
            .expect("read")
            .expect("row");
        assert_eq!(
            stored.file_size,
            existing.file_size + 1,
            "the row must describe the bytes the scan just read"
        );
        assert!(
            stored.metadata.get("video").is_none(),
            "a photo row must not acquire a video object: {}",
            stored.metadata
        );
        assert!(
            stored.metadata["location"].get("city").is_none(),
            "the name described coordinates the changed file no longer has: {}",
            stored.metadata
        );
    }

    /// The plan's T6 acceptance: the scan's document is MERGED into the row, so
    /// the members the scan does not own survive a rescan. Both are otherwise
    /// lost for good — `get_photos_needing_geo_resolution` selects only the rows
    /// with `geo_location_resolved = 0`, so a dropped place name is never
    /// re-derived (the map loses its label and `location:` search its hits), and
    /// a dropped `capability_version` makes `video_probe::record_is_complete`
    /// re-probe on every playback (FR-008). Every resume re-upserts even the
    /// rows the unchanged-file fast path built, so this is not a corner case.
    ///
    /// A member the file no longer describes still goes: coordinates are file
    /// facts the DB never stores, so a pair a legacy row kept is cleared with
    /// the document rather than left describing a position the file does not
    /// carry. An unchanged file's coordinates are still the ones its name was
    /// geocoded from, so that clear does not reach the `geo_location_resolved`
    /// gate either — the row stays out of the resolver's queue.
    #[tokio::test]
    async fn scan_upsert_merges_so_the_place_name_and_probe_record_survive() {
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-scan-merge");
        // GIVEN: a video row a previous scan wrote, with the name the resolver
        // derived and the probe's record on top of it.
        let mut existing = create_test_photo("rescan.mp4".to_string(), hash.clone());
        existing.mime_type = Some("video/mp4".to_string());
        existing.metadata = json!({
            "location": { "city": "Vienna" },
            "video": { "codec": "h264", "capability_version": 1 }
        });
        existing.create(&pool).await.expect("create");
        mark_photo_geo_resolved(&pool, &existing.file_path)
            .await
            .expect("resolved flag");
        // AND: a coordinate pair a legacy write left in the row — `stored_metadata`
        // strips them from anything the app writes, so only raw SQL can put one
        // there, which is exactly the row the merge has to repair.
        sqlx::query(
            "UPDATE photos SET metadata = json_set(metadata, '$.location.latitude', 48.2, \
             '$.location.longitude', 16.4) WHERE hash_sha256 = ?",
        )
        .bind(&hash)
        .execute(&pool)
        .await
        .expect("seed legacy coordinates");

        // WHEN: the scan writes what it found for the file the row describes —
        // the fingerprint the unchanged-file fast path carries over
        let mut extracted = scanned_photo("rescan.mp4", &hash, "video/mp4", None, None);
        extracted.file_size = existing.file_size;
        extracted.date_modified = existing.date_modified;
        extracted.video_codec = Some("h264".to_string());
        extracted.container = Some("mp4".to_string());
        let fresh: Photo = extracted.into();
        let mut tx = pool.begin().await.expect("tx");
        fresh
            .create_or_update_with_transaction(&mut tx)
            .await
            .expect("upsert");
        tx.commit().await.expect("commit");

        // THEN: the members the scan does not own are still there
        let stored = Photo::find_by_hash(&pool, &hash)
            .await
            .expect("read")
            .expect("row");
        assert_eq!(
            stored.metadata["location"]["city"], "Vienna",
            "the rescan dropped the resolved place name: {}",
            stored.metadata
        );
        assert_eq!(
            stored.metadata["video"]["capability_version"], 1,
            "the rescan dropped the probe record: {}",
            stored.metadata
        );
        assert!(
            crate::video_capability::record_is_complete(&stored),
            "a rescan of an unchanged file must not force a re-probe: {}",
            stored.metadata
        );
        // AND: the facts the scan does own landed
        assert_eq!(stored.metadata["video"]["codec"], "h264");
        assert_eq!(stored.metadata["video"]["container"], "mp4");
        // AND: the coordinates the file no longer claims are gone
        assert!(
            stored.metadata["location"].get("latitude").is_none()
                && stored.metadata["location"].get("longitude").is_none(),
            "stored coordinates survived a scan of the file: {}",
            stored.metadata
        );
        // AND: the gate the name was derived under is untouched, so the row
        // stays out of the resolver's queue: an unchanged file's coordinates
        // are the ones the name was geocoded from.
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT geo_location_resolved FROM photos WHERE hash_sha256 = ?"
            )
            .bind(&hash)
            .fetch_one(&pool)
            .await
            .expect("read flag"),
            1,
            "a rescan of an unchanged file must not re-queue its resolved name"
        );
    }

    /// A row that described DIFFERENT bytes than the file now does holds facts
    /// about the bytes that are gone, so the merge clears them before the fresh
    /// document lands: a place name geocoded from a position the changed file
    /// may not have, and a probe record whose `capability_version` would keep
    /// `record_is_complete` true while `plan` reads its dropped `moov_at_start`
    /// as "never established". This is the wholesale replace's behaviour,
    /// narrowed to the rows it is right for.
    #[tokio::test]
    async fn scan_upsert_clears_the_facts_a_replaced_file_no_longer_supports() {
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-scan-changed");
        let mut existing = create_test_photo("replaced.mp4".to_string(), hash.clone());
        existing.mime_type = Some("video/mp4".to_string());
        existing.metadata = json!({
            "location": { "city": "Vienna" },
            "video": { "codec": "h264", "capability_version": 1, "moov_at_start": false }
        });
        existing.create(&pool).await.expect("create");
        // AND: the resolver has already named the row's position
        mark_photo_geo_resolved(&pool, &existing.file_path)
            .await
            .expect("resolved flag");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT geo_location_resolved FROM photos WHERE hash_sha256 = ?"
            )
            .bind(&hash)
            .fetch_one(&pool)
            .await
            .expect("read flag"),
            1,
            "the row has to start out of the resolver's queue for this to mean anything"
        );

        // WHEN: the row's fingerprint no longer describes the file — the bytes
        // at this path were replaced since the last scan
        let mut extracted = scanned_photo("replaced.mp4", &hash, "video/mp4", None, None);
        extracted.file_size = existing.file_size + 1;
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
        assert!(
            stored.metadata["location"].get("city").is_none(),
            "the name described a position the changed file may not have: {}",
            stored.metadata
        );
        // The name is dropped, so the gate that makes the resolver look at the
        // row has to go back to 0 with it: the resolver is fed by
        // `get_photos_needing_geo_resolution`, which selects exactly the rows
        // with it clear, so a row left at 1 would keep no name and never be
        // handed its new coordinates — its label and `location:` hits gone for
        // good.
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT geo_location_resolved FROM photos WHERE hash_sha256 = ?"
            )
            .bind(&hash)
            .fetch_one(&pool)
            .await
            .expect("read flag"),
            0,
            "a row whose file changed has to reach the resolver again"
        );
        assert!(
            stored.metadata["video"].get("capability_version").is_none(),
            "a probe record on bytes that are gone must not survive: {}",
            stored.metadata
        );
        assert!(
            stored.metadata["video"].get("moov_at_start").is_none(),
            "a layout verdict on bytes that are gone must not survive: {}",
            stored.metadata
        );
        assert!(
            !crate::video_capability::record_is_complete(&stored),
            "the changed row has to be re-probed: {}",
            stored.metadata
        );
    }

    /// A video save re-reads its row, rewrites the container and only then
    /// records what the container holds, so the row can take a commit in
    /// between. Writing the snapshot the request was read against would revert
    /// it, and a resolved place name is lost for good that way: its
    /// `geo_location_resolved` flag is already 1, so nothing ever derives the
    /// name again. `record_video_edit` therefore names only the save's own
    /// members and merges them into the row as it is NOW.
    #[tokio::test]
    async fn video_metadata_mirror_keeps_a_commit_that_landed_after_the_row_was_read() {
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-mirror");
        let photo = create_test_photo("mirror.mp4".to_string(), hash.clone());
        photo.create(&pool).await.expect("create");
        update_photo_city(&pool, &photo.file_path, Some("Berlin"))
            .await
            .expect("resolve");

        // GIVEN: the row as the save read it, before the container was written
        let snapshot = Photo::find_by_hash(&pool, &hash)
            .await
            .expect("read")
            .expect("row");

        // WHEN: the resolver commits while the container is being written
        update_photo_city(&pool, &photo.file_path, Some("Cologne"))
            .await
            .expect("resolve again");

        // THEN: the save's own member lands and that commit survives. The save
        // reports no move, exactly as the handler does for a date-only save, so
        // the name the resolver derived for the position stays valid.
        Photo::record_video_edit(
            &pool,
            &hash,
            false,
            Some((snapshot.file_size, snapshot.date_modified)),
            MediaFacts::default(),
            Utc::now(),
        )
        .await
        .expect("record");

        let stored = Photo::find_by_hash(&pool, &hash)
            .await
            .expect("read")
            .expect("row");
        assert_eq!(
            stored.metadata["location"]["city"], "Cologne",
            "the name committed after the row was read is not the save's to overwrite: {}",
            stored.metadata
        );
        // The save stores the facts it read back from the container, and the
        // stored document stays coordinate-free.
        assert!(stored.taken_at.is_none(), "this save wrote no date");
        assert!(stored.metadata["location"].get("latitude").is_none());
        assert!(stored.metadata["location"].get("longitude").is_none());
    }

    /// A video save restates the file's fingerprint, so nothing downstream of
    /// the scan would correct a name geocoded from a position the save
    /// replaced: the resolver skips the row (its `geo_location_resolved` is
    /// already 1). The save therefore has to drop the name and re-queue the row
    /// itself, and the queue reads the NEW position from the row's columns.
    #[tokio::test]
    async fn video_metadata_mirror_rearms_the_resolver_when_the_save_moves_the_photo() {
        // GIVEN: a resolved row, named for the coordinates it held
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-rearm");
        let photo = create_test_photo("rearm.mp4".to_string(), hash.clone());
        photo.create(&pool).await.expect("create");
        update_photo_city(&pool, &photo.file_path, Some("Berlin"))
            .await
            .expect("resolve");
        let fingerprint = (photo.file_size, photo.date_modified);
        // The file now carries the position the save wrote; coordinates live in
        // the index, which is what the resolver reads.

        // WHEN: a save reports it moved the position
        Photo::record_video_edit(
            &pool,
            &hash,
            true,
            Some(fingerprint),
            MediaFacts {
                latitude: Some(48.2082),
                longitude: Some(16.3737),
                ..MediaFacts::default()
            },
            Utc::now(),
        )
        .await
        .expect("record");

        // THEN: the name it belonged to is gone and the row reaches the
        // resolver again, carrying the NEW position
        let stored = Photo::find_by_hash(&pool, &hash)
            .await
            .expect("read")
            .expect("row");
        assert!(
            stored.metadata["location"]["city"].is_null(),
            "the name of the position the save replaced must not survive it: {}",
            stored.metadata
        );
        assert_eq!(
            get_photos_needing_geo_resolution(&pool)
                .await
                .expect("candidates"),
            vec![(photo.file_path.clone(), 48.2082, 16.3737)],
            "only a cleared flag queues the row, and the queue carries the NEW position"
        );
    }

    /// The counterpart: a save that did not move the photo keeps its resolved
    /// name and stays out of the queue. A date-only save restates the file's
    /// fingerprint exactly like a positional one, so nothing downstream of the
    /// scan would ever correct a name dropped here for no reason.
    #[tokio::test]
    async fn video_metadata_mirror_keeps_the_resolved_name_when_the_save_holds_the_position() {
        // GIVEN: the same resolved row
        let pool = create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-keep-name");
        let photo = create_test_photo("keep_name.mp4".to_string(), hash.clone());
        photo.create(&pool).await.expect("create");
        update_photo_city(&pool, &photo.file_path, Some("Berlin"))
            .await
            .expect("resolve");
        let fingerprint = (photo.file_size, photo.date_modified);

        // WHEN: a save reports no move
        Photo::record_video_edit(
            &pool,
            &hash,
            false,
            Some(fingerprint),
            MediaFacts::default(),
            Utc::now(),
        )
        .await
        .expect("record");

        // THEN: the name survives and the row is not re-queued
        let stored = Photo::find_by_hash(&pool, &hash)
            .await
            .expect("read")
            .expect("row");
        assert_eq!(stored.metadata["location"]["city"], "Berlin");
        assert!(
            get_photos_needing_geo_resolution(&pool)
                .await
                .expect("candidates")
                .is_empty(),
            "a save that held the position must not invalidate its name: {}",
            stored.metadata
        );
    }

    #[tokio::test]
    async fn test_get_timeline_data_empty() {
        let pool = create_test_db_pool().await.unwrap();

        // Get timeline data from empty database
        let timeline = Photo::get_timeline_data(&pool).await.unwrap();

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
        let mut unresolved_photo =
            create_test_photo("needs-geo.jpg".to_string(), "needs-geo-hash".to_string());
        unresolved_photo.taken_latitude = Some(52.52);
        unresolved_photo.taken_longitude = Some(13.405);
        let mut resolved_photo = create_test_photo(
            "resolved-geo.jpg".to_string(),
            "resolved-geo-hash".to_string(),
        );
        resolved_photo.taken_latitude = Some(48.137);
        resolved_photo.taken_longitude = Some(11.575);
        let no_gps_photo = create_test_photo("no-gps.jpg".to_string(), "no-gps-hash".to_string());

        unresolved_photo.create(&pool).await.unwrap();
        resolved_photo.create(&pool).await.unwrap();
        no_gps_photo.create(&pool).await.unwrap();

        sqlx::query("UPDATE photos SET geo_location_resolved = 1 WHERE file_path = ?")
            .bind(&resolved_photo.file_path)
            .execute(&pool)
            .await
            .unwrap();

        let photos = get_photos_needing_geo_resolution(&pool).await.unwrap();

        assert_eq!(
            photos,
            vec![(unresolved_photo.file_path.clone(), 52.52, 13.405)]
        );
    }

    #[tokio::test]
    async fn test_mark_photo_geo_resolved() {
        let pool = create_test_db_pool().await.unwrap();
        let mut photo = create_test_photo(
            "mark-resolved.jpg".to_string(),
            "mark-resolved-hash".to_string(),
        );
        photo.taken_latitude = Some(52.52);
        photo.taken_longitude = Some(13.405);

        photo.create(&pool).await.unwrap();
        mark_photo_geo_resolved(&pool, &photo.file_path)
            .await
            .unwrap();

        let photos = get_photos_needing_geo_resolution(&pool).await.unwrap();

        assert!(photos.is_empty());
    }

    #[tokio::test]
    async fn test_update_photo_city() {
        let pool = create_test_db_pool().await.unwrap();
        let mut photo = create_test_photo(
            "city-update.jpg".to_string(),
            "city-update-hash".to_string(),
        );
        photo.taken_latitude = Some(52.52);
        photo.taken_longitude = Some(13.405);

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
        assert!(get_photos_needing_geo_resolution(&pool)
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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();

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
        for (hash, filename, taken_at) in [
            ("a", "feb2012.jpg", "2012-02-10T10:00:00Z"),
            ("b", "mar2012.jpg", "2012-03-15T10:00:00Z"),
            ("c", "dec2012.jpg", "2012-12-31T23:00:00Z"),
            ("d", "jan2013.jpg", "2013-01-01T00:30:00Z"),
            ("e", "aug2015.jpg", "2015-08-31T23:30:00Z"),
            ("f", "sep2015.jpg", "2015-09-01T00:00:00Z"),
        ] {
            create_photo_with_date(&pool, &hash.repeat(64), filename, taken_at).await;
        }

        // Single month (start bound only).
        let query = date_filter_query(Some(2012), Some(3), None, None);
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos[0].filename, "mar2012.jpg");

        // Whole year (no month) stays a single-year filter.
        let query = date_filter_query(Some(2012), None, None, None);
        let (_, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
        assert_eq!(total, 3);

        // Month-granular range spanning years, both bounds inclusive.
        let query = date_filter_query(Some(2012), Some(3), Some(2015), Some(8));
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
        assert_eq!(total, 4);
        let names: Vec<&str> = photos.iter().map(|p| p.filename.as_str()).collect();
        assert!(!names.contains(&"feb2012.jpg"));
        assert!(!names.contains(&"sep2015.jpg"));

        // Year-precision end bound stops at December of that year.
        let query = date_filter_query(Some(2012), Some(3), Some(2012), None);
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
        assert_eq!(total, 2);
        assert_eq!(photos[0].taken_at.unwrap().year(), 2012);

        // Reversed bounds match nothing (the router normalises before it gets here).
        let query = date_filter_query(Some(2015), Some(8), Some(2012), Some(3));
        let (_, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
        assert_eq!(total, 0);

        // A month bound without a year filters nothing.
        let query = date_filter_query(None, Some(3), None, None);
        let (_, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
        assert_eq!(total, 6);

        // An end year without a start year filters nothing either.
        let query = date_filter_query(None, None, Some(2015), None);
        let (_, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
        assert_eq!(total, 6);

        // An out-of-range start month matches nothing, with or without an end
        // bound: `2012-13` must not become a valid ordering point that lets a
        // later end year widen the range to 2013-01…2015-12.
        let query = date_filter_query(Some(2012), Some(13), None, None);
        let (_, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
        assert_eq!(total, 0);

        let query = date_filter_query(Some(2012), Some(13), Some(2015), None);
        let (_, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
        assert_eq!(total, 0);

        // Likewise for an out-of-range end month.
        let query = date_filter_query(Some(2012), None, Some(2015), Some(13));
        let (_, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn test_search_photos_range_ignores_unknown_dates() {
        let pool = create_test_db_pool().await.unwrap();
        // The undated photo gets NO index entry: its file yields no date.
        create_photo_with_date(
            &pool,
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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();

        assert_eq!(total, 1, "underscore must match literally");
        assert_eq!(photos[0].file_path, underscore_photo.file_path);

        // A literal % must match only rows whose filename contains '%'
        let percent_photo =
            create_test_photo_with_metadata("weird%name.jpg", "percent-hash", json!({}));
        percent_photo.create(&pool).await.unwrap();
        let query = create_search_query("100%");
        let (_, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
        assert_eq!(total, 0, "'100%' must not match every row");

        let query = create_search_query("weird%name");
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();

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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();

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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();

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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();

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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();

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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();

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
        let (_, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();

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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();

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
            let (photos, total) = search(&pool, &create_search_query(query), 50, 0, None, None)
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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();

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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
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
        let (photos, total) = search(&pool, &query, 50, 0, None, None).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, ny_video_photo.file_path);
    }

    #[tokio::test]
    async fn test_list_all_filtered_returns_every_match_without_pagination() {
        let pool = create_test_db_pool().await.unwrap();

        for index in 0..120 {
            // `create_test_photo` zero-pads short hashes to 64 chars, so
            // "bulk1" and "bulk10" would collapse to the same padded hash
            // (11 collisions across 0..120). Fixed-width digits keep them
            // distinct.
            let photo = create_test_photo(format!("bulk_{index}.jpg"), format!("bulk{index:03}"));
            photo.create(&pool).await.unwrap();
        }

        let photos = list_all_filtered(
            &pool,
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

        let mut berlin_2020 = create_photo_with_date(
            &pool,
            &"1".repeat(64),
            "berlin_2020.jpg",
            "2020-05-25T10:00:00Z",
        )
        .await;
        berlin_2020.metadata = json!({ "location": { "city": "Berlin" } });
        berlin_2020.update(&pool).await.unwrap();

        let mut berlin_2024 = create_photo_with_date(
            &pool,
            &"2".repeat(64),
            "berlin_2024.jpg",
            "2024-05-25T10:00:00Z",
        )
        .await;
        berlin_2024.metadata = json!({ "location": { "city": "Berlin" } });
        berlin_2024.update(&pool).await.unwrap();

        let mut rome =
            create_photo_with_date(&pool, &"3".repeat(64), "rome.jpg", "2020-05-25T10:00:00Z")
                .await;
        rome.metadata = json!({ "location": { "city": "Rome" } });
        rome.update(&pool).await.unwrap();

        let photos = list_all_filtered(
            &pool,
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

        let photos = list_all_filtered(
            &pool,
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
