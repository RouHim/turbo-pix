use serde::Serialize;
use sqlx::{FromRow, Row};

use crate::db::DbPool;

#[derive(Debug, Clone, Serialize)]
pub struct SavedSearch {
    pub id: i64,
    pub name: String,
    pub query: Option<String>,
    pub view: String,
    pub sort: String,
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub to_year: Option<i64>,
    pub to_month: Option<i64>,
    pub created_at: String,
}

/// Date-filter bounds of a saved search: start (`year`/`month`) and inclusive
/// end (`to_year`/`to_month`). A `None` end bound means "single period".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavedSearchFilter {
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub to_year: Option<i64>,
    pub to_month: Option<i64>,
}

impl FromRow<'_, sqlx::sqlite::SqliteRow> for SavedSearch {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(SavedSearch {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            query: row.try_get("query")?,
            view: row.try_get("view")?,
            sort: row.try_get("sort")?,
            year: row.try_get("year")?,
            month: row.try_get("month")?,
            to_year: row.try_get("to_year")?,
            to_month: row.try_get("to_month")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

pub const VALID_VIEWS: [&str; 5] = ["all", "favorites", "videos", "collages", "housekeeping"];
pub const VALID_SORTS: [&str; 6] = [
    "date_desc",
    "date_asc",
    "name_asc",
    "name_desc",
    "size_desc",
    "size_asc",
];

#[derive(Debug)]
pub enum CreateError {
    Duplicate(Box<SavedSearch>),
    Db(Box<dyn std::error::Error>),
}

const SELECT_COLUMNS: &str =
    "id, name, query, view, sort, year, month, to_year, to_month, created_at";

/// List saved searches newest-first (created_at second resolution, id tiebreak).
pub async fn list(pool: &DbPool) -> Result<Vec<SavedSearch>, Box<dyn std::error::Error>> {
    let rows = sqlx::query_as::<_, SavedSearch>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLUMNS} FROM saved_searches ORDER BY created_at DESC, id DESC"
    )))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Create a saved search. Exact duplicate view states are rejected with the
/// existing row; the unique index is the authority, so no transaction is
/// needed even under concurrent requests.
pub async fn create(
    pool: &DbPool,
    name: &str,
    query: Option<&str>,
    view: &str,
    sort: &str,
    filter: &SavedSearchFilter,
) -> Result<SavedSearch, CreateError> {
    let inserted: Option<(i64,)> = sqlx::query_as(
        "INSERT INTO saved_searches (name, query, view, sort, year, month, to_year, to_month)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT DO NOTHING
         RETURNING id",
    )
    .bind(name)
    .bind(query)
    .bind(view)
    .bind(sort)
    .bind(filter.year)
    .bind(filter.month)
    .bind(filter.to_year)
    .bind(filter.to_month)
    .fetch_optional(pool)
    .await
    .map_err(|e| CreateError::Db(Box::new(e)))?;

    let id = match inserted {
        Some((id,)) => id,
        None => {
            // Conflict: return the existing row. `IS` gives NULL == NULL
            // equality for the optional columns.
            let existing = sqlx::query_as::<_, SavedSearch>(sqlx::AssertSqlSafe(format!(
                "SELECT {SELECT_COLUMNS} FROM saved_searches
                 WHERE query IS ? AND view = ? AND sort = ? AND year IS ? AND month IS ?
                   AND to_year IS ? AND to_month IS ?"
            )))
            .bind(query)
            .bind(view)
            .bind(sort)
            .bind(filter.year)
            .bind(filter.month)
            .bind(filter.to_year)
            .bind(filter.to_month)
            .fetch_optional(pool)
            .await
            .map_err(|e| CreateError::Db(Box::new(e)))?;
            return match existing {
                Some(row) => Err(CreateError::Duplicate(Box::new(row))),
                // Unique index said conflict but the row is gone (race with a
                // delete); surface a Db error rather than invent a row.
                None => Err(CreateError::Db(
                    "conflicting saved search vanished during lookup".into(),
                )),
            };
        }
    };

    let row = sqlx::query_as::<_, SavedSearch>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLUMNS} FROM saved_searches WHERE id = ?"
    )))
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(|e| CreateError::Db(Box::new(e)))?;
    Ok(row)
}

/// Rename a saved search; `Ok(None)` when the id does not exist.
pub async fn rename(
    pool: &DbPool,
    id: i64,
    name: &str,
) -> Result<Option<SavedSearch>, Box<dyn std::error::Error>> {
    let row = sqlx::query_as::<_, SavedSearch>(sqlx::AssertSqlSafe(format!(
        "UPDATE saved_searches SET name = ? WHERE id = ?
         RETURNING {SELECT_COLUMNS}"
    )))
    .bind(name)
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Delete a saved search; `Ok(true)` when a row was removed.
pub async fn delete(pool: &DbPool, id: i64) -> Result<bool, Box<dyn std::error::Error>> {
    let result = sqlx::query("DELETE FROM saved_searches WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_test_db_pool;

    /// Unfiltered identity: the state most tests exercise.
    const NO_FILTER: SavedSearchFilter = SavedSearchFilter {
        year: None,
        month: None,
        to_year: None,
        to_month: None,
    };

    #[tokio::test]
    async fn test_create_and_list_newest_first() {
        let pool = create_test_db_pool().await.unwrap();
        let a = create(
            &pool,
            "A",
            Some("beach"),
            "all",
            "date_desc",
            &SavedSearchFilter {
                year: Some(2023),
                ..NO_FILTER
            },
        )
        .await
        .unwrap();
        let b = create(
            &pool,
            "B",
            Some("sunset"),
            "favorites",
            "date_asc",
            &NO_FILTER,
        )
        .await
        .unwrap();
        assert!(a.id < b.id);

        let all = list(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
        // created_at has second resolution, so the id tiebreak must order
        // same-second saves newest-first.
        assert_eq!(all[0].id, b.id);
        assert_eq!(all[1].id, a.id);
    }

    #[tokio::test]
    async fn test_create_duplicate_returns_existing_row() {
        let pool = create_test_db_pool().await.unwrap();
        let first = create(
            &pool,
            "Beach 2023",
            Some("beach"),
            "all",
            "date_desc",
            &SavedSearchFilter {
                year: Some(2023),
                ..NO_FILTER
            },
        )
        .await
        .unwrap();

        let second = create(
            &pool,
            "Other name",
            Some("beach"),
            "all",
            "date_desc",
            &SavedSearchFilter {
                year: Some(2023),
                ..NO_FILTER
            },
        )
        .await;
        match second {
            Err(CreateError::Duplicate(existing)) => {
                assert_eq!(existing.id, first.id);
                assert_eq!(existing.name, first.name);
            }
            Err(other) => panic!("expected Duplicate, got {:?}", other),
            Ok(_) => panic!("expected Duplicate, got Ok"),
        }
        assert_eq!(list(&pool).await.unwrap().len(), 1);

        // NULL identity path: query/year/month all NULL.
        create(&pool, "Null state", None, "videos", "date_desc", &NO_FILTER)
            .await
            .unwrap();
        let dup = create(
            &pool,
            "Null state 2",
            None,
            "videos",
            "date_desc",
            &NO_FILTER,
        )
        .await;
        match dup {
            Err(CreateError::Duplicate(existing)) => assert_eq!(existing.name, "Null state"),
            Err(other) => panic!("expected Duplicate for NULL state, got {:?}", other),
            Ok(_) => panic!("expected Duplicate for NULL state, got Ok"),
        }
    }

    #[tokio::test]
    async fn test_null_and_explicit_year_are_distinct() {
        let pool = create_test_db_pool().await.unwrap();
        create(&pool, "Cat", Some("cat"), "all", "date_desc", &NO_FILTER)
            .await
            .unwrap();
        create(
            &pool,
            "Cat 2023",
            Some("cat"),
            "all",
            "date_desc",
            &SavedSearchFilter {
                year: Some(2023),
                ..NO_FILTER
            },
        )
        .await
        .unwrap();
        assert_eq!(list(&pool).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_rename_updates_name() {
        let pool = create_test_db_pool().await.unwrap();
        let created = create(&pool, "Old", Some("beach"), "all", "date_desc", &NO_FILTER)
            .await
            .unwrap();
        let renamed = rename(&pool, created.id, "New").await.unwrap().unwrap();
        assert_eq!(renamed.name, "New");
        assert_eq!(renamed.id, created.id);
        let all = list(&pool).await.unwrap();
        assert_eq!(all[0].name, "New");
    }

    #[tokio::test]
    async fn test_rename_missing_returns_none() {
        let pool = create_test_db_pool().await.unwrap();
        assert!(rename(&pool, 999, "New").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_delete_removes_row() {
        let pool = create_test_db_pool().await.unwrap();
        let created = create(&pool, "Temp", Some("beach"), "all", "date_desc", &NO_FILTER)
            .await
            .unwrap();
        assert!(delete(&pool, created.id).await.unwrap());
        assert_eq!(list(&pool).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_delete_missing_returns_false() {
        let pool = create_test_db_pool().await.unwrap();
        assert!(!delete(&pool, 999).await.unwrap());
    }
}
