# Map View (OpenStreetMap Photos Map) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a URL-addressable Map view that plots every geo-located photo of the current filter state on a configurable OpenStreetMap tile layer, with clustering, popups, and handoff into the existing viewer.

**Architecture:** A new `GET /api/photos/map` endpoint returns the **entire** filtered, sorted photo set (no pagination) so the client can (a) derive marker locations, (b) count photos without coordinates, and (c) hand the same array to the photo viewer for next/previous parity with the grid. Leaflet 1.9 renders a configurable raster tile layer (URL served by `GET /api/config` from `TURBO_PIX_TILE_URL`, OSM default) with always-visible OSM attribution; supercluster 9 clusters the unique coordinate locations client-side and recomputes on pan/zoom. A new `MapView.svelte` mirrors `PhotoGrid`'s filter semantics (route-derived: q/year/month/sort/album, plus CLIP semantic search) and dispatches the existing `openViewer` window event.

**Tech Stack:** Rust + warp 0.4 + sqlx/SQLite (backend), Svelte 5 runes (frontend), Leaflet 1.9.4 + supercluster 9.1.0 (new npm deps), node:test unit tests, Playwright E2E.

**Spec:** `.spec/map-photos.md` — the plan implements FR-001…FR-020 and SC-001…SC-008; read it alongside each task.

## Global Constraints

- Breaking changes are allowed (pre-production), but the grid/viewer/sorting behavior must not change.
- Svelte 5 **runes only** (`$state`, `$derived`, `$effect`); components in `frontend/src/components/`, JS modules in `frontend/src/lib/`.
- i18n: every new user-visible string lands in **both** `frontend/src/i18n/en.json` and `de.json`; `npm run test:i18n` must pass; the App `titleKeys` regex inside `tests/i18n-integrity.test.js` must learn the new `map` view.
- Build order: `npm run build` **then** `cargo build --bin turbo-pix` (build.rs embeds `dist/` and panics without it).
- Zero warnings: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `npm run lint`, `npm run format:check` clean.
- No migration: GPS lives in the `photos.metadata` JSON blob (`$.location.latitude/longitude/city`) — no schema change.
- Rust: warp filter routes composed in `build_photo_routes`; sqlx runtime queries only (`sqlx::query_as`, `AssertSqlSafe` for dynamic SQL).
- OSM tile policy: no prefetching, no bulk download, visible attribution, configurable endpoint.
- E2E specs must stub tile requests with `page.route` (no external host dependency) and wait on selector state, never fixed timeouts.
- Commit per task with a conventional message.

## Review Focus

The five input classes/failure modes most likely to bite a user (each gets a test in its owning task):

1. **Malformed coordinates** — a photo whose `metadata.location` carries one coordinate, a non-numeric value, or an out-of-range value must be skipped, never plotted at `NaN`.
2. **Filtered set larger than one grid page** — >100 geo-located photos must all be represented (the paginated endpoint caps at 100; the map must not silently truncate).
3. **Unreachable/blocked tile endpoint** — markers, popups and attribution must still render on a plain background with a non-blocking notice (never a blank/frozen map).
4. **Many photos at one location** — the popup list must show every photo in current sort order, scrollable, not truncated.
5. **Keyboard-only operation** — Tab reaches a marker, Enter opens the popup, a thumbnail is focusable and Enter opens the viewer; focus is visibly indicated and restored on popup close.

## File Structure

**Created**

| File | Responsibility |
|---|---|
| `frontend/src/lib/map.js` | Pure map data helpers (coordinate extraction/validation, grouping by location, labels) + route→filter mapping + semantic result-set fetching. Node-testable. |
| `frontend/src/components/MapView.svelte` | The view: Leaflet map lifecycle, tile layer + attribution, supercluster rendering, notices/empty states, data loading, window-event handling. |
| `frontend/src/components/MapPopup.svelte` | Popup content: place name/coordinates, photo count, scrollable thumbnail list, viewer handoff. |
| `tests/map-aggregates.test.js` | node:test unit tests for `lib/map.js`. |
| `tests/e2e/specs/map.e2e.spec.js` | E2E: view registration, tiles/attribution, markers, clustering, popups, viewer handoff. |
| `tests/e2e/specs/map-filters.e2e.spec.js` | E2E: year/month filter, `location:` search, semantic search, album scoping, back/forward, videos with coordinates. |
| `tests/e2e/specs/map-a11y.e2e.spec.js` | E2E: keyboard flow, reduced motion, mobile attribution, axe rules, tile-failure degradation. |

**Modified**

| File | Change |
|---|---|
| `src/config.rs` | `tile_url` field + `TURBO_PIX_TILE_URL` env read + tests. |
| `src/handlers_config.rs` | `tile_url` in `ConfigResponse` + test. |
| `src/main.rs` | Pass `tile_url` into `build_config_routes`. |
| `src/db.rs` | Extract `build_search_where`, add `Photo::list_all_filtered`. |
| `src/handlers_photo.rs` | `MapPhotoQuery`, `MapPhotosResponse`, `list_map_photos`, `/api/photos/map` route + tests. |
| `frontend/src/lib/api.js` | `getMapPhotos(params, options)`. |
| `frontend/src/lib/state.svelte.js` | `appState.tileUrl`. |
| `frontend/src/lib/router.svelte.js` | `'map'` in `validViews`. |
| `frontend/src/App.svelte` | `titleKeys`/`titleFallbacks`, view branch, map-mode layout class, hide Select button, store `tile_url`. |
| `frontend/src/components/Sidebar.svelte` | New `map` nav entry. |
| `frontend/src/i18n/en.json`, `de.json` | New `ui.map` + `map.*` keys. |
| `tests/i18n-integrity.test.js` | `map` in the `titleKeys` regex. |
| `tests/e2e/setup/test-helpers.js` | `stubMapTiles`, `setPhotoCoordinates`, `setPhotoLocationInDb` helpers. |
| `package.json` / `package-lock.json` | `leaflet`, `supercluster` dependencies. |
| `.github/workflows/ci.yml` | Run the new unit test in the `lint-format` job. |
| `README.md` | Document `TURBO_PIX_TILE_URL`. |
| `AGENTS.md` | Learnings entry update (final task). |

---

### Task 1: Server-configurable tile endpoint

**Files:**
- Modify: `src/config.rs` (struct at :6-30, `from_env` at :32-98, tests at :119+)
- Modify: `src/handlers_config.rs` (whole file, 32 lines)
- Modify: `src/main.rs:142`
- Modify: `README.md` (env var table, ~:122-131)

**Interfaces:**
- Consumes: nothing.
- Produces: `Config.tile_url: String` (env `TURBO_PIX_TILE_URL`, default `https://tile.openstreetmap.org/{z}/{x}/{y}.png`); `GET /api/config` → `{"default_locale": "...", "tile_url": "..."}`; `build_config_routes(default_locale: String, tile_url: String)`.

- [ ] **Step 1: Write the failing config tests**

Add to `src/config.rs`'s `mod tests` (mirror the existing nominatim tests that use `with_env_lock` + `restore_env_var`):

```rust
    #[test]
    fn uses_default_tile_url_when_env_var_is_missing() {
        with_env_lock(|| {
            let original = std::env::var("TURBO_PIX_TILE_URL").ok();
            restore_env_var("TURBO_PIX_TILE_URL", None);
            let config = Config::from_env().expect("config should parse");
            assert_eq!(
                config.tile_url,
                "https://tile.openstreetmap.org/{z}/{x}/{y}.png"
            );
            restore_env_var("TURBO_PIX_TILE_URL", original);
        });
    }

    #[test]
    fn reads_custom_tile_url_from_env() {
        with_env_lock(|| {
            let original = std::env::var("TURBO_PIX_TILE_URL").ok();
            std::env::set_var(
                "TURBO_PIX_TILE_URL",
                "http://tiles.example.lan/{z}/{x}/{y}.png",
            );
            let config = Config::from_env().expect("config should parse");
            assert_eq!(config.tile_url, "http://tiles.example.lan/{z}/{x}/{y}.png");
            restore_env_var("TURBO_PIX_TILE_URL", original);
        });
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib config::tests::uses_default_tile_url -- --nocapture`
Expected: FAIL — `no field 'tile_url' on type 'Config'`.

- [ ] **Step 3: Implement the config field**

In `src/config.rs`: add `pub tile_url: String,` to `Config` (after `nominatim_url`), read it in `from_env` right after `nominatim_url`:

```rust
        // Operator-configurable raster tile endpoint (FR-002). The default is
        // the public OSM service; self-hosted/alternative OSM-compatible
        // endpoints need this env var only — no code change or rebuild.
        let tile_url = env::var("TURBO_PIX_TILE_URL")
            .unwrap_or_else(|_| "https://tile.openstreetmap.org/{z}/{x}/{y}.png".to_string());
```

and add `tile_url,` to the returned `Config { .. }` literal.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib config::tests`
Expected: PASS (all existing config tests too).

- [ ] **Step 5: Write the failing handler test**

Replace `src/handlers_config.rs`'s test with:

```rust
    #[tokio::test]
    async fn config_route_returns_default_locale_and_tile_url() {
        let routes = build_config_routes("de".to_string(), "http://tiles.lan/{z}/{x}/{y}.png".to_string());
        let res = warp::test::request()
            .path("/api/config")
            .reply(&routes)
            .await;
        assert_eq!(res.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(body["default_locale"], "de");
        assert_eq!(body["tile_url"], "http://tiles.lan/{z}/{x}/{y}.png");
    }
```

- [ ] **Step 6: Run to verify it fails**

Run: `cargo test --lib handlers_config`
Expected: FAIL — `build_config_routes` takes 1 argument.

- [ ] **Step 7: Implement the response field**

```rust
#[derive(Serialize)]
struct ConfigResponse {
    default_locale: String,
    tile_url: String,
}

pub fn build_config_routes(
    default_locale: String,
    tile_url: String,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("api" / "config").and(warp::get()).map(move || {
        warp::reply::json(&ConfigResponse {
            default_locale: default_locale.clone(),
            tile_url: tile_url.clone(),
        })
    })
}
```

Update the call site in `src/main.rs`:

```rust
    let config_routes = build_config_routes(config.locale.clone(), config.tile_url.clone());
```

- [ ] **Step 8: Run tests + gates**

Run: `cargo test --lib handlers_config && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 9: Document the env var**

Add a row to the README env table (next to `TURBO_PIX_NOMINATIM_URL`):

```markdown
| `TURBO_PIX_TILE_URL` | `https://tile.openstreetmap.org/{z}/{x}/{y}.png` | Raster tile endpoint (`{z}/{x}/{y}` placeholders) used by the Map view |
```

- [ ] **Step 10: Commit**

```bash
git add src/config.rs src/handlers_config.rs src/main.rs README.md
git commit -m "feat(map): make the OSM tile endpoint operator-configurable"
```

---

### Task 2: Backend — full filtered set query (`Photo::list_all_filtered`)

**Files:**
- Modify: `src/db.rs` (`search_photos` at :808-921; tests module at :1160+)
- Test: `src/db.rs` `mod tests`

**Interfaces:**
- Consumes: `SearchQuery` (`src/db_types.rs`), `build_order_clause` (existing, `src/db.rs:118`).
- Produces: private `fn build_search_where(query: &SearchQuery) -> (String, Vec<String>)` (WHERE clause starting `" WHERE 1=1"` + string params in placeholder order); `pub async fn Photo::list_all_filtered(pool: &DbPool, query: &SearchQuery, sort: Option<&str>, order: Option<&str>, album: Option<i64>) -> Result<Vec<Photo>, Box<dyn std::error::Error>>` (no LIMIT/OFFSET; optional album scoping via `album_members`).

- [ ] **Step 1: Write the failing tests**

Add to `src/db.rs`'s `mod tests` (uses the existing `create_test_photo`, `create_test_db_pool`, `photo.create(&pool)`):

```rust
    #[tokio::test]
    async fn test_list_all_filtered_returns_every_match_without_pagination() {
        let pool = create_test_db_pool().await.unwrap();

        for index in 0..120 {
            let mut photo =
                create_test_photo(format!("bulk_{index}.jpg"), format!("bulk{index}"));
            photo.metadata = json!({
                "location": { "latitude": 48.1, "longitude": 11.5 }
            });
            photo.create(&pool).await.unwrap();
        }

        let photos = Photo::list_all_filtered(
            &pool,
            &SearchQuery {
                q: None,
                year: None,
                month: None,
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

        let mut berlin_2020 = create_test_photo_with_date(
            &"1".repeat(64),
            "berlin_2020.jpg",
            DateTime::parse_from_rfc3339("2020-05-25T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        berlin_2020.metadata = json!({ "location": { "city": "Berlin" } });
        berlin_2020.create(&pool).await.unwrap();

        let mut berlin_2024 = create_test_photo_with_date(
            &"2".repeat(64),
            "berlin_2024.jpg",
            DateTime::parse_from_rfc3339("2024-05-25T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        berlin_2024.metadata = json!({ "location": { "city": "Berlin" } });
        berlin_2024.create(&pool).await.unwrap();

        let mut rome = create_test_photo_with_date(
            &"3".repeat(64),
            "rome.jpg",
            DateTime::parse_from_rfc3339("2020-05-25T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        rome.metadata = json!({ "location": { "city": "Rome" } });
        rome.create(&pool).await.unwrap();

        let photos = Photo::list_all_filtered(
            &pool,
            &SearchQuery {
                q: Some("location:Berlin".to_string()),
                year: Some(2020),
                month: None,
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

        let mut in_album = create_test_photo(format!("in_album.jpg"), "inalbum".to_string());
        in_album.metadata = json!({ "location": { "latitude": 1.0, "longitude": 2.0 } });
        in_album.create(&pool).await.unwrap();

        let mut outside = create_test_photo(format!("outside.jpg"), "outside".to_string());
        outside.create(&pool).await.unwrap();

        let album = crate::albums::create_with_members(
            &pool,
            "Trip",
            &[in_album.hash_sha256.clone()],
        )
        .await
        .unwrap();

        let photos = Photo::list_all_filtered(
            &pool,
            &SearchQuery {
                q: None,
                year: None,
                month: None,
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
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib db::tests::test_list_all_filtered -- --nocapture`
Expected: FAIL — no method `list_all_filtered`.

- [ ] **Step 3: Extract the WHERE builder**

In `src/db.rs`, move the body of `search_photos` that builds `where_clause`/`params` (from `let mut where_clause = String::from(" WHERE 1=1");` through the month clause) into a module-level function placed directly above `impl Photo`'s search section:

```rust
/// Builds the reusable WHERE clause for photo searches: the `q` token grammar
/// (`type:` / `is_favorite:` / `location:` / general LIKE), `year` and `month`.
/// Returns the clause (starting with `" WHERE 1=1"`) plus its string parameters
/// in placeholder order. Shared by `Photo::search_photos` and
/// `Photo::list_all_filtered` so both honor identical filter semantics.
fn build_search_where(query: &SearchQuery) -> (String, Vec<String>) {
    let mut where_clause = String::from(" WHERE 1=1");
    let mut params: Vec<String> = Vec::new();
    // ... verbatim body cut out of `search_photos` (q tokens, year, month) ...
    (where_clause, params)
}
```

`search_photos` then starts with:

```rust
        let (where_clause, params) = build_search_where(query);
```

and keeps its COUNT + `LIMIT ? OFFSET ?` queries unchanged. Run the existing `db::tests` search tests to prove the extraction is behavior-preserving.

- [ ] **Step 4: Add the unpaginated query**

In `impl Photo` (next to `search_photos`):

```rust
    /// Returns every photo matching `query` (optionally scoped to one album)
    /// with no pagination. The Map view needs the complete filtered set: it
    /// plots the geo-located subset and hands the same sorted array to the
    /// viewer so next/previous span exactly the grid's result set
    /// (FR-005, FR-011). Album scoping mirrors the grid's album detail view,
    /// which ignores q/year/month.
    pub async fn list_all_filtered(
        pool: &DbPool,
        query: &SearchQuery,
        sort: Option<&str>,
        order: Option<&str>,
        album: Option<i64>,
    ) -> Result<Vec<Photo>, Box<dyn std::error::Error>> {
        let (mut where_clause, params) = build_search_where(query);

        if album.is_some() {
            where_clause.push_str(
                " AND hash_sha256 IN (SELECT photo_hash FROM album_members WHERE album_id = ?)",
            );
        }

        let sql = format!(
            "SELECT * FROM photos{} ORDER BY {}",
            where_clause,
            build_order_clause(sort, order)
        );

        let mut data_query = sqlx::query_as::<_, Photo>(sqlx::AssertSqlSafe(sql));
        for param in &params {
            data_query = data_query.bind(param);
        }
        if let Some(album_id) = album {
            data_query = data_query.bind(album_id);
        }

        Ok(data_query.fetch_all(pool).await?)
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib db::tests`
Expected: PASS (new tests + all pre-existing search tests).

- [ ] **Step 6: Gates + commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/db.rs
git commit -m "feat(map): add unpaginated filtered photo query"
```

---

### Task 3: Backend — `GET /api/photos/map`

**Files:**
- Modify: `src/handlers_photo.rs` (structs near `PhotoQuery` at :31; handler after `list_photos` at :116; route in `build_photo_routes` at :1070; tests at :1273+)
- Test: `src/handlers_photo.rs` `mod tests`

**Interfaces:**
- Consumes: `Photo::list_all_filtered` (Task 2), `SearchQuery`, `with_db`, `DatabaseError`.
- Produces: `GET /api/photos/map?q=&year=&month=&sort=&order=&album=` → `{"photos": [Photo, ...]}` (200; sorted; no pagination; `album` scopes to that album's members).

- [ ] **Step 1: Write the failing handler tests**

In `src/handlers_photo.rs`'s `mod tests` (uses the existing `build_test_routes` helper):

```rust
    #[tokio::test]
    async fn test_map_photos_returns_all_matches_beyond_page_limit() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let routes = build_test_routes(db_pool.clone(), PathBuf::from("/tmp/turbo-pix-test-cache"));

        for index in 0..120 {
            let mut photo = crate::db::tests::create_test_photo(
                format!("map_{index}.jpg"),
                format!("map{index}"),
            );
            photo.metadata = json!({ "location": { "latitude": 48.1, "longitude": 11.5 } });
            photo.create(&db_pool).await.unwrap();
        }

        let response = warp::test::request()
            .path("/api/photos/map")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["photos"].as_array().unwrap().len(), 120);
        // The map listing is not a page: coordinates travel with the payload.
        assert_eq!(body["photos"][0]["metadata"]["location"]["latitude"], 48.1);
    }

    #[tokio::test]
    async fn test_map_photos_applies_filters_and_sort() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let routes = build_test_routes(db_pool.clone(), PathBuf::from("/tmp/turbo-pix-test-cache"));

        let mut older = crate::db::tests::create_test_photo_with_date(
            &"a".repeat(64),
            "older.jpg",
            chrono::DateTime::parse_from_rfc3339("2020-05-25T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        older.metadata = json!({ "location": { "city": "Berlin", "latitude": 52.5, "longitude": 13.4 } });
        older.create(&db_pool).await.unwrap();

        let mut newer = crate::db::tests::create_test_photo_with_date(
            &"b".repeat(64),
            "newer.jpg",
            chrono::DateTime::parse_from_rfc3339("2024-05-25T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        newer.metadata = json!({ "location": { "city": "Berlin", "latitude": 52.5, "longitude": 13.4 } });
        newer.create(&db_pool).await.unwrap();

        let response = warp::test::request()
            .path("/api/photos/map?q=location%3ABerlin&sort=date&order=asc")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        let photos = body["photos"].as_array().unwrap();
        assert_eq!(photos.len(), 2);
        assert_eq!(photos[0]["filename"], "older.jpg");
        assert_eq!(photos[1]["filename"], "newer.jpg");
    }

    #[tokio::test]
    async fn test_map_photos_scopes_to_album() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let routes = build_test_routes(db_pool.clone(), PathBuf::from("/tmp/turbo-pix-test-cache"));

        let mut member = crate::db::tests::create_test_photo("member.jpg".to_string(), "member".to_string());
        member.create(&db_pool).await.unwrap();
        let mut stranger = crate::db::tests::create_test_photo("stranger.jpg".to_string(), "stranger".to_string());
        stranger.create(&db_pool).await.unwrap();

        let album = crate::albums::create_with_members(&db_pool, "Trip", &[member.hash_sha256.clone()])
            .await
            .unwrap();

        let response = warp::test::request()
            .path(&format!("/api/photos/map?album={}", album.id))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        let photos = body["photos"].as_array().unwrap();
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0]["filename"], "member.jpg");
    }

    #[tokio::test]
    async fn test_map_photos_returns_empty_array_for_empty_library() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let routes = build_test_routes(db_pool, PathBuf::from("/tmp/turbo-pix-test-cache"));

        let response = warp::test::request()
            .path("/api/photos/map")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["photos"].as_array().unwrap().len(), 0);
    }
```

Note: `crate::db::tests` helpers are private to `db.rs` — if they are not `pub(crate)`, mark `create_test_photo`, `create_test_photo_with_date`, `create_test_db_pool` as `pub(crate)` inside the test module (the existing `handlers_photo` tests already reuse `create_in_memory_pool`).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib handlers_photo::tests::test_map_photos -- --nocapture`
Expected: FAIL — 404 (route not found).

- [ ] **Step 3: Implement the query structs + handler**

Add near `PhotoQuery` in `src/handlers_photo.rs`:

```rust
/// Query for the Map view listing: the entire filtered photo set, unpaginated
/// (FR-005). `album` scopes to an album's members, mirroring the grid's album
/// detail listing.
#[derive(Debug, Deserialize)]
pub struct MapPhotoQuery {
    pub sort: Option<String>,
    pub order: Option<String>,
    pub q: Option<String>,
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub album: Option<i64>,
}

#[derive(Debug, serde::Serialize)]
pub struct MapPhotosResponse {
    pub photos: Vec<Photo>,
}

pub async fn list_map_photos(
    query: MapPhotoQuery,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let search_query = SearchQuery {
        q: query.q.clone(),
        year: query.year,
        month: query.month,
    };

    match Photo::list_all_filtered(
        &db_pool,
        &search_query,
        query.sort.as_deref(),
        query.order.as_deref(),
        query.album,
    )
    .await
    {
        Ok(photos) => Ok(warp::reply::json(&MapPhotosResponse { photos })),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}
```

- [ ] **Step 4: Register the route (literal before the `{hash}` param route)**

In `build_photo_routes`, directly after `api_photo_timeline`:

```rust
    // Literal sub-paths (`/map`, `/timeline`, `/batch/...`) must be registered
    // BEFORE the parameterized `api_photo_get` route, otherwise `map` would be
    // captured as a photo hash (same rule as the NOTE below).
    let api_photos_map = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path("map"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<MapPhotoQuery>())
        .and(with_db(db_pool.clone()))
        .and_then(list_map_photos);
```

and add it to the `.or(...)` chain right after `api_photos_list`:

```rust
    api_photos_list
        .or(api_photos_map)
        .or(api_photo_timeline)
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib handlers_photo` and then `cargo test`
Expected: PASS (all).

- [ ] **Step 6: Gates + commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/handlers_photo.rs src/db.rs
git commit -m "feat(map): add unpaginated /api/photos/map listing"
```

---

### Task 4: Frontend data layer (`lib/map.js`, `api.getMapPhotos`, unit tests)

**Files:**
- Create: `frontend/src/lib/map.js`
- Create: `tests/map-aggregates.test.js`
- Modify: `frontend/src/lib/api.js` (add `getMapPhotos` next to `getPhotos` at :89)
- Modify: `.github/workflows/ci.yml` (lint-format job)

**Interfaces:**
- Consumes: `api.getPhotos`-style params; `isPrefixQuery` from `./utils.js`; `api.semanticSearch`, `api.getPhoto`.
- Produces (exact signatures, used by later tasks):
  - `getPhotoCoordinates(photo) -> { latitude, longitude } | null`
  - `groupPhotosByLocation(photos) -> Array<{ key, latitude, longitude, photos: Photo[] }>` (input order preserved)
  - `getLocationLabel(location) -> string | null`
  - `formatCoordinates({ latitude, longitude }) -> string` (6 decimals, comma separator)
  - `buildMapFilters(route) -> { query, sort, order, year, month }`
  - `isSemanticQuery(query) -> boolean`
  - `fetchSemanticPhotoSet(query, { signal } = {}) -> Promise<Photo[]>`
  - `api.getMapPhotos(params, options) -> Promise<{ photos: Photo[] }>`

- [ ] **Step 1: Write the failing unit tests**

Create `tests/map-aggregates.test.js` (node:test, mirroring `tests/timeline-aggregates.test.js`):

```js
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  buildMapFilters,
  fetchSemanticPhotoSet,
  formatCoordinates,
  getLocationLabel,
  getPhotoCoordinates,
  groupPhotosByLocation,
  isSemanticQuery,
} from '../frontend/src/lib/map.js';
import { api } from '../frontend/src/lib/api.js';

const photo = (hash, latitude, longitude, extra = {}) => ({
  hash_sha256: hash,
  metadata: { location: { latitude, longitude, ...extra } },
});

test('getPhotoCoordinates accepts a complete numeric pair', () => {
  assert.deepEqual(getPhotoCoordinates(photo('a', 48.1, 11.5)), {
    latitude: 48.1,
    longitude: 11.5,
  });
});

test('getPhotoCoordinates rejects missing, non-numeric, and out-of-range values', () => {
  assert.equal(getPhotoCoordinates(photo('a', undefined, 11.5)), null);
  assert.equal(getPhotoCoordinates(photo('a', 48.1, undefined)), null);
  assert.equal(getPhotoCoordinates(photo('a', '48.1', '11.5')), null);
  assert.equal(getPhotoCoordinates(photo('a', Number.NaN, 11.5)), null);
  assert.equal(getPhotoCoordinates(photo('a', 91, 11.5)), null);
  assert.equal(getPhotoCoordinates(photo('a', 48.1, 181)), null);
  assert.equal(getPhotoCoordinates({ hash_sha256: 'a', metadata: {} }), null);
  assert.equal(getPhotoCoordinates(null), null);
});

test('groupPhotosByLocation collapses identical coordinates and preserves order', () => {
  const berlin = photo('a', 52.52, 13.405, { city: 'Berlin' });
  const berlin2 = photo('b', 52.52, 13.405);
  const munich = photo('c', 48.137, 11.575);
  const unlocated = { hash_sha256: 'd', metadata: {} };

  const locations = groupPhotosByLocation([berlin, unlocated, berlin2, munich]);

  assert.equal(locations.length, 2);
  assert.equal(locations[0].key, '52.52,13.405');
  assert.deepEqual(
    locations[0].photos.map((entry) => entry.hash_sha256),
    ['a', 'b']
  );
  assert.equal(locations[1].key, '48.137,11.575');
});

test('getLocationLabel prefers the first resolved city and returns null otherwise', () => {
  const locations = groupPhotosByLocation([
    photo('a', 1, 2),
    photo('b', 1, 2, { city: 'Berlin' }),
  ]);
  assert.equal(getLocationLabel(locations[0]), 'Berlin');

  const withoutCity = groupPhotosByLocation([photo('c', 3, 4)]);
  assert.equal(getLocationLabel(withoutCity[0]), null);
});

test('formatCoordinates prints six decimals like the metadata panel', () => {
  assert.equal(formatCoordinates({ latitude: 48.1372, longitude: 11.5755 }), '48.137200, 11.575500');
});

test('buildMapFilters mirrors the grid filter construction', () => {
  assert.deepEqual(
    buildMapFilters({ query: 'location:Berlin', sort: 'date_asc', year: 2024, month: 5 }),
    { query: 'location:Berlin', sort: 'date', order: 'asc', year: 2024, month: 5 }
  );
  assert.deepEqual(buildMapFilters({ query: null, sort: 'size_desc', year: null, month: null }), {
    query: null,
    sort: 'size',
    order: 'desc',
    year: undefined,
    month: undefined,
  });
});

test('isSemanticQuery treats prefix queries as regular searches', () => {
  assert.equal(isSemanticQuery('location:Berlin'), false);
  assert.equal(isSemanticQuery('type:video'), false);
  assert.equal(isSemanticQuery('is_favorite:true'), false);
  assert.equal(isSemanticQuery('sunset over the lake'), true);
  assert.equal(isSemanticQuery(null), false);
});

test('fetchSemanticPhotoSet pages all results, keeps order, and skips broken photos', async () => {
  const originalSearch = api.semanticSearch;
  const originalGetPhoto = api.getPhoto;
  const calls = [];

  api.semanticSearch = async (query, limit, offset) => {
    calls.push({ query, limit, offset });
    if (offset === 0) {
      // Simulate a full page so the loop must continue with the next offset.
      return { results: Array.from({ length: 200 }, (_, i) => ({ hash: `h${i}` })) };
    }
    return { results: [{ hash: 'h200' }] };
  };
  api.getPhoto = async (hash) => {
    if (hash === 'h5') throw new Error('gone');
    return { hash_sha256: hash, metadata: { location: { latitude: 1, longitude: 2 } } };
  };

  try {
    const photos = await fetchSemanticPhotoSet('dogs');
    assert.deepEqual(calls, [
      { query: 'dogs', limit: 200, offset: 0 },
      { query: 'dogs', limit: 200, offset: 200 },
    ]);
    assert.equal(photos.length, 200);
    assert.equal(photos[0].hash_sha256, 'h0');
    assert.equal(photos.at(-1).hash_sha256, 'h200');
    assert.ok(!photos.some((entry) => entry.hash_sha256 === 'h5'));
  } finally {
    api.semanticSearch = originalSearch;
    api.getPhoto = originalGetPhoto;
  }
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `node --test tests/map-aggregates.test.js`
Expected: FAIL — cannot resolve `frontend/src/lib/map.js`.

- [ ] **Step 3: Implement `frontend/src/lib/map.js`**

```js
// Map view data helpers. Pure functions (unit-tested by tests/map-aggregates.test.js)
// plus the semantic-search result-set loader the Map view shares with the grid's
// search semantics.
import { api } from './api.js';
import { isPrefixQuery } from './utils.js';

/** Server cap for /api/search/semantic (src/handlers_search.rs MAX_LIMIT). */
const SEMANTIC_PAGE_SIZE = 200;

/** Bound parallel /api/photos/{hash} hydration so a semantic page cannot
 *  flood the server (the grid hydrates one page at a time; the map needs the
 *  whole result set). */
const HYDRATE_CONCURRENCY = 8;

/**
 * Extracts validated coordinates from a photo's metadata.
 * Photos with only one coordinate, non-numeric values, or out-of-range values
 * are never plotted (the library rejects such data at write time; legacy rows
 * are skipped here).
 * @param {{ metadata?: { location?: { latitude?: unknown, longitude?: unknown } } } | null} photo
 * @returns {{ latitude: number, longitude: number } | null}
 */
export function getPhotoCoordinates(photo) {
  const latitude = photo?.metadata?.location?.latitude;
  const longitude = photo?.metadata?.location?.longitude;
  if (typeof latitude !== 'number' || typeof longitude !== 'number') return null;
  if (!Number.isFinite(latitude) || !Number.isFinite(longitude)) return null;
  if (latitude < -90 || latitude > 90 || longitude < -180 || longitude > 180) return null;
  return { latitude, longitude };
}

/**
 * Groups photos by identical coordinates into map locations, preserving the
 * input (sorted) order inside each location and across locations (FR-009/FR-010).
 * @param {Array} photos
 * @returns {Array<{ key: string, latitude: number, longitude: number, photos: Array }>}
 */
export function groupPhotosByLocation(photos) {
  const locations = new Map();
  for (const photo of photos ?? []) {
    const coordinates = getPhotoCoordinates(photo);
    if (!coordinates) continue;
    const key = `${coordinates.latitude},${coordinates.longitude}`;
    const location = locations.get(key);
    if (location) {
      location.photos.push(photo);
    } else {
      locations.set(key, { key, ...coordinates, photos: [photo] });
    }
  }
  return [...locations.values()];
}

/**
 * Resolved place name for a location: the first photo that carries one.
 * @returns {string | null}
 */
export function getLocationLabel(location) {
  for (const photo of location?.photos ?? []) {
    const city = photo?.metadata?.location?.city;
    if (typeof city === 'string' && city.trim()) return city.trim();
  }
  return null;
}

/**
 * Coordinate fallback for locations without a resolved place name (FR-012).
 * Matches the metadata panel's six-decimal convention.
 */
export function formatCoordinates({ latitude, longitude }) {
  return `${latitude.toFixed(6)}, ${longitude.toFixed(6)}`;
}

/**
 * Route state → /api/photos/map params, mirroring PhotoGrid.buildFilters:
 * favorites/videos view tokens never apply (the map is its own view), the
 * route query travels verbatim, sort/order split like the grid.
 */
export function buildMapFilters(route) {
  const filters = {
    query: route.query || null,
    year: route.year ?? undefined,
    month: route.month ?? undefined,
  };
  if (route.sort) {
    const [field, order] = route.sort.split('_');
    filters.sort = field;
    filters.order = order || 'desc';
  }
  return filters;
}

/**
 * Non-prefix queries run through CLIP semantic search, exactly like the grid
 * (prefix queries type:/location:/is_favorite: stay regular).
 */
export function isSemanticQuery(query) {
  return Boolean(query) && !isPrefixQuery(query);
}

/**
 * Loads the complete semantic result set (all pages, hydrated to full photo
 * rows) so the map's viewer navigation matches what the grid shows for the
 * same query. Stale photos (deleted between search and hydration) are skipped.
 * @param {string} query
 * @param {{ signal?: AbortSignal }} [options]
 * @returns {Promise<Array>}
 */
export async function fetchSemanticPhotoSet(query, { signal } = {}) {
  const cleanQuery = query.startsWith('@') ? query.substring(1).trim() : query;
  const photos = [];

  for (let offset = 0; ; offset += SEMANTIC_PAGE_SIZE) {
    const page = await api.semanticSearch(cleanQuery, SEMANTIC_PAGE_SIZE, offset, { signal });
    const hashes = (page?.results ?? []).map((result) => result.hash);
    if (hashes.length === 0) break;

    for (let index = 0; index < hashes.length; index += HYDRATE_CONCURRENCY) {
      const chunk = hashes.slice(index, index + HYDRATE_CONCURRENCY);
      const hydrated = await Promise.all(
        chunk.map((hash) => api.getPhoto(hash, { signal }).catch(() => null))
      );
      photos.push(...hydrated.filter((photo) => photo !== null));
    }

    if (hashes.length < SEMANTIC_PAGE_SIZE) break;
  }

  return photos;
}
```

- [ ] **Step 4: Run to verify the unit tests pass**

Run: `node --test tests/map-aggregates.test.js`
Expected: PASS (9 tests).

- [ ] **Step 5: Add `api.getMapPhotos`**

In `frontend/src/lib/api.js`, after `getPhotos`:

```js
  /**
   * Retrieves the complete filtered photo set for the Map view (no pagination).
   * @param {Object} params - query, sort, order, year, month, album
   * @param {Object} options - Fetch options (signal for AbortController, etc.)
   * @returns {Promise<{photos: Array}>}
   */
  async getMapPhotos(params = {}, options = {}) {
    const searchParams = new URLSearchParams();

    if (params.query) searchParams.set('q', params.query);
    if (params.sort) searchParams.set('sort', params.sort);
    if (params.order) searchParams.set('order', params.order);
    if (params.year !== undefined && params.year !== null) searchParams.set('year', params.year);
    if (params.month !== undefined && params.month !== null) searchParams.set('month', params.month);
    if (params.album !== undefined && params.album !== null) searchParams.set('album', params.album);

    const queryString = searchParams.toString();
    const endpoint = `/api/photos/map${queryString ? `?${queryString}` : ''}`;

    return this.request(endpoint, options);
  }
```

- [ ] **Step 6: Wire the unit tests into CI**

In `.github/workflows/ci.yml`, in the `lint-format` job after the i18n step:

```yaml
      - name: Run frontend unit tests
        run: node --test tests/map-aggregates.test.js
```

- [ ] **Step 7: Lint + commit**

```bash
npm run lint && npm run format:check
git add frontend/src/lib/map.js frontend/src/lib/api.js tests/map-aggregates.test.js .github/workflows/ci.yml
git commit -m "feat(map): add map data helpers and unpaginated photo listing client"
```

---

### Task 5: Map view shell — routing, dependencies, tiles, attribution, states

**Files:**
- Modify: `package.json` (`npm install leaflet supercluster`)
- Modify: `frontend/src/lib/router.svelte.js:1`
- Modify: `frontend/src/lib/state.svelte.js` (`appState`)
- Modify: `frontend/src/App.svelte` (:34-46 titleKeys/fallbacks, ~:122 config fetch, :144 `<main>`, :166-176 Select button, view branch :202-209, styles)
- Modify: `frontend/src/components/Sidebar.svelte:11-18`
- Modify: `frontend/src/i18n/en.json`, `frontend/src/i18n/de.json`
- Modify: `tests/i18n-integrity.test.js` (~:154-163)
- Create: `frontend/src/components/MapView.svelte`
- Test: `tests/e2e/specs/map.e2e.spec.js` (part 1)

**Interfaces:**
- Consumes: `api.getMapPhotos`/`api.getConfig` (Task 4), `appState.tileUrl`, `buildMapFilters`/`groupPhotosByLocation`/`isSemanticQuery`/`fetchSemanticPhotoSet`.
- Produces: view id `'map'` (URL `/map`), `MapView.svelte` rendering `[data-testid="map-canvas"]` inside `[data-testid="map-view"]`, status notices `[data-testid="map-unlocated-notice"]` / `[data-testid="map-tiles-notice"]`, empty state `[data-testid="map-empty-state"]`, loading `[data-testid="map-loading"]`.

- [ ] **Step 1: Install the map dependencies**

```bash
npm install leaflet@1.9.4 supercluster@9.1.0
```

- [ ] **Step 2: Register the view (router, sidebar, App, i18n)**

1. `frontend/src/lib/router.svelte.js:1`:
```js
const validViews = ['all', 'favorites', 'videos', 'albums', 'collages', 'housekeeping', 'map'];
```
2. `frontend/src/components/Sidebar.svelte` `views` array — add before `collages`:
```js
    { id: 'map', key: 'ui.map', fallback: 'Map' },
```
No icon: existing nav items are text-only (`map-pin` exists in the Icon registry if a glyph is ever wanted).
3. `frontend/src/App.svelte` — `titleKeys`/`titleFallbacks`:
```js
  const titleKeys = {
    all: 'ui.all_photos',
    favorites: 'ui.favorites',
    videos: 'ui.videos',
    albums: 'albums.sectionTitle',
    collages: 'ui.collages',
    housekeeping: 'ui.housekeeping',
    map: 'ui.map',
  };

  const titleFallbacks = {
    all: 'All Photos',
    favorites: 'Favorites',
    videos: 'Videos',
    albums: 'Albums',
    collages: 'Collages',
    housekeeping: 'Housekeeping',
    map: 'Map',
  };
```
4. `frontend/src/App.svelte` view branch — add **before** the final `{:else}`:
```svelte
      {:else if route.view === 'map'}
        <MapView />
```
and import it: `import MapView from './components/MapView.svelte';`
5. `frontend/src/App.svelte` — stash the configured tile endpoint when the config loads (in the onMount config block):
```js
      try {
        const config = await api.getConfig();
        defaultLocale = config?.default_locale || 'en';
        appState.tileUrl = config?.tile_url ?? null;
      } catch {
```
6. `frontend/src/lib/state.svelte.js`:
```js
export const appState = $state({
  sidebarOpen: false,
  mobileSearchOpen: false,
  // Raster tile endpoint served by GET /api/config (TURBO_PIX_TILE_URL).
  // null = not configured → the map renders markers without a tile layer.
  tileUrl: null,
});
```
7. `frontend/src/i18n/en.json` — new `ui.map` key plus a `map` section:
```json
    "map": "Map",
```
```json
  "map": {
    "withoutLocation": "{count} photos without location data",
    "noGeoPhotos": "No photos with location data in the current filters",
    "tilesUnavailable": "Map tiles unavailable — markers are shown without a map background.",
    "markerLabel": "{count} photos at {place}",
    "clusterLabel": "{count} photos, activate to zoom in",
    "openPhoto": "Open photo from {date}",
    "attributionLabel": "OpenStreetMap contributors"
  },
```
`frontend/src/i18n/de.json` — mirror:
```json
    "map": "Karte",
```
```json
  "map": {
    "withoutLocation": "{count} Fotos ohne Standortdaten",
    "noGeoPhotos": "Keine Fotos mit Standortdaten in den aktuellen Filtern",
    "tilesUnavailable": "Kartenkacheln nicht verfügbar – Marker werden ohne Kartenhintergrund angezeigt.",
    "markerLabel": "{count} Fotos bei {place}",
    "clusterLabel": "{count} Fotos, zum Vergrößern aktivieren",
    "openPhoto": "Foto vom {date} öffnen",
    "attributionLabel": "OpenStreetMap-Mitwirkende"
  },
```
8. `tests/i18n-integrity.test.js` — extend the hardcoded titleKeys alternation:
```js
        /^\s*(?:all|favorites|videos|collages|housekeeping|map):\s*'([^']+)'/gm
```

- [ ] **Step 3: Adapt the shell layout for the map**

`frontend/src/App.svelte`:
- `<main class="main-content" class:map-mode={route.view === 'map'}>`
- Hide the selection button on the read-only map surface (leave `SortControls` and `TimelineSlider` visible — sort defines popup order, year/month are the filter UI):
```svelte
        {#if route.view !== 'map'}
          <button
            type="button"
            class="select-mode-btn"
            ...
          </button>
        {/if}
```
- Append to the scoped styles:
```css
  /* The Map view fills the shell instead of scrolling: the map owns the
     remaining height and pans internally. */
  .main-content.map-mode {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
```

- [ ] **Step 4: Write the failing E2E shell spec**

Create `tests/e2e/specs/map.e2e.spec.js` (part 1; marker/popup tests are appended in Task 6):

```js
import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Map view', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.stubMapTiles(page);
  });

  test('sidebar navigation opens the map and updates the URL', async ({ page }) => {
    await TestHelpers.goto(page);
    await TestHelpers.waitForSearchReady(page);

    await TestHelpers.navigateToView(page, 'map');

    await expect(page).toHaveURL(/\/map$/);
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
    await expect(page.locator('#current-view-title')).toHaveText('Map');
  });

  test('direct URL load renders the map identically', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
    await TestHelpers.verifyActiveView(page, 'map');
  });

  test('shows visible OSM attribution and no viewport in the URL', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const attribution = page.locator('.leaflet-control-attribution');
    await expect(attribution).toBeVisible();
    await expect(attribution).toContainText('OpenStreetMap');

    // FR-017: center/zoom are never serialized into the URL.
    await expect(page).not.toHaveURL(/(zoom|lat|lng|center)=/);
  });

  test('filters with matching photos but no coordinates show the empty state', async ({ page }) => {
    // The seeded videos carry no GPS; the map must explain instead of showing a blank map.
    await TestHelpers.goto(page, '/map?q=type%3Avideo');

    await expect(page.locator('[data-testid="map-empty-state"]')).toBeVisible();
    await expect(page.locator('[data-testid="map-empty-state"]')).toContainText(
      'No photos with location data'
    );
  });

  test('reports the number of matching photos without location data', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const expected = await page.evaluate(async () => {
      const response = await fetch('/api/photos/map');
      const { photos } = await response.json();
      return photos.filter((photo) => {
        const latitude = photo.metadata?.location?.latitude;
        const longitude = photo.metadata?.location?.longitude;
        return typeof latitude !== 'number' || typeof longitude !== 'number';
      }).length;
    });

    await expect(page.locator('[data-testid="map-unlocated-notice"]')).toContainText(
      `${expected} photos without location data`
    );
  });
});
```

- [ ] **Step 5: Run the E2E spec to verify it fails**

Run: `npm run build && npx playwright test tests/e2e/specs/map.e2e.spec.js --reporter=line`
Expected: FAIL — `/map` normalizes to the all-photos view / no map canvas (view not implemented yet).

- [ ] **Step 6: Implement `MapView.svelte` (shell)**

Create `frontend/src/components/MapView.svelte`:

```svelte
<script>
  import { get } from 'svelte/store';
  import { onMount, untrack } from 'svelte';
  import L from 'leaflet';
  import Supercluster from 'supercluster';
  import 'leaflet/dist/leaflet.css';

  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { addToast, appState } from '../lib/state.svelte.js';
  import { route } from '../lib/router.svelte.js';
  import { logger } from '../lib/logger.js';
  import {
    buildMapFilters,
    fetchSemanticPhotoSet,
    isSemanticQuery,
    groupPhotosByLocation,
  } from '../lib/map.js';

  const MAX_ZOOM = 19;
  const CLUSTER_MAX_ZOOM = 18;
  const INITIAL_CENTER = [20, 0];
  const INITIAL_ZOOM = 2;
  const prefersReducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  let photos = $state([]);
  let loading = $state(true);
  let loadError = $state(null);
  let tilesFailed = $state(false);

  let mapEl = null;
  let map = null;
  let tileLayer = null;
  let clusterLayer = null;
  let clusterIndex = null;
  let hasFittedOnce = false;
  let abortController = null;
  let loadToken = 0;
  let anyTileLoaded = false;
  let tileErrorCount = 0;
  let resizeObserver = null;

  const locations = $derived(groupPhotosByLocation(photos));
  const unlocatedCount = $derived(
    photos.length - locations.reduce((count, location) => count + location.photos.length, 0)
  );
  const showEmptyState = $derived(!loading && !loadError && locations.length === 0);

  // ── Data loading ──────────────────────────────────────────────────────────

  async function fetchPhotoSet(currentRoute, signal) {
    if (currentRoute.album != null) {
      // Mirror the grid's album detail listing: sort/order + album scope only.
      const response = await api.getMapPhotos({ album: currentRoute.album }, { signal });
      return response.photos ?? [];
    }

    if (isSemanticQuery(currentRoute.query)) {
      return fetchSemanticPhotoSet(currentRoute.query, { signal });
    }

    const response = await api.getMapPhotos(buildMapFilters(currentRoute), { signal });
    return response.photos ?? [];
  }

  async function loadPhotos() {
    const token = ++loadToken;
    abortController?.abort();
    abortController = new AbortController();
    const { signal } = abortController;

    loading = true;
    loadError = null;

    try {
      const loaded = await fetchPhotoSet(route, signal);
      if (token !== loadToken) return;
      photos = loaded;
      fitToLocationsOnce();
    } catch (error) {
      if (error?.name === 'AbortError') return;
      if (token !== loadToken) return;
      logger.error('Error loading map photos', error, { component: 'MapView' });
      loadError = error.message || error;
      addToast(
        get(t)('errors.error_loading_photos', { default: 'Error Loading Photos' }),
        error.message,
        'error',
        5000
      );
    } finally {
      if (token === loadToken) loading = false;
    }
  }

  // ── Map lifecycle ─────────────────────────────────────────────────────────

  function attributionHtml() {
    const label = get(t)('map.attributionLabel', { default: 'OpenStreetMap contributors' });
    return `&copy; <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noopener">${label}</a>`;
  }

  function fitToLocationsOnce() {
    if (hasFittedOnce || !map || locations.length === 0) return;
    hasFittedOnce = true;
    const bounds = L.latLngBounds(locations.map((location) => [location.latitude, location.longitude]));
    map.fitBounds(bounds, {
      padding: [32, 32],
      maxZoom: 14,
      animate: !prefersReducedMotion,
    });
  }

  onMount(() => {
    map = L.map(mapEl, {
      zoomControl: true,
      attributionControl: true,
      // FR-018: reduced-motion preferences suppress animated panning/zooming.
      zoomAnimation: !prefersReducedMotion,
      fadeAnimation: !prefersReducedMotion,
      markerZoomAnimation: !prefersReducedMotion,
      maxZoom: MAX_ZOOM,
    });

    // Attribution is registered independently of the tile layer so it stays
    // visible in the degraded state (SC-005), and never behind a toggle (FR-003).
    map.attributionControl.addAttribution(attributionHtml());

    if (appState.tileUrl) {
      tileLayer = L.tileLayer(appState.tileUrl, { maxZoom: MAX_ZOOM });
      tileLayer.on('tileerror', () => {
        tileErrorCount += 1;
        if (!anyTileLoaded) tilesFailed = true;
      });
      tileLayer.on('tileload', () => {
        anyTileLoaded = true;
        if (tilesFailed) tilesFailed = false;
      });
      tileLayer.addTo(map);
    } else {
      // No configured endpoint — markers still render (FR-015).
      tilesFailed = true;
    }

    map.setView(INITIAL_CENTER, INITIAL_ZOOM, { animate: false });

    clusterLayer = L.layerGroup().addTo(map);
    map.on('moveend zoomend', renderClusters);

    resizeObserver = new ResizeObserver(() => map?.invalidateSize());
    resizeObserver.observe(mapEl);

    return () => {
      resizeObserver?.disconnect();
      resizeObserver = null;
      abortController?.abort();
      map?.remove();
      map = null;
      clusterLayer = null;
    };
  });

  $effect(() => {
    // Reload whenever the map's filter state changes (FR-004/SC-003).
    route.view;
    route.query;
    route.sort;
    route.year;
    route.month;
    route.album;
    untrack(() => {
      if (map) loadPhotos();
    });
  });

  $effect(() => {
    const groups = locations;
    clusterIndex = new Supercluster({ radius: 60, maxZoom: CLUSTER_MAX_ZOOM }).load(
      groups.map((location) => ({
        type: 'Feature',
        geometry: { type: 'Point', coordinates: [location.longitude, location.latitude] },
        properties: { key: location.key },
      }))
    );
    untrack(() => renderClusters());
  });

  function renderClusters() {
    // Implemented in Task 6.
  }
</script>

<div class="map-view" data-testid="map-view">
  <div class="map-status">
    {#if !loading && !loadError && unlocatedCount > 0}
      <p class="map-notice" data-testid="map-unlocated-notice">
        {$t('map.withoutLocation', {
          values: { count: unlocatedCount },
          default: '{count} photos without location data',
        })}
      </p>
    {/if}
    {#if tilesFailed}
      <p class="map-notice map-notice-warning" role="status" data-testid="map-tiles-notice">
        {$t('map.tilesUnavailable', {
          default: 'Map tiles unavailable — markers are shown without a map background.',
        })}
      </p>
    {/if}
  </div>

  <div class="map-stage">
    <div class="map-canvas" bind:this={mapEl} data-testid="map-canvas"></div>

    {#if loading}
      <div class="map-overlay" data-testid="map-loading">{$t('ui.loading', { default: 'Loading...' })}</div>
    {:else if loadError}
      <div class="map-overlay" data-testid="map-error">
        <p>{$t('errors.error_loading_photos', { default: 'Error Loading Photos' })}</p>
        <button type="button" class="map-retry" onclick={() => loadPhotos()}>
          {$t('ui.retry', { default: 'Retry' })}
        </button>
      </div>
    {:else if showEmptyState}
      <div class="map-overlay" data-testid="map-empty-state">
        <p>
          {photos.length === 0
            ? $t('ui.no_photos_found', { default: 'No Photos Found' })
            : $t('map.noGeoPhotos', {
                default: 'No photos with location data in the current filters',
              })}
        </p>
      </div>
    {/if}
  </div>
</div>

<style>
  /* Leaflet creates its DOM outside Svelte's compile scope, so map styling is
     deliberately global. Scoped rules would silently no-op (AGENTS.md #3). */
  .map-view {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    min-height: 0;
  }

  .map-status {
    display: flex;
    flex-shrink: 0;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .map-notice {
    margin: 0 0 var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--surface-color);
    color: var(--text-secondary);
    font-size: var(--font-sm);
  }

  .map-notice-warning {
    color: var(--text-primary);
  }

  .map-stage {
    position: relative;
    flex: 1 1 auto;
    min-height: 260px;
  }

  .map-canvas {
    height: 100%;
    width: 100%;
    border-radius: var(--radius-md);
    background: var(--surface-color);
  }

  .map-overlay {
    position: absolute;
    inset: 0;
    z-index: 500;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--background-color);
    color: var(--text-secondary);
    text-align: center;
  }

  :global(.map-canvas:focus-visible) {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }
</style>
```

Adjust `--radius-sm`/`--surface-color`/`--font-sm` to the actual tokens present in `frontend/src/app.css` (check the token list before committing; hardcoded values are prohibited).

- [ ] **Step 7: Run the E2E shell spec**

```bash
npm run build && cargo build --bin turbo-pix && npx playwright test tests/e2e/specs/map.e2e.spec.js --reporter=line
```
Expected: PASS. If the i18n spec was missed, `npm run test:i18n` fails loudly — run it too.

- [ ] **Step 8: Lint, format, commit**

```bash
npm run test:i18n && npm run lint && npm run format:check
git add package.json package-lock.json frontend/src tests/i18n-integrity.test.js tests/e2e/specs/map.e2e.spec.js
git commit -m "feat(map): register the map view with tiles, attribution, and states"
```

---

### Task 6: Markers, clusters, popups, viewer handoff, keyboard

**Files:**
- Modify: `frontend/src/components/MapView.svelte` (`renderClusters` + events)
- Create: `frontend/src/components/MapPopup.svelte`
- Modify: `tests/e2e/setup/test-helpers.js` (add `stubMapTiles`, `setPhotoCoordinates`)
- Test: `tests/e2e/specs/map.e2e.spec.js` (part 2)

**Interfaces:**
- Consumes: `locations` (Task 5), `getLocationLabel`/`formatCoordinates` (Task 4), `openViewer` window event contract (`detail: { photo, photos }`).
- Produces: marker DOM contract — individual location markers `[data-map-location]` (`data-map-location-count`), cluster markers `[data-map-cluster]` (count text inside); popup items `[data-map-popup-photo="<hash>"]`.

- [ ] **Step 1: Add the E2E helpers**

In `tests/e2e/setup/test-helpers.js` add (top of the class, next to `selectors`):

```js
  // 1x1 transparent PNG — stub tiles so map specs never touch the network.
  static TINY_PNG = Buffer.from(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
    'base64'
  );

  static async stubMapTiles(page) {
    await page.route(/tile\.openstreetmap\.org/, (route) =>
      route.fulfill({ status: 200, contentType: 'image/png', body: this.TINY_PNG })
    );
  }

  static async setPhotoCoordinates(page, hash, latitude, longitude) {
    const response = await page.request.patch(`/api/photos/${hash}/metadata`, {
      data: { latitude, longitude },
    });
    if (!response.ok()) {
      throw new Error(`PATCH metadata for ${hash} failed: ${response.status()}`);
    }
  }
```

- [ ] **Step 2: Write the failing E2E tests (append to `map.e2e.spec.js`)**

```js
  test('plots every geo-located location as a marker or cluster', async ({ page }) => {
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();

    const expectedLocations = await page.evaluate(async () => {
      const response = await fetch('/api/photos/map');
      const { photos } = await response.json();
      const keys = new Set();
      for (const photo of photos) {
        const latitude = photo.metadata?.location?.latitude;
        const longitude = photo.metadata?.location?.longitude;
        if (typeof latitude === 'number' && typeof longitude === 'number') {
          keys.add(`${latitude},${longitude}`);
        }
      }
      return keys.size;
    });
    test.skip(expectedLocations === 0, 'No geo-located photos in the test library');

    // Supercluster guarantees every point is represented: a cluster reports how
    // many locations it aggregates (FR-006), so clusters + individual markers
    // must equal the library's unique coordinates (FR-009).
    await expect
      .poll(async () =>
        page.evaluate(() => {
          const clustered = [...document.querySelectorAll('[data-map-cluster]')].reduce(
            (sum, element) => sum + Number(element.getAttribute('data-map-cluster')),
            0
          );
          return clustered + document.querySelectorAll('[data-map-location]').length;
        })
      )
      .toBe(expectedLocations);
  });

  test('cluster click separates the cluster into individual markers', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const cluster = page.locator('[data-map-cluster]').first();
    test.skip((await cluster.count()) === 0, 'Test library has no cluster at the initial zoom');

    const before = await page.locator('[data-map-location]').count();
    await cluster.click();

    await expect
      .poll(async () => page.locator('[data-map-location]').count())
      .toBeGreaterThan(before);
  });

  // Expand clusters until individual location markers render, then open the
  // first one's popup.
  async function openFirstLocationPopup(page) {
    for (let attempt = 0; attempt < 5; attempt += 1) {
      if ((await page.locator('[data-map-location]').count()) > 0) break;
      const cluster = page.locator('[data-map-cluster]').first();
      if ((await cluster.count()) === 0) break;
      await cluster.click();
      await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
    }
    const marker = page.locator('[data-map-location]').first();
    await expect(marker).toBeVisible();
    await marker.click();
    return page.locator('.leaflet-popup');
  }

  test('popup shows the place name and opens the viewer on a thumbnail', async ({ page }) => {
    await TestHelpers.goto(page, '/map');
    const popup = await openFirstLocationPopup(page);

    const key = await page.locator('[data-map-location]').first().getAttribute('data-map-location');
    const expectedPlace = await page.evaluate(async (locationKey) => {
      const response = await fetch('/api/photos/map');
      const { photos } = await response.json();
      const match = photos.find((photo) => {
        const latitude = photo.metadata?.location?.latitude;
        const longitude = photo.metadata?.location?.longitude;
        return typeof latitude === 'number' && `${latitude},${longitude}` === locationKey;
      });
      const city = match?.metadata?.location?.city;
      if (typeof city === 'string' && city.trim()) return city.trim();
      const [latitude, longitude] = locationKey.split(',').map(Number);
      return `${latitude.toFixed(6)}, ${longitude.toFixed(6)}`;
    }, key);

    await expect(popup).toBeVisible();
    await expect(popup.locator('[data-testid="map-popup-place"]')).toHaveText(expectedPlace);
    await expect(popup.locator('[data-map-popup-photo]').first()).toBeVisible();

    const firstThumbnail = popup.locator('[data-map-popup-photo]').first();
    const hash = await firstThumbnail.getAttribute('data-map-popup-photo');
    await firstThumbnail.click();

    await expect(page.locator('#photo-viewer')).toBeVisible();
    await expect(page).toHaveURL(new RegExp(`photo=${hash}`));

    await page.keyboard.press('Escape');
    await expect(page.locator('#photo-viewer')).toBeHidden();
  });

  test('photos sharing coordinates collapse into one marker with a full, scrollable list', async ({
    page,
  }) => {
    await TestHelpers.goto(page, '/map');

    const expected = await page.evaluate(async () => {
      const response = await fetch('/api/photos/map');
      const { photos } = await response.json();
      const counts = new Map();
      for (const photo of photos) {
        const latitude = photo.metadata?.location?.latitude;
        const longitude = photo.metadata?.location?.longitude;
        if (typeof latitude !== 'number' || typeof longitude !== 'number') continue;
        const key = `${latitude},${longitude}`;
        counts.set(key, (counts.get(key) ?? 0) + 1);
      }
      return [...counts.entries()].sort((a, b) => b[1] - a[1])[0] ?? null;
    });
    test.skip(!expected, 'No geo-located photos in the test library');

    const [key, count] = expected;

    for (let attempt = 0; attempt < 6; attempt += 1) {
      if ((await page.locator(`[data-map-location="${key}"]`).count()) > 0) break;
      const cluster = page.locator('[data-map-cluster]').first();
      if ((await cluster.count()) === 0) break;
      await cluster.click();
      await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
    }

    const marker = page.locator(`[data-map-location="${key}"]`);
    await expect(marker).toBeVisible();
    await expect(marker).toHaveAttribute('data-map-location-count', String(count));
    await marker.click();

    const popup = page.locator('.leaflet-popup');
    // FR-009/FR-010: every photo of that location is listed (never truncated).
    await expect(popup.locator('[data-map-popup-photo]')).toHaveCount(count);

    if (count > 5) {
      const list = await popup.locator('.map-popup-list').evaluate((element) => ({
        scrollHeight: element.scrollHeight,
        clientHeight: element.clientHeight,
      }));
      expect(list.scrollHeight).toBeGreaterThan(list.clientHeight);
    }
  });
```

- [ ] **Step 3: Run to verify they fail**

```bash
npm run build && cargo build --bin turbo-pix && npx playwright test tests/e2e/specs/map.e2e.spec.js --reporter=line
```
Expected: FAIL — no `[data-map-location]` / `[data-map-cluster]` markers.

- [ ] **Step 4: Implement marker rendering in `MapView.svelte`**

Replace the `renderClusters` stub (and add the popup/lifecycle helpers):

```js
  import { mount, onMount, unmount, untrack } from 'svelte';
  import { formatDate } from '../lib/utils.js';
  import { formatCoordinates, getLocationLabel, wrapLongitudeForView } from '../lib/map.js';
  import MapPopup from './MapPopup.svelte';

  const popupHandles = new Map();

  // `wrapLongitudeForView` lives in lib/map.js: Leaflet projects markers with
  // latLngToLayerPoint and never wraps longitude (only tile URLs wrap), so
  // `map.getBounds()` is the raw visible window — the shift must be rejected
  // when the ±360° copy is not inside it (SC: antimeridian photos render).

  function clusterIcon(count) {
    const size = count < 10 ? 34 : count < 100 ? 42 : 48;
    return L.divIcon({
      className: 'map-cluster-marker',
      html: `<span>${count}</span>`,
      iconSize: [size, size],
      iconAnchor: [size / 2, size / 2],
    });
  }

  function labelFor(location) {
    return getLocationLabel(location) ?? formatCoordinates(location);
  }

  function activateOnKeyboard(element, handler) {
    element.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        handler();
      }
    });
  }

  function bindLocationMarker(marker, location) {
    const popupNode = document.createElement('div');
    popupNode.className = 'map-popup-host';
    marker.bindPopup(popupNode, { maxWidth: 320, minWidth: 260, autoPan: true, closeButton: true });

    marker.on('popupopen', () => {
      popupHandles.set(
        marker,
        mount(MapPopup, {
          target: popupNode,
          props: { location, onOpenPhoto: (photo) => openViewer(photo) },
        })
      );
      // Popup panes come after the marker pane in DOM order, but with many
      // markers Tab would walk through every marker first — move focus into
      // the popup so keyboard users land on the thumbnails (FR-018).
      requestAnimationFrame(() => popupNode.querySelector('button')?.focus());
    });

    marker.on('popupclose', () => {
      const wasFocusedInside = popupNode.contains(document.activeElement);
      const handle = popupHandles.get(marker);
      if (handle) {
        popupHandles.delete(marker);
        void unmount(handle);
      }
      const icon = marker.getElement();
      if (wasFocusedInside && icon?.isConnected) icon.focus();
    });
  }

  function expandCluster(clusterId, fallbackLatLng) {
    if (!map || !clusterIndex) return;
    const zoom = clusterIndex.getClusterExpansionZoom(clusterId);
    map.setView(fallbackLatLng, Math.min(zoom, MAX_ZOOM), { animate: !prefersReducedMotion });
  }

  function renderClusters() {
    if (!map || !clusterLayer || !clusterIndex) return;

    const bounds = map.getBounds();
    const zoom = Math.round(map.getZoom());
    const features = clusterIndex.getClusters(
      [bounds.getWest(), bounds.getSouth(), bounds.getEast(), bounds.getNorth()],
      zoom
    );

    for (const handle of popupHandles.values()) {
      void unmount(handle);
    }
    popupHandles.clear();
    clusterLayer.clearLayers();

    const byKey = new Map(locations.map((location) => [location.key, location]));

    for (const feature of features) {
      const [rawLng, latitude] = feature.geometry.coordinates;
      const longitude = wrapLongitudeForView(rawLng, bounds.getWest(), bounds.getEast());

      if (feature.properties.cluster) {
        const count = feature.properties.point_count;
        const marker = L.marker([latitude, longitude], {
          icon: clusterIcon(count),
          keyboard: true,
        });
        marker.addTo(clusterLayer);
        const element = marker.getElement();
        if (element) {
          element.setAttribute('data-map-cluster', String(count));
          element.setAttribute('aria-label', get(t)('map.clusterLabel', {
            values: { count },
            default: '{count} photos, activate to zoom in',
          }));
          activateOnKeyboard(element, () =>
            expandCluster(feature.properties.cluster_id, [latitude, longitude])
          );
        }
        continue;
      }

      const location = byKey.get(feature.properties.key);
      if (!location) continue;

      const marker = L.marker([latitude, longitude], { keyboard: true });
      marker.addTo(clusterLayer);
      const element = marker.getElement();
      if (element) {
        element.setAttribute('data-map-location', location.key);
        element.setAttribute('data-map-location-count', String(location.photos.length));
        element.setAttribute(
          'aria-label',
          get(t)('map.markerLabel', {
            values: { count: location.photos.length, place: labelFor(location) },
            default: '{count} photos at {place}',
          })
        );
        activateOnKeyboard(element, () => marker.openPopup());
      }
      bindLocationMarker(marker, location);
    }
  }

  function openViewer(photo) {
    // FR-011: the viewer gets the map's complete filtered, sorted set — the
    // same array the grid would page through — so next/previous stay in scope.
    window.dispatchEvent(new CustomEvent('openViewer', { detail: { photo, photos } }));
  }
```

Marker icon styling (inside `<style>`, global because Leaflet owns the DOM):

```css
  :global(.map-location-marker) {
    width: 18px;
    height: 18px;
    border: 2px solid var(--background-color);
    border-radius: 50%;
    background: var(--primary-color);
    box-shadow: 0 1px 4px rgb(0 0 0 / 40%);
    cursor: pointer;
  }

  :global(.map-cluster-marker) {
    display: flex;
    align-items: center;
    justify-content: center;
    border: 3px solid var(--background-color);
    border-radius: 50%;
    background: var(--primary-color);
    color: var(--background-color);
    font-size: var(--font-sm);
    font-weight: var(--font-semibold);
  }

  :global(.map-location-marker:focus-visible),
  :global(.map-cluster-marker:focus-visible) {
    outline: 2px solid var(--primary-color);
    outline-offset: 3px;
  }

  :global(.map-popup-list) {
    max-height: 232px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }
```

Add the window-event listeners alongside the data effect (mirror `PhotoGrid`'s handlers so viewer mutations stay coherent):

```js
  function handlePhotoRemoved(event) {
    const { hash } = event.detail || {};
    if (!hash) return;
    const index = photos.findIndex((photo) => photo.hash_sha256 === hash);
    if (index !== -1) photos.splice(index, 1);
  }

  function handlePhotoUpdated(event) {
    const updatedPhoto = event.detail?.photo;
    if (!updatedPhoto?.hash_sha256) return;
    const oldHash = event.detail?.oldHash;
    const index = photos.findIndex(
      (photo) =>
        (oldHash && photo.hash_sha256 === oldHash) ||
        photo.hash_sha256 === updatedPhoto.hash_sha256 ||
        (photo.file_path && updatedPhoto.file_path && photo.file_path === updatedPhoto.file_path)
    );
    if (index !== -1) photos[index] = updatedPhoto;
  }

  function handleFavoriteToggled(event) {
    const { photoHash, isFavorite } = event.detail || {};
    const index = photos.findIndex((photo) => photo.hash_sha256 === photoHash);
    if (index === -1) return;
    photos[index].is_favorite = isFavorite;
    // An `is_favorite:true` query stops matching the photo once unfavorited.
    if (!isFavorite && route.query?.split(/\s+/).includes('is_favorite:true')) {
      photos.splice(index, 1);
    }
  }

  $effect(() => {
    const reload = () => loadPhotos();
    const listeners = {
      photoRemoved: handlePhotoRemoved,
      photoUpdated: handlePhotoUpdated,
      favoriteToggled: handleFavoriteToggled,
      indexingCompleted: reload,
      photosReloadRequested: reload,
    };
    for (const [name, handler] of Object.entries(listeners)) {
      window.addEventListener(name, handler);
    }
    return () => {
      for (const [name, handler] of Object.entries(listeners)) {
        window.removeEventListener(name, handler);
      }
    };
  });
```

- [ ] **Step 5: Create `MapPopup.svelte`**

```svelte
<script>
  import { t } from '../lib/i18n.js';
  import { formatDate, getThumbnailUrl } from '../lib/utils.js';
  import { formatCoordinates, getLocationLabel } from '../lib/map.js';

  const { location, onOpenPhoto } = $props();

  const place = $derived(getLocationLabel(location) ?? formatCoordinates(location));
</script>

<div class="map-popup">
  <p class="map-popup-place" data-testid="map-popup-place">{place}</p>
  <p class="map-popup-count">
    {$t('ui.photos_count', { values: { count: location.photos.length }, default: '{count} photos' })}
  </p>
  <ul class="map-popup-list">
    {#each location.photos as photo (photo.hash_sha256)}
      <li>
        <button
          type="button"
          class="map-popup-item"
          data-map-popup-photo={photo.hash_sha256}
          aria-label={$t('map.openPhoto', {
            values: { date: photo.taken_at ? formatDate(photo.taken_at) : $t('ui.unknown', { default: 'Unknown' }) },
            default: 'Open photo from {date}',
          })}
          onclick={() => onOpenPhoto(photo)}
        >
          <img src={getThumbnailUrl(photo, 'small')} alt="" loading="lazy" decoding="async" />
          <span>{photo.taken_at ? formatDate(photo.taken_at) : $t('ui.unknown', { default: 'Unknown' })}</span>
        </button>
      </li>
    {/each}
  </ul>
</div>

<style>
  .map-popup-place {
    margin: 0;
    color: var(--text-primary);
    font-weight: var(--font-semibold);
  }

  .map-popup-count {
    margin: 0 0 var(--space-2);
    color: var(--text-secondary);
    font-size: var(--font-sm);
  }

  .map-popup-list {
    max-height: 232px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  .map-popup-item {
    display: flex;
    width: 100%;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
    text-align: left;
  }

  .map-popup-item:hover,
  .map-popup-item:focus-visible {
    background: var(--surface-color);
  }

  .map-popup-item img {
    width: 48px;
    height: 48px;
    border-radius: var(--radius-sm);
    object-fit: cover;
  }
</style>
```

- [ ] **Step 6: Run the E2E spec to verify it passes**

```bash
npm run build && cargo build --bin turbo-pix && npx playwright test tests/e2e/specs/map.e2e.spec.js --reporter=line
```
Expected: PASS (all shell + marker + popup tests).

- [ ] **Step 7: Lint, format, commit**

```bash
npm run lint && npm run format:check
git add frontend/src/components/MapView.svelte frontend/src/components/MapPopup.svelte tests/e2e/setup/test-helpers.js tests/e2e/specs/map.e2e.spec.js
git commit -m "feat(map): render clustered photo markers with popups and viewer handoff"
```

---

### Task 7: Filter parity E2E (year/month, search, semantic, album, videos, history)

**Files:**
- Modify: `tests/e2e/setup/test-helpers.js` (add `setPhotoLocationInDb`)
- Create: `tests/e2e/specs/map-filters.e2e.spec.js`
- Modify: `frontend/src/components/MapView.svelte` / `frontend/src/lib/map.js` **only if a test surfaces a real gap**

**Interfaces:**
- Consumes: everything above; `TestDataManager.DB_PATH` (`test-e2e-data/database/turbo-pix.db`).
- Produces: verified filter semantics.

- [ ] **Step 1: Add the DB-seed helper (videos cannot receive EXIF)**

In `tests/e2e/setup/test-helpers.js`:

```js
  /** Writes coordinates straight into the indexed row — videos cannot take EXIF writes. */
  static setPhotoLocationInDb(fileName, latitude, longitude) {
    const dbPath = 'test-e2e-data/database/turbo-pix.db';
    const metadata = JSON.stringify({ location: { latitude, longitude } });
    execSync(
      `sqlite3 "${dbPath}" "UPDATE photos SET metadata = json_set(metadata, '$.location.latitude', ${latitude}, '$.location.longitude', ${longitude}) WHERE filename = '${fileName}'"`,
      { stdio: 'pipe' }
    );
    return metadata;
  }
```
(`execSync` — add the import at the top of `test-helpers.js`.)

- [ ] **Step 2: Write the specs**

Create `tests/e2e/specs/map-filters.e2e.spec.js`:

```js
import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

const BASE_URL = 'http://localhost:18473';

/** Unique-coordinate count of a map endpoint response. */
async function expectedLocations(page, endpoint) {
  return page.evaluate(async (url) => {
    const response = await fetch(url);
    const { photos } = await response.json();
    const keys = new Set();
    for (const photo of photos) {
      const latitude = photo.metadata?.location?.latitude;
      const longitude = photo.metadata?.location?.longitude;
      if (typeof latitude === 'number' && typeof longitude === 'number') {
        keys.add(`${latitude},${longitude}`);
      }
    }
    return keys.size;
  }, endpoint);
}

/** Locations the map currently represents: cluster counts sum + single markers. */
async function renderedLocations(page) {
  return page.evaluate(() => {
    const clustered = [...document.querySelectorAll('[data-map-cluster]')].reduce(
      (sum, element) => sum + Number(element.getAttribute('data-map-cluster')),
      0
    );
    return clustered + document.querySelectorAll('[data-map-location]').length;
  });
}

/** Opens the popup for a location key, expanding clusters when needed. */
async function focusLocation(page, key) {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    if ((await page.locator(`[data-map-location="${key}"]`).count()) > 0) break;
    const cluster = page.locator('[data-map-cluster]').first();
    if ((await cluster.count()) === 0) break;
    await cluster.click();
    // The zoom animation pane detaches once the jump completes.
    await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
  }
  return page.locator(`[data-map-location="${key}"]`);
}

test.describe('Map filters', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.stubMapTiles(page);
  });

  test('year filter plots exactly the geo-located subset of that year', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const year = await page.evaluate(async () => {
      const response = await fetch('/api/photos/map');
      const { photos } = await response.json();
      const withDate = photos.find((photo) => photo.taken_at);
      return withDate ? new Date(withDate.taken_at).getFullYear() : null;
    });
    test.skip(year === null, 'No dated photos in the test library');

    const expected = await expectedLocations(page, `/api/photos/map?year=${year}`);

    await page.evaluate((value) => {
      window.history.pushState({}, '', `/map?year=${value}`);
      window.dispatchEvent(new PopStateEvent('popstate'));
    }, year);
    await expect(page).toHaveURL(new RegExp(`/map\\?year=${year}`));

    // Filtered locations are a subset of the fitted initial viewport, so the
    // rendered representation must match the filtered set exactly (SC-003).
    await expect.poll(() => renderedLocations(page)).toBe(expected);
  });

  test('location: search plots exactly the matching geo-located subset', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const city = await page.evaluate(async () => {
      const response = await fetch('/api/photos?limit=100');
      const { photos } = await response.json();
      return photos.find((photo) => photo.metadata?.location?.city)?.metadata.location.city ?? null;
    });
    test.skip(!city, 'No resolved city in the seeded photos');

    const expected = await expectedLocations(
      page,
      `/api/photos/map?q=${encodeURIComponent(`location:${city}`)}`
    );
    expect(expected).toBeGreaterThan(0);

    await TestHelpers.performSearch(page, `location:${city}`);

    await expect.poll(() => renderedLocations(page)).toBe(expected);
  });

  test('semantic search plots the geo-located subset of the CLIP result set', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const expected = await page.evaluate(async () => {
      const search = await fetch('/api/search/semantic?q=dog&limit=200&offset=0').then((response) =>
        response.json()
      );
      const hashes = (search.results ?? []).map((entry) => entry.hash);
      const photos = await Promise.all(
        hashes.map((hash) =>
          fetch(`/api/photos/${hash}`).then((response) => (response.ok ? response.json() : null))
        )
      );
      const keys = new Set();
      for (const photo of photos) {
        const latitude = photo?.metadata?.location?.latitude;
        const longitude = photo?.metadata?.location?.longitude;
        if (typeof latitude === 'number' && typeof longitude === 'number') {
          keys.add(`${latitude},${longitude}`);
        }
      }
      return keys.size;
    });

    await TestHelpers.performSearch(page, 'dog');

    // Results can lie outside the fitted viewport (the map deliberately keeps
    // the user's viewport on filter changes), so zoom out to the world first.
    for (let step = 0; step < 4; step += 1) {
      await page.locator('.leaflet-control-zoom-out').click();
    }

    if (expected === 0) {
      await expect(page.locator('[data-testid="map-empty-state"]')).toBeVisible();
    } else {
      await expect.poll(() => renderedLocations(page), { timeout: 30000 }).toBe(expected);
    }
  });

  test('album scoping plots exactly the album members', async ({ page }) => {
    const distinct = await page.evaluate(async () => {
      const response = await fetch('/api/photos/map');
      const { photos } = await response.json();
      const seen = new Map();
      for (const photo of photos) {
        const latitude = photo.metadata?.location?.latitude;
        const longitude = photo.metadata?.location?.longitude;
        if (typeof latitude !== 'number' || typeof longitude !== 'number') continue;
        const key = `${latitude},${longitude}`;
        if (!seen.has(key)) seen.set(key, photo.hash_sha256);
      }
      return [...seen.values()];
    });
    test.skip(distinct.length < 2, 'Need geo-located photos at two distinct locations');

    const hashes = distinct.slice(0, 2);
    const createResponse = await page.request.post(`${BASE_URL}/api/albums`, {
      data: { name: 'Map E2E Album', initial_hashes: hashes },
    });
    expect(createResponse.ok()).toBe(true);
    const album = await createResponse.json();

    try {
      await TestHelpers.goto(page, `/map?album=${album.id}`);
      await expect.poll(() => renderedLocations(page)).toBe(2);
    } finally {
      await page.request.delete(`${BASE_URL}/api/albums/${album.id}`);
    }
  });

  test('videos with coordinates are plotted and open in the viewer', async ({ page }) => {
    // Videos cannot take EXIF writes, so seed the indexed row directly (the
    // same DB-seeding pattern the collage/housekeeping specs use).
    TestHelpers.setPhotoLocationInDb('test_video.mp4', 52.52, 13.405);
    await TestHelpers.goto(page, '/map');

    const marker = await focusLocation(page, '52.52,13.405');
    await expect(marker).toBeVisible();
    await marker.click();

    const item = page.locator('.leaflet-popup [data-map-popup-photo]').first();
    await item.click();
    await expect(page.locator('#photo-viewer')).toBeVisible();
    // FR-016: video items play through the existing viewer behavior.
    await expect(page.locator('#viewer-video')).toBeVisible();
  });

  test('back/forward restores the map with the same filter state', async ({ page }) => {
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();

    await TestHelpers.navigateToView(page, 'videos');
    await page.goBack();

    await expect(page).toHaveURL(/\/map$/);
    await TestHelpers.verifyActiveView(page, 'map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
  });
});
```

- [ ] **Step 3: Run, then fix real gaps only**

```bash
npm run build && cargo build --bin turbo-pix && npx playwright test tests/e2e/specs/map-filters.e2e.spec.js --reporter=line
```
Expected: PASS. A failure is a real defect in Task 5/6 loading logic (e.g. `pushState` handling, semantic hydration) — fix there, not in the spec.

- [ ] **Step 4: Lint, format, commit**

```bash
npm run lint && npm run format:check
git add tests/e2e/setup/test-helpers.js tests/e2e/specs/map-filters.e2e.spec.js
git commit -m "test(map): cover filter, search, album and history parity"
```

---

### Task 8: Degradation, keyboard, reduced motion, mobile — E2E hardening

**Files:**
- Create: `tests/e2e/specs/map-a11y.e2e.spec.js`
- Modify: `frontend/src/components/MapView.svelte` **only if a test surfaces a real gap**

**Interfaces:**
- Consumes: `[data-testid="map-canvas"]`, `[data-map-location]`, `[data-map-cluster]`, `[data-map-popup-photo]`, `[data-testid="map-tiles-notice"]`.
- Produces: verified FR-003/015/018/019 + SC-005/006/008.

- [ ] **Step 1: Write the specs**

Create `tests/e2e/specs/map-a11y.e2e.spec.js`:

```js
import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { TestHelpers } from '../setup/test-helpers.js';

const AXE_RULES = ['color-contrast', 'target-size', 'aria-valid-attr-value', 'aria-prohibited-attr'];

const TILE_FAILURE_URL = 'http://127.0.0.1:9/{z}/{x}/{y}.png';

async function mockTileEndpoint(page, tileUrl) {
  await page.route('**/api/config', async (route) => {
    const response = await route.fetch();
    const body = await response.json();
    await route.fulfill({ response, json: { ...body, tile_url: tileUrl } });
  });
}

test.describe('Map degradation and accessibility', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
  });

  test('unreachable tile endpoint keeps markers, popups and attribution with a notice', async ({
    page,
  }) => {
    await mockTileEndpoint(page, TILE_FAILURE_URL);
    await TestHelpers.goto(page, '/map');

    await expect(page.locator('[data-testid="map-tiles-notice"]')).toBeVisible();
    await expect(page.locator('.leaflet-control-attribution')).toContainText('OpenStreetMap');
    await expect
      .poll(async () => page.locator('[data-map-location], [data-map-cluster]').count())
      .toBeGreaterThan(0);
  });

  test('attribution stays visible on a mobile viewport', async ({ page }) => {
    await TestHelpers.setMobileViewport(page);
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');

    const attribution = page.locator('.leaflet-control-attribution');
    await expect(attribution).toBeVisible();
    const box = await attribution.boundingBox();
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(375);
  });

  test('keyboard alone reaches a marker, its popup thumbnail, and the viewer', async ({ page }) => {
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();

    // The map container is tabbable (Leaflet sets tabindex); Tab must reach a
    // map feature — a cluster or an individual location marker.
    await page.locator('[data-testid="map-canvas"]').focus();
    await page.keyboard.press('Tab');
    const focusedFeature = await page.locator(':focus').evaluate((element) => ({
      location: element.getAttribute('data-map-location'),
      cluster: element.getAttribute('data-map-cluster'),
    }));
    expect(focusedFeature.location !== null || focusedFeature.cluster !== null).toBe(true);

    // Make an individual marker reachable (mouse expansion is setup only).
    for (let attempt = 0; attempt < 5; attempt += 1) {
      if ((await page.locator('[data-map-location]').count()) > 0) break;
      const cluster = page.locator('[data-map-cluster]').first();
      if ((await cluster.count()) === 0) break;
      await cluster.click();
      await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
    }

    const marker = page.locator('[data-map-location]').first();
    await expect(marker).toBeVisible();
    await marker.focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('.leaflet-popup')).toBeVisible();

    const thumbnail = page.locator('[data-map-popup-photo]').first();
    await expect(thumbnail).toBeFocused();

    await page.keyboard.press('Enter');
    await expect(page.locator('#photo-viewer')).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(page.locator('#photo-viewer')).toBeHidden();
  });

  test('cluster markers are keyboard activatable', async ({ page }) => {
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');

    const cluster = page.locator('[data-map-cluster]').first();
    test.skip((await cluster.count()) === 0, 'No cluster at the initial zoom');

    await cluster.focus();
    const before = await page.locator('[data-map-location]').count();
    await page.keyboard.press('Enter');
    await expect
      .poll(async () => page.locator('[data-map-location]').count())
      .toBeGreaterThan(before);
  });

  test('reduced motion disables animated zooming', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-view"]')).toHaveAttribute(
      'data-reduced-motion',
      'true'
    );

    // With zoomAnimation disabled Leaflet never enters the animated zoom path,
    // so no zoom-animation pane appears while zooming.
    await page.locator('.leaflet-control-zoom-in').click();
    await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
  });

  test('map view has no axe violations', async ({ page }) => {
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();

    const results = await new AxeBuilder({ page })
      .include('[data-testid="map-view"]')
      .withRules(AXE_RULES)
      .analyze();
    expect(results.violations).toEqual([]);
  });
});
```

Trim the reduced-motion test to the assertion that holds with the implementation: the honest check is that the map is constructed with animations off when the media query matches — assert via a DOM marker the component sets (`data-reduced-motion` on `[data-testid="map-view"]`, added in the implementation below) plus a zoom interaction that completes without a `.leaflet-zoom-anim` pane appearing.

- [ ] **Step 2: Add the reduced-motion marker to `MapView.svelte`**

```svelte
<div class="map-view" data-testid="map-view" data-reduced-motion={String(prefersReducedMotion)}>
```

- [ ] **Step 3: Run the specs**

```bash
npm run build && cargo build --bin turbo-pix && npx playwright test tests/e2e/specs/map-a11y.e2e.spec.js --reporter=line
```
Expected: PASS. Fix any real gap in `MapView.svelte` (e.g. focus restoration after `popupclose`, missing `aria-label`) rather than weakening an assertion.

- [ ] **Step 4: Commit**

```bash
npm run lint && npm run format:check
git add tests/e2e/specs/map-a11y.e2e.spec.js frontend/src/components/MapView.svelte
git commit -m "test(map): cover degradation, keyboard and reduced-motion behavior"
```

---

### Task 9: Full gates, docs, learnings, manual smoke

**Files:**
- Modify: `AGENTS.md` (Learnings section — fold into existing entries, cap 10)
- Modify: `README.md` (map feature blurb, if the README documents views)

**Interfaces:**
- Consumes: everything.
- Produces: green CI-equivalent run + a captured manual smoke screenshot.

- [ ] **Step 1: Run every gate exactly as CI does**

```bash
npm ci
npm run build
npm run test:i18n
node --test tests/map-aggregates.test.js
npm run format:check
npm run lint
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
npx playwright test
```
Expected: all green. Re-run `npx playwright test` once before suspecting regressions if the first run fails on the known port race (AGENTS.md learning 10).

- [ ] **Step 2: Manual smoke with real OSM tiles**

```bash
nohup cargo run --bin turbo-pix &
curl --retry 5 --retry-delay 2 http://localhost:18473/health
```
Then in a browser at `http://localhost:18473/map`: pan/zoom, open a cluster, open a popup, open a photo, use next/previous, change the year filter, and confirm the attribution is visible. Save a screenshot to `test-results/verification/map-view.png` (gitignored) and report the observed tile host + marker count in the completion summary. Kill the server afterwards.

- [ ] **Step 3: Update learnings**

Fold the session's findings into `AGENTS.md`'s Learnings list (cap 10 entries — replace the least relevant, never append an 11th). Candidates from this feature:

- Map data contract: `GET /api/photos/map` is the only unpaginated listing; `Photo::list_all_filtered` shares `build_search_where` with `search_photos` — never re-implement the token grammar.
- Tile endpoint comes from `/api/config` (`TURBO_PIX_TILE_URL`); the frontend must never hardcode a tile URL, and attribution is registered on the map control (not the tile layer) so it survives tile failure.
- Geo validation lives in `frontend/src/lib/map.js`; `photos.metadata.location` is JSON (no columns), and `PATCH /api/photos/{hash}/metadata` keeps an existing `city` — tests that need a nameless location must pick a photo without one.

- [ ] **Step 4: Commit**

```bash
git add AGENTS.md README.md
git commit -m "docs(map): document tile endpoint and map view learnings"
```

---

## Self-Review (completed while writing)

- **Spec coverage:** FR-001 (Task 5), FR-002/003 (Tasks 1, 5), FR-004/005 (Tasks 2, 3, 5, 7), FR-006/007/008/009/010 (Task 6), FR-011 (Task 6), FR-012 (Tasks 4, 6), FR-013/014 (Tasks 5), FR-015 (Tasks 5, 8), FR-016 (Task 7), FR-017 (Tasks 5, 7), FR-018 (Tasks 6, 8), FR-019 (Task 8), FR-020 (Tasks 5, 8).
- **Placeholder scan:** no TBD/TODO; every code step carries its code; the only deferred bodies are `renderClusters` (Task 5 explicitly stubs it, Task 6 implements it with the full listing) and the reduced-motion assertion finalized in Task 8 Step 2.
- **Type consistency:** `getMapPhotos(params, options)`, `list_all_filtered(pool, query, sort, order, album)`, `buildMapFilters(route)`, `groupPhotosByLocation(photos)`, `Location = { key, latitude, longitude, photos }`, `data-map-location` / `data-map-cluster` / `data-map-popup-photo` attribute names are used identically across backend, frontend and E2E tasks.
- **Review Focus → tests:** malformed coordinates (Task 4 unit test), >100 photos (Task 3 handler test), tile failure (Task 8), large single-location popup (Task 6 E2E), keyboard-only flow (Task 8). Antimeridian/extreme latitudes: supercluster 9 normalizes bboxes and splits seam-crossing queries internally (verified in its source), and `wrapLongitudeForView` shifts the displayed copy (Task 6).
