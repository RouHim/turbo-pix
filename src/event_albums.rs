use serde::Serialize;
use sqlx::{FromRow, Row};

use crate::db::DbPool;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_test_db_pool;

    fn h(tag: &str) -> String {
        // 64-char hash satisfying photos.hash_sha256 CHECK(length = 64).
        format!("{tag:0>64}")
    }

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
}
