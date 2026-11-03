//! The one photo listing: filters, SQL/in-memory strategy, ordering and
//! pagination.
//!
//! The database stores no capture date, so only an explicit column sort can be
//! ordered and paged by SQLite. Every date-ordered listing (the default, the
//! grid, the map, an album) fetches the filtered rows, enriches them from the
//! file-facts index, filters the month range in Rust and sorts with
//! [`sort_photos`]. Callers hand this module a request instead of re-sequencing
//! that ladder.

use crate::db::{
    build_order_clause, build_search_where, is_sql_sort, month_range_bounds, photo_in_month_range,
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

    // Date-ordered or month-filtered listing: the date lives in the files, so
    // the filtered set is fetched and the index does the rest.
    let mut photos = fetch_rows(
        pool,
        facts,
        &where_clause,
        &params,
        request.album,
        None,
        None,
    )
    .await?;
    if let Some(bounds) = &bounds {
        photos.retain(|photo| photo_in_month_range(photo, bounds));
    }
    sort_photos(&mut photos, request.sort, request.order);

    let total = photos.len();
    let photos = match request.page {
        Some(page) => paginate(photos, page.offset, page.limit),
        None => photos,
    };

    Ok(Listing { photos, total })
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

fn paginate(photos: Vec<Photo>, offset: i64, limit: i64) -> Vec<Photo> {
    let offset = usize::try_from(offset).unwrap_or(0);
    let limit = usize::try_from(limit).unwrap_or(0);
    photos.into_iter().skip(offset).take(limit).collect()
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
