//! The one photo listing: filters, SQL/in-memory strategy, ordering and
//! pagination.
//!
//! The database stores no capture date, so only an explicit column sort can be
//! ordered and paged by SQLite. Every date-ordered listing (the default, the
//! grid, an album) orders on the file-derived date the file-facts index holds:
//! the request reads only `(hash, path)` per row, orders those keys with
//! [`sort_photos`]'s contract and fetches just the rows it returns. The
//! unfiltered library's keys — what infinite scroll asks for — are cached until
//! the row set or the facts change, so a page costs the page, not the library.
//! Callers hand this module a request instead of re-sequencing that ladder.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{DateTime, Utc};

use crate::db::{
    build_order_clause, build_search_where, date_in_month_range, is_sql_sort, month_range_bounds,
    sort_photos, DbPool, Photo,
};
use crate::db_types::SearchQuery;
use crate::media_facts::MediaFactsIndex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListingPage {
    pub limit: i64,
    pub offset: i64,
}

/// What to list. `page: None` returns the complete filtered set (the Map view
/// contract); `album` scopes to one album's members.
#[derive(Debug, Clone, Copy)]
pub struct ListingRequest<'a> {
    pub query: &'a SearchQuery,
    pub album: Option<i64>,
    pub sort: Option<&'a str>,
    pub order: Option<&'a str>,
    pub page: Option<ListingPage>,
}

#[derive(Debug)]
pub struct Listing {
    pub photos: Vec<Photo>,
    pub total: usize,
}

/// Lists photos for the given request. The returned rows are enriched from the
/// file-facts index, because the strategy needs their file-derived date to
/// order and filter; API replies still cross the reply seam.
pub async fn list(
    pool: &DbPool,
    facts: &MediaFactsIndex,
    request: ListingRequest<'_>,
) -> Result<Listing, Box<dyn std::error::Error>> {
    let (mut where_clause, params) = build_search_where(request.query);
    if request.album.is_some() {
        where_clause.push_str(
            " AND hash_sha256 IN (SELECT photo_hash FROM album_members WHERE album_id = ?)",
        );
    }
    let bounds = month_range_bounds(
        request.query.year,
        request.query.month,
        request.query.to_year,
        request.query.to_month,
    );

    // SQL fast path: no date filter and an explicit column sort. The rows still
    // need enrichment: this module is also the map's source, coordinates
    // included.
    if bounds.is_none() && is_sql_sort(request.sort) {
        let total = match request.page {
            Some(_) => {
                let count_sql = format!("SELECT COUNT(*) FROM photos{where_clause}");
                let mut count_query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql));
                for param in &params {
                    count_query = count_query.bind(param);
                }
                if let Some(album_id) = request.album {
                    count_query = count_query.bind(album_id);
                }
                count_query.fetch_one(pool).await? as usize
            }
            None => 0,
        };

        let order_clause = build_order_clause(request.sort, request.order);
        let photos = fetch_rows(
            pool,
            facts,
            &where_clause,
            &params,
            request.album,
            Some(&order_clause),
            request.page,
        )
        .await?;
        let total = request.page.map_or(photos.len(), |_| total);
        return Ok(Listing { photos, total });
    }

    // Everything below needs the file-derived dates: the month filter always,
    // the default order does. Both therefore work on cheap `(date, hash)` keys
    // and only the rows that are actually returned get loaded — fetching every
    // row (and its metadata JSON) for a 50-row page made the grid's next-page
    // request scale with the whole library. The unfiltered library's keys are
    // served from the shared order cache; a filtered set is read per request.
    let unfiltered = request.query.q.is_none() && request.album.is_none();
    let keys: Arc<Vec<OrderKey>> = if unfiltered {
        unfiltered_order_keys(pool, facts).await?
    } else {
        let mut keys = fetch_order_keys(pool, facts, &where_clause, &params, request.album).await?;
        sort_order_keys(&mut keys, Some("asc"));
        Arc::new(keys)
    };

    // The keys are stored ascending; the default direction walks them
    // backwards, which reverses the `hash_sha256` tiebreak with it — the exact
    // reversal `sort_photos` applies.
    let ascending = matches!(request.order, Some("asc"));
    let total = ordered_keys(&keys, ascending)
        .filter(|key| in_range(key, bounds.as_ref()))
        .count();

    // A column sort behind a month filter cannot be paged by key: it needs the
    // whole filtered set, sorts it as rows, and pages afterwards.
    let key_page = if is_sql_sort(request.sort) {
        None
    } else {
        request.page
    };
    let hashes = ordered_hashes(ordered_keys(&keys, ascending), bounds.as_ref(), key_page);
    let mut photos = fetch_rows_by_hashes(pool, facts, &hashes).await?;
    if is_sql_sort(request.sort) {
        sort_photos(&mut photos, request.sort, request.order);
        if let Some(page) = request.page {
            photos = paginate(photos, page);
        }
    }

    Ok(Listing { photos, total })
}

/// One listing order entry: the file-derived capture date and the row's hash.
type OrderKey = (Option<DateTime<Utc>>, String);

/// The unfiltered library's date order, cached across requests: the whole
/// library's `(date, hash)` keys, ascending.
///
/// The capture date exists only in the files, so neither SQLite nor a cheap
/// query can order or page the default listing; without this cache every page
/// re-read the library's every row. The cached list is invalidated by two
/// exact, cheap signals — the row-set revision the `library_revision` triggers
/// bump (insert/delete/re-key) and the file-facts generation (any date change)
/// — and never by a time-to-live, which would silently serve a stale order.
struct CachedOrder {
    facts_instance: u64,
    revision: i64,
    generation: u64,
    keys: Arc<Vec<OrderKey>>,
}

static LISTING_ORDER: OnceLock<Mutex<Option<CachedOrder>>> = OnceLock::new();

fn order_cache() -> &'static Mutex<Option<CachedOrder>> {
    LISTING_ORDER.get_or_init(|| Mutex::new(None))
}

fn cached_order(
    facts: &MediaFactsIndex,
    revision: i64,
    generation: u64,
) -> Option<Arc<Vec<OrderKey>>> {
    let cached = order_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cached = cached.as_ref()?;
    let valid = cached.facts_instance == facts.instance_id()
        && cached.revision == revision
        && cached.generation == generation;
    valid.then(|| Arc::clone(&cached.keys))
}

fn store_order(facts: &MediaFactsIndex, revision: i64, generation: u64, keys: Arc<Vec<OrderKey>>) {
    *order_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(CachedOrder {
        facts_instance: facts.instance_id(),
        revision,
        generation,
        keys,
    });
}

/// The unfiltered library's cached date order, rebuilt only when the row set
/// or the file facts changed.
async fn unfiltered_order_keys(
    pool: &DbPool,
    facts: &MediaFactsIndex,
) -> Result<Arc<Vec<OrderKey>>, Box<dyn std::error::Error>> {
    // Both signals are read BEFORE the rows: a change landing afterwards only
    // makes the stored entry look older than the rows it describes, which
    // forces a rebuild on the next request — never a stale hit. Reading them
    // after the rows could label older rows with a newer revision instead.
    let generation = facts.generation();
    let revision = crate::db::library_revision(pool).await?;
    if let Some(keys) = cached_order(facts, revision, generation) {
        return Ok(keys);
    }

    // No WHERE clause: the cached order spans the whole library.
    let mut keys = fetch_order_keys(pool, facts, "", &[], None).await?;
    sort_order_keys(&mut keys, Some("asc"));
    let keys = Arc::new(keys);
    store_order(facts, revision, generation, Arc::clone(&keys));
    Ok(keys)
}

/// The listing order's keys for one filtered set: `(capture date, hash)` per
/// row, dates from the file-facts index. Only the two identifying columns are
/// read — never the rows' metadata.
async fn fetch_order_keys(
    pool: &DbPool,
    facts: &MediaFactsIndex,
    where_clause: &str,
    params: &[String],
    album: Option<i64>,
) -> Result<Vec<OrderKey>, Box<dyn std::error::Error>> {
    let sql = format!("SELECT hash_sha256, file_path FROM photos{where_clause}");
    let mut query = sqlx::query_as::<_, (String, String)>(sqlx::AssertSqlSafe(sql));
    for param in params {
        query = query.bind(param);
    }
    if let Some(album_id) = album {
        query = query.bind(album_id);
    }

    let rows = query.fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|(hash, file_path)| (facts.get(&file_path).and_then(|entry| entry.taken_at), hash))
        .collect())
}

/// Orders the listing keys exactly like [`sort_photos`]: primary ascending
/// (unknown date before every known one) with the `hash_sha256` tiebreak in the
/// same direction, the whole comparison reversed for the default `DESC`.
fn sort_order_keys(keys: &mut [OrderKey], order: Option<&str>) {
    let ascending = matches!(order, Some("asc"));
    keys.sort_by(|a, b| {
        let ordering = a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1));
        if ascending {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

/// Walks the ascending keys in the requested direction.
fn ordered_keys(keys: &[OrderKey], ascending: bool) -> Box<dyn Iterator<Item = &OrderKey> + '_> {
    if ascending {
        Box::new(keys.iter())
    } else {
        Box::new(keys.iter().rev())
    }
}

/// Whether a key lies inside the month range (always true without one).
fn in_range(key: &OrderKey, bounds: Option<&(String, String)>) -> bool {
    bounds.is_none_or(|bounds| date_in_month_range(key.0, bounds))
}

/// The hashes of an ordered key list, month range applied before the offset so
/// pagination never counts an excluded key. `page: None` yields the complete
/// filtered set (the Map contract).
fn ordered_hashes<'a>(
    ordered: impl Iterator<Item = &'a OrderKey>,
    bounds: Option<&(String, String)>,
    page: Option<ListingPage>,
) -> Vec<String> {
    let (offset, limit) = match page {
        Some(page) => (
            usize::try_from(page.offset).unwrap_or(0),
            usize::try_from(page.limit).unwrap_or(0),
        ),
        None => (0, usize::MAX),
    };
    ordered
        .filter(|key| in_range(key, bounds))
        .skip(offset)
        .take(limit)
        .map(|(_, hash)| hash.clone())
        .collect()
}

/// `photos`' slice of a listing page, skipping `offset` rows.
fn paginate(photos: Vec<Photo>, page: ListingPage) -> Vec<Photo> {
    let offset = usize::try_from(page.offset).unwrap_or(0);
    let limit = usize::try_from(page.limit).unwrap_or(0);
    photos.into_iter().skip(offset).take(limit).collect()
}

/// Fetches the page's rows by primary key and restores the order the keys
/// dictate (SQL's `IN` returns database order), enriching only those rows.
async fn fetch_rows_by_hashes(
    pool: &DbPool,
    facts: &MediaFactsIndex,
    hashes: &[String],
) -> Result<Vec<Photo>, Box<dyn std::error::Error>> {
    if hashes.is_empty() {
        return Ok(Vec::new());
    }

    // One placeholder per hash exceeds SQLite's variable limit for a large
    // filtered set (the Map contract passes all of it), so the lookup chunks.
    const HASH_CHUNK: usize = 500;
    let mut by_hash: HashMap<String, Photo> = HashMap::with_capacity(hashes.len());
    for chunk in hashes.chunks(HASH_CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!("SELECT * FROM photos WHERE hash_sha256 IN ({placeholders})");
        let mut query = sqlx::query_as::<_, Photo>(sqlx::AssertSqlSafe(sql));
        for hash in chunk {
            query = query.bind(hash);
        }
        by_hash.extend(
            query
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(|photo| (photo.hash_sha256.clone(), photo)),
        );
    }

    let mut photos = Vec::with_capacity(by_hash.len());
    for hash in hashes {
        if let Some(photo) = by_hash.remove(hash) {
            photos.push(photo);
        }
    }
    for photo in &mut photos {
        facts.enrich(photo);
    }
    Ok(photos)
}

/// Fetches the filtered rows, optionally ordered and paged by SQL, and
/// enriches them from the file-facts index.
async fn fetch_rows(
    pool: &DbPool,
    facts: &MediaFactsIndex,
    where_clause: &str,
    params: &[String],
    album: Option<i64>,
    order_clause: Option<&str>,
    page: Option<ListingPage>,
) -> Result<Vec<Photo>, Box<dyn std::error::Error>> {
    let mut sql = format!("SELECT * FROM photos{where_clause}");
    if let Some(order_clause) = order_clause {
        sql.push_str(" ORDER BY ");
        sql.push_str(order_clause);
    }
    if page.is_some() {
        sql.push_str(" LIMIT ? OFFSET ?");
    }

    let mut data_query = sqlx::query_as::<_, Photo>(sqlx::AssertSqlSafe(sql));
    for param in params {
        data_query = data_query.bind(param);
    }
    if let Some(album_id) = album {
        data_query = data_query.bind(album_id);
    }
    if let Some(page) = page {
        data_query = data_query.bind(page.limit).bind(page.offset);
    }

    let mut photos = data_query.fetch_all(pool).await?;
    for photo in &mut photos {
        facts.enrich(photo);
    }
    Ok(photos)
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::db::tests::create_test_photo;
    use crate::media_facts::{test_facts, MediaFacts};

    fn photo_with_date(filename: &str, hash: &str, taken_at: &str) -> Photo {
        let mut photo = create_test_photo(filename.to_string(), hash.to_string());
        photo.created_at = DateTime::parse_from_rfc3339(taken_at)
            .unwrap()
            .with_timezone(&Utc);
        photo
    }

    async fn seed(
        pool: &DbPool,
        facts: &MediaFactsIndex,
        photos: &[(&str, &str, &str)],
    ) -> Vec<Photo> {
        let mut rows = Vec::new();
        for (filename, hash, taken_at) in photos {
            let photo = photo_with_date(filename, hash, taken_at);
            facts.set(
                &photo.file_path,
                MediaFacts {
                    taken_at: Some(
                        DateTime::parse_from_rfc3339(taken_at)
                            .unwrap()
                            .with_timezone(&Utc),
                    ),
                    ..MediaFacts::default()
                },
            );
            photo.create(pool).await.unwrap();
            rows.push(photo);
        }
        rows
    }

    #[tokio::test]
    async fn sql_sort_pages_and_counts_the_filtered_set() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let facts = test_facts(&[]);
        seed(
            &pool,
            &facts,
            &[
                ("c.jpg", "c", "2020-03-01T10:00:00Z"),
                ("a.jpg", "a", "2020-01-01T10:00:00Z"),
                ("b.jpg", "b", "2020-02-01T10:00:00Z"),
            ],
        )
        .await;

        let listing = list(
            &pool,
            &facts,
            ListingRequest {
                query: &SearchQuery {
                    q: None,
                    year: None,
                    month: None,
                    to_year: None,
                    to_month: None,
                },
                album: None,
                sort: Some("filename"),
                order: Some("asc"),
                page: Some(ListingPage {
                    limit: 2,
                    offset: 1,
                }),
            },
        )
        .await
        .unwrap();

        assert_eq!(listing.total, 3);
        assert_eq!(
            listing
                .photos
                .iter()
                .map(|photo| photo.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["b.jpg", "c.jpg"]
        );
    }

    #[tokio::test]
    async fn date_sort_filters_the_month_range_before_paging() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let facts = test_facts(&[]);
        seed(
            &pool,
            &facts,
            &[
                ("jan.jpg", "jan", "2020-01-31T10:00:00Z"),
                ("feb.jpg", "feb", "2020-02-10T10:00:00Z"),
                ("mar.jpg", "mar", "2020-03-10T10:00:00Z"),
                ("apr.jpg", "apr", "2020-04-10T10:00:00Z"),
            ],
        )
        .await;

        let listing = list(
            &pool,
            &facts,
            ListingRequest {
                query: &SearchQuery {
                    q: None,
                    year: Some(2020),
                    month: Some(2),
                    to_year: Some(2020),
                    to_month: Some(3),
                },
                album: None,
                sort: Some("date"),
                order: Some("asc"),
                page: Some(ListingPage {
                    limit: 10,
                    offset: 0,
                }),
            },
        )
        .await
        .unwrap();

        assert_eq!(listing.total, 2);
        assert_eq!(
            listing
                .photos
                .iter()
                .map(|photo| photo.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["feb.jpg", "mar.jpg"]
        );
        assert_eq!(
            listing.photos[0].taken_at,
            Some(
                DateTime::parse_from_rfc3339("2020-02-10T10:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc)
            )
        );
    }

    fn unfiltered_query() -> SearchQuery {
        SearchQuery {
            q: None,
            year: None,
            month: None,
            to_year: None,
            to_month: None,
        }
    }

    /// A paginated date-ordered listing must be exactly the unpaginated order
    /// sliced per page, with the same filtered total — for both directions.
    #[tokio::test]
    async fn date_sort_pages_match_the_unpaginated_order() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let facts = test_facts(&[]);
        seed(
            &pool,
            &facts,
            &[
                ("a.jpg", "a", "2020-01-01T10:00:00Z"),
                ("b.jpg", "b", "2020-03-01T10:00:00Z"),
                ("c.jpg", "c", "2020-02-01T10:00:00Z"),
                ("d.jpg", "d", "2021-05-01T10:00:00Z"),
                ("e.jpg", "e", "2019-12-31T10:00:00Z"),
            ],
        )
        .await;
        let query = unfiltered_query();

        for order in [None, Some("asc")] {
            let complete = list(
                &pool,
                &facts,
                ListingRequest {
                    query: &query,
                    album: None,
                    sort: None,
                    order,
                    page: None,
                },
            )
            .await
            .unwrap();
            let expected: Vec<String> = complete
                .photos
                .iter()
                .map(|photo| photo.filename.clone())
                .collect();

            let mut paged = Vec::new();
            for page in 0..3 {
                let listing = list(
                    &pool,
                    &facts,
                    ListingRequest {
                        query: &query,
                        album: None,
                        sort: None,
                        order,
                        page: Some(ListingPage {
                            limit: 2,
                            offset: page * 2,
                        }),
                    },
                )
                .await
                .unwrap();
                assert_eq!(listing.total, complete.total, "order={order:?}");
                paged.extend(listing.photos.into_iter().map(|photo| photo.filename));
            }

            assert_eq!(paged, expected, "order={order:?}");
        }
    }

    /// One page of the unfiltered, date-ordered listing.
    async fn page_of(
        pool: &DbPool,
        facts: &MediaFactsIndex,
        order: Option<&str>,
        offset: i64,
    ) -> Listing {
        let query = unfiltered_query();
        list(
            pool,
            facts,
            ListingRequest {
                query: &query,
                album: None,
                sort: None,
                order,
                page: Some(ListingPage { limit: 10, offset }),
            },
        )
        .await
        .unwrap()
    }

    fn names(listing: &Listing) -> Vec<&str> {
        listing
            .photos
            .iter()
            .map(|photo| photo.filename.as_str())
            .collect()
    }

    /// The unfiltered order is cached across requests, so every way the row set
    /// can change must invalidate it: an insert, a delete and an in-place
    /// re-key (rotation rewrites `hash_sha256`, the path stays).
    #[tokio::test]
    async fn the_cached_order_follows_row_set_changes() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let facts = test_facts(&[]);
        let seeded = seed(
            &pool,
            &facts,
            &[
                ("a.jpg", "a", "2020-01-01T10:00:00Z"),
                ("b.jpg", "b", "2020-02-01T10:00:00Z"),
            ],
        )
        .await;

        let first = page_of(&pool, &facts, None, 0).await;
        assert_eq!(names(&first), vec!["b.jpg", "a.jpg"]);
        assert_eq!(first.total, 2);

        // A scan inserts a newer photo.
        seed(&pool, &facts, &[("c.jpg", "c", "2020-03-01T10:00:00Z")]).await;
        let second = page_of(&pool, &facts, None, 0).await;
        assert_eq!(names(&second), vec!["c.jpg", "b.jpg", "a.jpg"]);
        assert_eq!(second.total, 3);

        // An in-app rotation re-keys a row without changing its path.
        let rotated_hash = format!("{:0<64}", "a-rotated");
        sqlx::query("UPDATE photos SET hash_sha256 = ? WHERE hash_sha256 = ?")
            .bind(&rotated_hash)
            .bind(&seeded[0].hash_sha256)
            .execute(&pool)
            .await
            .unwrap();
        let third = page_of(&pool, &facts, None, 0).await;
        assert_eq!(names(&third), vec!["c.jpg", "b.jpg", "a.jpg"]);
        assert_eq!(third.photos[2].hash_sha256, rotated_hash);

        // A removal.
        sqlx::query("DELETE FROM photos WHERE hash_sha256 = ?")
            .bind(&seeded[1].hash_sha256)
            .execute(&pool)
            .await
            .unwrap();
        let fourth = page_of(&pool, &facts, None, 0).await;
        assert_eq!(names(&fourth), vec!["c.jpg", "a.jpg"]);
        assert_eq!(fourth.total, 2);
    }

    /// A date edit changes the order although no row and no column the DB
    /// stores changed: the facts generation must invalidate the cached order.
    #[tokio::test]
    async fn the_cached_order_follows_a_file_date_change() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let facts = test_facts(&[]);
        let seeded = seed(
            &pool,
            &facts,
            &[
                ("a.jpg", "a", "2020-01-01T10:00:00Z"),
                ("b.jpg", "b", "2020-02-01T10:00:00Z"),
            ],
        )
        .await;

        assert_eq!(
            names(&page_of(&pool, &facts, None, 0).await),
            vec!["b.jpg", "a.jpg"]
        );

        facts.set(
            &seeded[0].file_path,
            MediaFacts {
                taken_at: Some(
                    DateTime::parse_from_rfc3339("2021-01-01T10:00:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
                ..MediaFacts::default()
            },
        );

        assert_eq!(
            names(&page_of(&pool, &facts, None, 0).await),
            vec!["a.jpg", "b.jpg"]
        );
    }

    /// A row the file-facts index does not know is undated, not missing: it
    /// stays in the listing (and in the total) and orders first ascending /
    /// last descending, exactly like a `NULL` date did in SQL.
    #[tokio::test]
    async fn a_row_without_file_facts_stays_in_the_listing() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let facts = test_facts(&[]);
        let seeded = seed(
            &pool,
            &facts,
            &[
                ("old.jpg", "old", "2020-01-01T10:00:00Z"),
                ("unknown.jpg", "unknown", "2020-02-01T10:00:00Z"),
                ("new.jpg", "new", "2020-03-01T10:00:00Z"),
            ],
        )
        .await;
        facts.remove(&seeded[1].file_path);
        let query = unfiltered_query();

        let ascending = list(
            &pool,
            &facts,
            ListingRequest {
                query: &query,
                album: None,
                sort: Some("date"),
                order: Some("asc"),
                page: Some(ListingPage {
                    limit: 10,
                    offset: 0,
                }),
            },
        )
        .await
        .unwrap();
        assert_eq!(ascending.total, 3);
        assert_eq!(
            ascending
                .photos
                .iter()
                .map(|photo| photo.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["unknown.jpg", "old.jpg", "new.jpg"]
        );

        let descending = list(
            &pool,
            &facts,
            ListingRequest {
                query: &query,
                album: None,
                sort: Some("date"),
                order: None,
                page: Some(ListingPage {
                    limit: 2,
                    offset: 0,
                }),
            },
        )
        .await
        .unwrap();
        assert_eq!(descending.total, 3);
        assert_eq!(
            descending
                .photos
                .iter()
                .map(|photo| photo.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["new.jpg", "old.jpg"]
        );
    }

    /// A column sort behind a month filter must still order by that column —
    /// the month filter only decides which rows are listed, never their order.
    #[tokio::test]
    async fn a_column_sort_behind_a_month_filter_pages_by_that_column() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let facts = test_facts(&[]);
        seed(
            &pool,
            &facts,
            &[
                ("a.jpg", "a", "2020-01-01T10:00:00Z"),
                ("c.jpg", "c", "2020-02-01T10:00:00Z"),
                ("b.jpg", "b", "2020-03-01T10:00:00Z"),
                ("old.jpg", "old", "2019-12-01T10:00:00Z"),
            ],
        )
        .await;

        let listing = list(
            &pool,
            &facts,
            ListingRequest {
                query: &SearchQuery {
                    q: None,
                    year: Some(2020),
                    month: None,
                    to_year: None,
                    to_month: None,
                },
                album: None,
                sort: Some("filename"),
                order: Some("asc"),
                page: Some(ListingPage {
                    limit: 2,
                    offset: 1,
                }),
            },
        )
        .await
        .unwrap();

        assert_eq!(listing.total, 3);
        assert_eq!(
            listing
                .photos
                .iter()
                .map(|photo| photo.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["b.jpg", "c.jpg"]
        );
    }

    #[tokio::test]
    async fn album_scope_and_unpaginated_map_listing_agree() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let facts = test_facts(&[]);
        let seeded = seed(
            &pool,
            &facts,
            &[
                ("one.jpg", "one", "2020-01-01T10:00:00Z"),
                ("two.jpg", "two", "2020-02-01T10:00:00Z"),
            ],
        )
        .await;
        let album = crate::albums::create(&pool, "Trip").await.unwrap();
        crate::albums::add_members(&pool, album.id, &[seeded[0].hash_sha256.clone()])
            .await
            .unwrap();

        let listing = list(
            &pool,
            &facts,
            ListingRequest {
                query: &SearchQuery {
                    q: None,
                    year: None,
                    month: None,
                    to_year: None,
                    to_month: None,
                },
                album: Some(album.id),
                sort: Some("date"),
                order: Some("asc"),
                page: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(listing.total, 1);
        assert_eq!(listing.photos[0].filename, "one.jpg");
    }
}
