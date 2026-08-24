use serde::Serialize;
use sqlx::{FromRow, Row};

use crate::db::{build_order_clause, DbPool, Photo};

/// An event album: a named, rule-driven album whose membership is computed
/// live from photo metadata (taken date within `[start_date, end_date]` and,
/// when set, a case-insensitive match on the resolved city). There is no
/// membership join table — FR-005 requires query-time derivation.
#[derive(Debug, Clone, Serialize)]
pub struct EventAlbum {
    pub id: i64,
    pub name: String,
    /// Inclusive start of the date range, local calendar day (`YYYY-MM-DD`).
    pub start_date: String,
    /// Inclusive end of the date range, local calendar day (`YYYY-MM-DD`).
    pub end_date: String,
    /// Optional location matched case-insensitively against `metadata.location.city`.
    pub location: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl FromRow<'_, sqlx::sqlite::SqliteRow> for EventAlbum {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(EventAlbum {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            start_date: row.try_get("start_date")?,
            end_date: row.try_get("end_date")?,
            location: row.try_get("location")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

const SELECT_COLUMNS: &str = "id, name, start_date, end_date, location, created_at, updated_at";

/// List event albums newest-first (created_at second resolution, id tiebreak).
pub async fn list(pool: &DbPool) -> Result<Vec<EventAlbum>, Box<dyn std::error::Error>> {
    let rows = sqlx::query_as::<_, EventAlbum>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLUMNS} FROM event_albums ORDER BY created_at DESC, id DESC"
    )))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn find_by_id(
    pool: &DbPool,
    id: i64,
) -> Result<Option<EventAlbum>, Box<dyn std::error::Error>> {
    let row = sqlx::query_as::<_, EventAlbum>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLUMNS} FROM event_albums WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn create(
    pool: &DbPool,
    name: &str,
    start_date: &str,
    end_date: &str,
    location: Option<&str>,
) -> Result<EventAlbum, Box<dyn std::error::Error>> {
    let inserted: (i64,) = sqlx::query_as(
        "INSERT INTO event_albums (name, start_date, end_date, location)
         VALUES (?, ?, ?, ?)
         RETURNING id",
    )
    .bind(name)
    .bind(start_date)
    .bind(end_date)
    .bind(location)
    .fetch_one(pool)
    .await?;

    let row = sqlx::query_as::<_, EventAlbum>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLUMNS} FROM event_albums WHERE id = ?"
    )))
    .bind(inserted.0)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Fully replace an album's name/date-range/location; `Ok(None)` when the id
/// does not exist. Bumps `updated_at`.
pub async fn update(
    pool: &DbPool,
    id: i64,
    name: &str,
    start_date: &str,
    end_date: &str,
    location: Option<&str>,
) -> Result<Option<EventAlbum>, Box<dyn std::error::Error>> {
    let row = sqlx::query_as::<_, EventAlbum>(sqlx::AssertSqlSafe(format!(
        "UPDATE event_albums
         SET name = ?, start_date = ?, end_date = ?, location = ?, updated_at = datetime('now')
         WHERE id = ?
         RETURNING {SELECT_COLUMNS}"
    )))
    .bind(name)
    .bind(start_date)
    .bind(end_date)
    .bind(location)
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Delete an event album; `Ok(true)` when a row was removed.
pub async fn delete(pool: &DbPool, id: i64) -> Result<bool, Box<dyn std::error::Error>> {
    let result = sqlx::query("DELETE FROM event_albums WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Query photos matching an album's criteria, newest-first by default.
/// Membership is the conjunction: taken date within `[start_date, end_date]`
/// (inclusive, local calendar days) AND (no location OR city matches
/// case-insensitively). Photos with no taken date are excluded; photos with
/// no city are excluded when a location is set.
pub async fn photos_for_album(
    pool: &DbPool,
    album: &EventAlbum,
    limit: i64,
    offset: i64,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>> {
    // `date(taken_at, 'localtime')` normalizes the stored UTC RFC3339 value to
    // the server's local calendar day so the comparison honors the spec's
    // "local date boundaries" assumption. start/end are validated YYYY-MM-DD.
    let mut where_clause = String::from(
        " WHERE taken_at IS NOT NULL \
           AND date(taken_at, 'localtime') >= date(?) \
           AND date(taken_at, 'localtime') <= date(?)",
    );
    let mut params: Vec<String> = vec![album.start_date.clone(), album.end_date.clone()];

    if let Some(location) = &album.location {
        where_clause.push_str(" AND json_extract(metadata, '$.location.city') = ? COLLATE NOCASE");
        params.push(location.clone());
    }

    let count_sql = format!("SELECT COUNT(*) FROM photos{where_clause}");
    let mut count_query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql));
    for p in &params {
        count_query = count_query.bind(p);
    }
    let total = count_query.fetch_one(pool).await?;

    let data_sql = format!(
        "SELECT * FROM photos{where_clause} ORDER BY {} LIMIT ? OFFSET ?",
        build_order_clause(sort, order)
    );
    let mut data_query = sqlx::query_as::<_, Photo>(sqlx::AssertSqlSafe(data_sql));
    for p in &params {
        data_query = data_query.bind(p);
    }
    data_query = data_query.bind(limit).bind(offset);
    let photos = data_query.fetch_all(pool).await?;

    Ok((photos, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_test_db_pool;

    #[tokio::test]
    async fn test_create_and_list_newest_first() {
        let pool = create_test_db_pool().await.unwrap();
        let a = create(
            &pool,
            "Berlin trip",
            "2024-01-01",
            "2024-01-31",
            Some("Berlin"),
        )
        .await
        .unwrap();
        let b = create(&pool, "Summer", "2024-06-01", "2024-06-30", None)
            .await
            .unwrap();
        assert!(a.id < b.id);

        let all = list(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, b.id); // newest-first id tiebreak
        assert_eq!(all[1].id, a.id);
        assert_eq!(all[0].location, None);
        assert_eq!(all[1].location.as_deref(), Some("Berlin"));
    }

    #[tokio::test]
    async fn test_find_by_id_and_update() {
        let pool = create_test_db_pool().await.unwrap();
        let created = create(&pool, "Old", "2024-01-01", "2024-01-31", Some("Berlin"))
            .await
            .unwrap();

        assert_eq!(
            find_by_id(&pool, created.id).await.unwrap().unwrap().name,
            "Old"
        );
        assert!(find_by_id(&pool, 999).await.unwrap().is_none());

        let updated = update(&pool, created.id, "New", "2024-02-01", "2024-02-29", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.name, "New");
        assert_eq!(updated.start_date, "2024-02-01");
        assert_eq!(updated.location, None);
    }

    #[tokio::test]
    async fn test_update_missing_returns_none_and_delete() {
        let pool = create_test_db_pool().await.unwrap();
        assert!(update(&pool, 999, "x", "2024-01-01", "2024-01-31", None)
            .await
            .unwrap()
            .is_none());

        let created = create(&pool, "Temp", "2024-01-01", "2024-01-31", None)
            .await
            .unwrap();
        assert!(delete(&pool, created.id).await.unwrap());
        assert!(!delete(&pool, created.id).await.unwrap());
        assert_eq!(list(&pool).await.unwrap().len(), 0);
    }
    use chrono::{DateTime, Utc};

    fn h(tag: &str) -> String {
        // 64-char hash satisfying photos.hash_sha256 CHECK(length = 64).
        format!("{tag:0>64}")
    }

    fn test_photo(
        tag: &str,
        filename: &str,
        taken_at: Option<DateTime<Utc>>,
        city: Option<&str>,
    ) -> Photo {
        use crate::db::Photo;
        let metadata = match city {
            Some(city) => serde_json::json!({ "location": { "city": city } }),
            None => serde_json::json!({}),
        };
        Photo {
            hash_sha256: h(tag),
            file_path: format!("./test/{filename}"),
            filename: filename.to_string(),
            file_size: 0,
            mime_type: Some("image/jpeg".to_string()),
            taken_at,
            width: None,
            height: None,
            orientation: None,
            duration: None,
            thumbnail_path: None,
            has_thumbnail: None,
            blurhash: None,
            is_favorite: None,
            semantic_vector_indexed: None,
            metadata,
            date_modified: Utc::now(),
            date_indexed: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn album(start: &str, end: &str, location: Option<&str>) -> EventAlbum {
        EventAlbum {
            id: 1,
            name: "album".into(),
            start_date: start.into(),
            end_date: end.into(),
            location: location.map(str::to_string),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[tokio::test]
    async fn test_photos_for_album_conjunction_and_case_insensitive_location() {
        let pool = create_test_db_pool().await.unwrap();
        let jan15 = "2024-01-15T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let may15 = "2024-05-15T12:00:00Z".parse::<DateTime<Utc>>().unwrap();

        test_photo("a", "a.jpg", Some(jan15), Some("Berlin"))
            .create(&pool)
            .await
            .unwrap();
        test_photo("b", "b.jpg", Some(jan15), Some("Hamburg"))
            .create(&pool)
            .await
            .unwrap();
        test_photo("c", "c.jpg", Some(jan15), None)
            .create(&pool)
            .await
            .unwrap();
        test_photo("d", "d.jpg", Some(may15), Some("Berlin"))
            .create(&pool)
            .await
            .unwrap();
        test_photo("e", "e.jpg", None, Some("Berlin"))
            .create(&pool)
            .await
            .unwrap();

        // Location "berlin" (lowercase) matches city "Berlin" (case-insensitive);
        // date range excludes May and the null-date photo; wrong-city and no-city excluded.
        let (photos, total) = photos_for_album(
            &pool,
            &album("2024-01-01", "2024-01-31", Some("berlin")),
            50,
            0,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].hash_sha256, h("a"));
    }

    #[tokio::test]
    async fn test_photos_for_album_no_location_includes_every_date_match() {
        let pool = create_test_db_pool().await.unwrap();
        let jan15 = "2024-01-15T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        test_photo("a", "a.jpg", Some(jan15), Some("Berlin"))
            .create(&pool)
            .await
            .unwrap();
        test_photo("b", "b.jpg", Some(jan15), None)
            .create(&pool)
            .await
            .unwrap();

        // No location → every in-range photo, regardless of city presence.
        let (photos, total) = photos_for_album(
            &pool,
            &album("2024-01-01", "2024-01-31", None),
            50,
            0,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(total, 2);
        assert_eq!(photos.len(), 2);
    }

    #[tokio::test]
    async fn test_photos_for_album_pagination_and_out_of_range_exclusion() {
        let pool = create_test_db_pool().await.unwrap();
        let jan15 = "2024-01-15T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let dec15 = "2023-12-15T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        test_photo("a", "a.jpg", Some(jan15), None)
            .create(&pool)
            .await
            .unwrap();
        test_photo("b", "b.jpg", Some(jan15), None)
            .create(&pool)
            .await
            .unwrap();
        test_photo("c", "c.jpg", Some(dec15), None)
            .create(&pool)
            .await
            .unwrap();

        let (photos, total) = photos_for_album(
            &pool,
            &album("2024-01-01", "2024-01-31", None),
            1,
            0,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(total, 2); // the Dec photo is outside the range
        assert_eq!(photos.len(), 1); // limit honored
    }

    #[tokio::test]
    async fn test_photos_for_album_is_dynamic() {
        let pool = create_test_db_pool().await.unwrap();
        let a = album("2024-01-01", "2024-01-31", None);

        // SC-002: membership is derived live — empty before any photo matches.
        let (photos, total) = photos_for_album(&pool, &a, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!((photos.len(), total), (0, 0));

        let jan15 = "2024-01-15T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let photo = test_photo("a", "a.jpg", Some(jan15), None);
        photo.create(&pool).await.unwrap();
        let (photos, total) = photos_for_album(&pool, &a, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!((photos.len(), total), (1, 1));

        // Removing the photo (orphan cleanup) removes it from live membership.
        sqlx::query("DELETE FROM photos WHERE hash_sha256 = ?")
            .bind(&photo.hash_sha256)
            .execute(&pool)
            .await
            .unwrap();
        let (photos, total) = photos_for_album(&pool, &a, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!((photos.len(), total), (0, 0));
    }
}
