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
| `frontend/src/lib/map.js` | Pure map data helpers (coordinate extraction/validation, grouping by location, labels) + route→filter mapping + semantic result-set fetching (api client injected). Node-testable. |
| `frontend/src/lib/query.js` | Pure query-token classification (`isPrefixQuery`) shared by the grid, the search bar, and the map. Node-testable (has no `utils.js`/api imports). |
| `frontend/src/components/MapView.svelte` | The view: Leaflet map lifecycle, tile layer + attribution, supercluster rendering, notices/empty states, data loading, window-event handling. |
| `frontend/src/components/MapPopup.svelte` | Popup content: place name/coordinates, photo count, scrollable thumbnail list, viewer handoff. |
| `tests/map-aggregates.test.js` | node:test unit tests for `lib/map.js`. |
| `tests/e2e/specs/map.e2e.spec.js` | E2E: view registration, tiles/attribution, markers, clustering, popups, viewer handoff, keyboard popup cycle. |
| `tests/e2e/specs/map-filters.e2e.spec.js` | E2E: year/month filter, `location:` search, semantic search, album scoping, back/forward, videos with coordinates. |
| `tests/e2e/specs/map-a11y.e2e.spec.js` | E2E: keyboard Tab path, reduced motion, mobile attribution, axe rules, tile-failure degradation, loading/error/Retry recovery. |

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
| `tests/e2e/setup/test-helpers.js` | `stubMapTiles`, `setPhotoCoordinates`, `setPhotoLocationInDb`, `clearPhotoLocationInDb` helpers. |
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

and add `tile_url,` to the returned `Config { .. }` literal — and to every other
`Config { .. }` literal in the crate (`src/thumbnail_generator.rs`,
`src/video_processor.rs`, the test fixtures): they no longer compile until each one
carries the new field, so fix them before the test run below.

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
| `TURBO_PIX_TILE_URL` | Raster tile endpoint (`{z}/{x}/{y}` placeholders) used by the Map view | `https://tile.openstreetmap.org/{z}/{x}/{y}.png` | No |
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
            // `create_test_photo` zero-pads short hashes to 64 chars, so
            // "bulk1" and "bulk10" would collapse to the same padded hash
            // (11 collisions across 0..120). Fixed-width digits keep them
            // distinct.
            let mut photo =
                create_test_photo(format!("bulk_{index}.jpg"), format!("bulk{index:03}"));
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

        let mut in_album = create_test_photo("in_album.jpg".to_string(), "inalbum".to_string());
        in_album.metadata = json!({ "location": { "latitude": 1.0, "longitude": 2.0 } });
        in_album.create(&pool).await.unwrap();

        let outside = create_test_photo("outside.jpg".to_string(), "outside".to_string());
        outside.create(&pool).await.unwrap();

        let album =
            crate::albums::create_with_members(&pool, "Trip", &[in_album.hash_sha256.clone()])
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
                format!("map{index:03}"),
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

        let member =
            crate::db::tests::create_test_photo("member.jpg".to_string(), "member".to_string());
        member.create(&db_pool).await.unwrap();
        let stranger =
            crate::db::tests::create_test_photo("stranger.jpg".to_string(), "stranger".to_string());
        stranger.create(&db_pool).await.unwrap();

        let album = crate::albums::create_with_members(
            &db_pool,
            "Trip",
            std::slice::from_ref(&member.hash_sha256),
        )
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

    #[tokio::test]
    async fn test_map_invalid_query_param_returns_bad_request() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let routes = build_test_routes(db_pool, PathBuf::from("/tmp/turbo-pix-test-cache"));

        // `/api/photos/map` shares its path shape with the `{hash}` route, whose
        // NotFoundError used to win the combined rejection and answer 404
        // "Photo not found". The body message is what discriminates the two
        // (both branches are 4xx).
        for path in [
            "/api/photos/map?year=abc",
            "/api/photos/map?album=99999999999999999999",
        ] {
            let response = warp::test::request().path(path).reply(&routes).await;

            assert_eq!(response.status(), 400, "{} must be a client error", path);
            let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
            assert_eq!(body["error"], "Invalid query parameters", "{}", path);
        }

        // The `{hash}` route keeps its own answer: only the query contract
        // moved, so an unknown hash still reports the body the shadowed
        // rejection used to leak into the map route.
        let missing = warp::test::request()
            .path("/api/photos/does-not-exist")
            .reply(&routes)
            .await;

        assert_eq!(missing.status(), 404);
        let body: serde_json::Value = serde_json::from_slice(missing.body()).unwrap();
        assert_eq!(body["error"], "Photo not found");
    }
```

Note: `crate::db::tests` helpers are private to `db.rs` — if they are not `pub(crate)`, mark `create_test_photo`, `create_test_photo_with_date`, `create_test_db_pool` as `pub(crate)` inside the test module (the existing `handlers_photo` tests already reuse `create_in_memory_pool`).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib handlers_photo::tests::test_map_photos -- --nocapture`
Expected: FAIL — 404 (route not found).

- [ ] **Step 3: Implement the query structs + handler**

Add near `PhotoQuery` in `src/handlers_photo.rs` — and add `handle_rejection` to the existing `use crate::warp_helpers::{...}` import, since the handler answers the malformed-query case itself:

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
    query: Option<MapPhotoQuery>,
    db_pool: DbPool,
) -> Result<warp::reply::Response, Rejection> {
    // `None` is a query `warp::query` could not deserialize; see the map route.
    let Some(query) = query else {
        // Reuse the shared rejection handler so the 400 body is byte-identical
        // to `/api/photos?page=abc`. `handle_rejection` is infallible, hence the
        // unreachable arm.
        let reply = handle_rejection(reject::custom(ValidationError {
            message: "Invalid query parameters".to_string(),
        }))
        .await
        .unwrap_or_else(|never| match never {});
        return Ok(reply.into_response());
    };

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
        Ok(photos) => Ok(warp::reply::json(&MapPhotosResponse { photos }).into_response()),
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
        // A malformed parameter (`?year=abc`) makes `warp::query` reject with
        // `InvalidQuery`, but that rejection arrives *together* with the
        // `{hash}` route's `NotFoundError` (it matches `/api/photos/map` as
        // well) and `handle_rejection` tests `NotFoundError` first — answering
        // 404 "Photo not found" instead of the project's 400. Extract an
        // `Option` so the map route answers the malformed case itself.
        .and(
            warp::query::<MapPhotoQuery>()
                .map(Some)
                .or(warp::any().map(|| None))
                .unify(),
        )
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
- Create: `frontend/src/lib/query.js` (the `isPrefixQuery` check moves here out of `utils.js`, so `node --test` can load it without Svelte)
- Modify: `frontend/src/lib/utils.js` (drop the moved helper)
- Modify: `frontend/src/components/PhotoGrid.svelte` / `frontend/src/components/SearchBar.svelte` (import `isPrefixQuery` from `./query.js`)
- Create: `frontend/src/lib/map.js`
- Create: `tests/map-aggregates.test.js`
- Modify: `frontend/src/lib/api.js` (add `getMapPhotos` next to `getPhotos` at :89)
- Modify: `.github/workflows/ci.yml` (lint-format job)

**Interfaces:**
- Consumes: `api.getPhotos`-style params; `isPrefixQuery` from `./query.js` (pure, so plain `node --test` can load it); `api.semanticSearch`, `api.getPhoto` (via an injected client, never imported).
- Produces (exact signatures, used by later tasks):
  - `getPhotoCoordinates(photo) -> { latitude, longitude } | null`
  - `groupPhotosByLocation(photos) -> Array<{ key, latitude, longitude, photos: Photo[] }>` (input order preserved)
  - `getLocationLabel(location) -> string | null`
  - `formatCoordinates({ latitude, longitude }) -> string` (6 decimals, comma separator)
  - `wrapLongitudeForView(lng, west, east) -> number` (visible ±360° copy, `lng` unchanged when no copy is on screen)
  - `buildMapFilters(route) -> { query, sort, order, year, month }`
  - `isSemanticQuery(query) -> boolean`
  - `fetchSemanticPhotoSet(client, query, { signal } = {}) -> Promise<Photo[]>`
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
  wrapLongitudeForView,
} from '../frontend/src/lib/map.js';

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

test('wrapLongitudeForView leaves a longitude already inside the window untouched', () => {
  assert.equal(wrapLongitudeForView(13.4, -155.35, 182.15), 13.4);
  assert.equal(wrapLongitudeForView(-74, -155.35, 182.15), -74);
  assert.equal(wrapLongitudeForView(0, 0, 10), 0);
  assert.equal(wrapLongitudeForView(10, 0, 10), 10);
});

test('wrapLongitudeForView moves a longitude onto its visible ±360 copy', () => {
  // A window panned east across the seam shows the +360 copy.
  assert.equal(wrapLongitudeForView(-175, 170, 200), 185);
  assert.equal(wrapLongitudeForView(-170, 21.25, 358.75), 190);
  // The mirror: a window panned west across the seam shows the -360 copy.
  assert.equal(wrapLongitudeForView(175, -198.75, 138.75), -185);
  // Panning is unbounded, so the window can sit a whole world further east
  // than the probe above: the +720 copy is the visible one.
  assert.equal(wrapLongitudeForView(13.4, 720, 1058.8), 733.4);
});

test('wrapLongitudeForView keeps a longitude with no visible copy as-is', () => {
  assert.equal(wrapLongitudeForView(-100, 10, 20), -100);
  assert.equal(wrapLongitudeForView(100, 10, 20), 100);
});

test('wrapLongitudeForView never pushes an on-screen longitude out of view', () => {
  // 1920px at zoom 3 centred on Berlin (13.4°): window [-155.35, 182.15].
  // The old edge-only check shifted every negative longitude by +360 and drew
  // this photo at 286°, 104° beyond the right edge.
  assert.equal(wrapLongitudeForView(-74, -155.35, 182.15), -74);
  // Mirror window centred on -30° (west -198.75, east 138.75): a positive
  // longitude inside it must not become L - 360.
  assert.equal(wrapLongitudeForView(120, -198.75, 138.75), 120);
  assert.equal(wrapLongitudeForView(13.4, -198.75, 138.75), 13.4);
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
  const calls = [];
  const client = {
    semanticSearch: async (query, limit, offset) => {
      calls.push({ query, limit, offset });
      if (offset === 0) {
        // Simulate a full page so the loop must continue with the next offset.
        return { results: Array.from({ length: 200 }, (_, i) => ({ hash: `h${i}` })) };
      }
      return { results: [{ hash: 'h200' }] };
    },
    getPhoto: async (hash) => {
      if (hash === 'h5') throw new Error('gone');
      return { hash_sha256: hash, metadata: { location: { latitude: 1, longitude: 2 } } };
    },
  };

  const photos = await fetchSemanticPhotoSet(client, 'dogs');
  assert.deepEqual(calls, [
    { query: 'dogs', limit: 200, offset: 0 },
    { query: 'dogs', limit: 200, offset: 200 },
  ]);
  assert.equal(photos.length, 200);
  assert.equal(photos[0].hash_sha256, 'h0');
  assert.equal(photos.at(-1).hash_sha256, 'h200');
  assert.ok(!photos.some((entry) => entry.hash_sha256 === 'h5'));
});
```

(Plain `node --test` cannot import `lib/api.js` — it links `lib/utils.js` →
`lib/i18n.js` → `../i18n/en.json`, which Node rejects without an import
attribute. The client is injected instead, which is also what the Map view does.)

- [ ] **Step 2: Run to verify it fails**

Run: `node --test tests/map-aggregates.test.js`
Expected: FAIL — cannot resolve `frontend/src/lib/map.js`.

- [ ] **Step 3: Implement `frontend/src/lib/map.js`**

```js
// Map view data helpers. Pure functions (unit-tested by tests/map-aggregates.test.js)
// plus the semantic-search result-set loader the Map view shares with the grid's
// search semantics. Imports only the pure query-token module, so the whole file
// stays unit-testable with node --test — the api client is injected.
import { isPrefixQuery } from './query.js';

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
 * Moves a longitude into the copy of the viewport Leaflet actually shows.
 * Markers are placed via `latLngToLayerPoint`/`project`, which never wrap
 * longitude — only the tile URL wraps — so `bounds` is the raw visible
 * window, e.g. [-155.35, 182.15] for a 1920px window at zoom 3 centred on
 * Berlin. Panning is unbounded, so the window can sit more than one world away
 * from the canonical longitude: pick the `±360°` copy nearest the window
 * centre, and use it only when it falls inside the window, so a photo can
 * never be pushed off-screen by a seam. Returns `lng` unchanged when no copy
 * is visible.
 * @param {number} lng - photo longitude, between -180 and 180
 * @param {number} west - bounds.getWest()
 * @param {number} east - bounds.getEast()
 * @returns {number}
 */
export function wrapLongitudeForView(lng, west, east) {
  const shifted = lng + 360 * Math.round(((west + east) / 2 - lng) / 360);
  return shifted >= west && shifted <= east ? shifted : lng;
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
 * @param {{ semanticSearch: Function, getPhoto: Function }} client
 * @param {string} query
 * @param {{ signal?: AbortSignal }} [options]
 * @returns {Promise<Array>}
 */
export async function fetchSemanticPhotoSet(client, query, { signal } = {}) {
  const cleanQuery = query.startsWith('@') ? query.substring(1).trim() : query;
  const photos = [];

  for (let offset = 0; ; offset += SEMANTIC_PAGE_SIZE) {
    const page = await client.semanticSearch(cleanQuery, SEMANTIC_PAGE_SIZE, offset, { signal });
    const hashes = (page?.results ?? []).map((result) => result.hash);
    if (hashes.length === 0) break;

    for (let index = 0; index < hashes.length; index += HYDRATE_CONCURRENCY) {
      const chunk = hashes.slice(index, index + HYDRATE_CONCURRENCY);
      const hydrated = await Promise.all(
        chunk.map((hash) => client.getPhoto(hash, { signal }).catch(() => null))
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
Expected: PASS (12 tests).

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
- Modify: `tests/e2e/setup/test-helpers.js` (add `stubMapTiles`)
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

- [ ] **Step 4: Add the tile-stub helper, then write the failing E2E shell spec**

Global constraint: map specs never touch the network, and this task's shell spec already
stubs tiles in `beforeEach` — so the helper lands here, not in Task 6 (which consumes it).

In `tests/e2e/setup/test-helpers.js` add (top of the class, next to `selectors`):

```js
  /**
   * 1×1 PNG — a valid image response for stubbed tile requests. The committed
   * bytes decode to a single opaque pixel, RGBA (19, 87, 138, 255): the map
   * specs need a decodable image, never a transparent one.
   */
  static TINY_PNG = Buffer.from(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mMQDu/6DwADaQH0rwEuVwAAAABJRU5ErkJggg==',
    'base64'
  );

  /**
   * Answers every slippy-map tile request with TINY_PNG so map specs run
   * without network access. The pathname shape (`/{z}/{x}/{y}.png`) matches the
   * default OSM endpoint and any custom TURBO_PIX_TILE_URL template.
   */
  static async stubMapTiles(page) {
    await page.route(
      (url) => /\/\d+\/\d+\/\d+\.png$/.test(url.pathname),
      (route) =>
        route.fulfill({ status: 200, contentType: 'image/png', body: TestHelpers.TINY_PNG })
    );
  }
```

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
  import { flushSync, mount, onMount, unmount, untrack } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
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
  const CLUSTER_RADIUS = 60;
  const CLUSTER_MAX_ZOOM = 18;
  const INITIAL_CENTER = [20, 0];
  const INITIAL_ZOOM = 2;
  const FIT_MAX_ZOOM = 14;

  // FR-018: the OS "reduce motion" preference disables every map transition.
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
  let resizeObserver = null;
  let abortController = null;
  let loadToken = 0;
  let anyTileLoaded = false;
  let hasFittedOnce = false;
  // Location key of the open popup, and whether a marker re-render was asked
  // for while it was open (see renderClusters).
  let openPopupKey = null;
  let renderPending = false;
  // Set when the popup is dismissed from inside itself, so focus returns to the
  // marker without guessing from the browser's focus teardown.
  let restoreFocusOnClose = false;
  // Marker → mounted MapPopup component, so a popup's DOM is torn down with it.
  // (SvelteMap: the linter rejects a plain Map in component scope; this one is
  // only ever read imperatively.)
  const popupHandles = new SvelteMap();
  // Location key → its current marker, so focus can be restored to the marker
  // that survives a re-render.
  const locationMarkers = new SvelteMap();

  const locations = $derived(groupPhotosByLocation(photos));
  const unlocatedCount = $derived(
    photos.length - locations.reduce((count, location) => count + location.photos.length, 0)
  );
  const showEmptyState = $derived(!loading && !loadError && locations.length === 0);

  // ── Data loading ──────────────────────────────────────────────────────────

  async function fetchPhotoSet(currentRoute, signal) {
    if (currentRoute.album != null) {
      // Mirror the grid's album detail listing: sort/order + album scope only.
      const { sort, order } = buildMapFilters(currentRoute);
      const response = await api.getMapPhotos(
        { album: currentRoute.album, sort, order },
        { signal }
      );
      return response.photos ?? [];
    }

    if (isSemanticQuery(currentRoute.query)) {
      return fetchSemanticPhotoSet(api, currentRoute.query, { signal });
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
      maxZoom: FIT_MAX_ZOOM,
      animate: !prefersReducedMotion,
    });
  }

  onMount(() => {
    map = L.map(mapEl, {
      center: INITIAL_CENTER,
      zoom: INITIAL_ZOOM,
      zoomControl: true,
      attributionControl: true,
      maxZoom: MAX_ZOOM,
      // FR-018: reduced motion preference suppresses animated pan/zoom/fade.
      zoomAnimation: !prefersReducedMotion,
      fadeAnimation: !prefersReducedMotion,
      markerZoomAnimation: !prefersReducedMotion,
    });

    // Attribution is registered on the control itself, independently of the
    // tile layer, so it stays visible in the degraded no-tiles state (SC-005).
    map.attributionControl.addAttribution(attributionHtml());

    if (appState.tileUrl) {
      tileLayer = L.tileLayer(appState.tileUrl, { maxZoom: MAX_ZOOM });
      tileLayer.on('tileerror', () => {
        if (!anyTileLoaded) tilesFailed = true;
      });
      tileLayer.on('tileload', () => {
        anyTileLoaded = true;
        tilesFailed = false;
      });
      tileLayer.addTo(map);
    } else {
      // No configured endpoint — the map stays usable without a background.
      tilesFailed = true;
    }

    // Markers live in their own group so a re-render can replace them without
    // touching tiles or the map itself.
    clusterLayer = L.layerGroup().addTo(map);
    map.on('moveend zoomend', handleViewChange);
    const container = map.getContainer();
    container.addEventListener('keydown', handleMapKeydown);

    // The shell resizes (sidebar toggle, window resize) without remounting the
    // view, so Leaflet must be told to re-measure its container.
    resizeObserver = new ResizeObserver(() => map?.invalidateSize());
    resizeObserver.observe(mapEl);

    // If the result set resolved before the map existed, fit it now.
    fitToLocationsOnce();

    return () => {
      resizeObserver?.disconnect();
      resizeObserver = null;
      abortController?.abort();
      map?.off('moveend zoomend', handleViewChange);
      container.removeEventListener('keydown', handleMapKeydown);
      // Leaflet does not remove layers on map.remove(), so a popup left open by
      // a view change would keep its component instance alive.
      for (const handle of popupHandles.values()) {
        void unmount(handle);
      }
      popupHandles.clear();
      locationMarkers.clear();
      openPopupKey = null;
      renderPending = false;
      map?.remove();
      map = null;
      tileLayer = null;
      clusterLayer = null;
      clusterIndex = null;
    };
  });

  $effect(() => {
    // FR-004/SC-003: the map shows exactly the photos the active filters select.
    route.view;
    route.query;
    route.sort;
    route.year;
    route.month;
    route.album;
    untrack(() => loadPhotos());
  });

  $effect(() => {
    // FR-006: cluster the unique coordinate locations; each point carries its
    // location's photo count so the index itself can aggregate photos, and the
    // index is rebuilt whenever the result set changes (markers follow
    // immediately).
    clusterIndex = new Supercluster({
      radius: CLUSTER_RADIUS,
      maxZoom: CLUSTER_MAX_ZOOM,
      map: (properties) => ({ photoCount: properties.photoCount }),
      reduce: (accumulated, properties) => {
        accumulated.photoCount += properties.photoCount;
      },
    }).load(
      locations.map((location) => ({
        type: 'Feature',
        geometry: { type: 'Point', coordinates: [location.longitude, location.latitude] },
        properties: { key: location.key, photoCount: location.photos.length },
      }))
    );
    untrack(() => renderClusters());
  });

  function renderClusters() {
    // Implemented in Task 6.
  }

  function handleViewChange() {
    // Implemented in Task 6.
  }

  function handleMapKeydown() {
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
        <button type="button" class="btn-primary" onclick={() => loadPhotos()}>
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

  /* The canvas is absolutely positioned so Leaflet always measures a definite
     box, whatever the flex height resolution of the surrounding shell is. */
  .map-stage {
    position: relative;
    flex: 1 1 auto;
    min-height: 260px;
  }

  .map-canvas {
    position: absolute;
    inset: 0;
    background: var(--surface-color);
  }

  /* Above every Leaflet pane (markers 600, popups 700) but below the control
     corners (1000), so the zoom control and the attribution stay reachable. */
  .map-overlay {
    position: absolute;
    inset: 0;
    z-index: 800;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    background: var(--background-color);
    color: var(--text-secondary);
    text-align: center;
  }

  .map-canvas:focus-visible {
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
git add package.json package-lock.json frontend/src tests/i18n-integrity.test.js tests/e2e/setup/test-helpers.js tests/e2e/specs/map.e2e.spec.js
git commit -m "feat(map): register the map view with tiles, attribution, and states"
```

---

### Task 6: Markers, clusters, popups, viewer handoff, keyboard

**Files:**
- Modify: `frontend/src/components/MapView.svelte` (`renderClusters` + events)
- Create: `frontend/src/components/MapPopup.svelte`
- Modify: `tests/e2e/setup/test-helpers.js` (uses `stubMapTiles` from Task 5; add `setPhotoCoordinates`)
- Test: `tests/e2e/specs/map.e2e.spec.js` (part 2)

**Interfaces:**
- Consumes: `locations` (Task 5), `getLocationLabel`/`formatCoordinates` (Task 4), `openViewer` window event contract (`detail: { photo, photos }`).
- Produces: marker DOM contract — individual location markers `[data-map-location]` (`data-map-location-count`), cluster markers `[data-map-cluster]` (aggregated photo count, also the bubble text); popup items `[data-map-popup-photo="<hash>"]`.

- [ ] **Step 1: Add the coordinate-writing E2E helper**

`stubMapTiles` already landed in Task 5 Step 4 — the shell spec stubs tiles there. Add the
coordinate write next to it in `tests/e2e/setup/test-helpers.js`:

```js
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

Add the shared poll helper at the top of the file — the specs below wait on drawn features, not on the load overlay, because the marker assertions outlive it:

```js
/** Waits until the map has drawn at least one feature. */
async function waitForMapFeatures(page) {
  await expect
    .poll(async () => page.locator('[data-map-cluster], [data-map-location]').count(), {
      timeout: 15000,
    })
    .toBeGreaterThan(0);
}
```

Then append (the seeded library holds one unique coordinate, which would leave every cluster assertion skipped, so the `describe` seeds a second, nearby one first):

```js
  // The seeded library holds one unique coordinate, which would leave every
  // cluster assertion skipped. Moving one photo to a nearby-but-distinct point
  // makes the fitted view cluster deterministically: `radius: 60` is a ~30px
  // threshold at 256px tiles, and the pair sits ~8px apart at the fit zoom.
  test.beforeAll(async ({ browser }, testInfo) => {
    const context = await browser.newContext({ baseURL: testInfo.project.use.baseURL });
    try {
      const page = await context.newPage();
      // This page navigates on its own, so the per-test beforeEach stub does
      // not cover it: without the stub the seeding pass issues real
      // tile.openstreetmap.org requests.
      await TestHelpers.stubMapTiles(page);
      await TestHelpers.goto(page, '/map');
      const pair = await page.evaluate(async () => {
        const response = await fetch('/api/photos/map');
        const { photos } = await response.json();
        const byLocation = new Map();
        for (const photo of photos) {
          const latitude = photo.metadata?.location?.latitude;
          const longitude = photo.metadata?.location?.longitude;
          if (photo.mime_type !== 'image/jpeg') continue;
          if (typeof latitude !== 'number' || typeof longitude !== 'number') continue;
          const key = `${latitude},${longitude}`;
          if (!byLocation.has(key)) byLocation.set(key, { latitude, longitude, hashes: [] });
          byLocation.get(key).hashes.push(photo.hash_sha256);
        }
        // Anchor on the densest location so the pair always sits next to it,
        // whatever an earlier run left behind.
        const [anchor] = [...byLocation.values()].sort((a, b) => b.hashes.length - a.hashes.length);
        return { hash: anchor.hashes[0], latitude: anchor.latitude, longitude: anchor.longitude };
      });
      await TestHelpers.setPhotoCoordinates(
        page,
        pair.hash,
        pair.latitude + 0.0005,
        pair.longitude
      );
      // The metadata write makes the server re-index the file: wait until the
      // listing actually shows the split, so no test races that re-index.
      await expect
        .poll(
          async () =>
            page.evaluate(async () => {
              const response = await fetch('/api/photos/map');
              const { photos } = await response.json();
              const keys = new Set(
                photos
                  .filter((photo) => typeof photo.metadata?.location?.latitude === 'number')
                  .map(
                    (photo) =>
                      `${photo.metadata.location.latitude},${photo.metadata.location.longitude}`
                  )
              );
              return keys.size;
            }),
          { timeout: 15000 }
        )
        .toBeGreaterThan(1);
    } finally {
      await context.close();
    }
  });

  test('plots every geo-located photo as a marker or cluster', async ({ page }) => {
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();

    const expectedPhotos = await page.evaluate(async () => {
      const response = await fetch('/api/photos/map');
      const { photos } = await response.json();
      let located = 0;
      for (const photo of photos) {
        const latitude = photo.metadata?.location?.latitude;
        const longitude = photo.metadata?.location?.longitude;
        if (typeof latitude === 'number' && typeof longitude === 'number') located += 1;
      }
      return located;
    });
    test.skip(expectedPhotos === 0, 'No geo-located photos in the test library');

    // Supercluster guarantees every point is represented, and each feature
    // announces the PHOTOS it stands for — a cluster the ones it aggregates
    // (FR-006), a marker its location's count (FR-009) — so both kinds must add
    // up to the library's geo-located photo count.
    await expect
      .poll(async () =>
        page.evaluate(() => {
          const announced = (selector, attribute) =>
            [...document.querySelectorAll(selector)].reduce(
              (sum, element) => sum + Number(element.getAttribute(attribute)),
              0
            );
          return (
            announced('[data-map-cluster]', 'data-map-cluster') +
            announced('[data-map-location]', 'data-map-location-count')
          );
        })
      )
      .toBe(expectedPhotos);
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

  test('cluster markers expand on click and on Enter', async ({ page }) => {
    await TestHelpers.goto(page, '/map');
    await waitForMapFeatures(page);

    // FR-006: the fitted view holds the seeded pair in one cluster, whose
    // bubble, data attribute, and label carry the PHOTOS it aggregates. The
    // pair's cluster covers 2 locations but all the library's geo-located
    // photos, so announcing the location count (2) fails here.
    const expectedClusterPhotos = await page.evaluate(async () => {
      const response = await fetch('/api/photos/map');
      const { photos } = await response.json();
      return photos.filter((photo) => {
        const latitude = photo.metadata?.location?.latitude;
        const longitude = photo.metadata?.location?.longitude;
        return typeof latitude === 'number' && typeof longitude === 'number';
      }).length;
    });

    const cluster = page.locator('[data-map-cluster]').first();
    await expect(cluster).toBeVisible();
    await expect(cluster).toHaveAttribute('data-map-cluster', String(expectedClusterPhotos));
    await expect(cluster).toHaveAttribute(
      'aria-label',
      `${expectedClusterPhotos} photos, activate to zoom in`
    );
    await expect(cluster).toHaveText(String(expectedClusterPhotos));

    const before = await page.locator('[data-map-location]').count();
    await cluster.click();
    await expect
      .poll(async () => page.locator('[data-map-location]').count())
      .toBeGreaterThan(before);

    // A fresh load restores the fitted (clustered) view for the keyboard pass.
    await TestHelpers.goto(page, '/map');
    await waitForMapFeatures(page);
    const keyboardCluster = page.locator('[data-map-cluster]').first();
    await expect(keyboardCluster).toBeVisible();

    const beforeKeyboard = await page.locator('[data-map-location]').count();
    await keyboardCluster.focus();
    await page.keyboard.press('Enter');
    await expect
      .poll(async () => page.locator('[data-map-location]').count())
      .toBeGreaterThan(beforeKeyboard);
  });

  test('keyboard opens a popup, Escape closes it and restores focus, Enter reopens it', async ({
    page,
  }) => {
    await TestHelpers.goto(page, '/map');
    await waitForMapFeatures(page);

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

    const popup = page.locator('.leaflet-popup');
    await expect(popup).toBeVisible();
    await expect(popup.locator('[data-map-popup-photo]').first()).toBeFocused();

    await page.keyboard.press('Escape');
    await expect(popup).toBeHidden();
    await expect(page.locator('[data-map-location]:focus')).toHaveCount(1);

    // The reopen is the regression guard: focus has to land inside the popup
    // again, or Escape would have nothing to act on.
    await page.keyboard.press('Enter');
    await expect(popup).toBeVisible();
    await expect(popup.locator('[data-map-popup-photo]').first()).toBeFocused();

    await page.keyboard.press('Escape');
    await expect(popup).toBeHidden();
  });

  test('closing a popup with its close button returns focus to the marker', async ({ page }) => {
    // FR-018: the popup's own close button is the dismissal a keyboard user
    // reaches by Tab, so it must hand focus back exactly like Escape does. On
    // the default configuration Leaflet fades the popup out, which defers its
    // DOM removal — and the focus inside it — by 200 ms past `popupclose`, so
    // the assertion below can only pass if the handback reads where focus was
    // instead of where the browser's teardown left it.
    await TestHelpers.goto(page, '/map');
    await waitForMapFeatures(page);

    for (let attempt = 0; attempt < 5; attempt += 1) {
      if ((await page.locator('[data-map-location]').count()) > 0) break;
      const cluster = page.locator('[data-map-cluster]').first();
      if ((await cluster.count()) === 0) break;
      await cluster.click();
      await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
    }

    const marker = page.locator('[data-map-location]').first();
    await expect(marker).toBeVisible();
    const key = await marker.getAttribute('data-map-location');
    await marker.focus();
    await page.keyboard.press('Enter');

    const popup = page.locator('.leaflet-popup');
    await expect(popup).toBeVisible();
    await expect(popup.locator('[data-map-popup-photo]').first()).toBeFocused();

    await popup.locator('.leaflet-popup-close-button').click();
    // `toBeHidden` waits out the deferred removal, which is what drops focus to
    // <body> — asserting before it lands would pass on the broken behaviour.
    await expect(popup).toBeHidden();

    await expect(page.locator(`[data-map-location="${key}"]`)).toBeFocused();
  });
```

- [ ] **Step 3: Run to verify they fail**

```bash
npm run build && cargo build --bin turbo-pix && npx playwright test tests/e2e/specs/map.e2e.spec.js --reporter=line
```
Expected: FAIL — no `[data-map-location]` / `[data-map-cluster]` markers.

- [ ] **Step 4: Implement marker rendering in `MapView.svelte`**

Replace the `renderClusters`/`handleViewChange`/`handleMapKeydown` stubs (and add the popup/lifecycle helpers). The `<script>` already imports the Svelte entry points declared in Step 6 — patch only the genuinely new imports:

```js
  import {
    buildMapFilters,
    fetchSemanticPhotoSet,
    formatCoordinates,
    getLocationLabel,
    groupPhotosByLocation,
    isSemanticQuery,
    wrapLongitudeForView,
  } from '../lib/map.js';
  import MapPopup from './MapPopup.svelte';

  // `wrapLongitudeForView` lives in lib/map.js: Leaflet projects markers with
  // latLngToLayerPoint and never wraps longitude (only tile URLs wrap), so
  // `map.getBounds()` is the raw visible window — the shift must be rejected
  // when the ±360° copy is not inside it (SC: antimeridian photos render).

  /** Count bubble; grows with the number it carries so 3+ digits stay readable. */
  function clusterIcon(count) {
    const size = count < 10 ? 34 : count < 100 ? 42 : 48;
    return L.divIcon({
      className: 'map-cluster-marker',
      html: `<span>${count}</span>`,
      iconSize: [size, size],
      iconAnchor: [size / 2, size / 2],
    });
  }

  /** A location is one dot, whatever the number of photos behind it (FR-009). */
  function locationIcon() {
    return L.divIcon({ className: 'map-location-marker', iconSize: [18, 18], iconAnchor: [9, 9] });
  }

  function labelFor(location) {
    return getLocationLabel(location) ?? formatCoordinates(location);
  }

  /**
   * Leaflet only opens a popup on Enter (its own `keypress` path), so both
   * marker kinds handle Enter and Space themselves (FR-018).
   */
  function activateOnKeyboard(element, handler) {
    element.addEventListener('keydown', (event) => {
      if (event.key !== 'Enter' && event.key !== ' ') return;
      // The default would let Leaflet's keypress handler toggle the popup shut
      // again right after this opens it.
      event.preventDefault();
      handler();
    });
  }

  /**
   * Moves focus onto the popup's first thumbnail (FR-018).
   *
   * Leaflet's `DivOverlay.update()` writes and clears an inline
   * `visibility: hidden` on the popup container while it lays the popup out.
   * Under `prefers-reduced-motion` the global `transition-duration: 0.01ms` rule
   * pairs with the initial `transition-property: all`, so that write becomes a
   * transition: the container — and then the thumbnails inside it, one nesting
   * level per frame — still compute `hidden` when `popupopen` fires, and
   * Chromium refuses focus on a hidden element. Retry on the frames the
   * transition needs; the cap keeps a popup that never gets a thumbnail from
   * spinning, and a closed popup ends the retries.
   */
  function focusFirstPopupItem(popupNode, popupEl, framesLeft = 8) {
    popupNode.querySelector('button')?.focus();
    // Done once focus is inside the popup at all: the thumbnail focus landed, or
    // the user has already moved on within it.
    if (popupEl.contains(document.activeElement) || !popupNode.isConnected || framesLeft === 0) {
      return;
    }
    requestAnimationFrame(() => focusFirstPopupItem(popupNode, popupEl, framesLeft - 1));
  }

  function bindLocationMarker(marker, location) {
    const popupNode = document.createElement('div');
    popupNode.className = 'map-popup-host';
    marker.bindPopup(popupNode, {
      maxWidth: 320,
      minWidth: 260,
      // FR-018: `autoPan` animates a 250 ms `panBy`, so the preference turns
      // the pan off — the popup still opens, it just arrives without movement.
      autoPan: !prefersReducedMotion,
      closeButton: true,
    });

    // Where focus was inside the popup when it closed. With the fade animation
    // Leaflet's removal is deferred, so the popup is still connected — and still
    // holds focus — when `popupclose` fires; the reduced-motion path has already
    // detached it and dropped focus to <body>. Both signatures are read below.
    let lastFocused = null;
    popupNode.addEventListener('focusin', (event) => {
      lastFocused = event.target;
    });

    marker.on('popupopen', () => {
      lastFocused = null;
      openPopupKey = location.key;
      popupHandles.set(
        marker,
        mount(MapPopup, {
          target: popupNode,
          props: { location, onOpenPhoto: (photo) => openViewer(photo) },
        })
      );
      // Popup panes come after the marker pane in DOM order, but with many
      // markers Tab would walk through every marker first — move focus into the
      // popup so keyboard users land on the thumbnails (FR-018). flushSync makes
      // the freshly mounted markup available right here, so every (re)open lands
      // on a thumbnail.
      flushSync();
      focusFirstPopupItem(popupNode, marker.getPopup().getElement());
    });

    marker.on('popupclose', () => {
      // Keyboard dismissal sets the flag. Otherwise ask where focus was rather
      // than where the browser's teardown left it: on the default configuration
      // the popup is faded out, so its DOM keeps focus for another 200 ms and
      // `activeElement` still sits inside it here, while the reduced-motion path
      // has detached the popup and dropped focus to <body>.
      const popupEl = marker.getPopup()?.getElement();
      const focusDropped =
        restoreFocusOnClose ||
        (popupEl != null && popupEl.contains(document.activeElement)) ||
        (!lastFocused?.isConnected && document.activeElement === document.body);
      restoreFocusOnClose = false;
      lastFocused = null;
      const handle = popupHandles.get(marker);
      if (handle) {
        popupHandles.delete(marker);
        void unmount(handle);
      }
      if (openPopupKey === location.key) openPopupKey = null;
      // A render requested while the popup was open replaces every marker, so it
      // is replayed on the next frame instead of here: Leaflet closes the popup
      // from the synthetic `preclick` it dispatches BEFORE it resolves the
      // `click` target, so clearing the layers inside that dispatch would detach
      // the clicked marker and swallow its popup / cluster expansion until a
      // second click. `fromViewChange` keeps the wait-for-popup rule for the
      // popup that click may have opened by then; the replay moves focus to the
      // replacement marker itself, so the handback below stays on the element
      // still in the DOM.
      if (renderPending) {
        renderPending = false;
        requestAnimationFrame(() => renderClusters({ fromViewChange: true }));
      }
      // Only take focus back if the popup dropped it: a mouse user closing the
      // popup left focus on the map container and must not have it yanked away.
      const icon = (locationMarkers.get(location.key) ?? marker).getElement();
      if (focusDropped && icon?.isConnected) icon.focus();
    });
  }

  /** Zooms to the level at which the cluster's members become individual dots. */
  function expandCluster(clusterId, latlng) {
    if (!map || !clusterIndex) return;
    const zoom = clusterIndex.getClusterExpansionZoom(clusterId);
    map.setView(latlng, Math.min(zoom, MAX_ZOOM), { animate: !prefersReducedMotion });
  }

  /** Pan/zoom redraws are the ones that must wait for an open popup to close. */
  function handleViewChange() {
    renderClusters({ fromViewChange: true });
  }

  /**
   * Escape dismisses the open popup while it is the focused surface. Leaflet's
   * own Escape handling only runs while the map container itself has focus; with
   * focus inside the popup it is unhooked, so the popup would be undismissable.
   */
  function handleMapKeydown(event) {
    if (event.key !== 'Escape' || !openPopupKey) return;
    if (!map?.getPane('popupPane')?.contains(event.target)) return;
    event.preventDefault();
    event.stopPropagation();
    restoreFocusOnClose = true;
    map.closePopup();
  }

  /**
   * Redraws the markers. `fromViewChange` marks the pan/zoom path: removing a
   * marker closes the popup bound to it (Leaflet's `bindPopup` registers
   * `remove: closePopup`), and opening a popup pans the map — so rendering on
   * that pan's `moveend` would close the popup the pan was for. Only that path
   * waits for the popup to close; a data change drops the popup and redraws
   * immediately, so the markers never lag the active filters (FR-004/SC-003).
   */
  function renderClusters({ fromViewChange = false } = {}) {
    if (!map || !clusterLayer || !clusterIndex) return;

    if (openPopupKey) {
      if (fromViewChange) {
        renderPending = true;
        return;
      }
      // This path rebuilds every marker itself, so the popup's close handler
      // must not schedule the replay frame as well: consume the flag first.
      renderPending = false;
      map.closePopup();
    }

    const bounds = map.getBounds();
    const features = clusterIndex.getClusters(
      [bounds.getWest(), bounds.getSouth(), bounds.getEast(), bounds.getNorth()],
      Math.round(map.getZoom())
    );
    const byKey = new Map(locations.map((location) => [location.key, location]));
    // A re-render replaces every marker; a keyboard user parked on one keeps
    // their place by having focus moved to its replacement. A cluster is not
    // replaced — activating it zooms until its members become individual dots —
    // so its element disappears with nothing to inherit focus and the map
    // container takes it back instead of letting it fall to <body>.
    const activeElement = document.activeElement;
    const focusedLocation = activeElement?.getAttribute?.('data-map-location') ?? null;
    const focusedCluster =
      focusedLocation === null && activeElement?.hasAttribute?.('data-map-cluster');

    clusterLayer.clearLayers();
    locationMarkers.clear();

    for (const feature of features) {
      const [rawLongitude, latitude] = feature.geometry.coordinates;
      const longitude = wrapLongitudeForView(rawLongitude, bounds.getWest(), bounds.getEast());

      if (feature.properties.cluster) {
        // FR-006: the bubble, `data-map-cluster`, and the aria-label all carry
        // the photos the cluster aggregates — `point_count` counts the
        // locations behind it, which would understate a multi-photo location.
        const photoCount = feature.properties.photoCount;
        const marker = L.marker([latitude, longitude], {
          icon: clusterIcon(photoCount),
          keyboard: true,
        });
        const expand = () => expandCluster(feature.properties.cluster_id, [latitude, longitude]);
        marker.on('click', expand);
        marker.addTo(clusterLayer);
        const element = marker.getElement();
        if (element) {
          element.setAttribute('data-map-cluster', String(photoCount));
          element.setAttribute(
            'aria-label',
            get(t)('map.clusterLabel', {
              values: { count: photoCount },
              default: '{count} photos, activate to zoom in',
            })
          );
          activateOnKeyboard(element, expand);
        }
        continue;
      }

      const location = byKey.get(feature.properties.key);
      if (!location) continue;

      const marker = L.marker([latitude, longitude], {
        icon: locationIcon(),
        keyboard: true,
      });
      marker.addTo(clusterLayer);
      locationMarkers.set(location.key, marker);
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

    if (focusedLocation) {
      const focusedIcon = locationMarkers.get(focusedLocation)?.getElement();
      if (focusedIcon?.isConnected) focusedIcon.focus();
    } else if (focusedCluster) {
      map.getContainer().focus();
    }
  }

  function openViewer(photo) {
    // The viewer is a modal overlay: leaving the popup open behind it would keep
    // its thumbnails tabbable and let the map claim Escape from the viewer.
    map?.closePopup();
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
    /* White, not `--background-color`: on `--primary-color` the token reaches
       only 4.47:1, under the 4.5:1 the count needs at 13px (SC-008). White is
       what the app already puts on `--primary-color` (see `.btn-primary`) and
       measures 4.83:1. */
    color: white;
    font-size: var(--font-sm);
    font-weight: var(--font-semibold);
  }

  :global(.map-location-marker:focus-visible),
  :global(.map-cluster-marker:focus-visible) {
    outline: 2px solid var(--primary-color);
    outline-offset: 3px;
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
    width: var(--space-12);
    height: var(--space-12);
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
- Modify: `tests/e2e/setup/test-helpers.js` (add `setPhotoLocationInDb`, `clearPhotoLocationInDb`)
- Create: `tests/e2e/specs/map-filters.e2e.spec.js`
- Modify: `frontend/src/components/MapView.svelte` / `frontend/src/lib/map.js` **only if a test surfaces a real gap**

**Interfaces:**
- Consumes: everything above; the module-level `TEST_DB_PATH` (`test-e2e-data/database/turbo-pix.db`).
- Produces: verified filter semantics.

- [ ] **Step 1: Add the DB-seed helper (videos cannot receive EXIF)**

In `tests/e2e/setup/test-helpers.js` (with `import { execSync } from 'child_process'` and the module-level `const TEST_DB_PATH = 'test-e2e-data/database/turbo-pix.db'`):

```js
  /**
   * Writes coordinates straight into the indexed row — videos cannot take EXIF
   * writes, and a direct UPDATE needs no re-index. Every spec shares one
   * server and one database, so a spec that seeds a location must restore it
   * with clearPhotoLocationInDb before it finishes.
   */
  static setPhotoLocationInDb(fileName, latitude, longitude) {
    const metadata = JSON.stringify({ location: { latitude, longitude } });
    // `PRAGMA busy_timeout=5000` like every sibling seeding site: the sqlite3
    // CLI waits 0 ms by default, so a concurrent server write fails the UPDATE
    // and execSync throws.
    const sql =
      `PRAGMA busy_timeout=5000; ` +
      `UPDATE photos SET metadata = json_set(metadata, '$.location.latitude', ${latitude}, '$.location.longitude', ${longitude}) WHERE filename = '${fileName}'`;
    execSync(`sqlite3 "${TEST_DB_PATH}" "${sql}"`, { stdio: 'pipe' });
    return metadata;
  }

  /**
   * Reverts setPhotoLocationInDb: the row keeps a JSON null location, which the
   * map's coordinate validation (lib/map.js) and every query that reads
   * `metadata.location` treat as "no location".
   */
  static clearPhotoLocationInDb(fileName) {
    const sql =
      `PRAGMA busy_timeout=5000; ` +
      `UPDATE photos SET metadata = json_set(metadata, '$.location.latitude', null, '$.location.longitude', null) WHERE filename = '${fileName}'`;
    execSync(`sqlite3 "${TEST_DB_PATH}" "${sql}"`, { stdio: 'pipe' });
  }
```

- [ ] **Step 2: Write the specs**

Create `tests/e2e/specs/map-filters.e2e.spec.js`:

```js
import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

/** Geo-located photo count of a map endpoint response. */
async function expectedPhotos(page, endpoint) {
  return page.evaluate(async (url) => {
    const response = await fetch(url);
    const { photos } = await response.json();
    let located = 0;
    for (const photo of photos) {
      const latitude = photo.metadata?.location?.latitude;
      const longitude = photo.metadata?.location?.longitude;
      if (typeof latitude === 'number' && typeof longitude === 'number') located += 1;
    }
    return located;
  }, endpoint);
}

/**
 * Photos the map currently represents: a cluster announces the photos it
 * aggregates (FR-006) and a marker its location's photo count (FR-009), so
 * summing both element kinds covers every plotted photo exactly once.
 */
async function renderedPhotos(page) {
  return page.evaluate(() => {
    const announced = (selector, attribute) =>
      [...document.querySelectorAll(selector)].reduce(
        (sum, element) => sum + Number(element.getAttribute(attribute)),
        0
      );
    return (
      announced('[data-map-cluster]', 'data-map-cluster') +
      announced('[data-map-location]', 'data-map-location-count')
    );
  });
}

/**
 * Densest location of a map endpoint response as `[key, photoCount]`, derived
 * from the same filter the view is showing. Null when nothing is located.
 */
async function densestLocation(page, endpoint) {
  return page.evaluate(async (url) => {
    const response = await fetch(url);
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
  }, endpoint);
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

/**
 * The year of the newest dated photo. Read as UTC, like the server's
 * `strftime('%Y', taken_at)`: a local-year conversion could name the
 * neighbouring year right after New Year.
 */
async function latestYear(page) {
  return page.evaluate(async () => {
    const response = await fetch('/api/photos/map');
    const { photos } = await response.json();
    const withDate = photos.find((photo) => photo.taken_at);
    return withDate ? new Date(withDate.taken_at).getUTCFullYear() : null;
  });
}

/**
 * The map's own `/api/photos/map` request for a filter, so an assertion runs
 * against the filtered render instead of the one that was already on screen.
 */
function mapRequest(page, params) {
  return page.waitForResponse((response) => {
    const url = new URL(response.url());
    if (url.pathname !== '/api/photos/map') return false;
    return Object.entries(params).every(([key, value]) => url.searchParams.get(key) === value);
  });
}

/**
 * The loading overlay is removed after the loaded photo set has been applied,
 * so waiting for it to go away is what makes a state assertion (marker counts,
 * unlocated notice) read the finished render rather than a transient one.
 */
async function waitForMapLoad(page) {
  await expect(page.locator('[data-testid="map-loading"]')).toHaveCount(0);
}

test.describe('Map filters', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.stubMapTiles(page);
  });

  test('year filter plots exactly the geo-located subset of that year', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const year = await latestYear(page);
    test.skip(year === null, 'No dated photos in the test library');

    const filteredEndpoint = `/api/photos/map?year=${year}`;
    const expected = await expectedPhotos(page, filteredEndpoint);
    const densest = await densestLocation(page, filteredEndpoint);

    await page.evaluate((value) => {
      window.history.pushState({}, '', `/map?year=${value}`);
      window.dispatchEvent(new PopStateEvent('popstate'));
    }, year);
    await expect(page).toHaveURL(new RegExp(`/map\\?year=${year}`));

    // Filtered photos are a subset of the fitted initial viewport, so the
    // rendered representation must match the filtered set exactly (SC-003).
    await expect.poll(() => renderedPhotos(page)).toBe(expected);

    // Scoping, not just parity: the densest location's marker carries exactly
    // the filtered photos at that coordinate. Playwright retries the attribute
    // until the filtered reload has replaced the pre-filter render.
    if (densest) {
      const [key, count] = densest;
      const marker = await focusLocation(page, key);
      await expect(marker).toBeVisible();
      await expect(marker).toHaveAttribute('data-map-location-count', String(count));
    }
  });

  test('location: search plots exactly the matching geo-located subset', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    const city = await page.evaluate(async () => {
      const response = await fetch('/api/photos?limit=100');
      const { photos } = await response.json();
      return photos.find((photo) => photo.metadata?.location?.city)?.metadata.location.city ?? null;
    });
    test.skip(!city, 'No resolved city in the seeded photos');

    const query = `location:${city}`;
    const endpoint = `/api/photos/map?q=${encodeURIComponent(query)}`;
    const expected = await expectedPhotos(page, endpoint);
    const densest = await densestLocation(page, endpoint);
    expect(expected).toBeGreaterThan(0);

    // Wait for the map's own filtered request, so everything below reads the
    // filtered render instead of the unfiltered one still on screen.
    const filteredLoad = mapRequest(page, { q: query });
    await TestHelpers.performSearch(page, query);
    await filteredLoad;
    await waitForMapLoad(page);

    await expect.poll(() => renderedPhotos(page)).toBe(expected);
    // The unlocated notice renders only while the map is neither loading nor
    // errored, so a failed filtered load would leave it hidden and the check
    // below passing. Assert the load itself first.
    await expect(page.locator('[data-testid="map-error"]')).toHaveCount(0);
    // Teeth: every photo the city matches carries coordinates, while the
    // unfiltered render shows this notice for the unlocated videos, receipt,
    // and camera-EXIF fixture (`sample_with_exif.jpg` has no GPS tags).
    // A dropped query leaves the notice in place and fails here.
    await expect(page.locator('[data-testid="map-unlocated-notice"]')).toHaveCount(0);
    // Scoping, not just parity: the densest location's marker carries exactly
    // the filtered photos at its coordinate, not the library's photos there.
    if (densest) {
      const [key, count] = densest;
      const marker = await focusLocation(page, key);
      await expect(marker).toBeVisible();
      await expect(marker).toHaveAttribute('data-map-location-count', String(count));
    }
  });

  test('semantic search plots the geo-located subset of the CLIP result set', async ({ page }) => {
    // CLIP text encoding plus the sqlite-vec scan is CPU-bound on the E2E box:
    // measured 13-18s per query, and this test drives the map's query plus a
    // hydration pass. That is far beyond Playwright's 30s global budget, so the
    // budget is raised here (all waits below stay selector/response-driven).
    test.setTimeout(180_000);
    await TestHelpers.goto(page, '/map');

    // 'car' is what the seeded media actually show (test-data/car.jpg). Wait
    // for the map's own CLIP query and derive the expectation from its
    // response, so the parity assertion is about the same result set the map
    // plots — not about a second, separately-timed query.
    const semanticLoad = page.waitForResponse((response) =>
      response.url().includes('/api/search/semantic')
    );
    await TestHelpers.performSearch(page, 'car');
    const search = await (await semanticLoad).json();

    // lib/map.js keeps paging until a page comes back short, so a short first
    // page proves this response is the whole result set (no paging to mirror).
    const hashes = (search.results ?? []).map((entry) => entry.hash);
    expect(hashes.length).toBeLessThan(200);

    const photos = await Promise.all(
      hashes.map(async (hash) => {
        const response = await page.request.get(`/api/photos/${hash}`);
        return response.ok() ? response.json() : null;
      })
    );
    const locationCounts = new Map();
    for (const photo of photos) {
      const latitude = photo?.metadata?.location?.latitude;
      const longitude = photo?.metadata?.location?.longitude;
      if (typeof latitude === 'number' && typeof longitude === 'number') {
        const key = `${latitude},${longitude}`;
        locationCounts.set(key, (locationCounts.get(key) ?? 0) + 1);
      }
    }
    // FR-006: the map announces photos, so parity counts the geo-located CLIP
    // hits themselves, not the locations they sit on.
    const expected = [...locationCounts.values()].reduce((sum, count) => sum + count, 0);
    const densest = [...locationCounts.entries()].sort((a, b) => b[1] - a[1])[0] ?? null;

    // Teeth: the CLIP hits are the car photos, which all carry coordinates,
    // while the unfiltered render shows the unlocated notice for the videos and
    // the receipt. Waiting for the map's own semantic load to finish and then
    // asserting the notice is gone fails if the semantic path ever plots the
    // whole library instead of its result set.
    await waitForMapLoad(page);
    // The unlocated notice renders only while the map is neither loading nor
    // errored, so a failed semantic load would leave it hidden and the check
    // below passing while the previous markers stay on screen.
    await expect(page.locator('[data-testid="map-error"]')).toHaveCount(0);
    await expect(page.locator('[data-testid="map-unlocated-notice"]')).toHaveCount(0);

    // Results can lie outside the fitted viewport (the map deliberately keeps
    // the user's viewport on filter changes), so zoom out to the world first.
    for (let step = 0; step < 4; step += 1) {
      await page.locator('.leaflet-control-zoom-out').click();
      await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
    }

    if (expected === 0) {
      // A query the fixtures score below the CLIP threshold (0.615) still has a
      // defined map result: the empty state, never stale markers.
      await expect(page.locator('[data-testid="map-empty-state"]')).toBeVisible({ timeout: 30000 });
    } else {
      await expect.poll(() => renderedPhotos(page), { timeout: 30000 }).toBe(expected);
      // Scoping: the densest location's marker carries exactly the CLIP set's
      // photos at that coordinate, not the library's photos there.
      if (densest) {
        const [key, count] = densest;
        const marker = await focusLocation(page, key);
        await expect(marker).toBeVisible();
        await expect(marker).toHaveAttribute('data-map-location-count', String(count));
      }
    }
  });

  test('album scoping plots exactly the album members', async ({ page }) => {
    await TestHelpers.goto(page, '/map');

    // One hash per distinct coordinate, so the album spans as many locations as
    // the library has (never more than two: the assertion stays about scoping,
    // not about library size).
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
    test.skip(distinct.length === 0, 'Need geo-located photos to build an album from');

    const createResponse = await page.request.post('/api/albums', {
      data: { name: 'Map E2E Album', initial_hashes: distinct.slice(0, 2) },
    });
    expect(createResponse.ok()).toBe(true);
    const album = await createResponse.json();

    try {
      // The album's own location set, from the same filter the map applies.
      const albumLocations = await page.evaluate(async (albumId) => {
        const response = await fetch(`/api/photos/map?album=${albumId}`);
        const { photos } = await response.json();
        const counts = new Map();
        for (const photo of photos) {
          const latitude = photo.metadata?.location?.latitude;
          const longitude = photo.metadata?.location?.longitude;
          if (typeof latitude !== 'number' || typeof longitude !== 'number') continue;
          const key = `${latitude},${longitude}`;
          counts.set(key, (counts.get(key) ?? 0) + 1);
        }
        return [...counts.entries()];
      }, album.id);
      expect(albumLocations.length).toBeGreaterThan(0);
      const albumPhotos = albumLocations.reduce((sum, [, count]) => sum + count, 0);

      await TestHelpers.goto(page, `/map?album=${album.id}`);
      await expect.poll(() => renderedPhotos(page)).toBe(albumPhotos);

      // Scoping, not just parity: the marker carries only the album's members
      // at that coordinate, not every library photo sitting there.
      const [key, count] = albumLocations[0];
      const marker = await focusLocation(page, key);
      await expect(marker).toBeVisible();
      await expect(marker).toHaveAttribute('data-map-location-count', String(count));
    } finally {
      await page.request.delete(`/api/albums/${album.id}`);
    }
  });

  test('videos with coordinates are plotted and open in the viewer', async ({ page }) => {
    // Videos cannot take EXIF writes, so seed the indexed row directly (the
    // same DB-seeding pattern the collage/housekeeping specs use). The
    // coordinate is cleared again in the `finally`: the whole run shares one
    // server and one database, so a leaked coordinate would break the
    // `?q=type:video` empty-state assertion of the map shell spec.
    TestHelpers.setPhotoLocationInDb('test_video.mp4', 52.52, 13.405);
    try {
      const listing = await page.request.get('/api/photos?q=type:video&limit=100');
      const { photos } = await listing.json();
      const video = photos.find((photo) => photo.filename === 'test_video.mp4');
      expect(video, 'test_video.mp4 must be seeded and indexed').toBeTruthy();

      await TestHelpers.goto(page, '/map');

      const marker = await focusLocation(page, '52.52,13.405');
      await expect(marker).toBeVisible();
      await marker.click();

      const item = page.locator('.leaflet-popup [data-map-popup-photo]').first();
      await expect(item).toHaveAttribute('data-map-popup-photo', video.hash_sha256);
      await item.click();

      await expect(page.locator('#photo-viewer')).toBeVisible();
      await expect(page).toHaveURL(new RegExp(`photo=${video.hash_sha256}`));
      // FR-016: video items play through the existing viewer behavior.
      await expect(page.locator('#viewer-video')).toBeVisible({ timeout: 30000 });
    } finally {
      TestHelpers.clearPhotoLocationInDb('test_video.mp4');
    }
  });

  test('back/forward restores the map with the same filter state', async ({ page }) => {
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();

    // A real filter first: the router listens for popstate, so this is the same
    // mechanism a user's filter change goes through.
    const year = await latestYear(page);
    test.skip(year === null, 'No dated photos in the test library');
    const endpoint = `/api/photos/map?year=${year}`;
    const expected = await expectedPhotos(page, endpoint);
    const densest = await densestLocation(page, endpoint);

    const filteredLoad = mapRequest(page, { year: String(year) });
    await page.evaluate((value) => {
      window.history.pushState({}, '', `/map?year=${value}`);
      window.dispatchEvent(new PopStateEvent('popstate'));
    }, year);
    await expect(page).toHaveURL(new RegExp(`/map\\?year=${year}`));
    await filteredLoad;
    await waitForMapLoad(page);
    await expect.poll(() => renderedPhotos(page)).toBe(expected);

    // Leaving the map keeps the filter in the history entry (the router
    // serializes the whole state), so Back has to restore it.
    await TestHelpers.navigateToView(page, 'videos');
    await expect(page).toHaveURL(/\/videos/);
    await page.goBack();

    await expect(page).toHaveURL(new RegExp(`/map\\?year=${year}`));
    await TestHelpers.verifyActiveView(page, 'map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
    // The map remounts on the way back, so its markers have to be re-plotted
    // from the restored filter — a filterless return renders the unfiltered
    // photo count, which this poll rejects whenever the two differ. The densest
    // location's own photo count carries the regression when both sets happen
    // to hold the same number of photos.
    await expect.poll(() => renderedPhotos(page), { timeout: 15000 }).toBe(expected);
    if (densest) {
      const [key, count] = densest;
      const marker = await focusLocation(page, key);
      await expect(marker).toBeVisible();
      await expect(marker).toHaveAttribute('data-map-location-count', String(count));
    }
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

const AXE_RULES = [
  'color-contrast',
  'target-size',
  'aria-valid-attr-value',
  'aria-prohibited-attr',
];

/** Port 9 (discard) has no listener, so every tile request fails to connect. */
const TILE_FAILURE_URL = 'http://127.0.0.1:9/{z}/{x}/{y}.png';

const MAP_PHOTOS_ENDPOINT = '**/api/photos/map**';
const MAP_FEATURES = '[data-map-cluster], [data-map-location]';

/** Repoints the tile template at an endpoint the test controls (FR-002). */
async function mockTileEndpoint(page, tileUrl) {
  await page.route('**/api/config', async (route) => {
    const response = await route.fetch();
    const body = await response.json();
    await route.fulfill({ response, json: { ...body, tile_url: tileUrl } });
  });
}

/** Waits until the map has drawn at least one feature. */
async function waitForMapFeatures(page) {
  await expect
    .poll(async () => page.locator(MAP_FEATURES).count(), { timeout: 15000 })
    .toBeGreaterThan(0);
}

/**
 * Waits until marker replacement has stopped. Loading the result set fits the
 * map once, and that `moveend` re-render replaces every marker element — Tab
 * pressed between the two would hand focus to an element that is gone a moment
 * later. Two identical samples (the first a full animation length apart) mean
 * the DOM has settled, so keyboard focus sticks.
 */
async function waitForSettledFeatures(page) {
  let previous = null;
  await expect
    .poll(
      async () => {
        const signature = await page.evaluate(
          (selector) =>
            [...document.querySelectorAll(selector)]
              .map(
                (element) =>
                  element.getAttribute('data-map-cluster') ??
                  element.getAttribute('data-map-location')
              )
              .join('|'),
          MAP_FEATURES
        );
        const settled = signature !== '' && signature === previous;
        previous = signature;
        return settled;
      },
      { timeout: 15000, intervals: [500, 500, 1000] }
    )
    .toBe(true);
}

/** Mouse-only setup: expands clusters until individual location markers render. */
async function revealLocationMarker(page) {
  for (let attempt = 0; attempt < 6; attempt += 1) {
    if ((await page.locator('[data-map-location]').count()) > 0) break;
    const cluster = page.locator('[data-map-cluster]').first();
    if ((await cluster.count()) === 0) break;
    await cluster.click();
    await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
  }

  const marker = page.locator('[data-map-location]').first();
  await expect(marker).toBeVisible();
  return marker;
}

/** The zoom levels of the tiles in the DOM — Leaflet's observable zoom. */
function renderedTileZoomLevels(page) {
  return page.evaluate(() =>
    [...document.querySelectorAll('.leaflet-tile-pane img.leaflet-tile')]
      .map((image) => new URL(image.src).pathname.split('/')[1])
      .filter(Boolean)
  );
}

test.describe('Map degradation and accessibility', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
  });

  test('an unreachable tile endpoint still renders markers and attribution', async ({ page }) => {
    // FR-015/SC-005: tiles are decoration — losing them must not lose the data.
    await mockTileEndpoint(page, TILE_FAILURE_URL);
    await TestHelpers.goto(page, '/map');

    await expect(page.locator('[data-testid="map-tiles-notice"]')).toBeVisible();
    const attribution = page.locator('.leaflet-control-attribution');
    await expect(attribution).toBeVisible();
    await expect(attribution).toContainText('OpenStreetMap');
    await waitForMapFeatures(page);
  });

  test('attribution stays inside a 375px viewport', async ({ page }) => {
    // SC-006: the legally required attribution may never be pushed off-screen.
    await TestHelpers.setMobileViewport(page);
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');

    const attribution = page.locator('.leaflet-control-attribution');
    await expect(attribution).toBeVisible();
    const box = await attribution.boundingBox();
    expect(box).not.toBeNull();
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(375);
  });

  test('Tab reaches a map feature and a popup thumbnail hands off to the viewer', async ({
    page,
  }) => {
    // FR-018. The popup cycle itself (Enter, Escape, reopen) is covered by
    // map.e2e.spec.js; this covers the two ends of the keyboard path: Tab from
    // the map container into a feature, and the thumbnail → viewer handoff.
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
    await waitForMapFeatures(page);
    await waitForSettledFeatures(page);

    await page.locator('[data-testid="map-canvas"]').focus();
    await page.keyboard.press('Tab');
    await expect(page.locator('[data-map-cluster]:focus, [data-map-location]:focus')).toHaveCount(
      1
    );

    const marker = await revealLocationMarker(page);
    await marker.focus();
    await page.keyboard.press('Enter');

    const popup = page.locator('.leaflet-popup');
    await expect(popup).toBeVisible();
    const thumbnail = popup.locator('[data-map-popup-photo]').first();
    await expect(thumbnail).toBeFocused();

    const hash = await thumbnail.getAttribute('data-map-popup-photo');
    await page.keyboard.press('Enter');

    await expect(page.locator('#photo-viewer')).toBeVisible();
    await expect(page).toHaveURL(new RegExp(`photo=${hash}`));
  });

  test('shows the loading overlay, reports a failed load and recovers on Retry', async ({
    page,
  }) => {
    await TestHelpers.stubMapTiles(page);

    let phase = 'gated';
    let mapRequests = 0;
    let releaseFirstLoad;
    const firstLoadGate = new Promise((resolve) => {
      releaseFirstLoad = resolve;
    });

    await page.route(MAP_PHOTOS_ENDPOINT, async (route) => {
      mapRequests += 1;
      if (phase === 'gated') {
        await firstLoadGate;
        await route.continue();
        return;
      }
      if (phase === 'failing') {
        await route.fulfill({
          status: 500,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'map listing unavailable' }),
        });
        return;
      }
      await route.continue();
    });

    await TestHelpers.goto(page, '/map');
    // The listing request is intercepted and held open, so the overlay the user
    // sees while the map has no data is observable instead of a race.
    await expect.poll(() => mapRequests).toBeGreaterThan(0);
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
    await expect(page.locator('[data-testid="map-loading"]')).toBeVisible();

    phase = 'serving';
    releaseFirstLoad();
    await expect(page.locator('[data-testid="map-loading"]')).toBeHidden();
    await waitForMapFeatures(page);

    // A load that fails replaces the overlay with the error state and a retry.
    phase = 'failing';
    await page.reload({ waitUntil: 'domcontentloaded' });
    const errorOverlay = page.locator('[data-testid="map-error"]');
    await expect(errorOverlay).toBeVisible();
    const retry = errorOverlay.locator('button');
    await expect(retry).toBeVisible();

    phase = 'serving';
    await page.unroute(MAP_PHOTOS_ENDPOINT);
    await retry.click();

    await expect(errorOverlay).toBeHidden();
    await expect(page.locator('[data-testid="map-loading"]')).toBeHidden();
    await waitForMapFeatures(page);
  });

  test('reduced motion keeps the keyboard path into and out of the popup', async ({ page }) => {
    // FR-018/SC-007. The reduce-motion path opens the popup without the fade or
    // the pan, so the focus handoff has to survive without those frames: a user
    // with the OS preference set still has to land on a thumbnail, and Escape
    // still has to dismiss the popup from there.
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-view"]')).toHaveAttribute(
      'data-reduced-motion',
      'true'
    );
    await waitForMapFeatures(page);
    await waitForSettledFeatures(page);

    const marker = await revealLocationMarker(page);
    await marker.focus();
    await page.keyboard.press('Enter');

    const popup = page.locator('.leaflet-popup');
    await expect(popup).toBeVisible();
    // Landing on the first thumbnail is what keeps the popup's photos one Tab
    // away; without it a keyboard user walks the remaining markers first.
    await expect(popup.locator('[data-map-popup-photo]').first()).toBeFocused();

    // Escape only reaches the map's own handler while focus sits inside the
    // popup pane, and Leaflet's Escape hook is unhooked once the map container
    // has lost focus — so a thumbnail that is not focused leaves the popup
    // undismissable. The marker gets focus back on the way out.
    await page.keyboard.press('Escape');
    await expect(popup).toHaveCount(0);
    await expect(page.locator('[data-map-location]:focus')).toHaveCount(1);
  });

  test('reduced motion turns off animated zooming', async ({ page }) => {
    // FR-018/SC-007.
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-view"]')).toHaveAttribute(
      'data-reduced-motion',
      'true'
    );
    await waitForMapFeatures(page);

    // Leaflet marks an animated zoom by putting `.leaflet-zoom-anim` on the map
    // pane for the length of the transition. Watching for it while the zoom runs
    // is the only way to catch a pane that would come and go inside one tick.
    await page.evaluate(() => {
      const canvas = document.querySelector('[data-testid="map-canvas"]');
      window.reducedMotionProbe = { animated: false };
      window.reducedMotionProbeObserver = new MutationObserver(() => {
        if (canvas.querySelector('.leaflet-zoom-anim')) window.reducedMotionProbe.animated = true;
      });
      window.reducedMotionProbeObserver.observe(canvas, {
        subtree: true,
        childList: true,
        attributes: true,
        attributeFilter: ['class'],
      });
    });

    const zoomLevelsBefore = await renderedTileZoomLevels(page);
    await page.locator('.leaflet-control-zoom-in').click();

    // Prove the zoom really happened before claiming it was not animated.
    await expect
      .poll(
        async () => {
          const levels = await renderedTileZoomLevels(page);
          return levels.some((level) => !zoomLevelsBefore.includes(level));
        },
        { timeout: 10000 }
      )
      .toBe(true);
    await expect(page.locator('.leaflet-zoom-anim')).toHaveCount(0);
    expect(await page.evaluate(() => window.reducedMotionProbe.animated)).toBe(false);

    // Control: with no OS preference the same marker reports `false`, so a
    // hard-coded attribute could not pass this test.
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await page.reload({ waitUntil: 'domcontentloaded' });
    await expect(page.locator('[data-testid="map-view"]')).toHaveAttribute(
      'data-reduced-motion',
      'false'
    );
  });

  test('map view has no axe violations', async ({ page }) => {
    await TestHelpers.stubMapTiles(page);
    await TestHelpers.goto(page, '/map');
    await expect(page.locator('[data-testid="map-canvas"]')).toBeVisible();
    await waitForMapFeatures(page);

    const results = await new AxeBuilder({ page })
      .include('[data-testid="map-view"]')
      .withRules(AXE_RULES)
      .analyze();
    expect(results.violations).toEqual([]);
    // Guard against a pass over nothing: the scoped subtree has to have been
    // analyzed for the empty violation list to mean anything.
    expect(results.passes.flatMap((rule) => rule.nodes).length).toBeGreaterThan(0);
  });
});
```

The reduced-motion test asserts what holds with the implementation: the map is constructed with animations off when the media query matches — the DOM marker the component sets (`data-reduced-motion` on `[data-testid="map-view"]`, added in the implementation below) plus a zoom interaction that completes without a `.leaflet-zoom-anim` pane appearing, with a no-preference reload as the control that keeps the attribute honest.

Cluster activation by keyboard ships in `map.e2e.spec.js` (`cluster markers expand on click and on Enter`), next to the mouse click it mirrors — this suite stays on degradation, the popup handoff, reduced motion, mobile attribution and axe. The three map suites are 12 + 6 + 6 specs.

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
- Modify: `README.md` (Features list — add the Map View bullet: "Plot every geo-located photo on an OpenStreetMap tile layer, clustered by location, and open photos straight from a place")

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
