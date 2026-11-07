//! The one photo listing: filters, ordering and pagination.
//!
//! Every sort is a real column — the capture date included, as the scan-owned
//! derived copy on the row — so SQLite filters, orders and pages the whole
//! listing and this module only assembles the statement and the count. Callers
//! hand this module a request instead of re-sequencing that ladder.

use crate::db::{build_order_clause, build_search_where, DbPool, Photo};
use crate::db_types::SearchQuery;

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

/// Lists photos for the given request. A paginated request reports the filtered
/// total from `COUNT(*)`; the unpaginated Map contract reports what it returns.
pub async fn list(
    pool: &DbPool,
    request: ListingRequest<'_>,
) -> Result<Listing, Box<dyn std::error::Error>> {
    let (mut where_clause, params) = build_search_where(request.query);
    if request.album.is_some() {
        where_clause.push_str(
            " AND hash_sha256 IN (SELECT photo_hash FROM album_members WHERE album_id = ?)",
        );
    }

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
    let mut sql = format!("SELECT * FROM photos{where_clause} ORDER BY {order_clause}");
    if request.page.is_some() {
        sql.push_str(" LIMIT ? OFFSET ?");
    }

    let mut data_query = sqlx::query_as::<_, Photo>(sqlx::AssertSqlSafe(sql));
    for param in &params {
        data_query = data_query.bind(param);
    }
    if let Some(album_id) = request.album {
        data_query = data_query.bind(album_id);
    }
    if let Some(page) = request.page {
        data_query = data_query.bind(page.limit).bind(page.offset);
    }

    let photos = data_query.fetch_all(pool).await?;
    let total = request.page.map_or(photos.len(), |_| total);

    Ok(Listing { photos, total })
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::db::tests::create_test_photo;

    fn photo_with_date(filename: &str, hash: &str, taken_at: &str) -> Photo {
        let mut photo = create_test_photo(filename.to_string(), hash.to_string());
        photo.taken_at = Some(
            DateTime::parse_from_rfc3339(taken_at)
                .unwrap()
                .with_timezone(&Utc),
        );
        photo
    }

    async fn seed(pool: &DbPool, photos: &[(&str, &str, &str)]) -> Vec<Photo> {
        let mut rows = Vec::new();
        for (filename, hash, taken_at) in photos {
            let photo = photo_with_date(filename, hash, taken_at);
            photo.create(pool).await.unwrap();
            rows.push(photo);
        }
        rows
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

    /// One page of the unfiltered, date-ordered listing.
    async fn page_of(pool: &DbPool, order: Option<&str>, offset: i64) -> Listing {
        let query = unfiltered_query();
        list(
            pool,
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

    /// A paginated date-ordered listing is the filtered set sliced per page,
    /// with the same total — for both directions.
    #[tokio::test]
    async fn date_sort_pages_match_the_unpaginated_order() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        seed(
            &pool,
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

    /// An undated row is not missing: it stays in the listing and its total and
    /// orders first ascending / last descending — SQLite's `NULL` contract,
    /// the same position the removed in-memory comparator gave it.
    #[tokio::test]
    async fn an_undated_row_stays_in_the_listing() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let mut unknown = create_test_photo("unknown.jpg".to_string(), "unknown".to_string());
        unknown.taken_at = None;
        unknown.create(&pool).await.unwrap();
        seed(
            &pool,
            &[
                ("old.jpg", "old", "2020-01-01T10:00:00Z"),
                ("new.jpg", "new", "2020-03-01T10:00:00Z"),
            ],
        )
        .await;

        let ascending = page_of(&pool, Some("asc"), 0).await;
        assert_eq!(ascending.total, 3);
        assert_eq!(names(&ascending), vec!["unknown.jpg", "old.jpg", "new.jpg"]);

        let descending = page_of(&pool, None, 0).await;
        assert_eq!(descending.total, 3);
        assert_eq!(
            names(&descending),
            vec!["new.jpg", "old.jpg", "unknown.jpg"]
        );
    }

    /// Both month bounds are inclusive and an undated row never matches.
    #[tokio::test]
    async fn date_sort_filters_the_month_range_before_paging() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        seed(
            &pool,
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
        assert_eq!(names(&listing), vec!["feb.jpg", "mar.jpg"]);
        assert_eq!(
            listing.photos[0].taken_at,
            Some(
                DateTime::parse_from_rfc3339("2020-02-10T10:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc)
            )
        );
    }

    /// A column sort behind a month filter orders by that column, not by date.
    #[tokio::test]
    async fn a_column_sort_behind_a_month_filter_pages_by_that_column() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        seed(
            &pool,
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
        assert_eq!(names(&listing), vec!["b.jpg", "c.jpg"]);
    }

    /// Album scope and the unpaginated Map contract agree on the same order.
    #[tokio::test]
    async fn album_scope_and_unpaginated_map_listing_agree() {
        let pool = crate::db::create_test_db_pool().await.unwrap();
        let seeded = seed(
            &pool,
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
            ListingRequest {
                query: &unfiltered_query(),
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
