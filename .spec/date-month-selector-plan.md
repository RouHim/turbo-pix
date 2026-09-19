# Desktop Date/Month Selector (Zoomable Timeline Overview) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the desktop year-pill rail with a single zoomable overview of the library's whole date span — subtle density profile, adaptive collision-free labels, single-period (year / year+month) and month-granular range selection — while mobile dropdowns, the timeline data source, and everything else stay as they are.

**Architecture:** The selection model becomes one inclusive month-index range (`index = year * 12 + month - 1`) everywhere: URL → `route` store → `api.getPhotos` → `/api/photos` SQL. The route keeps today's `year`/`month` as the start bound and adds `to_year`/`to_month` for the end bound (absent = single period), so saved searches, existing links and the mobile dropdowns keep their shape. Three pure modules (`timeline.js` model, `timelineLayout.js` view geometry, `timelineRoute.js` route mapping) carry all logic and are unit-tested with `node --test`; a new `TimelineSelector.svelte` renders ruler lane + density lane over them and owns the view state; `TimelineSlider.svelte` stays the data container (fetch, error/empty paths, label, clear control, mobile dropdowns).

**Tech Stack:** Rust/warp/sqlx (backend range filter + saved-search columns), Svelte 5 runes, `svelte-i18n`, `node --test` unit tests, Playwright E2E against the real backend.

**Spec:** `.spec/date-month-selector.md` (this plan supersedes `.spec/new-timeline-component-for-the-desktop-view-plan.md`, whose year-rail design is what the spec discards).

## Global Constraints

- Breaking changes are allowed; no legacy support, no migration scripts, no backward compatibility (project rule). Database and cache may be recreated.
- Desktop scope only: the existing `@media (width <= 768px)` split stays exact; the mobile year/month dropdowns keep their behaviour and exactly one experience is visible at any width.
- Timeline data source unchanged: `GET /api/photos/timeline` → `{ min_date, max_date, density: [{ year, month, count }] }`; zero-count months are **not** in the payload, so the client zero-fills from `minIndex` to `maxIndex`.
- Selection is one inclusive month-granular range. Smallest selectable period is one month. `month` absent means the whole year (January for a start bound, December for an end bound) in **both** bounds.
- Single source of truth is the `route` store (`frontend/src/lib/router.svelte.js`); components render from it and write through `pushState`/`replaceState` only. A month never exists without its year.
- `$state` fields MUST use `let`; arrow functions; `const` over `let`; template literals; no Svelte 4 stores.
- `en.json` and `de.json` MUST stay structurally identical — every new key lands in BOTH. `$t` `values` go INSIDE the options object. New `` $t(`…${expr}…`) `` template keys need an `enums` entry in `tests/i18n-integrity.test.js`; use literal keys instead.
- Responsive overrides MUST live in the component's scoped `<style>`, never in `app.css`; keep `build.cssMinify: false`.
- Route-restore/adjustment `$effect`s MUST read their reactive dependencies BEFORE any early-return guard, or the effect unsubscribes permanently (AGENTS.md #2).
- Icons: feather only, registered in `frontend/src/components/Icon.svelte` (`plus`, `minus`, `maximize`, `x`, `calendar` are registered; there is no `zoom-in`/`zoom-out` icon). Size via `:global(svg)`, never `:global(.feather)`.
- CSS uses `app.css` design tokens only (`--space-*`, `--font-*`, `--radius-*`, `--primary-color`, `--primary-dark`, `--surface-color`, `--background-secondary`, `--divider-color`, `--text-secondary`, `--shadow-light`, `--transition-fast`); no hardcoded colours/spacings. Focus ring idiom: `outline: none; box-shadow: 0 0 0 2px var(--surface-color), 0 0 0 4px var(--primary-color);`.
- Rust: `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `sqlx::AssertSqlSafe` for dynamic SQL, no `query!` macros in this repo. Functions stay ≤ 7 arguments (clippy `too_many_arguments`).
- Frontend gates: `npm run format:check`, `npm run lint`, `npm run test:i18n`, `npm run test:unit`; then `npm run build` BEFORE `cargo build --bin turbo-pix` (build.rs embeds `dist/`).
- E2E: `npx playwright test <file>` starts the real backend through `tests/e2e/setup/global-setup.js`; sequential (`workers: 1`). Consecutive invocations race the port — let the previous teardown settle before re-running (AGENTS.md #10).
- Manual smoke test: `nohup cargo run &`, poll `curl --retry 5 --retry-delay 2 http://localhost:18473/health`, test at `http://localhost:18473`, kill the process afterwards.

## Review Focus

Failure modes the spec implies but no single task's tests fully exercise — each has a test added to the owning task:

1. **Drag ends on a column → click fires afterwards.** A pointer drag that finishes over a column must not also activate it (drill-in + range in one gesture). Owning task: Task 8 (E2E drag asserts the route is exactly the dragged range).
2. **Wheel/trackpad over an interactive column.** The wheel must zoom/pan regardless of whether the pointer is over a column button, not scroll the page. Owning task: Task 7 (E2E zooms with the cursor inside a column).
3. **Resize across the 768px breakpoint while zoomed and selected.** The lane can become 0px wide (hidden `display: none`); layout must skip instead of dividing by zero, labels must re-adapt, and the selection must stay visible on return. Owning task: Task 7 (E2E resizes mobile→desktop with a selection, plus a unit test for `width === 0`).
4. **Restored selection that no longer overlaps the library** (`?year=1900&to_year=1901` against a 2020s library) and a partially overlapping one. Expected: no overlap → cleared to unfiltered and the URL rewritten; partial → narrowed to the overlap; both keep the grid empty of stale results. Owning task: Tasks 5 and 6.
5. **Single-month library / span of one month.** Zoom limits collapse to one value, decade and year selection still produce that month, no `NaN` scale. Owning task: Task 4 (unit) plus Task 7 (E2E deep-link with one populated month is impossible with the fixture, so the unit test is the pin).
6. **Two-pointer pinch.** The zoom math is unit-tested, the two-pointer plumbing is not — Playwright cannot synthesise a real pinch. Owning task: Task 7 (unit test `zoomView` from pinch-equivalent factors) and the manual smoke test in Task 10.

---

## File Structure

**Backend**
- Modify `src/db_types.rs` — `SearchQuery` gains `to_year`, `to_month`.
- Modify `src/handlers_photo.rs` — `PhotoQuery` gains `to_year`, `to_month`; `fetch_photos` dispatch + mapping.
- Modify `src/db.rs` — `month_range_bounds()` predicate builder replaces the `strftime` equality filters; tests.
- Create `migrations/20250101000009_saved_search_range.sql` — `to_year`/`to_month` columns + widened unique index.
- Modify `src/saved_searches.rs` — `SavedSearch` fields, `SavedSearchFilter` struct, rewritten `create`.
- Modify `src/handlers_saved_searches.rs` — request fields, validation, tests.

**Frontend (pure, unit-tested)**
- Rewrite `frontend/src/lib/timeline.js` — month-index math, density model, selection model, label formatting.
- Create `frontend/src/lib/timelineLayout.js` — scale/pan/zoom geometry, column building, label placement, hit-testing.
- Create `frontend/src/lib/timelineRoute.js` — route-filter ⇄ selection mapping and canonicalisation.
- Create `tests/timeline-model.test.js`, `tests/timeline-layout.test.js`, `tests/timeline-route.test.js`; delete `tests/timeline-aggregates.test.js`.

**Frontend (wiring)**
- Modify `frontend/src/lib/router.svelte.js` — `to_year`/`to_month` params, normalisation via `timelineRoute.js`.
- Modify `frontend/src/lib/api.js` — `toYear`/`toMonth` → `to_year`/`to_month`; delete dead `dateFrom`/`dateTo`.
- Modify `frontend/src/components/PhotoGrid.svelte` — range filters, refetch deps, dedupe signature.
- Modify `frontend/src/components/SearchBar.svelte` — save range with a saved search; default-name/`canSave`.
- Modify `frontend/src/components/Sidebar.svelte` — restore/compare saved searches with the range.
- Modify `frontend/src/components/AlbumsView.svelte` — clear the range when opening an album.
- Modify `frontend/src/i18n/en.json`, `frontend/src/i18n/de.json` — new selector keys.
- Create `frontend/src/components/TimelineSelector.svelte` — the zoomable overview (view state, gestures, keyboard).
- Modify `frontend/src/components/TimelineSlider.svelte` — container rewrite: selector on desktop, mobile dropdowns keep working, label + clear control.
- Modify `package.json`, `.github/workflows/ci.yml` — `test:unit` script wired into CI.
- Modify `tests/e2e/setup/global-setup.js` — decade-spanning legacy seed photos.
- Modify `tests/e2e/specs/timeline.e2e.spec.js`, `tests/e2e/specs/timeline-a11y.e2e.spec.js`, `tests/e2e/specs/url-routing.e2e.spec.js`, `tests/e2e/specs/saved-searches.e2e.spec.js` — retargeted and extended.
- Modify `AGENTS.md` — fold the new learnings into the capped list.

---

### Task 1: Backend month-range filter on `GET /api/photos`

**Files:**
- Modify: `src/db_types.rs:5-9` (`SearchQuery`)
- Modify: `src/handlers_photo.rs:31-39` (`PhotoQuery`), `:51-64` (`fetch_photos`)
- Modify: `src/db.rs:889-897` (year/month predicates), `:1216-1222` (`create_search_query`), plus new tests in the `mod tests` block
- Test: same files (`cargo test`)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: query params `year`, `month`, `to_year`, `to_month` on `GET /api/photos`. Semantics: `year` (+optional `month`) is the **start** bound; `to_year` (+optional `to_month`) is the **end** bound; absent `to_year` makes the filter a single period (`year` or `year+month`); absent `month` means January, absent `to_month` means December; bounds are inclusive. `month` without `year` filters nothing (the frontend never emits it).

- [ ] **Step 1: Write the failing DB-level tests**

Append to `mod tests` in `src/db.rs` (after `test_search_photos_by_city`, which shows the fixture pattern):

```rust
    fn dated_photo(hash: &str, filename: &str, taken_at: &str) -> Photo {
        create_test_photo_with_date(
            hash,
            filename,
            DateTime::parse_from_rfc3339(taken_at)
                .unwrap()
                .with_timezone(&Utc),
        )
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
            dated_photo(&hash.repeat(64), filename, taken_at)
                .create(&pool)
                .await
                .unwrap();
        }

        // Single month (start bound only).
        let query = date_filter_query(Some(2012), Some(3), None, None);
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos[0].filename, "mar2012.jpg");

        // Whole year (no month) stays a single-year filter.
        let query = date_filter_query(Some(2012), None, None, None);
        let (_, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 3);

        // Month-granular range spanning years, both bounds inclusive.
        let query = date_filter_query(Some(2012), Some(3), Some(2015), Some(8));
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 4);
        let names: Vec<&str> = photos.iter().map(|p| p.filename.as_str()).collect();
        assert!(!names.contains(&"feb2012.jpg"));
        assert!(!names.contains(&"sep2015.jpg"));

        // Year-precision end bound stops at December of that year.
        let query = date_filter_query(Some(2012), Some(3), Some(2012), None);
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(photos[0].taken_at.unwrap().year(), 2012);

        // Reversed bounds match nothing (the router normalises before it gets here).
        let query = date_filter_query(Some(2015), Some(8), Some(2012), Some(3));
        let (_, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 0);

        // A month bound without a year filters nothing.
        let query = date_filter_query(None, Some(3), None, None);
        let (_, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 6);
    }

    #[tokio::test]
    async fn test_search_photos_range_ignores_null_taken_at() {
        let pool = create_test_db_pool().await.unwrap();
        let mut undated = create_test_photo("undated.jpg".to_string(), "undated".to_string());
        undated.taken_at = None;
        undated.create(&pool).await.unwrap();
        dated_photo(&"c".repeat(64), "mar2012.jpg", "2012-03-15T10:00:00Z")
            .create(&pool)
            .await
            .unwrap();

        let query = date_filter_query(Some(2012), Some(1), Some(2012), Some(12));
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos[0].filename, "mar2012.jpg");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib search_photos_filters_inclusive_month_range`
Expected: compile error — `SearchQuery` has no field `to_year` (the tests cannot even build yet, which is the correct red state).

- [ ] **Step 3: Add the fields to `SearchQuery` and implement the predicate builder**

In `src/db_types.rs`, replace the `SearchQuery` struct with:

```rust
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub to_year: Option<i32>,
    pub to_month: Option<i32>,
}
```

In `src/db.rs`, add this free function directly above `impl Photo` (next to `build_order_clause`):

```rust
/// Inclusive month-granular filter bounds as zero-padded `YYYY-MM` strings.
///
/// `year`/`month` is the start bound, `to_year`/`to_month` the end bound; an
/// absent end bound makes the filter a single period. A missing month means
/// January for a start bound and December for an end bound, so `year=2012`
/// still means "all of 2012" and `from=2012-03,to=2015` stops at December
/// 2015. Out-of-range months are formatted as-is, which makes the string
/// comparison match nothing — the same result the equality filter produced.
fn month_range_bounds(
    year: Option<i32>,
    month: Option<i32>,
    to_year: Option<i32>,
    to_month: Option<i32>,
) -> Option<(String, String)> {
    let year = year?;
    let from = format!("{:04}-{:02}", year, month.unwrap_or(1));
    let to = match to_year {
        Some(to_year) => format!("{:04}-{:02}", to_year, to_month.unwrap_or(12)),
        None => format!("{:04}-{:02}", year, month.unwrap_or(12)),
    };
    Some((from, to))
}
```

In `src/db.rs`, replace the `if let Some(year) = query.year { … } if let Some(month) = query.month { … }` block (lines 889-897) with:

```rust
        // Month-granular inclusive range. `strftime('%Y-%m', …)` is compared
        // lexicographically: zero-padded `YYYY-MM` sorts chronologically, and
        // NULL `taken_at` fails the comparison exactly like the old equality
        // filter did.
        if let Some((from, to)) = month_range_bounds(
            query.year,
            query.month,
            query.to_year,
            query.to_month,
        ) {
            where_clause
                .push_str(" AND strftime('%Y-%m', taken_at) >= ? AND strftime('%Y-%m', taken_at) <= ?");
            params.push(from);
            params.push(to);
        }
```

Keep `params` order intact (count and data queries replay the same vector).

- [ ] **Step 4: Accept the new params on the handler and route them to the search path**

In `src/handlers_photo.rs`, extend `PhotoQuery`:

```rust
#[derive(Debug, Deserialize)]
pub struct PhotoQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub q: Option<String>,
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub to_year: Option<i32>,
    pub to_month: Option<i32>,
}
```

and `fetch_photos` (the dispatch condition and the `SearchQuery` mapping must both learn the new fields, or the range silently lands on the unfiltered `list_with_pagination` path):

```rust
    if query.q.is_some()
        || query.year.is_some()
        || query.month.is_some()
        || query.to_year.is_some()
        || query.to_month.is_some()
    {
        let search_query = SearchQuery {
            q: query.q.clone(),
            year: query.year,
            month: query.month,
            to_year: query.to_year,
            to_month: query.to_month,
        };
```

Keep the route registration and its literal-before-parameterized ordering untouched (`src/handlers_photo.rs:1075-1095`, guarded by `test_timeline_route_not_shadowed`).

- [ ] **Step 5: Update the shared test helper**

In `src/db.rs` replace `create_search_query` with:

```rust
    fn create_search_query(query: &str) -> SearchQuery {
        SearchQuery {
            q: Some(query.to_string()),
            year: None,
            month: None,
            to_year: None,
            to_month: None,
        }
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib search_photos`
Expected: PASS — the two new tests plus every pre-existing `search_photos` test.

- [ ] **Step 7: Add the route-level test**

In `src/handlers_photo.rs` add, next to `test_timeline_route_not_shadowed` (which shows the `build_test_routes` + `warp::test::request()` pattern):

```rust
    #[tokio::test]
    async fn test_list_photos_month_range_filter() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().to_path_buf());

        for (hash, filename, taken_at) in [
            ("a", "mar2012.jpg", "2012-03-15T10:00:00Z"),
            ("b", "aug2015.jpg", "2015-08-31T23:30:00Z"),
            ("c", "sep2015.jpg", "2015-09-01T00:00:00Z"),
        ] {
            create_dated_photo_row(&db_pool, &temp_dir, &hash.repeat(64), filename, taken_at).await;
        }

        let response = warp::test::request()
            .path("/api/photos?year=2012&month=3&to_year=2015&to_month=8")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["total"], 2, "range must be inclusive on both bounds");
        let filenames: Vec<&str> = body["photos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["filename"].as_str().unwrap())
            .collect();
        assert!(filenames.contains(&"mar2012.jpg"));
        assert!(filenames.contains(&"aug2015.jpg"));
        assert!(!filenames.contains(&"sep2015.jpg"));
    }
```

Add the rows through a dated variant of the existing `create_photo_row` helper (which hardcodes `taken_at: 2020-01-01`) — put this next to it in the same test module:

```rust
    /// Same fixture as `create_photo_row` but with an explicit `taken_at`.
    async fn create_dated_photo_row(
        db_pool: &DbPool,
        temp_dir: &TempDir,
        hash: &str,
        filename: &str,
        taken_at: &str,
    ) {
        let test_image = Path::new("test-data/IMG_9377.jpg");
        let temp_image = temp_dir.path().join(filename);
        fs::copy(test_image, &temp_image).expect("Failed to copy test image");

        let photo = Photo {
            hash_sha256: hash.to_string(),
            file_path: temp_image.to_str().unwrap().to_string(),
            filename: filename.to_string(),
            file_size: 12345,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: Some(
                DateTime::parse_from_rfc3339(taken_at)
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            width: Some(800),
            height: Some(600),
            orientation: Some(1),
            duration: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: None,
            is_favorite: Some(false),
            semantic_vector_indexed: Some(false),
            metadata: json!({}),
            date_modified: Utc::now(),
            date_indexed: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        photo
            .create(db_pool)
            .await
            .expect("Failed to create test photo");
    }
```

Check the imports the module already has (`DateTime`, `Utc`, `json!`, `fs`, `Path`, `TempDir`) and add whatever is missing to the test module's `use` block.

- [ ] **Step 8: Run the backend gates**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all green, no warnings.

- [ ] **Step 9: Commit**

```bash
git add src/db_types.rs src/db.rs src/handlers_photo.rs
git commit -m "feat(api): filter photos by inclusive month range"
```

---

### Task 2: Saved searches persist the range

**Files:**
- Create: `migrations/20250101000009_saved_search_range.sql`
- Modify: `src/saved_searches.rs:6-16` (struct), `:49` (`SELECT_COLUMNS`), `:60-120` (`create`), tests
- Modify: `src/handlers_saved_searches.rs:20-27` (request), `:34-42` (`CreateFields`), `:79-110` (`validate_create`), `:115-125` (`create_saved_search`), tests

**Interfaces:**
- Consumes: the `to_year`/`to_month` naming from Task 1 (same payload field names on the wire).
- Produces: `POST/GET /api/saved-searches` accept and return `to_year`/`to_month` (nullable ints); `saved_searches::SavedSearchFilter { year, month, to_year, to_month }` is the argument bundle for `create`.

- [ ] **Step 1: Write the failing handler tests**

In `src/handlers_saved_searches.rs`, extend the existing validation tests (the module already has `test_create_month_without_year_returns_400`):

```rust
    #[tokio::test]
    async fn test_create_rejects_invalid_range_bounds() {
        for (to_year, to_month) in [(Some(2012), Some(13)), (Some(2012), Some(0)), (None, Some(3))] {
            let request = CreateSavedSearchRequest {
                name: "Range".to_string(),
                query: None,
                view: "all".to_string(),
                sort: "date_desc".to_string(),
                year: Some(2015),
                month: Some(8),
                to_year,
                to_month,
            };
            let result = validate_create(&request);
            assert!(result.is_err(), "({to_year:?}, {to_month:?}) must be rejected");
        }
    }

    #[tokio::test]
    async fn test_create_stores_range_bounds() {
        let pool = create_in_memory_pool().await.unwrap();
        let row = saved_searches::create(
            &pool,
            "Summer",
            None,
            "all",
            "date_desc",
            &saved_searches::SavedSearchFilter {
                year: Some(2012),
                month: Some(3),
                to_year: Some(2015),
                to_month: Some(8),
            },
        )
        .await
        .unwrap();

        assert_eq!(row.year, Some(2012));
        assert_eq!(row.month, Some(3));
        assert_eq!(row.to_year, Some(2015));
        assert_eq!(row.to_month, Some(8));

        // A different range is a different saved search, not a duplicate.
        let other = saved_searches::create(
            &pool,
            "Autumn",
            None,
            "all",
            "date_desc",
            &saved_searches::SavedSearchFilter {
                year: Some(2012),
                month: Some(3),
                to_year: Some(2015),
                to_month: Some(9),
            },
        )
        .await;
        assert!(other.is_ok(), "distinct ranges must not collide on the unique index");
    }
```

Use the module's existing pool helper (`create_in_memory_pool` from `crate::db_pool`, as the other tests in this file do).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test saved_search`
Expected: compile errors — `CreateSavedSearchRequest` has no `to_year`, `saved_searches::create` has no 6th parameter of type `&SavedSearchFilter`.

- [ ] **Step 3: Add the migration**

Create `migrations/20250101000009_saved_search_range.sql`:

```sql
-- Range bounds for saved searches: `year`/`month` are the start bound,
-- `to_year`/`to_month` the inclusive end bound (NULL = single period).
ALTER TABLE saved_searches ADD COLUMN to_year INTEGER;
ALTER TABLE saved_searches ADD COLUMN to_month INTEGER;

-- State identity now includes the end bound; a range is a distinct entry from
-- its start-period twin. COALESCE so NULL bounds participate in uniqueness.
DROP INDEX IF EXISTS idx_saved_searches_state;
CREATE UNIQUE INDEX IF NOT EXISTS idx_saved_searches_state
    ON saved_searches (
        COALESCE(query, ''),
        view,
        sort,
        COALESCE(year, 0),
        COALESCE(month, 0),
        COALESCE(to_year, 0),
        COALESCE(to_month, 0)
    );
```

Never edit `20250101000008_create_saved_searches_table.sql` — applied migrations are checksummed.

- [ ] **Step 4: Extend the Rust model**

In `src/saved_searches.rs`:

```rust
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
```

Update `FromRow` mapping and `SELECT_COLUMNS` to include the two columns, then replace `create` with the 6-argument form (clippy rejects 9 arguments):

```rust
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
```

Update every existing `create(...)` call in this file's tests to the struct form (e.g. `&SavedSearchFilter { year: None, month: None, to_year: None, to_month: None }`).

- [ ] **Step 5: Extend validation and the handler**

In `src/handlers_saved_searches.rs`:

```rust
#[derive(Debug, Deserialize)]
pub struct CreateSavedSearchRequest {
    pub name: String,
    pub query: Option<String>,
    pub view: String,
    pub sort: String,
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub to_year: Option<i64>,
    pub to_month: Option<i64>,
}

type CreateFields = (String, Option<String>, String, String, SavedSearchFilter);
```

`validate_create` mirrors the start-bound rules for the end bound and swaps reversed bounds into ascending order (the same rule the router applies):

```rust
fn validate_create(req: &CreateSavedSearchRequest) -> Result<CreateFields, ValidationError> {
    let name = validate_name(&req.name)?;
    let query = req
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_string);

    if !VALID_VIEWS.contains(&req.view.as_str()) {
        return Err(ValidationError { message: "Invalid view".to_string() });
    }
    if !VALID_SORTS.contains(&req.sort.as_str()) {
        return Err(ValidationError { message: "Invalid sort".to_string() });
    }
    if req.year.is_some_and(|y| y < 1) {
        return Err(ValidationError { message: "Invalid year".to_string() });
    }
    // Mirrors router normalizeState: a month requires a year.
    if req.month.is_some_and(|m| !(1..=12).contains(&m))
        || (req.month.is_some() && req.year.is_none())
    {
        return Err(ValidationError { message: "Invalid month".to_string() });
    }
    if req.to_year.is_some_and(|y| y < 1) {
        return Err(ValidationError { message: "Invalid end year".to_string() });
    }
    if req.to_month.is_some_and(|m| !(1..=12).contains(&m))
        || (req.to_month.is_some() && req.to_year.is_none())
    {
        return Err(ValidationError { message: "Invalid end month".to_string() });
    }

    let mut filter = SavedSearchFilter {
        year: req.year,
        month: req.month,
        to_year: req.to_year,
        to_month: req.to_month,
    };
    if let (Some(year), Some(to_year)) = (filter.year, filter.to_year) {
        let start = year * 12 + filter.month.unwrap_or(1) - 1;
        let end = to_year * 12 + filter.to_month.unwrap_or(12) - 1;
        if end < start {
            filter = SavedSearchFilter {
                year: Some(to_year),
                month: filter.to_month,
                to_year: Some(year),
                to_month: filter.month,
            };
        }
    }

    Ok((name, query, req.view.clone(), req.sort.clone(), filter))
}
```

`create_saved_search` passes `&filter` to `saved_searches::create`.

- [ ] **Step 6: Run the backend gates**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: PASS. The migration is new, so an existing dev DB upgrades cleanly on next start.

- [ ] **Step 7: Commit**

```bash
git add migrations/20250101000009_saved_search_range.sql src/saved_searches.rs src/handlers_saved_searches.rs
git commit -m "feat(saved-searches): persist inclusive date-range bounds"
```

---

### Task 3: Pure timeline model (`frontend/src/lib/timeline.js`)

**Files:**
- Rewrite: `frontend/src/lib/timeline.js`
- Create: `tests/timeline-model.test.js`
- Delete: `tests/timeline-aggregates.test.js`
- Modify: `frontend/src/components/TimelineSlider.svelte:20-29` (keep the legacy rail compiling until Task 7)
- Modify: `package.json` (add `test:unit`), `.github/workflows/ci.yml` (run it in `lint-format`)

**Interfaces:**
- Consumes: density rows `{ year, month, count }` from `GET /api/photos/timeline`.
- Produces:
  - `toMonthIndex(year, month) => number`, `fromMonthIndex(index) => { year, month }`
  - `buildTimelineModel(density) => { minIndex, maxIndex, length, counts: Int32Array, prefix: Float64Array, total, years: number[] }` (years descending, populated years only)
  - `countInRange(model, startIndex, endIndex) => number` (clamped, 0 outside)
  - `normalizeSelection(a, b) => { startIndex, endIndex } | null`, `selectionEquals(a, b) => boolean`
  - `clampSelectionToModel(selection, model) => selection | null` (null when no overlap)
  - `clampIndexToModel(model, index) => number`
  - `formatSelectionLabel(selection, format) => string`, `formatPeriodName(index, monthName) => string`
  - `MONTHS_PER_YEAR = 12`, `MONTHS_PER_DECADE = 120`
  - `format` shape: `{ allDates, rangeTemplate(start, end), monthName(month) }`
  - Column labels (decade/year/month) live in `timelineLayout.js` (Task 4) — this module holds no column-label rule.

- [ ] **Step 1: Write the failing unit tests**

Create `tests/timeline-model.test.js`:

```js
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  MONTHS_PER_YEAR,
  MONTHS_PER_DECADE,
  buildTimelineModel,
  clampIndexToModel,
  clampSelectionToModel,
  countInRange,
  formatPeriodName,
  formatSelectionLabel,
  fromMonthIndex,
  normalizeSelection,
  selectionEquals,
  toMonthIndex,
} from '../frontend/src/lib/timeline.js';

const format = {
  allDates: 'All Dates',
  rangeTemplate: (start, end) => `${start} – ${end}`,
  monthName: (month) =>
    [
      'January', 'February', 'March', 'April', 'May', 'June',
      'July', 'August', 'September', 'October', 'November', 'December',
    ][month - 1],
};

test('month index round-trips', () => {
  assert.equal(toMonthIndex(1998, 3), 1998 * 12 + 2);
  assert.deepEqual(fromMonthIndex(toMonthIndex(1998, 3)), { year: 1998, month: 3 });
  assert.equal(MONTHS_PER_YEAR, 12);
  assert.equal(MONTHS_PER_DECADE, 120);
});

test('model spans the populated range and zero-fills the gaps', () => {
  const model = buildTimelineModel([
    { year: 2012, month: 3, count: 4 },
    { year: 2015, month: 8, count: 2 },
    { year: 2015, month: 8, count: 1 },
  ]);
  assert.equal(model.minIndex, toMonthIndex(2012, 3));
  assert.equal(model.maxIndex, toMonthIndex(2015, 8));
  assert.equal(model.length, model.maxIndex - model.minIndex + 1);
  assert.equal(model.total, 7);
  assert.equal(countInRange(model, toMonthIndex(2012, 3), toMonthIndex(2012, 3)), 4);
  assert.equal(countInRange(model, model.minIndex, model.maxIndex), 7);
  assert.equal(countInRange(model, toMonthIndex(2013, 1), toMonthIndex(2015, 7)), 0);
  assert.deepEqual(model.years, [2015, 2012]);
});

test('empty density yields an empty model', () => {
  const model = buildTimelineModel([]);
  assert.equal(model.total, 0);
  assert.deepEqual(model.years, []);
  assert.equal(countInRange(model, 0, 100), 0);
});

test('normalizeSelection orders bounds and preserves null', () => {
  assert.deepEqual(normalizeSelection(5, 2), { startIndex: 2, endIndex: 5 });
  assert.deepEqual(normalizeSelection(2, 5), { startIndex: 2, endIndex: 5 });
  assert.equal(normalizeSelection(null, 5), null);
  assert.ok(selectionEquals({ startIndex: 2, endIndex: 5 }, { startIndex: 2, endIndex: 5 }));
  assert.ok(!selectionEquals({ startIndex: 2, endIndex: 5 }, { startIndex: 2, endIndex: 6 }));
  assert.ok(selectionEquals(null, null));
});

test('clampSelectionToModel narrows to the overlap and clears without one', () => {
  const model = buildTimelineModel([
    { year: 2012, month: 3, count: 1 },
    { year: 2015, month: 8, count: 1 },
  ]);
  assert.deepEqual(
    clampSelectionToModel({ startIndex: toMonthIndex(1900, 1), endIndex: toMonthIndex(2013, 5) }, model),
    { startIndex: model.minIndex, endIndex: toMonthIndex(2013, 5) }
  );
  assert.equal(
    clampSelectionToModel({ startIndex: toMonthIndex(1900, 1), endIndex: toMonthIndex(1901, 5) }, model),
    null
  );
  assert.equal(clampSelectionToModel(null, model), null);
  assert.equal(clampIndexToModel(model, toMonthIndex(1900, 1)), model.minIndex);
  assert.equal(clampIndexToModel(model, toMonthIndex(2100, 1)), model.maxIndex);
});

test('selection labels name the period or both bounds', () => {
  const march2012 = { startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2012, 3) };
  assert.equal(formatSelectionLabel(null, format), 'All Dates');
  assert.equal(formatSelectionLabel(march2012, format), 'March 2012');

  const year2012 = { startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2012, 12) };
  assert.equal(formatSelectionLabel(year2012, format), '2012');

  const years = { startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2015, 12) };
  assert.equal(formatSelectionLabel(years, format), '2012 – 2015');

  const range = { startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2015, 8) };
  assert.equal(formatSelectionLabel(range, format), 'March 2012 – August 2015');

  const partial = { startIndex: toMonthIndex(2011, 12), endIndex: toMonthIndex(2012, 3) };
  assert.equal(formatSelectionLabel(partial, format), 'December 2011 – March 2012');
});

test('period labels name the month and year', () => {
  assert.equal(formatPeriodName(toMonthIndex(1998, 3), format.monthName), 'March 1998');
  assert.equal(formatPeriodName(toMonthIndex(2012, 12), format.monthName), 'December 2012');
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test tests/timeline-model.test.js`
Expected: FAIL — `buildTimelineModel is not a function` (the module still exports the year-aggregate helpers).

- [ ] **Step 3: Write the module**

Replace `frontend/src/lib/timeline.js` entirely:

```js
// Pure date/data model for the desktop timeline selector.
// No Svelte imports: testable with node --test.

export const MONTHS_PER_YEAR = 12;
export const MONTHS_PER_DECADE = 120;

const EMPTY_MODEL = {
  minIndex: 0,
  maxIndex: -1,
  length: 0,
  counts: new Int32Array(0),
  prefix: new Float64Array(1),
  total: 0,
  years: [],
};

/** @param {number} year @param {number} month 1-12 */
export const toMonthIndex = (year, month) => year * MONTHS_PER_YEAR + (month - 1);

/** @param {number} index */
export const fromMonthIndex = (index) => ({
  year: Math.floor(index / MONTHS_PER_YEAR),
  month: (index % MONTHS_PER_YEAR) + 1,
});

/**
 * Dense month model over `[minIndex, maxIndex]`: the payload only contains
 * months with photos, so gaps are zero-filled here once instead of at every
 * paint. `prefix` enables O(1) range counts during pan/zoom.
 *
 * @param {Array<{ year: number, month: number, count: number }>} density
 */
export const buildTimelineModel = (density) => {
  const rows = (density ?? []).filter((row) => row && row.count > 0);
  if (rows.length === 0) return EMPTY_MODEL;

  let minIndex = Infinity;
  let maxIndex = -Infinity;
  for (const row of rows) {
    const index = toMonthIndex(row.year, row.month);
    if (index < minIndex) minIndex = index;
    if (index > maxIndex) maxIndex = index;
  }

  const length = maxIndex - minIndex + 1;
  const counts = new Int32Array(length);
  for (const row of rows) {
    counts[toMonthIndex(row.year, row.month) - minIndex] += row.count;
  }

  const prefix = new Float64Array(length + 1);
  for (let i = 0; i < length; i += 1) prefix[i + 1] = prefix[i] + counts[i];

  const years = [];
  for (let index = maxIndex; index >= minIndex; index -= 1) {
    const { year } = fromMonthIndex(index);
    if (years[years.length - 1] !== year) years.push(year);
  }

  return { minIndex, maxIndex, length, counts, prefix, total: prefix[length], years };
};

/** Photo count of `[startIndex, endIndex]`, clamped to the model span. */
export const countInRange = (model, startIndex, endIndex) => {
  if (model.length === 0) return 0;
  const start = Math.max(startIndex, model.minIndex);
  const end = Math.min(endIndex, model.maxIndex);
  if (end < start) return 0;
  return model.prefix[end - model.minIndex + 1] - model.prefix[start - model.minIndex];
};

/** @returns {{ startIndex: number, endIndex: number } | null} */
export const normalizeSelection = (a, b) =>
  a === null || a === undefined || b === null || b === undefined
    ? null
    : { startIndex: Math.min(a, b), endIndex: Math.max(a, b) };

export const selectionEquals = (a, b) =>
  (a === null && b === null) ||
  (a !== null && b !== null && a.startIndex === b.startIndex && a.endIndex === b.endIndex);

/** Narrow a selection to the months the library actually spans; null without overlap. */
export const clampSelectionToModel = (selection, model) => {
  if (!selection || model.length === 0) return null;
  const startIndex = Math.max(selection.startIndex, model.minIndex);
  const endIndex = Math.min(selection.endIndex, model.maxIndex);
  return endIndex < startIndex ? null : { startIndex, endIndex };
};

export const clampIndexToModel = (model, index) =>
  Math.min(Math.max(index, model.minIndex), model.maxIndex);

/** `March 1998` for a month index, using the caller's localised month names. */
export const formatPeriodName = (index, monthName) => {
  const { year, month } = fromMonthIndex(index);
  return `${monthName(month)} ${year}`;
};

/**
 * `All Dates`, `2012`, `March 2012`, `2012 – 2015` or
 * `March 2012 – August 2015` — a full-year range collapses to bare years,
 * because a range covering January through December *is* the year filter.
 */
export const formatSelectionLabel = (selection, format) => {
  if (!selection) return format.allDates;
  const start = fromMonthIndex(selection.startIndex);
  const end = fromMonthIndex(selection.endIndex);
  const wholeYears = start.month === 1 && end.month === 12;

  if (wholeYears && start.year === end.year) return String(start.year);
  if (wholeYears) {
    return format.rangeTemplate(String(start.year), String(end.year));
  }
  if (selection.startIndex === selection.endIndex) {
    return formatPeriodName(selection.startIndex, format.monthName);
  }
  return format.rangeTemplate(
    formatPeriodName(selection.startIndex, format.monthName),
    formatPeriodName(selection.endIndex, format.monthName)
  );
};
```

Delete `tests/timeline-aggregates.test.js`:

```bash
git rm tests/timeline-aggregates.test.js
```

- [ ] **Step 4: Keep the legacy rail compiling**

`TimelineSlider.svelte` imports `buildYearAggregates`/`getYearAggregate`, which no longer exist. Replace lines 20-29 with a local adapter over the new model (the whole desktop block is deleted in Task 7):

```js
  const model = $derived(buildTimelineModel(data?.density ?? []));

  // Transitional adapter for the legacy year rail + month strip, which is
  // replaced by TimelineSelector in a later task.
  const aggregates = $derived(
    model.years.map((year) => ({
      year,
      total: countInRange(model, toMonthIndex(year, 1), toMonthIndex(year, 12)),
      months: Array.from({ length: 12 }, (_, i) => ({
        month: i + 1,
        count: countInRange(
          model,
          toMonthIndex(year, i + 1),
          toMonthIndex(year, i + 1)
        ),
      })),
    }))
  );

  const years = $derived(aggregates.map((a) => a.year));

  const selectedAggregate = $derived(
    selectedYear === null ? null : (aggregates.find((a) => a.year === selectedYear) ?? null)
  );
```

and update the import line to `import { buildTimelineModel, countInRange, toMonthIndex } from '../lib/timeline.js';`.

- [ ] **Step 5: Run the tests to verify they pass, then wire the suite into CI**

Run: `node --test tests/timeline-model.test.js`
Expected: PASS, 7 tests.

Add to `package.json` scripts (alphabetically next to the other test scripts):

```json
    "test:unit": "node --test tests/*.test.js",
```

Add a step to `.github/workflows/ci.yml` in the `lint-format` job, directly after `Run i18n integrity check`:

```yaml
      - name: Run frontend unit tests
        run: npm run test:unit
```

Run: `npm run test:unit`
Expected: `timeline-model.test.js` passes and `tests/i18n-integrity.test.js` is not picked up by the glob (it is, and it also passes — 2 files, all green).

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/timeline.js frontend/src/components/TimelineSlider.svelte tests/timeline-model.test.js package.json .github/workflows/ci.yml
git rm --cached tests/timeline-aggregates.test.js 2>/dev/null || true
git commit -m "feat(timeline): add pure month-index model, drop year aggregates"
```

---

### Task 4: Pure view geometry (`frontend/src/lib/timelineLayout.js`)

**Files:**
- Create: `frontend/src/lib/timelineLayout.js`
- Create: `tests/timeline-layout.test.js`

**Interfaces:**
- Consumes: the model from Task 3 (`{ minIndex, maxIndex, length }`) and its `countInRange(model, a, b)`, `fromMonthIndex(index)`.
- Produces (all pure):
  - `MIN_COLUMN_PX = 28`, `MIN_LABEL_GAP_PX = 8`, `HANDLE_HIT_PX = 12`, `LABEL_SAFETY_PX = 4`, `ZOOM_STEP = 1.6`, `DECADE_UNITS = [1, 12, 120]`
  - `fitAllScale(width, model) => number`, `maxScale(width, model) => number`
  - `createView(width, model) => { scale, origin }`, `clampView(view, width, model) => view`
  - `zoomView({ view, factor, anchorPx, width, model }) => view`
  - `panView({ view, deltaPx, width, model }) => view` (positive `deltaPx` = content dragged right)
  - `zoomToRange(range, width, model, paddingRatio = 0.2) => view`
  - `ensureSelectionVisible(selection, view, width, model) => view`
  - `chooseUnit(scale) => 1 | 12 | 120`
  - internal (not exported): `formatColumnLabel(unit, gridStart, format)` — `format` is `{ periodName(index) }`; decade columns render `"1990s"`, year columns `"1990"`, month columns `"March 1998"`
  - `buildColumns({ unit, view, width, model, format, countInRange }) => Column[]` with `Column = { gridStart, startIndex, endIndex, x, width, count, label }`
  - `placeLabels(columns, { width, measure }) => Array<Column & { labelX: number | null, labelWidth: number | null }>`
  - `indexFromX(x, view) => number`, `xFromIndex(index, view) => number`
  - `selectionZoneAtX(x, { selection, view, width, handlePx }) => 'start' | 'end' | 'body' | null`
  - `translateSelection(selection, deltaMonths, model) => selection`
  - `clampBound(index, selection, bound, model) => number`
  - `panView` semantics: `deltaPx > 0` means the content was dragged that many pixels to the right.

- [ ] **Step 1: Write the failing unit tests**

Create `tests/timeline-layout.test.js`:

```js
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  HANDLE_HIT_PX,
  MIN_COLUMN_PX,
  buildColumns,
  chooseUnit,
  clampBound,
  clampView,
  createView,
  ensureSelectionVisible,
  fitAllScale,
  indexFromX,
  panView,
  placeLabels,
  selectionZoneAtX,
  translateSelection,
  xFromIndex,
  zoomToRange,
  zoomView,
} from '../frontend/src/lib/timelineLayout.js';
import { formatPeriodName, toMonthIndex } from '../frontend/src/lib/timeline.js';

const monthName = (month) =>
  ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'][month - 1];
const format = { monthName, periodName: (index) => formatPeriodName(index, monthName) };

// A 60-year span with 240 populated months (every third month).
const denseModel = {
  minIndex: 0,
  maxIndex: 60 * 12 - 1,
  length: 60 * 12,
  populated: new Set(),
};
for (let i = 0; i < 60 * 12; i += 3) denseModel.populated.add(i);
const countInRange = (model, a, b) => {
  let count = 0;
  for (let i = a; i <= b; i += 1) if (model.populated.has(i)) count += 1;
  return count;
};
const buildColumnsFor = (unit, view, width) =>
  buildColumns({ unit, view, width, model: denseModel, format, countInRange });

test('fit-all scale shows the whole span and the view cannot escape it', () => {
  const width = 1200;
  const view = createView(width, denseModel);
  assert.equal(view.scale, fitAllScale(width, denseModel));
  assert.equal(view.origin, denseModel.minIndex);
  assert.equal(Math.round((denseModel.maxIndex + 1 - view.origin) * view.scale), width);

  const panned = panView({ view, deltaPx: 500, width, model: denseModel });
  assert.deepEqual(panned, view, 'a fully visible span cannot pan');

  const zoomedOut = clampView({ scale: view.scale / 10, origin: -500 }, width, denseModel);
  assert.deepEqual(zoomedOut, view, 'zooming out past the full span snaps back to it');
});

test('zooming anchors on the pointer and clamps to one month at the closest view', () => {
  const width = 1200;
  const view = createView(width, denseModel);
  const zoomed = zoomView({ view, factor: 4, anchorPx: 600, width, model: denseModel });
  const anchored = indexFromX(600, view);
  assert.ok(Math.abs(indexFromX(600, zoomed) - anchored) < 1, 'anchor index stays under the pointer');

  const maxed = zoomView({ view, factor: 1e6, anchorPx: 0, width, model: denseModel });
  assert.equal(maxed.scale, width, 'closest view spans exactly one month');
  assert.equal(Math.round(width / maxed.scale), 1);

  const pinned = zoomView({ view: maxed, factor: 1e6, anchorPx: 0, width, model: denseModel });
  assert.deepEqual(pinned, maxed, 'input at the zoom limit changes nothing');
});

test('panning clamps to the data span', () => {
  const width = 1200;
  const view = zoomView({ view: createView(width, denseModel), factor: 8, anchorPx: 600, width, model: denseModel });
  const farLeft = panView({ view, deltaPx: 1e6, width, model: denseModel });
  assert.equal(farLeft.origin, denseModel.minIndex);
  const farRight = panView({ view, deltaPx: -1e6, width, model: denseModel });
  assert.equal(farRight.origin, denseModel.maxIndex + 1 - width / farRight.scale);
});

test('granularity follows the pixel budget', () => {
  assert.equal(chooseUnit(0.2), 120, 'a 500-year span can only afford decades');
  assert.equal(chooseUnit(2.5), 12, 'a 40-year span affords years');
  assert.equal(chooseUnit(40), 1, 'a 2-year span affords months');
  assert.ok(12 * 2.5 >= MIN_COLUMN_PX);
});

test('columns tile the viewport, clip to the data and aggregate their counts', () => {
  const width = 1200;
  const view = createView(width, denseModel);
  const unit = chooseUnit(view.scale);
  const columns = buildColumnsFor(unit, view, width);

  assert.ok(columns.length > 0);
  assert.ok(columns.length <= Math.ceil(width / MIN_COLUMN_PX) + 1, 'node count stays bounded');
  assert.equal(columns[0].startIndex, denseModel.minIndex, 'first column clips to the data span');
  assert.equal(columns[columns.length - 1].endIndex, denseModel.maxIndex);
  for (let i = 1; i < columns.length; i += 1) {
    assert.equal(columns[i].gridStart, columns[i - 1].gridStart + unit, 'columns are contiguous');
    assert.equal(columns[i].count, countInRange(denseModel, columns[i].startIndex, columns[i].endIndex));
  }
  assert.equal(columns[0].label, '0s', 'decade labels come from the grid-aligned start');
});

test('label placement never overlaps, clips or crowds a neighbour', () => {
  const width = 1200;
  const measure = (text) => text.length * 8;
  for (const scale of [0.5, 1, 2.5, 8, 40, 200, 1200]) {
    const view = clampView({ scale, origin: 300 }, width, denseModel);
    const columns = buildColumnsFor(chooseUnit(view.scale), view, width);
    const placed = placeLabels(columns, { width, measure });
    let lastRight = -Infinity;
    for (const column of placed) {
      if (column.labelX === null) continue;
      assert.ok(column.labelX >= 0, 'no label clipped on the left');
      assert.ok(column.labelX + column.labelWidth <= width, 'no label clipped on the right');
      assert.ok(column.labelX >= lastRight, `labels overlap at scale ${view.scale}`);
      lastRight = column.labelX + column.labelWidth + 8;
    }
  }
});

test('a 60-year span at every supported width keeps labels legible and bounded', () => {
  const measure = (text) => text.length * 7.5;
  for (const width of [769, 1024, 1280, 1374, 1920, 2560]) {
    const view = createView(width, denseModel);
    const columns = buildColumnsFor(chooseUnit(view.scale), view, width);
    const placed = placeLabels(columns, { width, measure });
    const labels = placed.filter((c) => c.labelX !== null);
    assert.ok(labels.length > 0, `width ${width} must label something`);
    for (const label of labels) {
      assert.ok(label.labelWidth + label.x >= 0);
      assert.ok(label.labelX + label.labelWidth <= width);
    }
  }
});

test('layout of a 100-year, 1200-month model stays far below the 100 ms budget', () => {
  const model = { minIndex: 0, maxIndex: 100 * 12 - 1, length: 100 * 12 };
  const count = (_, a, b) => Math.max(0, b - a + 1);
  const width = 1920;
  const started = performance.now();
  for (let i = 0; i < 200; i += 1) {
    const view = clampView({ scale: 1 + i * 0.05, origin: i * 7 }, width, model);
    const columns = buildColumns({ unit: chooseUnit(view.scale), view, width, model, format, countInRange: count });
    placeLabels(columns, { width, measure: (text) => text.length * 8 });
  }
  const elapsed = performance.now() - started;
  assert.ok(elapsed < 100, `200 layouts took ${elapsed.toFixed(1)} ms`);
});

test('hit-testing maps pixels to months and separates handles from the body', () => {
  const width = 1200;
  const view = { scale: 10, origin: toMonthIndex(2012, 1) };
  const selection = { startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2012, 8) };
  const startX = xFromIndex(selection.startIndex, view);
  const endX = xFromIndex(selection.endIndex + 1, view);

  assert.equal(selectionZoneAtX(startX + HANDLE_HIT_PX - 1, { selection, view, width }), 'start');
  assert.equal(selectionZoneAtX(endX - HANDLE_HIT_PX + 1, { selection, view, width }), 'end');
  assert.equal(selectionZoneAtX((startX + endX) / 2, { selection, view, width }), 'body');
  assert.equal(selectionZoneAtX(startX - 200, { selection, view, width }), null);
  assert.equal(selectionZoneAtX(50, { selection: null, view, width }), null);
  assert.equal(indexFromX(startX, view), selection.startIndex);
});

test('bound clamping and translation keep the range inside the data span', () => {
  const model = denseModel;
  const selection = { startIndex: 300, endIndex: 320 };

  assert.equal(clampBound(100, selection, 'start', model), 100, 'a start bound may move left');
  assert.equal(clampBound(400, selection, 'start', model), 320, 'a start bound cannot pass the end');
  assert.equal(clampBound(100, selection, 'end', model), 300, 'an end bound cannot pass the start');
  assert.equal(clampBound(1e6, selection, 'end', model), model.maxIndex);

  assert.deepEqual(translateSelection(selection, 5, model), { startIndex: 305, endIndex: 325 });
  assert.deepEqual(translateSelection(selection, 0, model), selection);
  assert.deepEqual(translateSelection({ startIndex: 2, endIndex: 5 }, -10, model), {
    startIndex: 0,
    endIndex: 3,
  });
  assert.deepEqual(translateSelection({ startIndex: 700, endIndex: 719 }, 10, model), {
    startIndex: 719 - 19,
    endIndex: 719,
  });
});

test('selection visibility pans minimally and never zooms in', () => {
  const width = 1200;
  const view = zoomView({ view: createView(width, denseModel), factor: 10, anchorPx: 0, width, model: denseModel });
  const selection = { startIndex: 500, endIndex: 520 };
  const fixed = ensureSelectionVisible(selection, view, width, denseModel);

  assert.ok(fixed.scale <= view.scale, 'zooms out only when needed');
  const startX = xFromIndex(selection.startIndex, fixed);
  const endX = xFromIndex(selection.endIndex + 1, fixed);
  assert.ok(startX >= -0.001 && endX <= width + 0.001, 'the whole selection is inside the viewport');

  const zoomedOut = { scale: fitAllScale(width, denseModel), origin: denseModel.minIndex };
  assert.deepEqual(
    ensureSelectionVisible(selection, zoomedOut, width, denseModel),
    zoomedOut,
    'an already visible selection leaves the view untouched'
  );

  const wide = { startIndex: 100, endIndex: 900 };
  assert.ok(
    ensureSelectionVisible(wide, view, width, denseModel).scale < view.scale,
    'a selection wider than the viewport zooms out to fit'
  );
});

test('zooming to a range fills the viewport with it', () => {
  const width = 1200;
  const range = { startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2012, 12) };
  const view = zoomToRange(range, width, denseModel);
  const startX = xFromIndex(range.startIndex, view);
  const endX = xFromIndex(range.endIndex + 1, view);
  assert.ok(startX >= 0 && endX <= width);
  assert.ok(endX - startX > width * 0.6, 'the range dominates the viewport');
  assert.equal(view.scale, width / (12 * 1.2));
});

test('a zero-width viewport (hidden at the mobile breakpoint) yields an empty layout', () => {
  const view = createView(0, denseModel);
  assert.ok(Number.isFinite(view.scale));
  assert.equal(buildColumnsFor(1, view, 0).length, 0);
});

test('a single-month library has one zoom level', () => {
  const model = { minIndex: 100, maxIndex: 100, length: 1 };
  const view = createView(1200, model);
  assert.equal(view.scale, 1200);
  assert.deepEqual(zoomView({ view, factor: 4, anchorPx: 0, width: 1200, model }), view);
  assert.deepEqual(panView({ view, deltaPx: 400, width: 1200, model }), view);
  const columns = buildColumns({
    unit: chooseUnit(view.scale),
    view,
    width: 1200,
    model,
    format,
    countInRange: () => 1,
  });
  assert.equal(columns.length, 1);
  assert.equal(columns[0].startIndex, 100);
  assert.equal(columns[0].endIndex, 100);
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test tests/timeline-layout.test.js`
Expected: FAIL — `Cannot find module '../frontend/src/lib/timelineLayout.js'`.

- [ ] **Step 3: Write the module**

Create `frontend/src/lib/timelineLayout.js`:

```js
// Pure view geometry for the desktop timeline selector: month indices in,
// pixels out. No Svelte imports and no DOM access: label measurement is
// injected, so the collision rules are unit-testable.
import { fromMonthIndex, MONTHS_PER_DECADE, MONTHS_PER_YEAR } from './timeline.js';

/** Pointer/keyboard target floor and axe `target-size` minimum. */
export const MIN_COLUMN_PX = 28;
/** Minimum spacing between two rendered labels (FR-005). */
export const MIN_LABEL_GAP_PX = 8;
/** Measured text is a hair narrower than rendered text; reserve a margin. */
export const LABEL_SAFETY_PX = 4;
/** Pointer grab zone around a selection edge. */
export const HANDLE_HIT_PX = 12;
/** Multiplicative step of the zoom controls. */
export const ZOOM_STEP = 1.6;
/** Candidate column units, finest first: month, year, decade. */
const UNITS = [1, MONTHS_PER_YEAR, MONTHS_PER_DECADE];
/** Years drawn around a drilled-into range. */
const ZOOM_TO_RANGE_PADDING = 0.2;

export const fitAllScale = (width, model) =>
  width <= 0 || model.length === 0 ? 1 : width / model.length;

/** Closest view: exactly one month fills the viewport. */
export const maxScale = (width, model) => Math.max(width, fitAllScale(width, model));

export const clampView = (view, width, model) => ({
  scale: Math.min(Math.max(view.scale, fitAllScale(width, model)), maxScale(width, model)),
  origin: 0,
});

export const createView = (width, model) => ({ scale: fitAllScale(width, model), origin: model.minIndex });

export const clampOrigin = (origin, { width, scale, model }) => {
  if (width <= 0 || model.length === 0) return model.minIndex;
  const visible = width / scale;
  const maxOrigin = model.maxIndex + 1 - visible;
  if (maxOrigin <= model.minIndex) return model.minIndex;
  return Math.min(Math.max(origin, model.minIndex), maxOrigin);
};

export const clampScale = (scale, width, model) =>
  Math.min(Math.max(scale, fitAllScale(width, model)), maxScale(width, model));

export const zoomView = ({ view, factor, anchorPx, width, model }) => {
  const anchorIndex = view.origin + anchorPx / view.scale;
  const scale = clampScale(view.scale * factor, width, model);
  return { scale, origin: clampOrigin(anchorIndex - anchorPx / scale, { width, scale, model }) };
};

/** Positive `deltaPx` means the content was dragged that many pixels to the right. */
export const panView = ({ view, deltaPx, width, model }) => ({
  scale: view.scale,
  origin: clampOrigin(view.origin - deltaPx / view.scale, { width, scale: view.scale, model }),
});

export const zoomToRange = (range, width, model, paddingRatio = ZOOM_TO_RANGE_PADDING) => {
  const span = range.endIndex - range.startIndex + 1;
  const scale = clampScale(width / (span * (1 + 2 * paddingRatio)), width, model);
  const center = (range.startIndex + range.endIndex + 1) / 2;
  return { scale, origin: clampOrigin(center - width / (2 * scale), { width, scale, model }) };
};

export const xFromIndex = (index, view) => (index - view.origin) * view.scale;
export const indexFromX = (x, view) => Math.floor(view.origin + x / view.scale);

/** The finest unit whose columns still meet the pointer-target floor. */
export const chooseUnit = (scale) =>
  UNITS.find((unit) => unit * scale >= MIN_COLUMN_PX) ?? UNITS[UNITS.length - 1];

export const buildColumns = ({ unit, view, width, model, format, countInRange }) => {
  const columns = [];
  if (width <= 0 || model.length === 0) return columns;

  const firstGridStart = Math.floor(view.origin / unit) * unit;
  const lastIndex = view.origin + width / view.scale;

  for (let gridStart = firstGridStart; gridStart < lastIndex; gridStart += unit) {
    const startIndex = Math.max(gridStart, model.minIndex);
    const endIndex = Math.min(gridStart + unit - 1, model.maxIndex);
    if (endIndex < startIndex) continue;
    columns.push({
      gridStart,
      startIndex,
      endIndex,
      x: xFromIndex(startIndex, view),
      width: (endIndex - startIndex + 1) * view.scale,
      count: countInRange(model, startIndex, endIndex),
      label: formatColumnLabel(unit, gridStart, format),
    });
  }
  return columns;
};

const formatColumnLabel = (unit, gridStart, format) => {
  const { year } = fromMonthIndex(gridStart);
  if (unit === MONTHS_PER_DECADE) return `${year - (year % 10)}s`;
  if (unit === MONTHS_PER_YEAR) return String(year);
  return format.periodName(gridStart);
};

/**
 * Distribute labels left to right, dropping any that would be clipped by the
 * viewport or crowd the previously placed one. Returning `labelX: null` is the
 * whole collision strategy: no overlap, no clipping, no fake spacing maths.
 */
export const placeLabels = (columns, { width, measure }) => {
  let lastRight = -Infinity;
  return columns.map((column) => {
    const labelWidth = measure(column.label) + LABEL_SAFETY_PX;
    const labelX = Math.round(column.x + column.width / 2 - labelWidth / 2);
    if (labelX < 0 || labelX + labelWidth > width || labelX < lastRight + MIN_LABEL_GAP_PX) {
      return { ...column, labelX: null, labelWidth: null };
    }
    lastRight = labelX + labelWidth;
    return { ...column, labelX, labelWidth };
  });
};

export const selectionZoneAtX = (x, { selection, view, width, handlePx = HANDLE_HIT_PX }) => {
  if (!selection || width <= 0) return null;
  const startX = xFromIndex(selection.startIndex, view);
  const endX = xFromIndex(selection.endIndex + 1, view);
  if (x >= startX - handlePx && x <= startX + handlePx) return 'start';
  if (x >= endX - handlePx && x <= endX + handlePx) return 'end';
  if (x > startX && x < endX) return 'body';
  return null;
};

/** Clamp a dragged bound: it may not pass the opposite bound (no inversion). */
export const clampBound = (index, selection, bound, model) => {
  const clamped = Math.min(Math.max(index, model.minIndex), model.maxIndex);
  if (!selection) return clamped;
  return bound === 'start'
    ? Math.min(clamped, selection.endIndex)
    : Math.max(clamped, selection.startIndex);
};

export const translateSelection = (selection, deltaMonths, model) => {
  if (!selection || deltaMonths === 0) return selection;
  const span = selection.endIndex - selection.startIndex;
  const startIndex = Math.min(
    Math.max(selection.startIndex + deltaMonths, model.minIndex),
    model.maxIndex - span
  );
  return { startIndex, endIndex: startIndex + span };
};

/** FR-010: pan minimally, zoom out only when the selection cannot fit. */
export const ensureSelectionVisible = (selection, view, width, model) => {
  if (!selection || width <= 0) return view;
  const span = selection.endIndex - selection.startIndex + 1;
  const scale = clampScale(Math.min(view.scale, width / span), width, model);
  const startX = xFromIndex(selection.startIndex, { ...view, scale });
  const endX = xFromIndex(selection.endIndex + 1, { ...view, scale });
  let origin = view.origin;
  if (startX < 0) origin = selection.startIndex;
  else if (endX > width) origin = selection.endIndex + 1 - width / scale;
  return { scale, origin: clampOrigin(origin, { width, scale, model }) };
};
```

`formatColumnLabel` is the single definition of the column-label rule and is used by `buildColumns` — it is not exported to the component, which only passes `format` through.

- [ ] **Step 4: Run the test to verify it passes**

Run: `node --test tests/timeline-layout.test.js`
Expected: PASS, 14 tests.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/timelineLayout.js tests/timeline-layout.test.js
git commit -m "feat(timeline): add pure zoom/pan/label geometry with tests"
```

---

### Task 5: Pure route mapping (`frontend/src/lib/timelineRoute.js`)

**Files:**
- Create: `frontend/src/lib/timelineRoute.js`
- Create: `tests/timeline-route.test.js`

**Interfaces:**
- Consumes: `toMonthIndex`, `fromMonthIndex`, `clampSelectionToModel` from Task 3.
- Produces:
  - `EMPTY_DATE_FILTER = { year: null, month: null, to_year: null, to_month: null }`
  - `normalizeDateFilter({ year, month, to_year, to_month }) => filter` — validates, swaps reversed bounds into ascending order, canonicalises (`to_month` null when the end is December; a degenerate range collapses to a single period), and clears everything when there is no year.
  - `selectionFromFilter(filter, model) => { startIndex, endIndex } | null` — clamped to the model span (no overlap ⇒ null).
  - `filterFromSelection(selection) => filter`
  - `filterEquals(a, b) => boolean`

- [ ] **Step 1: Write the failing unit tests**

Create `tests/timeline-route.test.js`:

```js
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  EMPTY_DATE_FILTER,
  filterEquals,
  filterFromSelection,
  normalizeDateFilter,
  selectionFromFilter,
} from '../frontend/src/lib/timelineRoute.js';
import { buildTimelineModel, toMonthIndex } from '../frontend/src/lib/timeline.js';

const model = buildTimelineModel([
  { year: 2012, month: 3, count: 1 },
  { year: 2012, month: 8, count: 1 },
  { year: 2015, month: 8, count: 1 },
  { year: 2026, month: 1, count: 1 },
]);

test('a cleared filter stays cleared and drops orphan months', () => {
  assert.deepEqual(normalizeDateFilter({ year: null, month: 5, to_year: 2012, to_month: 3 }), EMPTY_DATE_FILTER);
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 13, to_year: null, to_month: null }), {
    year: 2012,
    month: null,
    to_year: null,
    to_month: null,
  });
});

test('single periods survive canonicalisation unchanged', () => {
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 3 }), {
    year: 2012,
    month: 3,
    to_year: null,
    to_month: null,
  });
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: null }), {
    year: 2012,
    month: null,
    to_year: null,
    to_month: null,
  });
});

test('ranges canonicalise to whole-year bounds and never carry a redundant end', () => {
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 1, to_year: 2012, to_month: 12 }), {
    year: 2012,
    month: null,
    to_year: null,
    to_month: null,
  });
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 3, to_year: 2012, to_month: 8 }), {
    year: 2012,
    month: 3,
    to_year: 2012,
    to_month: 8,
  });
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 1, to_year: 2015, to_month: 12 }), {
    year: 2012,
    month: null,
    to_year: 2015,
    to_month: null,
  });
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 3, to_year: 2015, to_month: 12 }), {
    year: 2012,
    month: 3,
    to_year: 2015,
    to_month: null,
  });
  assert.deepEqual(normalizeDateFilter({ year: 2012, month: 3, to_year: 2015, to_month: null }), {
    year: 2012,
    month: 3,
    to_year: 2015,
    to_month: null,
  });
});

test('reversed restored bounds normalise to ascending order', () => {
  assert.deepEqual(normalizeDateFilter({ year: 2015, month: 8, to_year: 2012, to_month: 3 }), {
    year: 2012,
    month: 3,
    to_year: 2015,
    to_month: 8,
  });
  assert.deepEqual(normalizeDateFilter({ year: 2015, month: null, to_year: 2012, to_month: null }), {
    year: 2012,
    month: null,
    to_year: 2015,
    to_month: null,
  });
});

test('filter to selection clamps to the library and clears without overlap', () => {
  const range = normalizeDateFilter({ year: 2012, month: 3, to_year: 2015, to_month: 8 });
  assert.deepEqual(selectionFromFilter(range, model), {
    startIndex: toMonthIndex(2012, 3),
    endIndex: toMonthIndex(2015, 8),
  });

  const year = normalizeDateFilter({ year: 2012, month: null });
  assert.deepEqual(selectionFromFilter(year, model), {
    startIndex: toMonthIndex(2012, 1),
    endIndex: toMonthIndex(2012, 12),
  });

  const lost = normalizeDateFilter({ year: 1900, month: 4, to_year: 1901, to_month: 6 });
  assert.equal(selectionFromFilter(lost, model), null, 'no overlap clears the filter');

  const partial = normalizeDateFilter({ year: 1900, month: 4, to_year: 2012, to_month: 8 });
  assert.deepEqual(selectionFromFilter(partial, model), {
    startIndex: model.minIndex,
    endIndex: toMonthIndex(2012, 8),
  });

  assert.equal(selectionFromFilter(EMPTY_DATE_FILTER, model), null);
  assert.equal(selectionFromFilter(null, model), null);
});

test('selection to filter is canonical and round-trips', () => {
  const selection = { startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2015, 8) };
  const filter = filterFromSelection(selection);
  assert.deepEqual(filter, { year: 2012, month: 3, to_year: 2015, to_month: 8 });
  assert.deepEqual(selectionFromFilter(filter, model), selection);

  assert.deepEqual(filterFromSelection(null), EMPTY_DATE_FILTER);
  assert.deepEqual(filterFromSelection({ startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2012, 3) }), {
    year: 2012,
    month: 3,
    to_year: null,
    to_month: null,
  });
  assert.deepEqual(filterFromSelection({ startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2012, 12) }), {
    year: 2012,
    month: null,
    to_year: null,
    to_month: null,
  });
  assert.deepEqual(filterFromSelection({ startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2015, 12) }), {
    year: 2012,
    month: null,
    to_year: 2015,
    to_month: null,
  });
});

test('filterEquality is structural', () => {
  assert.ok(filterEquals(EMPTY_DATE_FILTER, { year: null, month: null, to_year: null, to_month: null }));
  assert.ok(!filterEquals(EMPTY_DATE_FILTER, { year: 2012, month: null, to_year: null, to_month: null }));
  assert.ok(
    !filterEquals(
      { year: 2012, month: 3, to_year: 2015, to_month: 8 },
      { year: 2012, month: 3, to_year: 2015, to_month: 9 }
    )
  );
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test tests/timeline-route.test.js`
Expected: FAIL — module not found.

- [ ] **Step 3: Write the module**

Create `frontend/src/lib/timelineRoute.js`:

```js
// Route ⇄ selection mapping for the timeline filter. The route carries the
// start bound in `year`/`month` (today's params, so existing links and saved
// searches keep working) and the end bound in `to_year`/`to_month`.
import { clampSelectionToModel, fromMonthIndex, toMonthIndex } from './timeline.js';

export const EMPTY_DATE_FILTER = { year: null, month: null, to_year: null, to_month: null };

const parseYear = (value) => (Number.isInteger(value) && value >= 1 ? value : null);
const parseMonth = (value) =>
  Number.isInteger(value) && value >= 1 && value <= 12 ? value : null;

/**
 * Validate, order and canonicalise a route date filter.
 *
 * Canonical form matters twice: the URL round-trips (Back/Forward, saved
 * searches) and the clamp effect can compare route against clamped filter
 * without writing on every render.
 */
export const normalizeDateFilter = (raw) => {
  const year = parseYear(raw?.year);
  if (year === null) return EMPTY_DATE_FILTER;
  const month = parseMonth(raw?.month);
  const toYear = parseYear(raw?.to_year);
  if (toYear === null) return { year, month, to_year: null, to_month: null };

  const toMonth = parseMonth(raw?.to_month);
  let startIndex = toMonthIndex(year, month ?? 1);
  let endIndex = toMonthIndex(toYear, toMonth ?? 12);
  if (endIndex < startIndex) [startIndex, endIndex] = [endIndex, startIndex];

  const start = fromMonthIndex(startIndex);
  const end = fromMonthIndex(endIndex);

  // A range of one period is that period; a range of one whole year is that year.
  if (start.year === end.year && start.month === end.month) {
    return { year: start.year, month: start.month, to_year: null, to_month: null };
  }
  if (start.year === end.year && start.month === 1 && end.month === 12) {
    return { year: start.year, month: null, to_year: null, to_month: null };
  }

  return {
    year: start.year,
    month: start.month === 1 ? null : start.month,
    to_year: end.year,
    to_month: end.month === 12 ? null : end.month,
  };
};

/** Active selection, clamped to the months the library actually has. */
export const selectionFromFilter = (filter, model) => {
  if (!filter || filter.year === null || !model || model.length === 0) return null;
  const startIndex = toMonthIndex(filter.year, filter.month ?? 1);
  const endIndex =
    filter.to_year === null
      ? toMonthIndex(filter.year, filter.month ?? 12)
      : toMonthIndex(filter.to_year, filter.to_month ?? 12);
  return clampSelectionToModel({ startIndex, endIndex }, model);
};

export const filterFromSelection = (selection) =>
  selection === null
    ? EMPTY_DATE_FILTER
    : normalizeDateFilter({
        year: fromMonthIndex(selection.startIndex).year,
        month: fromMonthIndex(selection.startIndex).month,
        to_year: fromMonthIndex(selection.endIndex).year,
        to_month: fromMonthIndex(selection.endIndex).month,
      });

export const filterEquals = (a, b) =>
  a.year === b.year && a.month === b.month && a.to_year === b.to_year && a.to_month === b.to_month;
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `node --test tests/timeline-route.test.js`
Expected: PASS, 8 tests.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/timelineRoute.js tests/timeline-route.test.js
git commit -m "feat(timeline): add route date-filter mapping with tests"
```

---

### Task 6: Route, API client, grid and saved searches carry the range

**Files:**
- Modify: `frontend/src/lib/router.svelte.js:3-12` (defaults), `:22-30` (parse), `:33-50` (normalize), `:96-102` (buildUrl)
- Modify: `frontend/src/lib/api.js:98-104` (params)
- Modify: `frontend/src/components/PhotoGrid.svelte:53-65`, `:218`, `:396-405`
- Modify: `frontend/src/components/SearchBar.svelte:20-55`
- Modify: `frontend/src/components/Sidebar.svelte:50-80`
- Modify: `frontend/src/components/AlbumsView.svelte:105`
- Modify: `frontend/src/components/TimelineSlider.svelte:69-73` (legacy rail clears the range until Task 7 deletes it)
- Modify: `frontend/src/App.svelte:64-66` (comment)
- Test: `tests/e2e/specs/url-routing.e2e.spec.js`, `tests/e2e/specs/saved-searches.e2e.spec.js`

**Interfaces:**
- Consumes: `normalizeDateFilter`, `filterEquals` from Task 5; backend params from Task 1; saved-search fields from Task 2.
- Produces: `route.to_year` / `route.to_month` (number|null) as the only range state; `api.getPhotos({ toYear, toMonth })` → `to_year`, `to_month` query params.

- [ ] **Step 1: Write the failing E2E tests**

In `tests/e2e/specs/url-routing.e2e.spec.js`, inside the `Timeline URL` describe block, add:

```js
    test('should filter the grid by an inclusive month range deep link', async ({ page }) => {
      // GIVEN: the two oldest/newest populated months of the fixture library
      await TestHelpers.goto(page);
      const density = await page.evaluate(() =>
        fetch('/api/photos/timeline')
          .then((r) => r.json())
          .then((d) => d.density || [])
      );
      test.skip(density.length < 2, 'Timeline needs at least two month buckets');
      const first = density[0];
      const last = density[density.length - 1];

      // WHEN: opening a deep link covering both ends of the library
      await TestHelpers.goto(
        page,
        `/?year=${first.year}&month=${first.month}&to_year=${last.year}&to_month=${last.month}`
      );
      await TestHelpers.waitForPhotosToLoad(page);

      // THEN: the range survives the round-trip
      const state = TestHelpers.getUrlState(page);
      expect(state.toYear).toBe(last.year);
      expect(state.toMonth).toBe(last.month);

      // AND: the grid shows exactly what the same range returns over the API
      const expectedTotal = await page.evaluate(async (params) => {
        const response = await fetch(`/api/photos?limit=1&${params}`);
        return (await response.json()).total;
      }, new URLSearchParams({
        year: state.year,
        ...(state.month === null ? {} : { month: state.month }),
        to_year: state.toYear,
        ...(state.toMonth === null ? {} : { to_month: state.toMonth }),
      }).toString());
      const cards = await page.locator('.photo-card').count();
      expect(cards).toBe(Math.min(expectedTotal, 50));
      expect(expectedTotal).toBeGreaterThan(0);
    });
```

A restored range with reversed bounds is covered by Task 5's unit tests (the route normalises before any URL write); the URL string itself is only rewritten when the selection is clamped (Task 7).

Extend `TestHelpers.getUrlState` in `tests/e2e/setup/test-helpers.js`:

```js
      toYear: url.searchParams.get('to_year') !== null
        ? parseInt(url.searchParams.get('to_year'), 10)
        : null,
      toMonth: url.searchParams.get('to_month') !== null
        ? parseInt(url.searchParams.get('to_month'), 10)
        : null,
```

In `tests/e2e/specs/saved-searches.e2e.spec.js`, add:

```js
  test('should save and restore a month range', async ({ page }) => {
    // GIVEN: a range filter deep link over two populated months
    await TestHelpers.goto(page);
    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((r) => r.json())
        .then((d) => d.density || [])
    );
    test.skip(density.length < 2, 'Timeline needs at least two month buckets');
    const first = density[0];
    const last = density[density.length - 1];
    await TestHelpers.goto(
      page,
      `/?year=${first.year}&month=${first.month}&to_year=${last.year}&to_month=${last.month}`
    );
    await TestHelpers.waitForPhotosToLoad(page);

    // WHEN: the search is saved
    await page.click('[data-testid="save-search-btn"]');
    await expect(page.locator('[data-testid="saved-search-row"]')).toHaveCount(1);

    // AND: the user navigates away and reopens it
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator('[data-testid="saved-search-row"]').first().click();
    await TestHelpers.waitForPhotosToLoad(page);

    // THEN: both bounds come back
    const state = TestHelpers.getUrlState(page);
    expect(state.year).toBe(first.year);
    expect(state.month).toBe(first.month === 1 ? null : first.month);
    expect(state.toYear).toBe(last.year);
    expect(state.toMonth).toBe(last.month === 12 ? null : last.month);
  });
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx playwright test tests/e2e/specs/url-routing.e2e.spec.js -g "month range"`
Expected: FAIL — the range params are dropped by the router.

- [ ] **Step 3: Teach the router the range params**

In `frontend/src/lib/router.svelte.js`:

```js
import { EMPTY_DATE_FILTER, normalizeDateFilter } from './timelineRoute.js';

const defaultState = {
  view: 'all',
  photo: null,
  query: null,
  sort: 'date_desc',
  year: null,
  month: null,
  to_year: null,
  to_month: null,
  album: null,
};
```

`parseUrl` reads the raw integers (validation is `normalizeDateFilter`'s job):

```js
    year: parsePositiveInteger(url.searchParams.get('year')),
    month: parsePositiveInteger(url.searchParams.get('month')),
    to_year: parsePositiveInteger(url.searchParams.get('to_year')),
    to_month: parsePositiveInteger(url.searchParams.get('to_month')),
```

`normalizeState` delegates the whole date filter:

```js
function normalizeState(state) {
  const view = validViews.includes(state.view) ? state.view : defaultState.view;
  const sort = validSorts.includes(state.sort) ? state.sort : defaultState.sort;
  const dateFilter = normalizeDateFilter(state);

  return {
    view,
    photo: normalizeString(state.photo),
    query: normalizeString(state.query),
    sort,
    year: dateFilter.year,
    month: dateFilter.month,
    to_year: dateFilter.to_year,
    to_month: dateFilter.to_month,
    album: parsePositiveInteger(state.album),
  };
}
```

`buildUrl` writes both bounds:

```js
  if (normalizedState.year !== null) {
    url.searchParams.set('year', String(normalizedState.year));

    if (normalizedState.month !== null) {
      url.searchParams.set('month', String(normalizedState.month));
    }
  }

  if (normalizedState.to_year !== null) {
    url.searchParams.set('to_year', String(normalizedState.to_year));

    if (normalizedState.to_month !== null) {
      url.searchParams.set('to_month', String(normalizedState.to_month));
    }
  }
```

Delete the now-unused local `EMPTY_DATE_FILTER` import if prettier/eslint flags it.

- [ ] **Step 4: Send and consume the range in the API client and grid**

`frontend/src/lib/api.js` — replace the dead `dateFrom`/`dateTo` lines (the backend has never supported them) and add the range:

```js
    if (params.year !== undefined) searchParams.set('year', params.year);
    if (params.month !== undefined) searchParams.set('month', params.month);
    if (params.toYear !== undefined) searchParams.set('to_year', params.toYear);
    if (params.toMonth !== undefined) searchParams.set('to_month', params.toMonth);
```

`frontend/src/components/PhotoGrid.svelte` — `buildFilters`:

```js
    if (route.year) filters.year = route.year;
    if (route.month) filters.month = route.month;
    if (route.to_year) filters.toYear = route.to_year;
    if (route.to_month) filters.toMonth = route.to_month;
```

and both dependency lists (dedupe signature at line 218 and the refetch `$effect` at 396-405) gain `route.to_year` / `route.to_month`.

- [ ] **Step 5: Carry the range through saved searches**

`frontend/src/components/SearchBar.svelte`:

```js
  const canSave = $derived(
    ['all', 'favorites', 'videos'].includes(route.view) &&
      !(
        route.view === 'all' &&
        !route.query &&
        route.sort === 'date_desc' &&
        !route.year &&
        !route.month &&
        !route.to_year &&
        !route.to_month
      )
  );

  function buildDefaultName() {
    const yearPart = route.year
      ? ` ${route.year}${route.month ? '-' + String(route.month).padStart(2, '0') : ''}`
      : '';
    const endPart = route.to_year
      ? `..${route.to_year}${route.to_month ? '-' + String(route.to_month).padStart(2, '0') : ''}`
      : '';
    return (
      ((route.query ?? '') + yearPart + endPart).trim() ||
      get(t)('savedSearches.defaultName', { default: 'Saved search' })
    );
  }
```

and the create payload gains `to_year: route.to_year, to_month: route.to_month`.

`frontend/src/components/Sidebar.svelte` — `isActiveSearch` compares `route.to_year === item.to_year && route.to_month === item.to_month`, and `openSavedSearch` pushes `to_year: item.to_year, to_month: item.to_month`.

`frontend/src/components/AlbumsView.svelte` line 105:

```js
    pushState({ album: item.id, view: 'all', query: null, year: null, month: null, to_year: null, to_month: null });
```

`frontend/src/components/TimelineSlider.svelte` `pushFilter` (the legacy rail still owns desktop until Task 7):

```js
  const pushFilter = () => {
    const year = currentFilter?.year ?? null;
    const month = currentFilter?.month ?? null;
    pushState({ year, month: year ? month : null, to_year: null, to_month: null });
  };
```

`frontend/src/App.svelte` comment at 64-66 becomes:

```js
  // different query is a different surface (semantic search results are one);
  // sort/date-range changes keep the same result set, so they do not clear.
```

- [ ] **Step 6: Run the E2E tests, then the full frontend gates**

Run: `npx playwright test tests/e2e/specs/url-routing.e2e.spec.js tests/e2e/specs/saved-searches.e2e.spec.js`
Expected: PASS (wait for the previous run's teardown before re-invoking).

Run: `npm run format:check && npm run lint && npm run test:i18n && npm run test:unit && npm run build && cargo build --bin turbo-pix`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/lib/router.svelte.js frontend/src/lib/api.js frontend/src/components/PhotoGrid.svelte frontend/src/components/SearchBar.svelte frontend/src/components/Sidebar.svelte frontend/src/components/AlbumsView.svelte frontend/src/components/TimelineSlider.svelte frontend/src/App.svelte tests/e2e/setup/test-helpers.js tests/e2e/specs/url-routing.e2e.spec.js tests/e2e/specs/saved-searches.e2e.spec.js
git commit -m "feat(timeline): carry the date range through route, API and saved searches"
```

---

### Task 7: Zoomable selector — full span, density, adaptive labels, zoom/pan

**Files:**
- Modify: `frontend/src/i18n/en.json`, `frontend/src/i18n/de.json`
- Modify: `tests/e2e/setup/global-setup.js` (decade-spanning fixture)
- Create: `frontend/src/components/TimelineSelector.svelte`
- Modify: `frontend/src/components/TimelineSlider.svelte` (container rewrite; delete the rail block, `:169-231` and rail styles `:342-427`)
- Modify: `tests/e2e/specs/timeline.e2e.spec.js` (replace the rail tests)
- Modify: `tests/e2e/specs/timeline-a11y.e2e.spec.js` (retarget selectors)

**Interfaces:**
- Consumes: everything above, plus `buildTimelineModel`/`formatSelectionLabel` (Task 3), layout functions (Task 4), route mapping (Task 5).
- Produces: `TimelineSelector` props `{ model, selection, onchange, resetNonce }`; `onchange(selection, { commit })` where `commit: false` is a live scrub (`replaceState`) and `commit: true` a committed action (`pushState`); `selection` is `{ startIndex, endIndex } | null`. DOM contract for tests: `.timeline-selector`, `.timeline-ruler`, `.timeline-lane`, `.timeline-column[data-period-start]`, `.timeline-column-bar`, `.timeline-column.active`, `.timeline-status`, `.timeline-zoom-in`, `.timeline-zoom-out`, `.timeline-fit-all`.

- [ ] **Step 1: Seed a decade-spanning fixture and write the failing E2E test**

In `tests/e2e/setup/global-setup.js` add, above `updateTestPhotoDates`:

```js
// Legacy seeds: five photos spread over six decades give the timeline real
// decade/year granularity, a populated gap-free modern cluster, and one empty
// decade (the 1990s) so gaps are exercised. Dates are written by
// updateTestPhotoDates() — the source image's own EXIF/mtime never matters.
const LEGACY_PHOTOS = [
  ['legacy_01.jpg', '1962-03-15T12:00:00.000Z'],
  ['legacy_02.jpg', '1974-09-02T12:00:00.000Z'],
  ['legacy_03.jpg', '1985-06-20T12:00:00.000Z'],
  ['legacy_04.jpg', '2004-11-05T12:00:00.000Z'],
  ['legacy_05.jpg', '2012-03-15T12:00:00.000Z'],
  ['legacy_06.jpg', '2019-07-01T12:00:00.000Z'],
];
```

In `seedTestMedia`, after the archive loop:

```js
  const legacySource = path.join('test-data', 'test_image_1.jpg');
  if (!existsSync(legacySource)) {
    throw new Error(`Missing legacy source image at ${legacySource}`);
  }
  for (const [filename] of LEGACY_PHOTOS) {
    const filePath = path.join(photosDir, filename);
    await copyFile(legacySource, filePath);
    await utimes(filePath, archiveDate, archiveDate);
  }
```

In `updateTestPhotoDates`, extend the SQL:

```js
  const legacySql = LEGACY_PHOTOS.map(
    ([filename, takenAt]) =>
      `UPDATE photos SET taken_at = '${takenAt}', updated_at = CURRENT_TIMESTAMP ` +
      `WHERE filename = '${filename}';`
  ).join(' ');

  const sql =
    `PRAGMA busy_timeout=5000; ` +
    `UPDATE photos SET taken_at = '${recentTakenAt}', updated_at = CURRENT_TIMESTAMP ` +
    `WHERE filename LIKE 'cluster_%'; ` +
    `UPDATE photos SET taken_at = '${archiveTakenAt}', updated_at = CURRENT_TIMESTAMP ` +
    `WHERE filename LIKE 'archive_%'; ` +
    legacySql;
```

Then replace the rail tests in `tests/e2e/specs/timeline.e2e.spec.js` with the selector tests:

```js
  test('should render the whole span with density columns and legible labels', async ({ page }) => {
    // GIVEN: a library spanning six decades
    const density = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((r) => r.json())
        .then((d) => d.density || [])
    );
    test.skip(density.length === 0, 'Timeline needs at least one month bucket');

    const lane = page.locator('.timeline-lane');
    await expect(lane).toBeVisible();

    // THEN: the selector needs no horizontal scrolling
    const scroll = await lane.evaluate((el) => ({ scrollWidth: el.scrollWidth, clientWidth: el.clientWidth }));
    expect(scroll.scrollWidth).toBe(scroll.clientWidth);

    // AND: every rendered label sits fully inside the ruler and never overlaps
    const labels = await page
      .locator('.timeline-ruler-label')
      .evaluateAll((els) => els.map((el) => el.getBoundingClientRect().toJSON()));
    const ruler = await page.locator('.timeline-ruler').boundingBox();
    expect(labels.length).toBeGreaterThan(0);
    for (let i = 0; i < labels.length; i += 1) {
      expect(labels[i].left).toBeGreaterThanOrEqual(ruler.x - 1);
      expect(labels[i].right).toBeLessThanOrEqual(ruler.x + ruler.width + 1);
      for (let j = i + 1; j < labels.length; j += 1) {
        const overlaps = labels[i].left < labels[j].right && labels[j].left < labels[i].right;
        expect(overlaps, `labels ${i} and ${j} overlap`).toBe(false);
      }
    }
  });

  test('should zoom with the wheel and the zoom controls, and fit back to the full span', async ({
    page,
  }) => {
    const span = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((r) => r.json())
        .then((d) => (d.density || []).length)
    );
    test.skip(span === 0, 'Timeline needs at least one month bucket');

    const columnCount = () => page.locator('.timeline-column').count();
    const fitAllCount = await columnCount();

    // WHEN: zooming in with the wheel over a column (not the empty background)
    const column = page.locator('.timeline-column').nth(1);
    const box = await column.boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.wheel(0, -400);
    await expect.poll(columnCount).toBeLessThan(fitAllCount);

    // WHEN: zooming back out with the control
    await page.click('.timeline-zoom-out');
    await expect.poll(columnCount).toBeGreaterThan(0);

    // WHEN: fitting all
    await page.click('.timeline-fit-all');
    await expect.poll(columnCount).toBe(fitAllCount);
  });

  test('should keep the selector off the page when the timeline fails to load', async ({ page }) => {
    // GIVEN: the timeline endpoint is down
    await page.route('**/api/photos/timeline', (route) =>
      route.fulfill({ status: 500, contentType: 'application/json', body: '{}' })
    );

    // WHEN: the user opens a deep link with an active range
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=8');
    await TestHelpers.waitForPhotosToLoad(page);

    // THEN: no selector renders and the filter is left alone (a load failure
    // must not be mistaken for "the data is gone" and wipe the selection)
    await expect(page.locator('.timeline-selector')).toHaveCount(0);
    const state = TestHelpers.getUrlState(page);
    expect(state.year).toBe(2012);
    expect(state.toMonth).toBe(8);
  });

  test('should clamp a restored selection to the library instead of hiding it', async ({ page }) => {
    // GIVEN: a range that starts before the library exists
    await page.goto('/?year=1900&month=4&to_year=2012&to_month=8');
    await TestHelpers.waitForPhotosToLoad(page);

    // THEN: the filter narrows to the overlap and the selection is on screen
    const state = TestHelpers.getUrlState(page);
    expect(state.year).toBe(1962);
    const selection = await page.locator('.timeline-selection').boundingBox();
    const lane = await page.locator('.timeline-lane').boundingBox();
    expect(selection.x).toBeGreaterThanOrEqual(lane.x - 1);
    expect(selection.x + selection.width).toBeLessThanOrEqual(lane.x + lane.width + 1);

    // AND: a range with no overlap clears the filter entirely
    await page.goto('/?year=1900&month=4&to_year=1901&to_month=6');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page).not.toHaveURL(/year=/);
  });

  test('should keep the selection visible when the window is resized across the breakpoint', async ({
    page,
  }) => {
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=8');
    await TestHelpers.waitForPhotosToLoad(page);

    // WHEN: the viewport narrows below the desktop breakpoint and back
    await TestHelpers.setMobileViewport(page);
    await expect(page.locator('#timeline-year-select')).toHaveCount(1);
    await TestHelpers.setDesktopViewport(page);

    // THEN: the selector returns with the selection inside its viewport
    await expect(page.locator('.timeline-selection')).toBeVisible();
    const selection = await page.locator('.timeline-selection').boundingBox();
    const lane = await page.locator('.timeline-lane').boundingBox();
    expect(selection.x).toBeGreaterThanOrEqual(lane.x - 1);
    expect(selection.x + selection.width).toBeLessThanOrEqual(lane.x + lane.width + 1);
  });
```

  test('should keep the visible span inside the data when panning the ruler', async ({ page }) => {
    const span = await page.evaluate(() =>
      fetch('/api/photos/timeline')
        .then((r) => r.json())
        .then((d) => (d.density || []).length)
    );
    test.skip(span < 2, 'Panning needs at least two buckets');

    const firstColumn = page.locator('.timeline-column').first();
    const firstStartBefore = await firstColumn.getAttribute('data-period-start');

    const ruler = await page.locator('.timeline-ruler').boundingBox();
    await page.mouse.move(ruler.x + ruler.width * 0.6, ruler.y + ruler.height / 2);
    await page.mouse.down();
    await page.mouse.move(ruler.x + ruler.width * 0.2, ruler.y + ruler.height / 2, { steps: 10 });
    await page.mouse.up();

    // Panning is clamped to the data span, so the first column can only move
    // forward in time, never before the library start.
    const firstStartAfter = await page
      .locator('.timeline-column')
      .first()
      .getAttribute('data-period-start');
    expect(Number(firstStartAfter)).toBeGreaterThanOrEqual(Number(firstStartBefore));
  });
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx playwright test tests/e2e/specs/timeline.e2e.spec.js`
Expected: FAIL — `.timeline-lane` never renders (the rail is still the desktop UI).

- [ ] **Step 3: Add the i18n keys**

`frontend/src/i18n/en.json`, inside `"ui"` after `"timeline_no_photos_month"`:

```json
"timeline_overview_label": "Timeline overview",
"timeline_fit_all": "Fit all",
"timeline_range_start": "Range start",
"timeline_range_end": "Range end",
"timeline_range_label": "{start} – {end}",
"timeline_span_summary": "{range} · {count} photos",
```

`frontend/src/i18n/de.json`, identical structure:

```json
"timeline_overview_label": "Zeitleisten-Übersicht",
"timeline_fit_all": "Alle anzeigen",
"timeline_range_start": "Bereichsanfang",
"timeline_range_end": "Bereichsende",
"timeline_range_label": "{start} – {end}",
"timeline_span_summary": "{range} · {count} Fotos",
```

Delete the now-unused `timeline_years_label` and `timeline_months_label` from **both** files.

- [ ] **Step 4: Write the selector component**

Create `frontend/src/components/TimelineSelector.svelte`. Structure (full file; keep the listed pieces exactly and follow the existing component style conventions):

```svelte
<script>
  import { locale } from 'svelte-i18n';
  import { t } from '../lib/i18n.js';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import {
    ZOOM_STEP,
    buildColumns,
    chooseUnit,
    clampBound,
    clampView,
    createView,
    ensureSelectionVisible,
    indexFromX,
    panView,
    placeLabels,
    selectionZoneAtX,
    translateSelection,
    xFromIndex,
    zoomToRange,
    zoomView,
  } from '../lib/timelineLayout.js';
  import {
    clampIndexToModel,
    countInRange,
    formatPeriodName,
    formatSelectionLabel,
  } from '../lib/timeline.js';
  import Icon from './Icon.svelte';

  let { model, selection, onchange, resetNonce = 0 } = $props();

  const activeLocale = $derived($locale || 'en');
  const LANE_HEIGHT = 56;
  const DRAG_THRESHOLD_PX = 4;

  let laneEl = $state(null);
  let rulerEl = $state(null);
  let probeEl = $state(null);
  let width = $state(0);
  let view = $state(null);
  let drag = $state(null);
  let hoveredColumn = $state(null);
  let focusedColumnStart = $state(null);
  let suppressClick = false;
  let measureContext = null;

  const monthName = (month) =>
    $t(`ui.months.${APP_CONSTANTS.MONTH_KEYS[month - 1]}`, {
      locale: activeLocale,
      default: APP_CONSTANTS.MONTH_KEYS[month - 1],
    });

  const periodName = (index) => formatPeriodName(index, monthName);

  const format = $derived({
    allDates: $t('ui.all_dates', { default: 'All Dates' }),
    monthName,
    periodName,
    rangeTemplate: (start, end) =>
      $t('ui.timeline_range_label', { values: { start, end }, default: '{start} – {end}' }),
  });

  const measure = (text) => {
    if (!probeEl) return text.length * 8;
    measureContext ??= document.createElement('canvas').getContext('2d');
    measureContext.font = getComputedStyle(probeEl).font;
    return measureContext.measureText(text).width;
  };

  const unit = $derived(chooseUnit(view?.scale ?? 1));
  const columns = $derived(
    view === null
      ? []
      : buildColumns({ unit, view, width, model, format, countInRange })
  );
  const placedColumns = $derived(placeLabels(columns, { width, measure }));
  const columnMax = $derived(columns.reduce((max, column) => Math.max(max, column.count), 0) || 1);
  const effectiveSelection = $derived(drag?.selection ?? selection);
  const statusText = $derived.by(() => {
    const column = hoveredColumn ?? columns.find((c) => c.gridStart === focusedColumnStart);
    if (column) {
      return `${periodName(column.gridStart)}, ${
        column.count === 0
          ? $t('ui.timeline_no_photos_month', { default: 'No photos' })
          : $t('ui.photos_count', { values: { count: column.count }, default: '{count} photos' })
      }`;
    }
    return formatSelectionLabel(effectiveSelection, format);
  });
  ...
</script>

<div class="timeline-selector" role="group" aria-label={$t('ui.timeline_overview_label', { default: 'Timeline overview' })}>
  <span class="timeline-ruler-label timeline-label-probe" bind:this={probeEl} aria-hidden="true">MMM 0000</span>

  <div
    class="timeline-ruler"
    bind:this={rulerEl}
    onpointerdown={startPan}
    onpointermove={handlePointerMove}
    onpointerup={endGesture}
    onpointercancel={endGesture}
    onwheel={handleWheel}
  >
    {#each placedColumns as column (column.gridStart)}
      {#if column.labelX !== null}
        <span class="timeline-ruler-label" style="left: {column.labelX}px">{column.label}</span>
      {/if}
    {/each}
  </div>

  <div
    class="timeline-lane"
    bind:this={laneEl}
    onpointerdown={startLaneGesture}
    onpointermove={handlePointerMove}
    onpointerup={endGesture}
    onpointercancel={endGesture}
    onwheel={handleWheel}
  >
    {#each placedColumns as column (column.gridStart)}
      <button
        type="button"
        class="timeline-column"
        class:active={effectiveSelection !== null
          && column.startIndex <= effectiveSelection.endIndex
          && column.endIndex >= effectiveSelection.startIndex}
        class:empty={column.count === 0}
        class:hovered={hoveredColumn?.gridStart === column.gridStart}
        data-period-start={column.gridStart}
        data-unit={unit}
        style="left: {column.x}px; width: {column.width}px;"
        aria-label={`${periodName(column.gridStart)}, ${
          column.count === 0
            ? $t('ui.timeline_no_photos_month', { default: 'No photos' })
            : $t('ui.photos_count', { values: { count: column.count }, default: '{count} photos' })
        }`}
        aria-pressed={effectiveSelection !== null
          && column.startIndex <= effectiveSelection.endIndex
          && column.endIndex >= effectiveSelection.startIndex}
        aria-disabled={column.count === 0}
        tabindex={column.gridStart === focusedColumnStart ? 0 : -1}
        onclick={() => activateColumn(column)}
        onkeydown={(event) => handleColumnKeydown(event, column)}
        onmouseenter={() => (hoveredColumn = column)}
        onmouseleave={() => (hoveredColumn = null)}
        onfocus={() => (hoveredColumn = column)}
        onblur={() => (hoveredColumn = null)}
      >
        <span
          class="timeline-column-bar"
          style="height: {column.count === 0
            ? 0
            : Math.max(3, Math.round((column.count / columnMax) * (LANE_HEIGHT - 12)))}px"
        ></span>
      </button>
    {/each}

    {#if effectiveSelection}
      <div
        class="timeline-selection"
        style="left: {xFromIndex(effectiveSelection.startIndex, view)}px; width: {(effectiveSelection.endIndex - effectiveSelection.startIndex + 1) * view.scale}px"
      >
        <!-- The handle drag attributes (`onpointerdown`) are added in Task 8 and
             the keyboard attribute (`onkeydown`) in Task 9; this task renders
             them as inert but fully announced elements so the ARIA contract and
             `.timeline-handle.start` / `.timeline-handle.end` selectors exist. -->
        <div
          class="timeline-handle start"
          role="slider"
          tabindex="0"
          aria-label={$t('ui.timeline_range_start', { default: 'Range start' })}
          aria-valuemin={model.minIndex}
          aria-valuemax={model.maxIndex}
          aria-valuenow={effectiveSelection.startIndex}
          aria-valuetext={periodName(effectiveSelection.startIndex)}
        ></div>
        <div
          class="timeline-handle end"
          role="slider"
          tabindex="0"
          aria-label={$t('ui.timeline_range_end', { default: 'Range end' })}
          aria-valuemin={model.minIndex}
          aria-valuemax={model.maxIndex}
          aria-valuenow={effectiveSelection.endIndex}
          aria-valuetext={periodName(effectiveSelection.endIndex)}
        ></div>
      </div>
    {/if}
  </div>

  <div class="timeline-footer">
    <div class="timeline-status" role="status" aria-live="polite">{statusText}</div>
    <div class="timeline-controls">
      <button type="button" class="timeline-control timeline-zoom-out" aria-label={$t('ui.zoom_out', { default: 'Zoom Out' })} onclick={zoomOut}>
        <Icon name="minus" width={16} height={16} />
      </button>
      <button type="button" class="timeline-control timeline-zoom-in" aria-label={$t('ui.zoom_in', { default: 'Zoom In' })} onclick={zoomIn}>
        <Icon name="plus" width={16} height={16} />
      </button>
      <button type="button" class="timeline-control timeline-fit-all" aria-label={$t('ui.timeline_fit_all', { default: 'Fit all' })} onclick={fitAll}>
        <Icon name="maximize" width={16} height={16} />
      </button>
    </div>
  </div>
</div>
```

Implementation rules the steps below pin down:

1. **View lifecycle.** `$effect` creating the `ResizeObserver` on `laneEl` (cleanup via the effect's return) sets `width` from `contentRect.width`. A second `$effect` re-clamps on `model` / `width` / `view` changes, but **only assigns when something actually changed** — `clampView` returns a fresh object, so an unconditional assign would re-trigger itself forever:

```js
  $effect(() => {
    const next = view === null ? createView(width, model) : clampView(view, width, model);
    if (view === null || view.scale !== next.scale || view.origin !== next.origin) {
      view = next;
    }
  });
```

The `resetNonce` effect is the opposite case: it must reset the view only when the nonce changes, so it reads the nonce and pulls the rest untracked (reading `width`/`model` as dependencies would reset the zoom on every resize):

```js
  $effect(() => {
    resetNonce;
    untrack(() => {
      drag = null;
      view = createView(width, model);
    });
  });
```
2. **Selection visibility.** `$effect` reading `selection`, `drag`, `view`, `width`, `model` first, then `if (drag !== null || view === null) return;` and finally the same change guard as above — a scrub in progress must not fight the pointer, and an already-visible selection must leave the view untouched (FR-010 + `ensureSelectionVisible`'s own no-op guarantee):

```js
  $effect(() => {
    const dragged = drag;
    const current = view;
    const visible = selection;
    const next = visible === null ? current : ensureSelectionVisible(visible, current, width, model);
    if (dragged !== null || current === null || next === current) return;
    if (next.scale !== current.scale || next.origin !== current.origin) view = next;
  });
```
3. **Wheel.** `onwheel` handler: `event.preventDefault()`; pan when `event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)` via `panView({ view, deltaPx: -(event.shiftKey ? event.deltaY : event.deltaX), width, model })`; otherwise `zoomView({ view, factor: Math.exp(-event.deltaY * 0.002), anchorPx: event.clientX - rect.left, width, model })`.
4. **Ruler drag = pan, column lane drag = brush/handles/translate.** `pointerdown` on the ruler enters `pan` with `baseOrigin: view.origin`; on the lane it resolves `selectionZoneAtX(x, { selection: effectiveSelection, view, width })` into `start` / `end` / `body` / `brush`. `setPointerCapture` on the lane element. `pointermove` applies `panView` / `clampBound` / `translateSelection` / `normalizeSelection(anchor, indexFromX(x, view))` and calls `onchange(next, { commit: false })` with the live selection; `pointerup` calls `onchange(final, { commit: true })` when the pointer moved more than `DRAG_THRESHOLD_PX` and sets `suppressClick = true` so the column's `click` handler ignores the gesture (Review Focus 1). `Escape` cancels back to the pre-drag selection.
5. **Activation.** Each column is a `<button class="timeline-column" data-period-start={column.gridStart} data-unit={unit} aria-label={...} aria-pressed={...} aria-disabled={column.count === 0}>` whose `click` handler runs, unless `suppressClick`: decade column (`unit === 120`) → `view = zoomToRange({ startIndex: column.gridStart, endIndex: column.gridStart + 119 }, width, model)` and no filter change; year column → `onchange({ startIndex: column.startIndex, endIndex: column.endIndex }, { commit: true })` **and** `view = zoomToRange({ startIndex: column.startIndex, endIndex: column.endIndex }, width, model)` (drill-in, so months become reachable in three interactions); month column → `onchange({ startIndex: column.startIndex, endIndex: column.endIndex }, { commit: true })`. Columns with `column.count === 0` are `aria-disabled="true"`, never `disabled` (they must stay focusable), render with `class:empty`, and their activation returns without changing the filter. `data-period-start` is the *grid-aligned* start (`column.gridStart`), so a clipped first column still reports the decade/year it belongs to and the E2E can compute expected month indices as `year * 12 + month - 1`.
6. **Density + selection painting.** Each column renders `<span class="timeline-column-bar" style="height:{...}px">` with `height = count === 0 ? 0 : Math.max(3, Math.round((count / columnMax) * (LANE_HEIGHT - 12)))`. The selection overlay is one absolutely positioned `.timeline-selection` div spanning `xFromIndex(effectiveSelection.startIndex)` → `xFromIndex(effectiveSelection.endIndex + 1)` plus the two `.timeline-handle` divs from the markup below, which this task renders with their full ARIA slider contract but without drag or keyboard handlers (Task 8 adds `onpointerdown`, Task 9 adds `onkeydown`) — nothing inert beyond that, so no stub code exists.
7. **Status row.** `.timeline-status` shows the hovered/focused column ("March 1998 · 12 photos" via `formatPeriodName` + `ui.photos_count`), else the selection label, else the span summary via `ui.timeline_span_summary` with `values: { range, count }`. `role="status"` + `aria-live="polite"`.
8. **Controls.** `.timeline-controls` holds `.timeline-zoom-out` (`Icon name="minus"`), `.timeline-zoom-in` (`Icon name="plus"`) and `.timeline-fit-all` (`Icon name="maximize"`), each `32px` square (`width: var(--space-8); height: var(--space-8)`), `aria-label={$t('ui.zoom_out')}` / `'ui.zoom_in'` / `'ui.timeline_fit_all'`, focus ring via the shared `:focus-visible` box-shadow rule, and `:global(svg)` sizing. Zoom buttons call `zoomView({ view, factor: ZOOM_STEP, anchorPx: width / 2, … })` / `1 / ZOOM_STEP`; fit-all assigns `createView(width, model)` **without** touching the selection.
9. **Styles.** `.timeline-selector { display: flex; flex-direction: column; gap: var(--space-2); width: 100%; }`; `.timeline-ruler { position: relative; height: var(--space-5); cursor: grab; touch-action: none; }` with `.timeline-ruler-label { position: absolute; top: 0; font-size: var(--font-sm); color: var(--text-secondary); white-space: nowrap; letter-spacing: normal; }`; `.timeline-lane { position: relative; height: 56px; display: block; overflow: hidden; touch-action: none; }`; `.timeline-column { position: absolute; bottom: 0; border: 0; background: transparent; padding: 0; display: flex; align-items: flex-end; justify-content: center; cursor: pointer; }`; `.timeline-column-bar { width: 100%; border-radius: var(--radius-sm) var(--radius-sm) 0 0; background: color-mix(in oklch, var(--primary-color) 22%, transparent); }`; `.timeline-column.empty .timeline-column-bar { background: var(--background-secondary); }`; `.timeline-column.active .timeline-column-bar, .timeline-column.hovered .timeline-column-bar { background: color-mix(in oklch, var(--primary-color) 45%, transparent); }`; `.timeline-selection { position: absolute; bottom: 0; top: 0; background: color-mix(in oklch, var(--primary-color) 14%, transparent); border: 1px solid var(--primary-color); border-radius: var(--radius-sm); pointer-events: none; }`; `.timeline-handle { position: absolute; top: 0; bottom: 0; width: 12px; pointer-events: auto; cursor: ew-resize; }`; `.timeline-footer { display: flex; align-items: center; gap: var(--space-3); }` with `.timeline-status { flex: 1; font-size: var(--font-xs); color: var(--text-secondary); min-height: var(--space-5); }` and `.timeline-controls { display: flex; gap: var(--space-2); margin-left: auto; }`; `.timeline-label-probe { position: absolute; visibility: hidden; pointer-events: none; }`; `.timeline-control { width: var(--space-8); height: var(--space-8); display: flex; align-items: center; justify-content: center; border: 1px solid var(--divider-color); border-radius: var(--radius-full); background: transparent; color: var(--text-secondary); cursor: pointer; transition: border-color var(--transition-fast), color var(--transition-fast), background-color var(--transition-fast); }` with `.timeline-control:hover { border-color: var(--primary-color); color: var(--primary-color); background: color-mix(in oklch, var(--primary-color) 10%, transparent); }` and `.timeline-selector :global(svg) { width: 16px; height: 16px; }`. Add a `@media (prefers-reduced-motion: reduce)` block that disables the transitions.

- [ ] **Step 5: Rewrite the container**

`frontend/src/components/TimelineSlider.svelte` keeps `fetchTimelineData`, the `!data` skeleton and the `aggregates.length === 0` empty path, and replaces the desktop rail with the selector:

```svelte
    <div class="timeline-container">
      <!-- Desktop: zoomable overview -->
      <div class="timeline-rail desktop-only">
        <div class="timeline-header">
          <div class="timeline-label" class:filtered={selection !== null}>{labelText}</div>
          <button
            type="button"
            class="timeline-reset"
            title={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
            aria-label={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
            onclick={clearFilter}
          >
            <Icon name="x" width={14} height={14} />
          </button>
        </div>
        <TimelineSelector {model} {selection} onchange={handleChange} {resetNonce} />
      </div>

      <!-- Mobile: Dropdowns (unchanged behaviour; a desktop range collapses to
           its start period when one of these is used) -->
      <div class="timeline-dropdowns mobile-only">
        <select
          id="timeline-year-select"
          class="timeline-year-select"
          bind:this={yearSelectEl}
          aria-label={$t('ui.year_select', { default: 'Year' })}
          value={filter.year === null ? '' : String(filter.year)}
          onchange={handleDropdownChange}
        >
          <option value="">{$t('ui.all_years', { default: 'All Years' })}</option>
          {#each model.years as year (year)}
            <option value={String(year)}>{year}</option>
          {/each}
        </select>
        <select
          id="timeline-month-select"
          class="timeline-month-select"
          bind:this={monthSelectEl}
          aria-label={$t('ui.month_select', { default: 'Month' })}
          disabled={filter.year === null}
          value={filter.month === null ? '' : String(filter.month)}
          onchange={handleDropdownChange}
        >
          <option value="">{$t('ui.all_months', { default: 'All Months' })}</option>
          {#each APP_CONSTANTS.MONTH_KEYS as monthKey, i (i)}
            <option value={String(i + 1)}>{$t(`ui.months.${monthKey}`, { default: monthKey })}</option>
          {/each}
        </select>
        <button
          type="button"
          class="timeline-reset"
          title={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
          aria-label={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
          onclick={clearFilter}
        >
          <Icon name="x" width={14} height={14} />
        </button>
      </div>
    </div>
```

Script side of the container — imports first (replacing the current `route`/`pushState` and library imports):

```js
  import { untrack } from 'svelte';
  import { route, pushState, replaceState } from '../lib/router.svelte.js';
  import { buildTimelineModel, formatPeriodName, formatSelectionLabel } from '../lib/timeline.js';
  import {
    filterEquals,
    filterFromSelection,
    normalizeDateFilter,
    selectionFromFilter,
  } from '../lib/timelineRoute.js';
  import TimelineSelector from './TimelineSelector.svelte';
```

then the state:

```js
  const model = $derived(buildTimelineModel(data?.density ?? []));
  const filter = $derived(
    normalizeDateFilter({
      year: route.year,
      month: route.month,
      to_year: route.to_year,
      to_month: route.to_month,
    })
  );
  const selection = $derived(selectionFromFilter(filter, model));
  const labelText = $derived.by(() => {
    const monthName = (month) =>
      $t(`ui.months.${APP_CONSTANTS.MONTH_KEYS[month - 1]}`, {
        locale: activeLocale,
        default: APP_CONSTANTS.MONTH_KEYS[month - 1],
      });
    return formatSelectionLabel(selection, {
      allDates: $t('ui.all_dates', { locale: activeLocale, default: 'All Dates' }),
      monthName,
      periodName: (index) => formatPeriodName(index, monthName),
      rangeTemplate: (start, end) =>
        $t('ui.timeline_range_label', { values: { start, end }, default: '{start} – {end}' }),
    });
  });

  const LIVE_COMMIT_MS = 100;
  let resetNonce = $state(0);
  let liveTimer = null;
  let pendingLive = null;

  // Live scrubbing must not spam history or the grid: the overlay follows the
  // pointer immediately, the route (and therefore the grid) trails by <=100 ms.
  const handleChange = (next, { commit = true } = {}) => {
    if (commit) {
      if (liveTimer !== null) {
        clearTimeout(liveTimer);
        liveTimer = null;
        pendingLive = null;
      }
      pushState(filterFromSelection(next));
      return;
    }
    pendingLive = next;
    liveTimer ??= setTimeout(() => {
      liveTimer = null;
      const pending = pendingLive;
      pendingLive = null;
      if (pending) replaceState(filterFromSelection(pending));
    }, LIVE_COMMIT_MS);
  };

  const clearFilter = () => {
    resetNonce += 1;
    pushState({ year: null, month: null, to_year: null, to_month: null });
  };

  // Mobile dropdowns: the plan shape is a single period, so using either
  // dropdown collapses an active desktop range to its start period.
  let yearSelectEl = $state(null);
  let monthSelectEl = $state(null);

  const handleDropdownChange = () => {
    const year = yearSelectEl?.value ? parseInt(yearSelectEl.value, 10) : null;
    const month = year !== null && monthSelectEl?.value ? parseInt(monthSelectEl.value, 10) : null;
    pushState({ year, month, to_year: null, to_month: null });
  };
```

The `value={...}` attribute keeps both selects in sync with the route; the refs exist only for reading on change.

Delete with the rail: `currentFilter`, `selectedYear`, `selectYear`, `selectMonth`, `pushFilter`, the `restoreFilterFromRoute` effect, the `aggregates`/`years`/`selectedAggregate` adapter from Task 3, the `.timeline-year*` / `.timeline-month*` / `.timeline-year-rail` / `.timeline-month-strip` CSS, and the now-unused `pushState`-only import. Update the container's own styles so the desktop block stacks:

```css
  .timeline-rail.desktop-only {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    width: 100%;
  }

  .timeline-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
```

(the `@media (width <= 768px)` block keeps swapping `.desktop-only` / `.mobile-only` exactly as today).

Finally, the clamp effect that makes Scenario 3.3 real — the route is rewritten to the clamped, canonical filter, but only once the density has actually loaded (a failed or in-flight fetch must never be mistaken for "the data is gone" and wipe a restored filter):

```js
  // Reads every dependency before the guard (AGENTS.md #2): the effect must
  // stay subscribed to route and model changes.
  $effect(() => {
    const loaded = data !== null;
    const current = filter;
    const canonical = filterFromSelection(selection);
    untrack(() => {
      if (!loaded || filterEquals(current, canonical)) return;
      replaceState(canonical);
    });
  });
```

- [ ] **Step 6: Retarget the a11y spec and delete the obsolete rail tests**

`tests/e2e/specs/timeline-a11y.e2e.spec.js`: replace the desktop wait with

```js
  await expect(page.locator('.timeline-column').first()).toBeVisible();
```

and keep the mobile test's `#timeline-year-select` assertion untouched.

Delete the rail-specific tests (`should display timeline controls`, `should show date range when timeline is available`, `should filter to year then month in two clicks`, `should render 50+ sparse years with zero label overlap`, `should disable empty months and clear via reset`) from `tests/e2e/specs/timeline.e2e.spec.js` — they are replaced by the tests written in Step 1 and Task 8/9. Keep `beforeEach` as is.

- [ ] **Step 7: Run the tests**

Run: `npm run format:check && npm run lint && npm run test:i18n && npm run test:unit && npm run build && cargo build --bin turbo-pix`
Expected: all green.

Run: `npx playwright test tests/e2e/specs/timeline.e2e.spec.js tests/e2e/specs/timeline-a11y.e2e.spec.js`
Expected: PASS.

- [ ] **Step 8: Manual smoke test the visual result**

```bash
nohup cargo run & 
until curl -sf --retry 5 --retry-delay 2 http://localhost:18473/health >/dev/null; do sleep 1; done
```

Open `http://localhost:18473`, confirm: the whole span is visible without scrolling, the density profile is visible and subtle, labels never collide while zooming with the wheel over the columns, `+`/`−`/fit-all behave, the ruler drag pans and stops at the ends. Save screenshots to `test-results/verification/` (gitignored) and kill the server (`pkill -f 'target/debug/turbo-pix'`).

- [ ] **Step 9: Commit**

```bash
git add frontend/src/i18n/en.json frontend/src/i18n/de.json frontend/src/components/TimelineSelector.svelte frontend/src/components/TimelineSlider.svelte tests/e2e/setup/global-setup.js tests/e2e/specs/timeline.e2e.spec.js tests/e2e/specs/timeline-a11y.e2e.spec.js
git commit -m "feat(timeline): replace the desktop rail with a zoomable overview"
```

---

### Task 8: Selection — activation, drill-in, brush, handles, translation

**Files:**
- Modify: `frontend/src/components/TimelineSelector.svelte` (activation, gestures, handles, live selection)
- Modify: `tests/e2e/specs/timeline.e2e.spec.js`

**Interfaces:**
- Consumes: the selector contract from Task 7.
- Produces: the behaviours the spec's Scenarios 1-3 depend on, expressed only through `onchange(selection, { commit })`.

- [ ] **Step 1: Write the failing E2E tests**

Append to `tests/e2e/specs/timeline.e2e.spec.js`:

```js
  test('should select any month in three interactions from the full span', async ({ page }) => {
    // GIVEN: the decade-spanning fixture, starting from a cleared filter
    await expect(page.locator('.timeline-column').first()).toBeVisible();
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '120');

    // The calendar mapping is asserted through the labels: the full span shows
    // decade labels, and March 1962 is 1962 * 12 + 2 = 23546.
    await expect(page.locator('.timeline-ruler-label', { hasText: '1960s' })).toHaveCount(1);

    // WHEN: drilling into the 1960s by activating the decade column
    await page.locator('.timeline-column[data-period-start="23520"]').click();

    // THEN: years appear and nothing is filtered yet
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');
    expect(TestHelpers.getUrlState(page).year).toBeNull();

    // WHEN: activating 1962 (23544 === 1962 * 12)
    await page.locator('.timeline-column[data-period-start="23544"]').click();
    await TestHelpers.waitForUrlParam(page, 'year', '1962');

    // THEN: months appear and the year filter is active
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');

    // WHEN: activating March (23546 === 1962 * 12 + 2)
    await page.locator('.timeline-column[data-period-start="23546"]').click();
    await TestHelpers.waitForUrlParam(page, 'month', '3');

    // THEN: the grid shows exactly the one seeded March 1962 photo
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.photo-card')).toHaveCount(1);
  });

  test('should set an inclusive month-granular range with one drag', async ({ page }) => {
    // GIVEN: the 2012 year view (legacy_05 seeded in March 2012)
    await page.goto('/?year=2012');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');

    // WHEN: dragging from February to March (24145 → 24146; January would be
    // canonicalised into the whole-year form and blur the assertion)
    const lane = await page.locator('.timeline-lane').boundingBox();
    const february = await page.locator('.timeline-column[data-period-start="24145"]').boundingBox();
    const march = await page.locator('.timeline-column[data-period-start="24146"]').boundingBox();
    await page.mouse.move(february.x + february.width / 2, lane.y + lane.height / 2);
    await page.mouse.down();
    await page.mouse.move(march.x + march.width / 2, lane.y + lane.height / 2, { steps: 8 });
    await page.mouse.up();

    // THEN: both bounds are written, inclusive, and no drill-in click fired
    const state = TestHelpers.getUrlState(page);
    expect(state.year).toBe(2012);
    expect(state.month).toBe(2);
    expect(state.toYear).toBe(2012);
    expect(state.toMonth).toBe(3);
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.photo-card')).toHaveCount(1);
  });

  test('should adjust a range bound by dragging its handle', async ({ page }) => {
    await page.goto('/?year=2012&month=2&to_year=2012&to_month=3');
    await TestHelpers.waitForPhotosToLoad(page);

    // WHEN: dragging the start handle one month further right (Feb → Mar would
    // cross the end, so clampBound holds it at March)
    const lane = await page.locator('.timeline-lane').boundingBox();
    const startHandle = await page.locator('.timeline-handle.start').boundingBox();
    const march = await page.locator('.timeline-column[data-period-start="24146"]').boundingBox();
    await page.mouse.move(startHandle.x + startHandle.width / 2, lane.y + lane.height / 2);
    await page.mouse.down();
    await page.mouse.move(march.x + march.width / 2, lane.y + lane.height / 2, { steps: 6 });
    await page.mouse.up();

    // THEN: the start moved, the end stayed, and the range never inverted
    const state = TestHelpers.getUrlState(page);
    expect(state.month).toBe(3);
    expect(state.toMonth).toBe(3);
  });

  test('should ignore activation of an empty period and clear back to the full span', async ({ page }) => {
    await page.goto('/?year=2012');
    await TestHelpers.waitForPhotosToLoad(page);

    // April 2012 (24147) has no photos: activating it must not filter
    await page.locator('.timeline-column[data-period-start="24147"]').click();
    const state = TestHelpers.getUrlState(page);
    expect(state.month).toBeNull();
    expect(state.toMonth).toBeNull();

    // WHEN: clearing from a zoomed, filtered state
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=3');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.click('.timeline-reset');

    // THEN: unfiltered and back to the full span in one action
    await expect(page).not.toHaveURL(/year=/);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '120');
  });
```

Month indices are `year * 12 + month - 1` (March 1962 → `23546`, February 2012 → `24145`, April 2012 → `24147`); `data-period-start` carries the **grid-aligned** start, so the 1960s decade column is `1960 * 12 = 23520` even though the library's first photo is from March 1962.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx playwright test tests/e2e/specs/timeline.e2e.spec.js`
Expected: FAIL — activation/drag do nothing yet.

- [ ] **Step 3: Implement activation, drill-in, brush, handles and translation**

In `TimelineSelector.svelte` implement, exactly as specified in Task 7's Step 4 rules 4-5:

- `data-unit={unit}` on every column button (the E2E contract for granularity).
- Column `click`: `if (suppressClick) { suppressClick = false; return; }` then the decade / year / month branches; empty columns return immediately **before** any state change.
- Lane `pointerdown/move/up/cancel` implementing brush, handle drag (`clampBound`), translation (`translateSelection` with `round(deltaPx / view.scale)` months) and the drag-vs-click suppression flag.
- Ruler `pointerdown/move/up` implementing pan.
- `.timeline-selection` overlay from `effectiveSelection`, with `.timeline-handle.start` / `.timeline-handle.end` divs (`12px` wide, `cursor: ew-resize`, `pointer-events: auto` inside a `pointer-events: none` overlay) so the handles are the drag targets resolved by `selectionZoneAtX`; this task adds the `onpointerdown={(event) => startHandleGesture(event, 'start' | 'end')}` attribute the markup reserved for it.
- Hover/focus feedback: `mouseenter`/`focus` set `hoveredColumn`, `mouseleave`/`blur` clear it; the hovered or focused column gets `class:hovered` with a `color-mix(in oklch, var(--primary-color) 30%, transparent)` bar tint, and the status row shows its name and count, never touching the filter (FR-006).
- `Escape` during a drag restores the pre-drag selection via `onchange(baseSelection, { commit: true })`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npx playwright test tests/e2e/specs/timeline.e2e.spec.js`
Expected: PASS, including the Task 7 tests (regression: drag must not trigger a drill-in).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/TimelineSelector.svelte tests/e2e/specs/timeline.e2e.spec.js
git commit -m "feat(timeline): month-granular selection with drill-in, brush and handles"
```

---

### Task 9: Keyboard operation and accessibility

**Files:**
- Modify: `frontend/src/components/TimelineSelector.svelte` (roving tabindex, handle sliders, live region)
- Modify: `tests/e2e/specs/timeline.e2e.spec.js`
- Modify: `tests/e2e/specs/timeline-a11y.e2e.spec.js`

**Interfaces:**
- Consumes: the selector contract from Tasks 7-8.
- Produces: the same selection behaviours, reachable without a pointer, with every target announcing period name and count (FR-014, SC-005).

- [ ] **Step 1: Write the failing E2E tests**

Append to `tests/e2e/specs/timeline.e2e.spec.js`:

```js
  test('should select a year, a month and a range with the keyboard only', async ({ page }) => {
    await page.goto('/?year=2012');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');

    // Focus the roving column and read its announcement
    const column = page.locator('.timeline-column[tabindex="0"]');
    await column.focus();
    const name = await column.getAttribute('aria-label');
    expect(name).toMatch(/\d{4}/);
    expect(name).toMatch(/photos|No photos/);

    // Arrow keys move focus one month at a time
    await page.keyboard.press('ArrowRight');
    const focused = page.locator('.timeline-column:focus');
    await expect(focused).toHaveCount(1);

    // Enter applies exactly what a pointer click would
    const start = await focused.getAttribute('data-period-start');
    await page.keyboard.press('Enter');
    await TestHelpers.waitForPhotosToLoad(page);
    expect(TestHelpers.getUrlState(page).month).toBe((Number(start) % 12) + 1);

    // AND: Shift+Enter on a neighbouring column extends the selection into a range
    await page.keyboard.press('ArrowRight');
    const extendTo = await page.locator('.timeline-column:focus').getAttribute('data-period-start');
    await page.keyboard.press('Shift+Enter');
    await TestHelpers.waitForPhotosToLoad(page);
    const extended = TestHelpers.getUrlState(page);
    expect(extended.month).toBe((Number(start) % 12) + 1);
    expect(extended.toMonth).toBe((Number(extendTo) % 12) + 1);
  });

  test('should adjust both range bounds by keyboard, one month per activation', async ({ page }) => {
    await page.goto('/?year=2012&month=3&to_year=2012&to_month=5');
    await TestHelpers.waitForPhotosToLoad(page);

    // The start handle is a keyboard-reachable slider announcing its period
    const startHandle = page.locator('.timeline-handle.start');
    await startHandle.focus();
    expect(await startHandle.getAttribute('aria-label')).toContain('Range start');

    await page.keyboard.press('ArrowLeft');
    await TestHelpers.waitForPhotosToLoad(page);
    expect(TestHelpers.getUrlState(page).month).toBe(2);

    const endHandle = page.locator('.timeline-handle.end');
    await endHandle.focus();
    await page.keyboard.press('ArrowRight');
    await TestHelpers.waitForPhotosToLoad(page);
    expect(TestHelpers.getUrlState(page).toMonth).toBe(6);
  });

  test('should announce zoom, fit-all and clear with a visible focus ring', async ({ page }) => {
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);

    for (const selector of ['.timeline-zoom-in', '.timeline-zoom-out', '.timeline-fit-all', '.timeline-reset']) {
      const control = page.locator(selector).first();
      await expect(control).toHaveAttribute('aria-label', /.+/);
      await control.focus();
      const shadow = await control.evaluate((el) => getComputedStyle(el).boxShadow);
      expect(shadow).not.toBe('none');
    }
  });
```

In `tests/e2e/specs/timeline-a11y.e2e.spec.js`, before the axe scan, add a selection so the handles are part of the scanned tree:

```js
  await page.goto('/?year=2012&month=3&to_year=2012&to_month=8');
  await TestHelpers.waitForPhotosToLoad(page);
  await expect(page.locator('.timeline-handle.start')).toBeVisible();
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx playwright test tests/e2e/specs/timeline.e2e.spec.js tests/e2e/specs/timeline-a11y.e2e.spec.js`
Expected: FAIL — no handles exist yet, columns have no roving tabindex.

- [ ] **Step 3: Implement the keyboard model**

In `TimelineSelector.svelte`:

1. **Roving tabindex over columns.** `focusedColumnStart` state (default: the column containing `selection?.startIndex ?? model.minIndex`, snapping to the grid: `Math.floor(index / unit) * unit`). Only that column has `tabindex="0"`; all others `-1`. Effects that keep it inside the visible grid when `unit`/`view` change must read their dependencies before guarding.
2. **Arrow navigation.** `ArrowRight`/`ArrowLeft` move `focusedColumnStart` by `±unit` clamped to the model span, then `.focus()` the new column (keep a `columnEls` map keyed by `gridStart`) and ensure it is visible: `view = ensureSelectionVisible({ startIndex, endIndex: gridStart + unit - 1 }, view, width, model)`.
   `Home`/`End` jump to `model.minIndex`-aligned and `model.maxIndex`-aligned grid starts.
3. **Activation and range extension.** `Enter`/`Space` run the same activation function as `click` (extract it as `activateColumn(column)`). `Shift+Enter`/`Shift+Space` set the selection to `normalizeSelection(selection?.startIndex ?? column.startIndex, column.endIndex)` with `commit: true` — the keyboard path to range selection.
4. **Handles as sliders.** `.timeline-handle` divs get `role="slider"`, `tabindex="0"`, `aria-label={$t('ui.timeline_range_start')}` / `'ui.timeline_range_end'`, `aria-valuemin={model.minIndex}`, `aria-valuemax={model.maxIndex}`, `aria-valuenow`, `aria-valuetext={periodName + count}`. `ArrowLeft`/`ArrowRight` move the bound by one month through `clampBound`, `Home`/`End` jump to the model ends; every change calls `onchange(next, { commit: true })`.
5. **Live region.** `.timeline-status` carries `role="status"` and `aria-live="polite"`, and its text is hovered period → focused period → selection label → span summary, with `ui.photos_count` (or `ui.timeline_no_photos_month`) attached to every period announcement.
6. **Focus ring.** All interactive elements (columns, handles, controls, reset) use the shared `:focus-visible` box-shadow ring; `prefers-reduced-motion` disables transitions. Column buttons keep `aria-pressed={isSelected(column)}` and `aria-disabled="true"` for empty periods.

- [ ] **Step 4: Run the tests to verify they pass, then the gates**

Run: `npx playwright test tests/e2e/specs/timeline.e2e.spec.js tests/e2e/specs/timeline-a11y.e2e.spec.js`
Expected: PASS, including `target-size` and `color-contrast` clean on the selector.

Run: `npm run format:check && npm run lint && npm run test:i18n && npm run test:unit`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/TimelineSelector.svelte tests/e2e/specs/timeline.e2e.spec.js tests/e2e/specs/timeline-a11y.e2e.spec.js
git commit -m "feat(timeline): keyboard operation and announcements for the selector"
```

---

### Task 10: Learnings, docs and full-suite verification

**Files:**
- Modify: `AGENTS.md` (Learnings list, capped at 10 entries)
- Verification only for everything else

**Interfaces:**
- Consumes: the finished feature.
- Produces: updated project memory and a green full suite.

- [ ] **Step 1: Fold the learnings into `AGENTS.md`**

Do not append a new entry — merge into the existing ones (the list is capped at 10):

- Entry 2 (state & routing): the route now carries the range as `year`/`month` + `to_year`/`to_month`; the clamp effect in `TimelineSlider.svelte` rewrites the route with `replaceState` when the restored selection does not overlap the library; live scrubbing throttles to 100 ms while the overlay follows the pointer immediately.
- Entry 3 (scoped styles): the selector's geometry (ruler/lane heights, handle width) lives in `TimelineSelector.svelte`'s scoped styles; label collision avoidance is measured with `getComputedStyle(probeEl).font`, so the probe label must stay in the DOM.
- Entry 10 (E2E): the fixture library is seeded with `legacy_01..06.jpg` (1962-2019) so decade/year granularity and the three-interaction drill-in are reachable; their `taken_at` values come from `updateTestPhotoDates`, not from the source image's EXIF.

- [ ] **Step 2: Run the complete gate set**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
npm run format:check
npm run lint
npm run test:i18n
npm run test:unit
npm run build
cargo build --bin turbo-pix
npx playwright test
```

Expected: every command green, including the specs not touched by this feature (the shared fixture library changed, so a full E2E run is mandatory — if the first run fails, let the teardown settle and re-run once before suspecting a regression, per AGENTS.md #10).

- [ ] **Step 3: Manual acceptance pass on a real library**

Start the server, then walk the spec's scenarios by hand and record screenshots in `test-results/verification/`:

1. Cleared filter → whole span visible, no horizontal scrolling, label "All Dates".
2. Zoom toward March 1998 → activate it → grid filters to exactly March 1998, label names it, selection fully visible.
3. Drag a multi-year range, adjust both bounds, translate it, reverse a drag.
4. Reload, Back, Forward, resize, clear — selection and grid agree every time.
5. Keyboard-only pass: Tab into the selector, arrow to a period, Enter, Shift+Enter, handle arrows, zoom/fit/clear.
6. Drag on a touch-capable viewport: one-finger pan on the ruler, one-finger brush on the lane, two-finger pinch zoom.

Kill the server afterwards.

- [ ] **Step 4: Commit**

```bash
git add AGENTS.md
git commit -m "docs: fold timeline selector learnings into AGENTS.md"
```

---

## Self-Review Notes

- **Spec coverage:** FR-001/002/003/004 → Task 7 (full span, density, zoom/pan/fit); FR-005 → Tasks 4 and 7 (adaptive granularity + measured label placement); FR-006 → Task 8 (hover/focus highlight + count, filter untouched); FR-007 → Task 8 (activation rules + empty-period guard); FR-008/009 → Task 8 (brush, normalisation, handles, translation); FR-010 → Tasks 5 and 7 (clamping + `ensureSelectionVisible`); FR-011/012 → Task 6 (single source of truth, round-trip); FR-013 → Task 7/8 (clear cancels the gesture and resets the view via `resetNonce`); FR-014 → Task 9; FR-015 → Tasks 7-9 (keys in both dictionaries); FR-016 → Tasks 6-7 (mobile dropdowns kept, single-period semantics preserved); FR-017 → Task 7 (fetch error path and empty library unchanged); FR-018 → Task 4 (200 layouts of a 1200-month model under 100 ms) and Task 7 (bounded node count). Scenarios 1-5 map to the E2E tests in Tasks 7-9, and Success Criteria SC-001/SC-002/SC-003/SC-005/SC-006/SC-007 map to named tests there; SC-004 to Task 6 plus Task 10's manual pass; SC-008 to Task 4's timing test.
- **Type consistency:** `onchange(selection, { commit })`, `selection = { startIndex, endIndex } | null`, `model = { minIndex, maxIndex, length, counts, prefix, total, years }`, `view = { scale, origin }`, `filter = { year, month, to_year, to_month }` are used with the same shape in every task.
- **Deliberate gaps:** day-level granularity, album-photo date filtering, and the semantic-search path are out of scope (spec Assumptions);
- TimelineSlider's fetch effect still runs once on mount, so a background date-shift invalidates the density only on reload — unchanged from today.
