//! Persistence and clustering for detected faces.
//!
//! Clustering is online single-link assignment: a new embedding joins the
//! nearest existing person within `CLUSTER_DISTANCE`, otherwise it starts a
//! new person. Manual corrections do not exist in v1 beyond rename/hide, so a
//! stored `person_id` is never overwritten by the clusterer.

use anyhow::Result;
use serde::Serialize;
use sqlx::{Row, Sqlite, Transaction};
use zerocopy::IntoBytes;

use crate::db::{DbPool, Photo};

use super::engine::DetectedFace;
use super::geometry::{cosine_distance, iou, Rect};
use super::model::MODEL_VERSION;

/// Cosine distance at which a face joins an existing person. The equivalent
/// cosine similarity is 0.5.
pub const CLUSTER_DISTANCE: f32 = 0.5;
/// The People view only shows a cluster once it has this many faces.
pub const MIN_VISIBLE_FACES: i64 = 3;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Person {
    pub id: i64,
    pub name: Option<String>,
    pub hidden: bool,
    pub face_count: i64,
    pub cover_face_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PersonRow {
    pub id: i64,
    pub name: Option<String>,
    pub hidden: bool,
}

/// The merge that can still be undone. Exactly one exists while a merge is
/// recorded; the next merge replaces it.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct MergeSlot {
    pub undo_id: i64,
    pub target_id: i64,
    pub target_name: Option<String>,
    pub source_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeOutcome {
    pub moved_faces: u64,
    pub removed_people: u64,
    pub undo_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndoOutcome {
    pub restored_faces: u64,
    pub total_faces: u64,
    pub restored_people: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoResult {
    Done(UndoOutcome),
    /// No merge is recorded.
    Missing,
    /// The recorded merge is newer than the requested one.
    Stale,
}

#[derive(Debug, Clone)]
pub struct OldFace {
    pub id: i64,
    pub bbox: Rect,
    pub person_id: Option<i64>,
    pub embedding: Vec<f32>,
}

/// Photos whose face index is missing, stale for the current model, or whose
/// file fingerprint no longer matches the indexed bytes.
pub async fn pending_photos(
    pool: &DbPool,
    model_version: &str,
    after_hash: &str,
    limit: u32,
) -> Result<Vec<Photo>> {
    let rows = sqlx::query_as::<_, Photo>(
        "SELECT p.* FROM photos p
         LEFT JOIN face_index_state s ON s.photo_hash = p.hash_sha256
         WHERE p.mime_type LIKE 'image/%'
           AND p.hash_sha256 > ?
           AND (s.photo_hash IS NULL
                OR s.model_version <> ?
                OR s.file_size <> p.file_size
                OR s.file_modified <> p.file_modified)
         ORDER BY p.hash_sha256
         LIMIT ?",
    )
    .bind(after_hash)
    .bind(model_version)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn count_pending(pool: &DbPool, model_version: &str) -> Result<u64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM photos p
         LEFT JOIN face_index_state s ON s.photo_hash = p.hash_sha256
         WHERE p.mime_type LIKE 'image/%'
           AND (s.photo_hash IS NULL
                OR s.model_version <> ?
                OR s.file_size <> p.file_size
                OR s.file_modified <> p.file_modified)",
    )
    .bind(model_version)
    .fetch_one(pool)
    .await?;
    Ok(count.max(0) as u64)
}

pub async fn old_faces(pool: &DbPool, photo_hash: &str) -> Result<Vec<OldFace>> {
    let rows = sqlx::query(
        "SELECT f.id, f.bbox_x, f.bbox_y, f.bbox_w, f.bbox_h, f.person_id,
                v.face_vector
         FROM faces f
         LEFT JOIN face_vector_mapping m ON m.face_id = f.id
         LEFT JOIN face_vectors v ON v.rowid = m.id
         WHERE f.photo_hash = ?",
    )
    .bind(photo_hash)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let embedding = row
                .get::<Option<Vec<u8>>, _>("face_vector")
                .map(|bytes| {
                    bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|chunk| f32::from_le_bytes(*chunk))
                        .collect::<Vec<f32>>()
                })
                .unwrap_or_default();
            Ok(OldFace {
                id: row.get("id"),
                bbox: Rect::new(
                    row.get("bbox_x"),
                    row.get("bbox_y"),
                    row.get("bbox_w"),
                    row.get("bbox_h"),
                ),
                person_id: row.get("person_id"),
                embedding,
            })
        })
        .collect()
}

/// Replaces a photo's faces and returns each new face id with its crop bytes.
/// The index state is cleared in the same transaction; the caller writes the
/// crops and then calls [`mark_indexed`], so an interrupted run is retried.
pub async fn write_photo_faces(
    pool: &DbPool,
    photo: &Photo,
    faces: &[DetectedFace],
    old: &[OldFace],
) -> Result<Vec<(i64, Vec<u8>)>> {
    let mut tx = pool.begin().await?;

    // `pending_photos` trusts this row, so it is removed here and only
    // rewritten by `mark_indexed` once the crops are on disk. A run that
    // dies before then leaves the photo pending for the next scan.
    sqlx::query("DELETE FROM face_index_state WHERE photo_hash = ?")
        .bind(&photo.hash_sha256)
        .execute(&mut *tx)
        .await?;

    // sqlite-vec rows are not covered by the FK cascade; remove them before
    // the face rows so the mapping cascade cannot strand a vector.
    sqlx::query(
        "DELETE FROM face_vectors WHERE rowid IN (
            SELECT m.id FROM face_vector_mapping m
            JOIN faces f ON f.id = m.face_id
            WHERE f.photo_hash = ?
         )",
    )
    .bind(&photo.hash_sha256)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM faces WHERE photo_hash = ?")
        .bind(&photo.hash_sha256)
        .execute(&mut *tx)
        .await?;

    let mut crops = Vec::with_capacity(faces.len());
    for (index, face) in faces.iter().enumerate() {
        let person_id = if let Some(person_id) = match_old_person(face, old) {
            Some(person_id)
        } else {
            cluster_person(&mut tx, &face.embedding).await?
        };

        let inserted = sqlx::query(
            "INSERT INTO faces
                (photo_hash, face_index, person_id, bbox_x, bbox_y, bbox_w, bbox_h, quality)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&photo.hash_sha256)
        .bind(index as i64)
        .bind(person_id)
        .bind(face.bbox.x)
        .bind(face.bbox.y)
        .bind(face.bbox.w)
        .bind(face.bbox.h)
        .bind(face.quality)
        .execute(&mut *tx)
        .await?;
        let face_id = inserted.last_insert_rowid();

        let mapping_id: i64 =
            sqlx::query_scalar("INSERT INTO face_vector_mapping (face_id) VALUES (?) RETURNING id")
                .bind(face_id)
                .fetch_one(&mut *tx)
                .await?;

        sqlx::query("INSERT INTO face_vectors (rowid, face_vector) VALUES (?, ?)")
            .bind(mapping_id)
            .bind(face.embedding.as_bytes())
            .execute(&mut *tx)
            .await?;

        crops.push((face_id, face.crop.clone()));
    }

    tx.commit().await?;
    Ok(crops)
}

/// Records a completed face index. Called only after every crop file is on
/// disk, so a crash mid-write leaves the photo pending for the next scan.
pub async fn mark_indexed(pool: &DbPool, photo: &Photo) -> Result<()> {
    sqlx::query(
        "INSERT INTO face_index_state
            (photo_hash, model_version, file_size, file_modified, indexed_at)
         VALUES (?, ?, ?, ?, datetime('now'))
         ON CONFLICT(photo_hash) DO UPDATE SET
            model_version = excluded.model_version,
            file_size = excluded.file_size,
            file_modified = excluded.file_modified,
            indexed_at = excluded.indexed_at",
    )
    .bind(&photo.hash_sha256)
    .bind(MODEL_VERSION)
    .bind(photo.file_size)
    .bind(photo.date_modified.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// A face that moved only slightly (rotation, re-export) keeps its manual
/// person; if the box moved too far, a matching embedding is the fallback.
fn match_old_person(face: &DetectedFace, old: &[OldFace]) -> Option<i64> {
    let by_box = old
        .iter()
        .filter(|candidate| candidate.person_id.is_some())
        .map(|candidate| (candidate, iou(candidate.bbox, face.bbox)))
        .filter(|(_, overlap)| *overlap >= 0.5)
        .max_by(|(_, left), (_, right)| {
            left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
        })
        .and_then(|(candidate, _)| candidate.person_id);
    if by_box.is_some() {
        return by_box;
    }

    old.iter()
        .filter(|candidate| candidate.person_id.is_some() && !candidate.embedding.is_empty())
        .map(|candidate| {
            (
                candidate,
                cosine_distance(&candidate.embedding, &face.embedding),
            )
        })
        .filter(|(_, distance)| *distance <= CLUSTER_DISTANCE)
        .min_by(|(_, left), (_, right)| {
            left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
        })
        .and_then(|(candidate, _)| candidate.person_id)
}

async fn cluster_person(
    tx: &mut Transaction<'_, Sqlite>,
    embedding: &[f32],
) -> Result<Option<i64>> {
    if embedding.is_empty() {
        return Ok(None);
    }

    let rows: Vec<(Option<i64>, f32)> = sqlx::query_as(
        // vec0's MATCH ranks by its default L2 metric. The stored embeddings
        // are L2-normalised, so that ranking is cosine-equivalent; the
        // distance we threshold on must still be the cosine distance.
        "SELECT f.person_id, vec_distance_cosine(v.face_vector, ?) AS distance
         FROM face_vectors v
         JOIN face_vector_mapping m ON m.id = v.rowid
         JOIN faces f ON f.id = m.face_id
         WHERE v.face_vector MATCH ? AND k = 5",
    )
    .bind(embedding.as_bytes())
    .bind(embedding.as_bytes())
    .fetch_all(&mut **tx)
    .await?;

    if let Some((person_id, _)) = rows
        .into_iter()
        .filter_map(|(person_id, distance)| {
            person_id
                .filter(|_| distance <= CLUSTER_DISTANCE)
                .map(|person_id| (person_id, distance))
        })
        .min_by(|(_, left), (_, right)| {
            left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
        })
    {
        return Ok(Some(person_id));
    }

    let id: i64 = sqlx::query_scalar("INSERT INTO people DEFAULT VALUES RETURNING id")
        .fetch_one(&mut **tx)
        .await?;
    Ok(Some(id))
}

pub async fn people(pool: &DbPool, include_hidden: bool) -> Result<Vec<Person>> {
    let rows = sqlx::query_as::<_, Person>(
        "SELECT p.id, p.name, p.hidden, COUNT(f.id) AS face_count,
                (SELECT f2.id FROM faces f2
                 WHERE f2.person_id = p.id
                 ORDER BY f2.quality DESC, f2.id
                 LIMIT 1) AS cover_face_id
         FROM people p
         JOIN faces f ON f.person_id = p.id
         WHERE (? = 1 OR p.hidden = 0)
         GROUP BY p.id
         HAVING COUNT(f.id) >= ?
         ORDER BY face_count DESC, p.name COLLATE NOCASE, p.id",
    )
    .bind(include_hidden)
    .bind(MIN_VISIBLE_FACES)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_person(
    pool: &DbPool,
    id: i64,
    name: Option<Option<String>>,
    hidden: Option<bool>,
) -> Result<Option<PersonRow>> {
    let set_name = name.is_some();
    let name = name.flatten();
    let set_hidden = hidden.is_some();
    let hidden = hidden.unwrap_or(false);

    // One conditional UPDATE instead of a read-modify-write: two overlapping
    // PATCHes (one `{name}`, one `{hidden}`) cannot restore each other's
    // stale column. `rows_affected` doubles as the existence check.
    let result = sqlx::query(
        "UPDATE people SET
            name = CASE WHEN ? THEN ? ELSE name END,
            hidden = CASE WHEN ? THEN ? ELSE hidden END,
            updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(set_name)
    .bind(&name)
    .bind(set_hidden)
    .bind(hidden)
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Ok(None);
    }

    let updated =
        sqlx::query_as::<_, PersonRow>("SELECT id, name, hidden FROM people WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await?;
    Ok(Some(updated))
}

/// Merges every selected source person into the keeper in one transaction.
///
/// The faces are reassigned before the person rows are deleted: `faces.person_id`
/// is `ON DELETE SET NULL`, so a delete first would orphan the faces instead of
/// moving them. The undo slot is snapshotted before the update and replaces any
/// previous slot, keeping exactly one merge undoable.
pub async fn merge_people(
    pool: &DbPool,
    target_id: i64,
    source_ids: &[i64],
    name: Option<Option<String>>,
    hidden: Option<bool>,
) -> Result<Option<MergeOutcome>> {
    // IMMEDIATE takes the writer lock before the first SELECT: a deferred
    // transaction pins a WAL snapshot and a concurrent face-phase commit in
    // that window makes the later write fail with SQLITE_BUSY_SNAPSHOT, which
    // the 30 s busy timeout cannot resolve.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

    for id in std::iter::once(target_id).chain(source_ids.iter().copied()) {
        let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM people WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Ok(None);
        }
    }

    sqlx::query("DELETE FROM person_merge_undo")
        .execute(&mut *tx)
        .await?;
    let undo_id: i64 =
        sqlx::query_scalar("INSERT INTO person_merge_undo (target_id) VALUES (?) RETURNING id")
            .bind(target_id)
            .fetch_one(&mut *tx)
            .await?;

    for source in source_ids {
        sqlx::query(
            "INSERT INTO person_merge_undo_source (undo_id, person_id, name, hidden)
             SELECT ?, id, name, hidden FROM people WHERE id = ?",
        )
        .bind(undo_id)
        .bind(source)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO person_merge_undo_face (undo_id, person_id, photo_hash, face_index)
             SELECT ?, ?, photo_hash, face_index FROM faces WHERE person_id = ?",
        )
        .bind(undo_id)
        .bind(source)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    }

    let mut moved_faces = 0;
    let mut removed_people = 0;
    for source in source_ids {
        let moved = sqlx::query("UPDATE faces SET person_id = ? WHERE person_id = ?")
            .bind(target_id)
            .bind(source)
            .execute(&mut *tx)
            .await?;
        moved_faces += moved.rows_affected();
        let removed = sqlx::query("DELETE FROM people WHERE id = ?")
            .bind(source)
            .execute(&mut *tx)
            .await?;
        removed_people += removed.rows_affected();
    }

    let set_name = name.is_some();
    let name = name.flatten();
    let set_hidden = hidden.is_some();
    let hidden = hidden.unwrap_or(false);
    sqlx::query(
        "UPDATE people SET
            name = CASE WHEN ? THEN ? ELSE name END,
            hidden = CASE WHEN ? THEN ? ELSE hidden END,
            updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(set_name)
    .bind(&name)
    .bind(set_hidden)
    .bind(hidden)
    .bind(target_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Some(MergeOutcome {
        moved_faces,
        removed_people,
        undo_id,
    }))
}

/// Restores the snapshot of the recorded merge, best effort.
///
/// Only faces that still belong to the keeper and whose `(photo_hash,
/// face_index)` survived a re-index return to their source person; the counts
/// say how many did. A source person is re-created only when at least one face
/// returned, and the slot is consumed either way.
pub async fn undo_merge(pool: &DbPool, undo_id: i64) -> Result<UndoResult> {
    // Same snapshot rule as `merge_people`: the slot/source reads must not
    // pin a WAL snapshot before the transaction upgrades to a writer.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

    let Some(current) = sqlx::query_scalar::<_, i64>("SELECT id FROM person_merge_undo")
        .fetch_optional(&mut *tx)
        .await?
    else {
        return Ok(UndoResult::Missing);
    };
    if current != undo_id {
        return Ok(UndoResult::Stale);
    }

    let target_id: i64 = sqlx::query_scalar("SELECT target_id FROM person_merge_undo WHERE id = ?")
        .bind(current)
        .fetch_one(&mut *tx)
        .await?;
    let total_faces: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM person_merge_undo_face WHERE undo_id = ?")
            .bind(current)
            .fetch_one(&mut *tx)
            .await?;
    let sources: Vec<(i64, Option<String>, bool)> = sqlx::query_as(
        "SELECT person_id, name, hidden FROM person_merge_undo_source
         WHERE undo_id = ? ORDER BY person_id",
    )
    .bind(current)
    .fetch_all(&mut *tx)
    .await?;

    let mut restored_faces = 0;
    let mut restored_people = 0;
    for (person_id, name, hidden) in sources {
        // The person row must exist before faces can point at it again; when
        // no face returns, the freshly created row is removed below.
        let inserted = sqlx::query(
            "INSERT INTO people (id, name, hidden) VALUES (?, ?, ?)
             ON CONFLICT(id) DO NOTHING",
        )
        .bind(person_id)
        .bind(&name)
        .bind(hidden)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        let moved = sqlx::query(
            "UPDATE faces SET person_id = ?
             WHERE person_id = ? AND EXISTS (
                SELECT 1 FROM person_merge_undo_face uf
                WHERE uf.undo_id = ? AND uf.person_id = ?
                  AND uf.photo_hash = faces.photo_hash
                  AND uf.face_index = faces.face_index)",
        )
        .bind(person_id)
        .bind(target_id)
        .bind(current)
        .bind(person_id)
        .execute(&mut *tx)
        .await?;
        let restored = moved.rows_affected();
        if restored == 0 {
            if inserted > 0 {
                sqlx::query("DELETE FROM people WHERE id = ?")
                    .bind(person_id)
                    .execute(&mut *tx)
                    .await?;
            }
            continue;
        }
        restored_people += 1;
        restored_faces += restored;
    }

    sqlx::query("DELETE FROM person_merge_undo WHERE id = ?")
        .bind(current)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(UndoResult::Done(UndoOutcome {
        restored_faces,
        total_faces: total_faces as u64,
        restored_people,
    }))
}

/// The merge that can still be undone, if any.
pub async fn last_merge(pool: &DbPool) -> Result<Option<MergeSlot>> {
    Ok(sqlx::query_as::<_, MergeSlot>(
        "SELECT u.id AS undo_id, u.target_id, p.name AS target_name,
                (SELECT COUNT(*) FROM person_merge_undo_source s WHERE s.undo_id = u.id)
                    AS source_count
         FROM person_merge_undo u
         LEFT JOIN people p ON p.id = u.target_id
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await?)
}

/// Hides or unhides every known person; unknown ids are skipped.
pub async fn set_people_hidden(pool: &DbPool, ids: &[i64], hidden: bool) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let mut updated = 0;
    for id in ids {
        let result =
            sqlx::query("UPDATE people SET hidden = ?, updated_at = datetime('now') WHERE id = ?")
                .bind(hidden)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        updated += result.rows_affected();
    }
    tx.commit().await?;
    Ok(updated)
}

pub async fn face_crop_info(pool: &DbPool, face_id: i64) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT photo_hash FROM faces WHERE id = ?")
            .bind(face_id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Forgets that a photo was indexed, so the next scan retries it.
pub async fn clear_index_state(pool: &DbPool, photo_hash: &str) -> Result<()> {
    sqlx::query("DELETE FROM face_index_state WHERE photo_hash = ?")
        .bind(photo_hash)
        .execute(pool)
        .await?;
    Ok(())
}

/// Removes vector rows whose face mapping disappeared through a cascade.
pub async fn cleanup_orphan_vectors(pool: &DbPool) -> Result<()> {
    sqlx::query(
        "DELETE FROM face_vectors
         WHERE rowid NOT IN (SELECT id FROM face_vector_mapping)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::create_test_photo;

    fn face(quality: f32, x: f32) -> DetectedFace {
        DetectedFace {
            bbox: Rect::new(x, 0.1, 0.2, 0.2),
            quality,
            embedding: vec![1.0; 512],
            crop: vec![1, 2, 3],
        }
    }

    async fn seed_photo(pool: &DbPool, hash: &str) -> Photo {
        let photo = create_test_photo(format!("{hash}.jpg"), hash.to_string());
        photo.create(pool).await.unwrap();
        photo
    }

    #[tokio::test]
    async fn pending_photos_detects_new_and_changed_files() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let photo = seed_photo(&pool, "pending").await;

        assert_eq!(count_pending(&pool, MODEL_VERSION).await.unwrap(), 1);

        sqlx::query(
            "INSERT INTO face_index_state
             (photo_hash, model_version, file_size, file_modified)
             VALUES (?, ?, ?, ?)",
        )
        .bind(&photo.hash_sha256)
        .bind(MODEL_VERSION)
        .bind(photo.file_size)
        .bind(photo.date_modified.to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(count_pending(&pool, MODEL_VERSION).await.unwrap(), 0);

        sqlx::query("UPDATE photos SET file_size = file_size + 1 WHERE hash_sha256 = ?")
            .bind(&photo.hash_sha256)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(count_pending(&pool, MODEL_VERSION).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn people_hide_short_clusters_and_hidden_people_by_default() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let photo = seed_photo(&pool, "people").await;
        let visible: i64 = sqlx::query_scalar("INSERT INTO people DEFAULT VALUES RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
        let hidden: i64 = sqlx::query_scalar("INSERT INTO people DEFAULT VALUES RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE people SET name = 'Visible' WHERE id = ?")
            .bind(visible)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE people SET name = 'Hidden', hidden = 1 WHERE id = ?")
            .bind(hidden)
            .execute(&pool)
            .await
            .unwrap();

        for (index, person_id) in [
            (0, visible),
            (1, visible),
            (2, visible),
            (3, hidden),
            (4, hidden),
            (5, hidden),
        ] {
            sqlx::query(
                "INSERT INTO faces
                 (photo_hash, face_index, person_id, bbox_x, bbox_y, bbox_w, bbox_h, quality)
                 VALUES (?, ?, ?, 0.0, 0.0, 1.0, 1.0, 1.0)",
            )
            .bind(&photo.hash_sha256)
            .bind(index)
            .bind(person_id)
            .execute(&pool)
            .await
            .unwrap();
        }

        let visible_people = people(&pool, false).await.unwrap();
        assert_eq!(visible_people.len(), 1);
        assert_eq!(visible_people[0].name.as_deref(), Some("Visible"));
        assert_eq!(visible_people[0].face_count, 3);

        let all = people(&pool, true).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn update_person_can_rename_clear_and_hide() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let id: i64 = sqlx::query_scalar("INSERT INTO people DEFAULT VALUES RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();

        let renamed = update_person(&pool, id, Some(Some("Anna".to_string())), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(renamed.name.as_deref(), Some("Anna"));

        let cleared = update_person(&pool, id, Some(None), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cleared.name, None);

        let hidden = update_person(&pool, id, None, Some(true))
            .await
            .unwrap()
            .unwrap();
        assert!(hidden.hidden);
    }

    #[tokio::test]
    async fn a_photo_hash_rewrite_moves_faces_and_the_index_state() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let photo = seed_photo(&pool, "rotate").await;
        let person: i64 = sqlx::query_scalar("INSERT INTO people DEFAULT VALUES RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO faces
             (photo_hash, face_index, person_id, bbox_x, bbox_y, bbox_w, bbox_h, quality)
             VALUES (?, 0, ?, 0.0, 0.0, 1.0, 1.0, 1.0)",
        )
        .bind(&photo.hash_sha256)
        .bind(person)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO face_index_state
             (photo_hash, model_version, file_size, file_modified)
             VALUES (?, ?, ?, ?)",
        )
        .bind(&photo.hash_sha256)
        .bind(MODEL_VERSION)
        .bind(photo.file_size)
        .bind(photo.date_modified.to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        let new_hash = "b".repeat(64);
        sqlx::query("UPDATE photos SET hash_sha256 = ? WHERE hash_sha256 = ?")
            .bind(&new_hash)
            .bind(&photo.hash_sha256)
            .execute(&pool)
            .await
            .unwrap();

        let face_hash: String = sqlx::query_scalar("SELECT photo_hash FROM faces")
            .fetch_one(&pool)
            .await
            .unwrap();
        let state_hash: String = sqlx::query_scalar("SELECT photo_hash FROM face_index_state")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(face_hash, new_hash);
        assert_eq!(state_hash, new_hash);
    }
    #[tokio::test]
    async fn clustering_picks_the_nearest_person_not_the_lowest_id() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let photo = seed_photo(&pool, "cluster").await;
        let first: i64 = sqlx::query_scalar("INSERT INTO people DEFAULT VALUES RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
        let second: i64 = sqlx::query_scalar("INSERT INTO people DEFAULT VALUES RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();

        let mut vector_a = vec![0.0_f32; 512];
        vector_a[0] = 0.9;
        vector_a[1] = 0.435_89;
        let mut vector_b = vec![0.0_f32; 512];
        vector_b[0] = 1.0;
        for (index, (person_id, vector)) in [(first, vector_a), (second, vector_b)]
            .into_iter()
            .enumerate()
        {
            let face_id: i64 = sqlx::query_scalar(
                "INSERT INTO faces
                 (photo_hash, face_index, person_id, bbox_x, bbox_y, bbox_w, bbox_h, quality)
                 VALUES (?, ?, ?, 0.0, 0.0, 1.0, 1.0, 1.0)
                 RETURNING id",
            )
            .bind(&photo.hash_sha256)
            .bind(index as i64)
            .bind(person_id)
            .fetch_one(&pool)
            .await
            .unwrap();
            let mapping_id: i64 = sqlx::query_scalar(
                "INSERT INTO face_vector_mapping (face_id) VALUES (?) RETURNING id",
            )
            .bind(face_id)
            .fetch_one(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO face_vectors (rowid, face_vector) VALUES (?, ?)")
                .bind(mapping_id)
                .bind(vector.as_slice().as_bytes())
                .execute(&pool)
                .await
                .unwrap();
        }

        let mut probe = vec![0.0_f32; 512];
        probe[0] = 1.0;
        let mut tx = pool.begin().await.unwrap();
        let assigned = cluster_person(&mut tx, &probe).await.unwrap();
        tx.commit().await.unwrap();

        assert_eq!(assigned, Some(second));
    }
    #[tokio::test]
    async fn write_photo_faces_creates_a_person_and_reuses_it_on_reindex() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let photo = seed_photo(&pool, "reindex").await;

        let first = vec![face(0.9, 0.1)];
        let crops = write_photo_faces(&pool, &photo, &first, &[]).await.unwrap();
        assert_eq!(crops.len(), 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM people")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );

        let old = old_faces(&pool, &photo.hash_sha256).await.unwrap();
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].person_id, Some(1));
        assert_eq!(old[0].embedding.len(), 512);
        let second = vec![face(0.8, 0.1)];
        write_photo_faces(&pool, &photo, &second, &old)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM people")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1,
            "the reindexed face must keep its person"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM faces")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
    }

    async fn seed_person(pool: &DbPool, name: Option<&str>, hidden: bool) -> i64 {
        let id: i64 = sqlx::query_scalar("INSERT INTO people DEFAULT VALUES RETURNING id")
            .fetch_one(pool)
            .await
            .unwrap();
        if let Some(name) = name {
            sqlx::query("UPDATE people SET name = ? WHERE id = ?")
                .bind(name)
                .bind(id)
                .execute(pool)
                .await
                .unwrap();
        }
        if hidden {
            sqlx::query("UPDATE people SET hidden = 1 WHERE id = ?")
                .bind(id)
                .execute(pool)
                .await
                .unwrap();
        }
        id
    }

    async fn seed_faces(pool: &DbPool, hash: &str, person_id: i64, first: i64, count: i64) {
        for index in first..first + count {
            sqlx::query(
                "INSERT INTO faces
                 (photo_hash, face_index, person_id, bbox_x, bbox_y, bbox_w, bbox_h, quality)
                 VALUES (?, ?, ?, 0.0, 0.0, 1.0, 1.0, 1.0)",
            )
            .bind(hash)
            .bind(index)
            .bind(person_id)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    async fn faces_with_person(pool: &DbPool, person_id: i64) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM faces WHERE person_id = ?")
            .bind(person_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn person_snapshot(pool: &DbPool, id: i64) -> Option<(Option<String>, bool)> {
        sqlx::query_as("SELECT name, hidden FROM people WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn merge_moves_faces_records_the_undo_slot_and_renames_the_keeper() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let photo = seed_photo(&pool, "merge").await;
        let keeper = seed_person(&pool, Some("Anna"), false).await;
        let source = seed_person(&pool, None, true).await;
        seed_faces(&pool, &photo.hash_sha256, keeper, 0, 3).await;
        seed_faces(&pool, &photo.hash_sha256, source, 3, 2).await;

        let outcome = merge_people(
            &pool,
            keeper,
            &[source],
            Some(Some("Anna".to_string())),
            None,
        )
        .await
        .unwrap()
        .expect("both persons exist");

        assert_eq!(outcome.moved_faces, 2);
        assert_eq!(outcome.removed_people, 1);
        assert_eq!(faces_with_person(&pool, keeper).await, 5);
        assert_eq!(person_snapshot(&pool, source).await, None);
        assert_eq!(
            person_snapshot(&pool, keeper).await,
            Some((Some("Anna".into()), false))
        );

        let slot = last_merge(&pool).await.unwrap().expect("slot recorded");
        assert_eq!(slot.undo_id, outcome.undo_id);
        assert_eq!(slot.target_id, keeper);
        assert_eq!(slot.target_name.as_deref(), Some("Anna"));
        assert_eq!(slot.source_count, 1);
    }

    #[tokio::test]
    async fn merge_applies_the_supplied_hidden_state_to_the_keeper() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let photo = seed_photo(&pool, "merge-hidden").await;
        let keeper = seed_person(&pool, Some("Anna"), false).await;
        let source = seed_person(&pool, None, false).await;
        seed_faces(&pool, &photo.hash_sha256, keeper, 0, 1).await;
        seed_faces(&pool, &photo.hash_sha256, source, 1, 1).await;

        merge_people(&pool, keeper, &[source], None, Some(true))
            .await
            .unwrap()
            .expect("both persons exist");

        assert_eq!(
            person_snapshot(&pool, keeper).await,
            Some((Some("Anna".into()), true))
        );
    }

    #[tokio::test]
    async fn merge_with_an_unknown_person_writes_nothing() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let photo = seed_photo(&pool, "merge-unknown").await;
        let keeper = seed_person(&pool, Some("Anna"), false).await;
        seed_faces(&pool, &photo.hash_sha256, keeper, 0, 2).await;

        let outcome = merge_people(&pool, keeper, &[9999], None, None)
            .await
            .unwrap();
        assert!(outcome.is_none());
        assert_eq!(faces_with_person(&pool, keeper).await, 2);
        assert!(last_merge(&pool).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn undo_restores_the_source_person_with_its_snapshot_fields() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let photo = seed_photo(&pool, "undo").await;
        let keeper = seed_person(&pool, Some("Anna"), false).await;
        let source = seed_person(&pool, Some("Oma"), true).await;
        seed_faces(&pool, &photo.hash_sha256, keeper, 0, 3).await;
        seed_faces(&pool, &photo.hash_sha256, source, 3, 2).await;

        let merge = merge_people(
            &pool,
            keeper,
            &[source],
            Some(Some("Anna".to_string())),
            None,
        )
        .await
        .unwrap()
        .expect("both persons exist");

        let UndoResult::Done(outcome) = undo_merge(&pool, merge.undo_id).await.unwrap() else {
            panic!("the current slot must undo");
        };
        assert_eq!(outcome.restored_faces, 2);
        assert_eq!(outcome.total_faces, 2);
        assert_eq!(outcome.restored_people, 1);
        assert_eq!(faces_with_person(&pool, source).await, 2);
        assert_eq!(faces_with_person(&pool, keeper).await, 3);
        assert_eq!(
            person_snapshot(&pool, source).await,
            Some((Some("Oma".into()), true))
        );
        assert!(
            last_merge(&pool).await.unwrap().is_none(),
            "slot is consumed"
        );
    }

    #[tokio::test]
    async fn undo_restores_only_the_face_positions_that_survive_a_reindex() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let first_photo = seed_photo(&pool, "undo-a").await;
        let second_photo = seed_photo(&pool, "undo-b").await;
        let keeper = seed_person(&pool, Some("Anna"), false).await;
        let source = seed_person(&pool, None, false).await;
        seed_faces(&pool, &first_photo.hash_sha256, source, 0, 2).await;
        seed_faces(&pool, &second_photo.hash_sha256, source, 0, 1).await;

        let merge = merge_people(&pool, keeper, &[source], None, None)
            .await
            .unwrap()
            .expect("both persons exist");

        // A re-index replaces the photo's faces; the person association
        // survives, the face indexes do not have to.
        sqlx::query("DELETE FROM faces WHERE photo_hash = ?")
            .bind(&second_photo.hash_sha256)
            .execute(&pool)
            .await
            .unwrap();
        seed_faces(&pool, &second_photo.hash_sha256, keeper, 7, 1).await;

        let UndoResult::Done(outcome) = undo_merge(&pool, merge.undo_id).await.unwrap() else {
            panic!("the current slot must undo");
        };
        assert_eq!(outcome.restored_faces, 2, "only surviving positions return");
        assert_eq!(
            outcome.total_faces, 3,
            "the re-indexed face is still counted"
        );
        assert_eq!(outcome.restored_people, 1);
        assert_eq!(faces_with_person(&pool, source).await, 2);
        assert_eq!(faces_with_person(&pool, keeper).await, 1);
    }

    #[tokio::test]
    async fn undo_rejects_a_slot_replaced_by_a_newer_merge() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let photo = seed_photo(&pool, "undo-stale").await;
        let keeper = seed_person(&pool, Some("Anna"), false).await;
        let first = seed_person(&pool, None, false).await;
        let second = seed_person(&pool, None, false).await;
        seed_faces(&pool, &photo.hash_sha256, keeper, 0, 1).await;
        seed_faces(&pool, &photo.hash_sha256, first, 1, 1).await;
        seed_faces(&pool, &photo.hash_sha256, second, 2, 1).await;

        let first_merge = merge_people(&pool, keeper, &[first], None, None)
            .await
            .unwrap()
            .expect("first merge");
        let second_merge = merge_people(&pool, keeper, &[second], None, None)
            .await
            .unwrap()
            .expect("second merge");

        assert!(matches!(
            undo_merge(&pool, first_merge.undo_id).await.unwrap(),
            UndoResult::Stale
        ));
        assert!(matches!(
            undo_merge(&pool, second_merge.undo_id).await.unwrap(),
            UndoResult::Done(_)
        ));
    }

    #[tokio::test]
    async fn undo_without_a_slot_reports_missing() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        assert!(matches!(
            undo_merge(&pool, 1).await.unwrap(),
            UndoResult::Missing
        ));
    }

    #[tokio::test]
    async fn bulk_hide_updates_known_people_and_skips_missing_ids() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let first = seed_person(&pool, None, false).await;
        let second = seed_person(&pool, None, false).await;

        assert_eq!(
            set_people_hidden(&pool, &[first, second, 9999], true)
                .await
                .unwrap(),
            2
        );
        assert_eq!(person_snapshot(&pool, first).await, Some((None, true)));
        assert_eq!(person_snapshot(&pool, second).await, Some((None, true)));

        assert_eq!(set_people_hidden(&pool, &[first], false).await.unwrap(), 1);
        assert_eq!(person_snapshot(&pool, first).await, Some((None, false)));
    }
}
