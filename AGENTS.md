# Agent Guidelines for TurboPix

## Project Context

Breaking changes are allowed, this application is not in production yet!
Breaking changes are allowed, this application is not in production yet!
Breaking changes are allowed, this application is not in production yet!
This means, no legacy support, no migration scripts, no backward compatibility.
Development/personal project - breaking changes acceptable, database and cache can be recreated!

## Development Commands

**Backend:** `cargo run` | `cargo test` | `cargo clippy` | `cargo fmt`  
**Frontend:** `npm run build` (required before cargo build — embeds dist/) | `npm run lint` | `npm run format`

## Code Style

**Backend / Rust:**

- Iterator chains over loops: `.iter().filter_map().next()`
- Arrays over vecs: `[A, B]` vs `vec![A, B]`
- Error handling: `Result<T, E>` with `?`
- Imports: std, external crates, local (blank lines between)
- Zero warnings policy

**Frontend / Svelte 5 (runes) + Vite:**

- `const` over `let` (no reassignment)
- Arrow functions: `() => {}` over `function() {}`
- Template literals: `` `string ${var}` `` over `'string ' + var`
- When adding visible text to the frontend, add them to the `i18n` translation system.
- When changing frontend files: run `npm run build` first, then `cargo build --bin turbo-pix` (build.rs embeds dist/ and panics if it is missing)

**General:**

- KISS: Keep It Simple, Stupid (DRY, YAGNI, etc.)
- SOLID principles (single responsibility, open/closed, etc.)
- Zero linting issues - investigate each issue in detail, don't just silence it

# Commit Gatekeeping

- Lint and format before commiting
- Tests must pass
- Meaningful commit messages

# Development pattern

- TDD: Test Driven Development, write tests first, then implement the feature
- BDD: Behavior Driven Development, focus on the behavior of the application, use GIVEN, WHEN, THEN style
- E2E: End to End testing, test the application as a whole, use Playwright or Puppeteer
- when changing frontend files: run `npm run build` first, then `cargo build --bin turbo-pix` (build.rs embeds dist/ and panics if it is missing)
- **Avoid:** Hardcoded paths and fallback logic mask bugs
- When troubleshooting bugs, try to reproduce the bug first writing a test
- After finishing a Task (feature, bug fix, etc) extract relevant learnings from the session/task (if there are ones),
  and merge them with the Learnings section in the Agents.md file. Also verify all entry in the learnings section are still valid.

## Testing

- Test images and videos are located in `test-data/`

**E2E:**

TurboPix uses Playwright for end-to-end testing with real backend integration.

**Quick Start:**

```bash
npm run test:e2e          # Run all tests
npm run test:e2e:ui       # Interactive UI mode
npm run test:e2e:headed   # See browser
npm run test:e2e:debug    # Debug mode
npm run test:e2e:report   # View test report
```

**Test Structure:**

- `tests/e2e/setup/` - Global setup, teardown, test helpers
- `tests/e2e/specs/` - Test files organized by feature
- Sequential execution (workers: 1) to avoid DB conflicts
- Real backend: Auto-builds binary, starts server, waits for indexing

**Test Helpers Available:**

- `TestHelpers.navigateToView(page, 'favorites')`
- `TestHelpers.verifyActiveView(page, 'videos')`
- `TestHelpers.getPhotoCards(page)`
- `TestHelpers.waitForPhotosToLoad(page)`
- `TestHelpers.openViewer(page, hash)` / `closeViewer(page)`
- `TestHelpers.setMobileViewport(page)` / `setDesktopViewport(page)`
- And 20+ more utilities

**Writing Tests:**

1. Use `data-*` attribute selectors for stability
2. Use TestHelpers for common operations
3. Wait for elements with Playwright's auto-waiting (avoid hard timeouts)
4. Test should be order-independent
5. Use `test.skip()` when test data is unavailable

**Manual E2E Testing:**

- Start: `nohup cargo run &` + wait for `curl --retry 5 --retry-delay 2 http://localhost:18473/health`
- Test at `http://localhost:18473`
- Kill process after testing

## Learnings

**Capped at 10 entries (decision 2026-08-11).** These are the top-10 most relevant project traps. When a session produces new learnings, fold them into an existing entry or replace the least relevant one — never append standalone entries.

1. **i18n parity & integrity guard:** keys are dot-paths into nested JSON dictionaries; `en.json` and `de.json` MUST stay structurally identical — every new key lands in BOTH. `tests/i18n-integrity.test.js` (`npm run test:i18n`, wired into the CI lint-format job) scans every `$t`/`get(t)` literal, template, and map key (Sidebar/SortControls `key:` fields, App's `titleKeys`) in `frontend/src` and fails listing ALL missing keys plus parity drift; template `${…}` placeholders must be one of its `enums` map entries. `$t` options: pass `values` INSIDE the options object for keys containing `{...}` (a third argument is silently dropped); the positional fallback `get(t)(key, 'fallback')` does not work — use `{ default: '…' }`.

2. **State & routing:** the `route` store (`router.svelte.js`) and `$state` stores (`state.svelte.js`) are the single source of truth — components render from them, never mirror values into write-only fields; `$state` fields MUST use `let` (const trips eslint's `no-const-assign`). The timeline filter lives in the URL as `year`/`month` (start bound) plus `to_year`/`to_month` (end bound; absent = a single period, absent `month` = the whole year). Route-restore trap: TimelineSlider's sync effect must read `route.year`/`route.month` BEFORE the `dragInProgress` guard — reading a debounced-writer field first wipes the in-progress filter on every drag tick, and an early return that reads nothing replaces the effect's dependency set with `{}`, permanently unsubscribing it (Back/Forward never restores). A restored selection that no longer overlaps the library is rewritten to the canonical clamped filter by `TimelineSlider`'s replaceState `$effect`, which reads `data`/`filter`/`filterFromSelection` BEFORE its loaded guard (an in-flight fetch must never be read as "the data is gone"); live scrubbing `replaceState`s at most every 100 ms while the overlay follows the pointer immediately. In `TimelineSelector`, view `$effect`s MUST assign `view` only when `scale`/`origin` actually changed (`clampView` returns a fresh object, so an unconditional assign self-retriggers), MUST read `width`/`model` inside `untrack` for the `resetNonce` reset (otherwise every resize throws the zoom away), and MUST skip while the lane width is 0 (the mobile `display: none` breakpoint). FR-007 — a period with no photos is never selectable: activating a zero-count column returns early and a selection that is exactly one zero-photo month is refused on every commit path (a wider range merely containing empty months stays legal); handles and the overlay paint clamped into the lane because a bare `?year=` deep link may start left of the first populated month.    The selector's granularity IS the rendered column unit (`chooseUnit(view.scale)`,
   `frontend/src/lib/timelineLayout.js`): the Decade/Year/Month pills are view-only and press whatever the lane
   renders (`aria-pressed={unit === level.unit}`, an activation while already at that level is a no-op), a click on a
   decade/year/month column commits exactly that period and drills one level in by clamping the period's span into the
   next unit's band (`zoomToUnitRange`, band edges from `unitScaleMin`/`unitScaleMax` with a 1e-3 margin so a clamped
   scale can never land on `chooseUnit`'s `>=` threshold; below the band the unit is kept and the period shows
   partially — never a coarser fallback), and the level pills frame the active filter when it fits the level's window
   and otherwise a window around the current view centre (`frameUnit`) without ever writing a filter. Both framings set
   `reframeSuppressed` before writing `view`, or the FR-010 selection-following effect reverts them. A grid-aligned
   decade filter (`?year=1960&to_year=1969`) is a *period*: `selectionFromFilter` keeps its own January–December-ten-year
   bounds (`isDecadeFilter`), so the header reads `1960s` instead of the clamped `March 1962 – December 1969` and the
   round trip stays the identity; a non-decade or shifted ten-year range keeps clamping as before. A commit whose
   canonical filter already equals the route filter is skipped, so re-activation adds no history entry. Decade labels go
   through `ui.timeline_decade_label` (`{start}s` / `{start}er`) — never hardcode the `s` — and the year dropdown gains
   an option for a filtered year the library has no bucket in, or it silently reads "All Years" over a filtered grid.
   Node-testable modules MUST NOT import `utils.js` — it imports `../i18n/en.json` without the JSON import attribute, so `node --test` dies at link time with `TypeError: Module "…/frontend/src/i18n/en.json" needs an import attribute of "type: json"`; keep shared pure helpers in their own module (`lib/query.js` `isPrefixQuery`, `lib/map.js`) and inject the api client instead of importing it. Geo-coordinate validation lives solely in `frontend/src/lib/map.js`'s `getPhotoCoordinates` on the frontend — non-numeric, single-coordinate, and out-of-range values are dropped there before clustering (the backend is the other gate: `src/metadata_writer.rs` rejects out-of-range and unpaired coordinates, which `PATCH /api/photos/{hash}/metadata` maps to a 400). GPS is stored in the `photos.metadata` JSON blob at `$.location.latitude`/`$.location.longitude`, with the resolved place name at `$.location.city` (read via `json_extract`; there are no dedicated columns). `PATCH /api/photos/{hash}/metadata` MERGES into the existing location object, so an already-resolved `city` survives a coordinate edit — a test that needs a nameless location must pick a photo that has none.

3. **Scoped styles beat global overrides:** global `@media`/`@container` rules and `@layer utilities` helpers of equal-or-lower specificity are outranked by scoped `svelte-*` selectors — responsive overrides MUST live in the component's scoped `<style>`, never in `app.css` where they silently no-op. Dead-CSS deletion is property-level: a global rule is only dead where the scoped rule sets the SAME properties. Keep `build.cssMinify: false` in `vite.config.js` (Lightning CSS collapses `backdrop-filter` pairs to the `-webkit-` form Chromium ignores). The timeline selector's geometry (ruler/lane heights, the 24px handle hit zone) likewise lives in `TimelineSelector.svelte`'s scoped `<style>`; its label collision avoidance measures `getComputedStyle(probeEl).font`, so the hidden `.timeline-label-probe` span must stay in the DOM. A native `<video>`'s control strip is painted INSIDE the element's own bottom edge, so a height-bound video (any portrait clip, or a 16:9 source in a 16:9 viewport) drops the scrubber onto the floating viewer action bar: the scoped `.viewer-controls` rule in `frontend/src/components/ViewerControls.svelte` and the `.viewer-main.video-mode` reserve in the sibling `PhotoViewer.svelte` both read `--viewer-controls-offset`/`--viewer-controls-height`, with the mobile offset override in an `app.css` `:root` media query so the media box sees the same value. Like `<video>` default boxing, video E2E geometry assertions MUST first wait for `videoWidth > 0` — before metadata resolves the element lays out at Chromium's 300×150 default and overlap assertions pass vacuously. Stylelint blindspot: `lint:css` only globs `frontend/src/**/*.css`, so invalid CSS pasted into a Svelte `<style>` block (e.g. stray diff `+` markers) passes lint and breaks only at `vite build` — always run `npm run build` after style edits.

4. **Viewer async staleness:** every async continuation in `PhotoViewer.svelte` must re-check `currentPhoto?.hash_sha256 === photo.hash_sha256` before acting (`displayVideo` after the HEVC probe AND after the transcode `fetch`, `pollTranscodeStatus` at interval top AND after every `await`, `onerror` handlers); continuations touching the URL/navigating must also bail when `!isOpen`. `pollTranscodeStatus` shares a module-level timer — capture the id in a local `const`, only clear/null the shared field when it still points at YOUR interval. Guard `startViewTransition` callbacks with `if (!isOpen) return;` (the callback runs on the next frame).

5. **Silent-failure registries (icons, tiles):** feather only, no emojis. An unregistered `<Icon name>` (including dynamic bindings) renders an EMPTY string silently — no build error, no console warning; every icon MUST be registered in `frontend/src/components/Icon.svelte`; grep `name="`/`name={` usages against the registry. The runtime regex strips the raw `class="feather …"`, so `:global(.feather)` never matches — size icons via `:global(svg)`. Same class of trap in the Map view: the Leaflet tile URL comes from `GET /api/config` (`TURBO_PIX_TILE_URL` → `appState.tileUrl`) and must never be hardcoded, and attribution is registered on `map.attributionControl` — NOT on the tile layer — so it stays visible when every tile request fails.

6. **Rotation & photo identity:** `image::save` re-encodes from pixels and drops EXIF — read EXIF from the ORIGINAL before the transform and write it into the temp file with Orientation forced to 1 (skip `Value::Unknown` and `In::THUMBNAIL` fields), via `src/exif_helpers.rs` (never hand-roll `exif::Reader::new()`); EXIF writes are atomic sibling-temp + rename. Rewriting `photos.hash_sha256` violates the `housekeeping_candidates` FK (no `ON UPDATE`) — delete the stale candidate row in the SAME transaction; `Photo::update_with_old_hash` takes a `&mut sqlx::Transaction` and checks `SELECT changes()`. `hash_sha256` is a PATH-string hash on purpose (favorites stay stable across in-place edits) — thumbnail/transcode/collage caches fold a size+mtime CONTENT VERSION into their keys, and `clear_for_hash` removes every `{hash}_*` file. Album membership (`album_members`) references `photos(hash_sha256)` AND `albums(id)` with `ON DELETE CASCADE` both ways — library removals and album deletions clean up membership for free, but only because sqlx enables FK enforcement; cascade claims need a dedicated test, never a pragma.

7. **Native-first video serving & codec-agnostic transcode:** playback decisions live SERVER-SIDE — `get_video_file` runs `decide()` over the capability record (codec/container/bit-depth/`moov_at_start`) against the client's declared codecs (`X-TurboPix-Codecs` header or `client` query param; missing → conservative h264-8). Appending `?decision` (bare or `=true`) returns JSON `{action: direct|remux|transcode|empty, url, reason}` — always 200, never 202; the 202/`poll_url` handshake belongs to the transcode spawn, not the decision probe. Transcode is codec-AGNOSTIC (`transcode_codec_to_h264`, not hevc-only) and gated by a worker pool: `transcode_semaphore()` is `OnceLock` and locks in the FIRST `transcode_max_pool()` it saw — `TURBO_PIX_MAX_TRANSCODES` (0 = disabled, checked before touching the semaphore; absent → `min(max(nproc/2,1),4)`), timeout via `TURBO_PIX_TRANSCODE_TIMEOUT_SECS` (default 300); no test reset hook, so tests assert `transcode_max_pool()` directly. `claim_transcode` still consults AND inserts `InProgress` under the status-store lock (the semaphore only serializes, never prevents duplicate spawns); `Failed`/`Timeout` remove leftover `*.mp4.tmp` and serve the original with `X-Transcode-Warning` (`TRANSCODE_RETRY_COOLDOWN` = 15 min). `/video/status` reports `percent` + `deadline_ms`; the Svelte poller stops on the server deadline (+30s grace), never a hard-coded client cap. Status store capped at 128 (`InProgress` never evicted).

8. **Migrations are content-checksummed:** `sqlx::migrate!` records a SHA-384 of each migration file and verifies it on every startup — NEVER edit an applied migration (bricks DBs with `VersionMismatch`); schema cleanups on shipped migrations belong in a NEW migration. Repairing an applied DB: `sqlite3 <db> "UPDATE _sqlx_migrations SET checksum = X'<sha384 of CURRENT file>' WHERE version = ..."`.

9. **SQLite query & bulk-write traps:** `delete_orphaned_photos` inserts scanned paths into a per-connection `TEMP TABLE scanned_paths` (multi-row chunks of 500) then runs `NOT IN (SELECT path FROM scanned_paths)` — one placeholder per file exceeds `SQLITE_MAX_VARIABLE_NUMBER` (32766) and silently killed nightly cleanup. All statements on ONE held `pool.acquire()` connection; `DROP TABLE IF EXISTS` before CREATE. Partial scans (`scan_complete = false` when a root is missing / directory unreadable) skip orphan cleanup entirely — deleting rows for temporarily unreachable files would permanently lose favorites/manual metadata. Membership `IN (...)` lists chunk at the same 500; `INSERT OR IGNORE INTO album_members … SELECT … FROM photos WHERE hash IN (…)` makes add/create-from-selection robust to photos that left the library between selection and submit (unknown hashes silently skip). `build_search_where` (`src/db.rs:209`) is the ONE token grammar behind `/api/photos` (`Photo::search_photos`) and the unpaginated `Photo::list_all_filtered` serving `GET /api/photos/map` — never re-implement it; the map listing is the only listing without `LIMIT`/`OFFSET` (the grid endpoint caps at 100, so a filter change must not silently truncate the map).

10. **E2E & test harness — port races, fixtures, reaping:** global-setup binds `TURBO_PIX_E2E_PORT` (default 18473, also Playwright's `baseURL`) so sibling worktrees do not share a port; its stale-server reap is worktree-scoped — `pgrep -f 'target/(debug|release)/turbo-pix'` (ONLY that deliberately narrow pattern: a broad `-f turbo-pix` match kills the Playwright runner itself) then keep only the PIDs whose `/proc/<pid>/exe` starts with THIS checkout's `target/`, because the spawned binary's argv is relative and a path-anchored pattern can never match it. A machine-wide kill still reaps a sibling's server mid-run; consecutive `npx playwright test` invocations in one checkout race the same way, so let the previous run's teardown settle before re-running and treat a failed first run as infra — re-run once before suspecting app regressions. The fixture library seeds `legacy_01..06.jpg` — six copies of `test-data/test_image_1.jpg`, so only the filenames differ and the source EXIF/mtime is identical — with `taken_at` (1962-03-15 … 2019-07-01, one photo per decade, 1990s left empty) written by `updateTestPhotoDates()`, so decade/year granularity and gaps are reachable and the source image's dates never matter.
   The timeline specs also lean on the fixture's calendar: decade starts are `1960*12 = 23520`, `1980*12 = 23760`,
   `1990*12 = 23880` (the only empty decade) and `2012*12 = 24144`, and a decade activation writes
   `?year=1960&to_year=1969`, so a drill into the 1960s renders eleven year columns (1962…1972: the origin pins to the
   model's March 1962 start inside a 120-month span) — assert the first/last `data-period-start`, never a hard-coded
   lane width.
   `npm run test:unit` is `node --test tests/*.test.js`, so every root-level `tests/*.test.js` runs in the CI `lint-format` job. Cold model cache: the server downloads CLIP weights synchronously before binding, so with an empty `./data/models` the 30s health check always times out — pre-seed with `./target/debug/turbo-pix --download-models` first. Map specs MUST stay offline: `TestHelpers.stubMapTiles(page)` fulfills `/{z}/{x}/{y}.png` through `page.route`, and per-test coordinates come from `TestHelpers.setPhotoCoordinates` (metadata PATCH) or `setPhotoLocationInDb` (direct `sqlite3` UPDATE with `PRAGMA busy_timeout=5000` — videos reject EXIF writes, the CLI waits 0 ms by default); because all specs share one server and one DB, seeded coordinates are a shared resource with per-helper lifetimes: `setPhotoLocationInDb` MUST be paired with `clearPhotoLocationInDb` in a `finally`, while the metadata-PATCH path seeded in `map.e2e.spec.js`'s `beforeAll` deliberately leaves its residue in place — it anchors on the densest location and tolerates earlier runs' residue.
   Teardown can miss the server it started: it kills the PID file's PID, and when that PID is a wrapper the real
   binary survives, keeps the E2E port bound and makes the NEXT run's health and indexing checks pass against a process
   whose database was just deleted by `setupTestDataDirectory()`, while `waitForPhotosTable` polls the fresh, empty
   path — the symptom is a global-setup error (`Indexing did not reach is_complete`, `Photos table not ready after
   retries`), never a test failure. Before re-running, confirm nothing of THIS checkout is listening on the E2E port
   (`pgrep -f 'target/(debug|release)/turbo-pix'` plus `/proc/<pid>/exe`).
