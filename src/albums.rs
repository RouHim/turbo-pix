use serde::Serialize;
use sqlx::{FromRow, Row};

use crate::db::{build_order_clause, DbPool, Photo};

/// A manual, hand-curated album: a named set of explicitly chosen photos.
/// Membership lives in `album_members`; there are no rule/criteria fields.
#[derive(Debug, Clone, Serialize)]
pub struct Album {
    pub id: i64,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}

impl FromRow<'_, sqlx::sqlite::SqliteRow> for Album {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Album {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

const SELECT_COLUMNS: &str = "id, name, created_at, updated_at";

/// Chunk size for `IN (...)` placeholder lists (AGENTS learning #9:
/// one placeholder per file exceeds SQLITE_MAX_VARIABLE_NUMBER).
const IN_CHUNK_SIZE: usize = 500;

/// List albums newest-first (created_at second resolution, id tiebreak).
pub async fn list(pool: &DbPool) -> Result<Vec<Album>, Box<dyn std::error::Error>> {
    let rows = sqlx::query_as::<_, Album>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLUMNS} FROM albums ORDER BY created_at DESC, id DESC"
    )))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn find_by_id(
    pool: &DbPool,
    id: i64,
) -> Result<Option<Album>, Box<dyn std::error::Error>> {
    let row = sqlx::query_as::<_, Album>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLUMNS} FROM albums WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn create(pool: &DbPool, name: &str) -> Result<Album, Box<dyn std::error::Error>> {
    let inserted: (i64,) = sqlx::query_as("INSERT INTO albums (name) VALUES (?) RETURNING id")
        .bind(name)
        .fetch_one(pool)
        .await?;

    let row = sqlx::query_as::<_, Album>(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLUMNS} FROM albums WHERE id = ?"
    )))
    .bind(inserted.0)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Create an album and include the given photo hashes. Unknown hashes
/// (no `photos` row) are silently skipped; empty list skips the insert.
pub async fn create_with_members(
    pool: &DbPool,
    name: &str,
    hashes: &[String],
) -> Result<Album, Box<dyn std::error::Error>> {
    let album = create(pool, name).await?;
    if !hashes.is_empty() {
        add_members(pool, album.id, hashes).await?;
    }
    find_by_id(pool, album.id)
        .await?
        .ok_or_else(|| "album vanished after create".into())
}

/// Add members idempotently (`INSERT OR IGNORE`); skips hashes with no
/// photo row. Returns the number of newly inserted rows.
pub async fn add_members(
    pool: &DbPool,
    album_id: i64,
    hashes: &[String],
) -> Result<usize, Box<dyn std::error::Error>> {
    if hashes.is_empty() {
        return Ok(0);
    }
    let mut added = 0_usize;
    for chunk in hashes.chunks(IN_CHUNK_SIZE) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "INSERT OR IGNORE INTO album_members (album_id, photo_hash) \
             SELECT ?, hash_sha256 FROM photos WHERE hash_sha256 IN ({placeholders})"
        );
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(album_id);
        for hash in chunk {
            query = query.bind(hash);
        }
        added += query.execute(pool).await?.rows_affected() as usize;
    }
    Ok(added)
}

/// Remove membership only; never touches photo rows. Returns removed count.
pub async fn remove_members(
    pool: &DbPool,
    album_id: i64,
    hashes: &[String],
) -> Result<u64, Box<dyn std::error::Error>> {
    if hashes.is_empty() {
        return Ok(0);
    }
    let mut removed = 0_u64;
    for chunk in hashes.chunks(IN_CHUNK_SIZE) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "DELETE FROM album_members WHERE album_id = ? AND photo_hash IN ({placeholders})"
        );
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(album_id);
        for hash in chunk {
            query = query.bind(hash);
        }
        removed += query.execute(pool).await?.rows_affected();
    }
    Ok(removed)
}

/// Count members of one album.
pub async fn count_members(
    pool: &DbPool,
    album_id: i64,
) -> Result<i64, Box<dyn std::error::Error>> {
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM album_members WHERE album_id = ?")
        .bind(album_id)
        .fetch_one(pool)
        .await?;
    Ok(total)
}

/// Rename an album; `Ok(None)` when the id does not exist. Bumps `updated_at`.
pub async fn rename(
    pool: &DbPool,
    id: i64,
    name: &str,
) -> Result<Option<Album>, Box<dyn std::error::Error>> {
    let row = sqlx::query_as::<_, Album>(sqlx::AssertSqlSafe(format!(
        "UPDATE albums SET name = ?, updated_at = datetime('now') WHERE id = ? \
         RETURNING {SELECT_COLUMNS}"
    )))
    .bind(name)
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Delete an album; membership rows cascade. Photos survive. `Ok(true)` when removed.
pub async fn delete(pool: &DbPool, id: i64) -> Result<bool, Box<dyn std::error::Error>> {
    let result = sqlx::query("DELETE FROM albums WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Query member photos via the membership join, with the shared sort/order
/// contract. Returns `(photos, total)`.
pub async fn photos_for_album(
    pool: &DbPool,
    album_id: i64,
    limit: i64,
    offset: i64,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>> {
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos \
         JOIN album_members ON photos.hash_sha256 = album_members.photo_hash \
         WHERE album_members.album_id = ?",
    )
    .bind(album_id)
    .fetch_one(pool)
    .await?;

    let data_sql = format!(
        "SELECT photos.* FROM photos \
         JOIN album_members ON photos.hash_sha256 = album_members.photo_hash \
         WHERE album_members.album_id = ? ORDER BY {} LIMIT ? OFFSET ?",
        build_order_clause(sort, order)
    );
    let photos = sqlx::query_as::<_, Photo>(sqlx::AssertSqlSafe(data_sql))
        .bind(album_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

    Ok((photos, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_test_db_pool;

    #[tokio::test]
    async fn test_create_and_list_newest_first() {
        let pool = create_test_db_pool().await.unwrap();
        let a = create(&pool, "Berlin trip").await.unwrap();
        let b = create(&pool, "Summer").await.unwrap();
        assert!(a.id < b.id);
        let all = list(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, b.id);
        assert_eq!(all[1].id, a.id);
    }

    #[tokio::test]
    async fn test_find_rename_delete() {
        let pool = create_test_db_pool().await.unwrap();
        assert!(find_by_id(&pool, 999).await.unwrap().is_none());
        let created = create(&pool, "Old").await.unwrap();
        assert_eq!(
            find_by_id(&pool, created.id).await.unwrap().unwrap().name,
            "Old"
        );
        let renamed = rename(&pool, created.id, "New").await.unwrap().unwrap();
        assert_eq!(renamed.name, "New");
        assert!(rename(&pool, 999, "x").await.unwrap().is_none());
        assert!(delete(&pool, created.id).await.unwrap());
        assert!(!delete(&pool, created.id).await.unwrap());
        assert_eq!(list(&pool).await.unwrap().len(), 0);
    }

    fn h(tag: &str) -> String {
        format!("{tag:0>64}")
    }

    async fn seed_photo(pool: &DbPool, tag: &str) {
        Photo {
            hash_sha256: h(tag),
            file_path: format!("./test/{tag}.jpg"),
            filename: format!("{tag}.jpg"),
            file_size: 0,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: None,
            width: None,
            height: None,
            orientation: None,
            duration: None,
            thumbnail_path: None,
            has_thumbnail: None,
            blurhash: None,
            is_favorite: None,
            semantic_vector_indexed: None,
            metadata: serde_json::json!({}),
            date_modified: chrono::Utc::now(),
            date_indexed: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
        .create(pool)
        .await
        .unwrap();
    }

    async fn photo_still_in_library(pool: &DbPool, hash: &str) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM photos WHERE hash_sha256 = ?")
            .bind(hash)
            .fetch_one(pool)
            .await
            .unwrap()
            > 0
    }

    async fn delete_photo_row(pool: &DbPool, hash: &str) {
        sqlx::query("DELETE FROM photos WHERE hash_sha256 = ?")
            .bind(hash)
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_add_is_idempotent_and_skips_unknown() {
        let pool = create_test_db_pool().await.unwrap();
        let album = create(&pool, "A").await.unwrap();
        seed_photo(&pool, "a").await;
        assert_eq!(
            add_members(&pool, album.id, &[h("a"), h("a"), h("missing")])
                .await
                .unwrap(),
            1
        );
        assert_eq!(count_members(&pool, album.id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_remove_is_membership_only() {
        let pool = create_test_db_pool().await.unwrap();
        let album = create(&pool, "A").await.unwrap();
        seed_photo(&pool, "b").await;
        add_members(&pool, album.id, &[h("b")]).await.unwrap();
        assert_eq!(remove_members(&pool, album.id, &[h("b")]).await.unwrap(), 1);
        assert_eq!(count_members(&pool, album.id).await.unwrap(), 0);
        assert!(photo_still_in_library(&pool, &h("b")).await);
    }

    #[tokio::test]
    async fn test_photo_delete_cascades_and_album_delete_cascades() {
        let pool = create_test_db_pool().await.unwrap();
        let album = create(&pool, "A").await.unwrap();
        seed_photo(&pool, "c").await;
        add_members(&pool, album.id, &[h("c")]).await.unwrap();
        delete_photo_row(&pool, &h("c")).await;
        assert_eq!(count_members(&pool, album.id).await.unwrap(), 0);
        seed_photo(&pool, "d").await;
        add_members(&pool, album.id, &[h("d")]).await.unwrap();
        delete(&pool, album.id).await.unwrap();
        assert!(photo_still_in_library(&pool, &h("d")).await);
    }
}
