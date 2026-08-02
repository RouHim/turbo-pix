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

**UI state:** The `route` store (router.svelte.js) and the `$state` stores in state.svelte.js are the single source of truth; components render from them and must not mirror values into write-only fields.

**Video bugs:** Use `[data-photo-id]` selectors, test GET/HEAD requests, verify `mime_type` in DB

**Icons:** Do not use emojis, use feather icons instead. `Icon.svelte` imports them per-icon via `feather-icons/dist/icons/<name>.svg?raw` (feather-icons@4.29.2 has no `exports` field, so subpaths resolve and only the used icons enter the bundle). The raw opening tag carries `class="feather feather-*"` — the runtime regex must strip `width|height|class|aria-hidden` from the opening tag before injecting props (browsers keep the FIRST duplicate attribute). Consequence: the bundle still contains the `feather feather-*` class strings (raw source literals), so `grep -c` against `dist/assets/index.js` counts lines, not occurrences — rendered output has no feather class thanks to the strip.

**`sqlx::Query::execute` needs no `Executor` import:** passing `&mut *tx` (or `&mut **tx` for `&mut Transaction`) as the executor argument resolves via the inherent method's `E: Executor` bound — `use sqlx::Executor;` is NOT required and would be an unused-import warning. This applies to `db.rs` and `image_editor.rs` alike.

**Indexing phases:** When adding a new indexing phase to scheduler.rs, also update: (1) CANONICAL*PHASES in handlers_indexing.rs, (2) the PHASES array + phase UI in frontend/src/components/IndexingOrbit.svelte, (3) indexing_phase*\* keys in both i18n files (en + de). Add a regression test for the new phase.

**sqlite-vec:** Uses the vlasky/sqlite-vec community fork (git dep, not crates.io). Drop-in replacement API — same `sqlite3_vec_init`, `vec_distance_cosine`, `vec0` virtual table. Fork includes native musl fix, so no build-time sed patches needed in the Containerfile.

**SQLite native dep constraint:** `Cargo.toml` cannot bump `libsqlite3-sys` independently while `sqlx = 0.8.x` is in use — `cargo tree -i libsqlite3-sys` resolves both TurboPix and `sqlx-sqlite` through the same `0.30.x` native `sqlite3` link target, and `0.37.x` causes a links conflict.

**Glassmorphism visibility:** `backdrop-filter` on CSS Grid children has no visible blur effect — the element must be `position: fixed` overlaying scrollable content for the blur to actually show. Header and sidebar need fixed positioning with content scrolling behind them.

**InfiniteScroll layout dependency:** PhotoGrid.svelte binds to `.main-content` as the scroll container (`scrollTop`, `scrollHeight`, `clientHeight`). Any layout refactor must keep `.main-content` as a scrollable element with `overflow-y: auto` — removing this breaks infinite scroll silently.

**Router month-without-year guard:** In `router.svelte.js buildUrl()`, `?month=` must be nested inside the `year !== null` check. Writing `?month=3` without `?year=` is semantically invalid and the restore path already ignores it.

**Router anti-loop pattern:** Components called from the `popstate` handler / route `$effect` must accept `updateUrl=false` to skip re-pushing to history. Pattern: `applyFilter(updateUrl=true)` normally, `applyFilter(updateUrl=false)` from popstate handler — prevents infinite push loops.

**E2E port collision:** `npm run test:e2e` global-setup may pass health check against a stale dev server on 18473, then `cargo run` fails with "port in use". global-setup.js kills stale servers itself before building via `pkill -9 -f 'target/(debug|release)/turbo-pix'` — only that deliberately narrow pattern is safe; a broad `-f turbo-pix` match kills the Playwright runner itself (its argv contains the repo path via node_modules).

**i18n key format:** svelte-i18n keys are dot-paths into the nested JSON dictionaries (e.g. `ui.refresh`); en.json and de.json must stay structurally identical — every key in both.

**Startup indexing isolation:** `src/main.rs:start_background_tasks()` must keep `run_startup_rescan()` on a dedicated `std::thread` with its own `tokio::runtime::Runtime`; moving startup indexing back onto the main async runtime starves HTTP requests and makes `/api/indexing/status` look hung.

**Indexing empty-state contract:** `frontend/src/components/PhotoGrid.svelte` (template empty-state branch) must check `indexingState.isIndexing && !currentQuery` (frontend/src/lib/state.svelte.js) before treating `photos.length === 0` as a true empty state; otherwise first-run indexing regresses to a misleading “No Photos Found” screen.

**Video taken-at extraction order:** `src/metadata_extractor.rs:extract_taken_at_from_ffprobe_json()` must check `format.tags.creation_time` → `format.tags.com.apple.quicktime.creationdate` → `streams[].tags.creation_time` → `format.tags.date` / `format.tags.date-{lang}`, then fall back via `apply_file_creation_fallback()` using `created().or_else(modified)`; ffprobe metadata varies by container.

**Filename date parsing:** `metadata_extractor.rs` parses `taken_at` from filenames as a fallback before file creation time. Supported patterns: `%Y%m%d_%H%M%S`, `%Y%m%d%H%M%S`, `%Y-%m-%d-%H-%M-%S` (full stem), plus shard-based `%Y%m%d` and `%Y-%m-%d` with optional adjacent `%H%M%S`/`%H-%M-%S`. Years < 1990 are rejected (consistent with `parse_video_creation_time`). Fallback chain: EXIF/video metadata → filename → file creation time.

**Svelte migration toolchain:** Vite 8.2's default CSS minifier (Lightning CSS) collapses adjacent `backdrop-filter` + `-webkit-backdrop-filter` pairs to the `-webkit-` form, which modern Chromium ignores (computed `backdrop-filter: none`). `vite.config.js` must keep `build.cssMinify: false`.

**Svelte i18n:** svelte-i18n `$t(key, { values: {...} })` returns the raw message (ICU placeholders render literally) when `values` is omitted — always pass `values` for keys containing `{...}` placeholders. `values` must be inside the options object — a third argument to `$t` is silently dropped and the raw message (literal `{…}` braces) is returned.

**Scoped styles beat global media/@container/@layer overrides:** global `@media`/`@container` rules and `@layer utilities` helpers of equal-or-lower specificity are outranked by scoped component rules (scoped selectors carry the `svelte-*` hash), so responsive overrides must live in the component's scoped `<style>` and must not be left in `app.css` where they silently no-op. Examples: PhotoViewer's mobile `.viewer-content` single-column grid and `.viewer-sidebar` bottom-sheet padding/shadow/transition, IndexingOrbit's ring mobile sizing/placement, Header's responsive `.header-content` padding, Sidebar's mobile `.sidebar`/`.sidebar-overlay`, App's `.content-header` ≤480px layout, and explicit scoped `.foo.hidden { display: none }` rules for `@layer utilities` helpers.

**`startViewTransition` defers its callback:** `document.startViewTransition(cb)` runs `cb` on the next frame. If the viewer is closed (Escape) inside that window, `close()` removes classes that were never added, then the deferred callback re-adds `active`/`fade-in` — viewer visibly open with `isOpen=false`, so Escape appears dead. Guard the callback with `if (!isOpen) return;`.

**E2E server log strips query strings:** the warp access log shows `GET /api/photos` even when the request carries `?page=1&limit=50&q=...`. When debugging E2E request issues, trust the browser-side `page.on('request')`/`waitForResponse` URLs, not the server log.

**Playwright click stability vs CSS animations:** an animated `transform: scale(...)` on the viewer (`.photo-viewer.fade-in`) makes every descendant's bounding box move every frame, so `locator.click()` on viewer children (e.g. the mobile sidebar close button) never stabilizes. Keep open/close animations opacity-only.

**Viewer metadata sidebar starts hidden:** `showSidebar` is `false` when the viewer opens — the whole `.viewer-sidebar` (incl. the Edit Metadata button) sits at `x > viewport` with `width: 0; overflow: hidden`. Drive the `[title="View Details"]` (`metadata-btn`) toggle first, then wait for `.viewer-sidebar.show`, before clicking `#metadata-edit-btn` in E2E/manual checks.

**Semantic search latency:** `/api/search/semantic` takes ~3s server-side per query (embedding generation) even on tiny collections. E2E/manual checks that search must allow ≥5-8s after the request fires; don't mistake the loading skeleton for a hang.

**Rotate on housekeeping-candidate photos:** rewriting `photos.hash_sha256` violates `housekeeping_candidates.photo_hash`'s FK (`ON DELETE CASCADE`, no `ON UPDATE`) — SQLite rejects the parent-PK update with `FOREIGN KEY constraint failed`. Fix (implemented in `image_editor::rotate_image`): delete the stale candidate row inside the SAME transaction as the PK rewrite. `Photo::update_with_old_hash` therefore requires a `&mut sqlx::Transaction` (it cannot run against a bare pool). Regression test: `test_rotate_db_update_removes_housekeeping_candidate`.

**Card-level keydown vs inner buttons:** card-level keydown handlers (HousekeepingCard, and any card whose root still owns click/keyboard) must NOT `preventDefault` bubbled Enter/Space from inner action buttons — that kills the button's native click. Guard with `if (e.target !== e.currentTarget) return;` before `preventDefault`. PhotoCard and CollagesView avoid the problem structurally: the card root is plain and a stretched `.photo-card-open-layer` (`position: absolute; inset: 0; z-index: 3; role="button"`) owns the open action with no focusable descendants — action buttons are siblings at z-index 15 (`.photo-card-actions` global rule) and never bubble through the layer.

**Semantic search staleness:** the semantic path needs its own AbortController signal AND a staleness guard: capture `queryAtStart` before `api.semanticSearch(query, limit, offset, options)` and bail out (`queryAtStart !== currentQuery || signal.aborted`) before pushing results — a ~3s embedding response can otherwise pollute a grid the user has already navigated away from.

**Viewer video-path staleness:** every async continuation in `PhotoViewer.svelte` that outlives the current photo must re-check `currentPhoto?.hash_sha256 === photo.hash_sha256` before acting — `displayVideo` (after the HEVC support probe and after the transcode `fetch`), `pollTranscodeStatus` (interval top AND after every `await` — a late fetch/json response can otherwise act for the previous photo), `videoEl.onerror` (no retry/toast for a stale photo), and `displayImage`'s `img.onerror` (mirroring its existing `onload` guard).

**`get(t)(key, 'fallback string')` silently drops the fallback:** svelte-i18n's `get(t)` with a positional fallback string does not apply it (raw key is returned). Always pass `{ default: '…' }` as the options object.

**Global media overrides lose to scoped styles:** IndexingOrbit ring mobile sizing/placement and PhotoViewer sidebar mobile transition must live in the component's scoped `<style>` — global `@media (width <= 768px)` rules are outranked by scoped rules of equal specificity and silently no-op.

**Load-more dedupe must reset on failure:** PhotoGrid's `lastLoadSignature` dedupe (guards concurrent duplicate loads) is set before the request and MUST be cleared in the error path — otherwise every retry of a failed page load (scroll-triggered `loadMore()` and the Load More button) rebuilds the identical signature and is silently swallowed until a route change or reload. Scroll-triggered retries additionally respect a 5s `LOAD_RETRY_COOLDOWN_MS` after a failure (toast/request spam with a dead backend); manual retry paths bypass the cooldown and the success path resets `lastLoadErrorAt` so a recovered backend is retried immediately.

**Test-env lock needs a drop-guarded depth wrapper:** `video_processor::tests::acquire_test_env_lock()` returns a `TestEnvGuard` whose `Drop` decrements the thread-local nesting depth. A plain acquire + manual `release_test_env_lock()` leaks the depth: after the first locked test per thread, every later acquire on that thread silently returns without locking, so the FFPROBE_PATH/FFMPEG_PATH race between test modules quietly returns. Regression test: `test_env_lock_guard_drop_resets_nesting_depth`.

**Ported DOM helpers keep their element-id arguments:** when porting vanilla JS helpers of the form `setField(id, value)` (id = DOM element id) to Svelte, drop the id parameter — a leftover id string is truthy and renders literally ("meta-filesize") instead of the formatted value. Grep ported call sites for two-arg calls to single-arg functions.

**Route-restore `$effect` must not read state a debounced writer mutates:** TimelineSlider's route-sync effect reads `currentFilter`, which `handleSliderInput` writes before a 300ms-debounced `pushState`. Svelte 5 re-runs the effect on every drag tick while `route.year` is still null, so the reset branch wipes the in-progress filter and snaps the thumb back — desktop slider filtering silently dies. Guard with a `dragInProgress` flag checked at the effect top (set in the input handler, cleared in the debounced callback), and read `route.year`/`route.month` BEFORE the guard: an early return that reads nothing replaces the effect's dependency set with `{}`, permanently unsubscribing it (Back/Forward then never restores the slider).

**`svelte-ignore` a11y comments trip eslint's `svelte/no-unused-svelte-ignore`:** the Vite build warns `a11y_no_static_element_interactions` for decorative hover containers, but eslint's own parser does not raise that warning, so the ignore comment is "unused" to lint (13 errors). Use `role="presentation"` on the container instead — it silences both, and descendants (e.g. the range input) stay accessible.

**Svelte actions bind before `$effect`-created handlers on the same element:** `use:gestures` registers its touch listeners at element creation, SwipeableViewer's handlers in a mount `$effect` — so on the pan-initiating touchmove the manager runs first with `activeGesture` still null and cannot `preventDefault`. The viewer-side handler must call `event.preventDefault()` itself when it calls `startPan()` (old code avoided this because the manager was on the viewer root and ran in the bubble phase).

**Canvas colors should read CSS vars, with alpha injected into oklch:** `getComputedStyle(document.documentElement).getPropertyValue('--primary-color')` returns `oklch(0.55 0.08 250)` — build alpha variants by inserting ` / <alpha>` before the closing paren (`oklch(0.55 0.08 250 / 0.3)` is a valid canvas `fillStyle`).

**Dead-CSS deletion needs property-level verification:** "duplicate" global media rules are only dead where the scoped rule sets the SAME properties at higher specificity. The global `@media (width <= 768px)` `.viewer-sidebar` block (position/transform/height/z-index) is LIVE — the scoped mobile block only sets transition/padding/box-shadow — and the mobile-sidebar E2E asserts its z-index:15. Conversely `.viewer-content`/`.photo-grid` rules at ≤1200/≤1024 are fully shadowed and removable.

**Shared interval fields must be cleared only by their owner:** `PhotoViewer.pollTranscodeStatus` shares the module-level `transcodePollTimer` across polls — capture the interval id in a local `const`, `clearInterval` that, and only null the shared field (and hide the shared toast) when it still points at your interval. A stale poll clearing the shared field kills a newer photo's poll: its promise never settles, the spinner hangs, and the video never receives its completed transcode URL.

**Backend `q` is tokenized:** `db::search_photos` previously matched `type:`/`location:`/`is_favorite:` only when the whole query started with them; combined queries (`sunset is_favorite:true`) silently matched nothing. The parser now splits on whitespace and ANDs per-token; `location:` absorbs following words until the next prefix token.

**Semantic mode must reset off the `all` view:** PhotoGrid's reset block exits `semanticSearchMode` when `route.view !== 'all'`; semantic results are unfilterable, so filtered views (favorites/videos) always use the regular path with the view filter merged into the query (`cat` + Favorites → `q=cat is_favorite:true`). Returning to `all` with a non-prefix query must re-enable the mode (`isPrefixQuery` in utils.js): SearchBar routes every non-prefix query semantically, so the same URL (`/?q=cat`) would otherwise degrade to a text search after a view round-trip (the route-sync effect no-ops when `route.query === currentQuery`).

**build.rs stale-dist guard:** `build.rs` walks `frontend/{index.html,src,public}` (emitting rerun-if-changed per entry) and panics when the newest frontend source is newer than `dist/index.html` — `cargo build` fails loudly instead of silently embedding a stale bundle. CI jobs must run `npm run build` before any cargo step that embeds dist/. Strict `>` comparison: equal mtimes count as fresh.

**Icon.svelte renders nothing for unregistered names:** the ICONS map only contains explicitly imported feather SVGs; any `<Icon name>` not in the map silently renders an empty string (no build error, no console warning) — icon-only buttons become invisible. Every icon name passed to `<Icon>` (including dynamic `name={...}` bindings — toasts, suggestions, phase icons, theme toggle, transcode spinner) MUST be registered in `frontend/src/lib/Icon.svelte`; grep `name="` and `name={` usages against the map before adding a new one. Also, the `class="feather"` attribute is stripped from the raw SVG, so `:global(.feather)` selectors never match — size icon elements via `:global(svg)`.

**Reduced-motion E2E contract is the inline `style.animation`:** the old `indexingOrbit.js` set `group.style.animation = prefersReducedMotion() ? 'none' : 'orbit-segment 2s ...'` inline, and `indexing-orbit.e2e.spec.js` polls `element.style.animation` — a CSS class toggle alone (or a `@media (prefers-reduced-motion)` rule) leaves the inline style empty and fails the test. Keep animation properties in the inline style binding.

**Gesture manager touch scope:** `GestureManager` listens on a sub-element (`.viewer-main`) but `e.touches` is document-global — a simultaneous touch on chrome outside the element is tracked and its `touchend` never arrives, leaving a stale touch that breaks double-tap/pinch until unmount. Filter tracked touches with `this.element.contains(touch.target)`.

**`$state` fields must use `let`, not `const`:** Svelte 5's `const x = $state(...)` trips eslint's `no-const-assign` (the assignment is emitted into the compiled output). PhotoCard's `let imageLoaded = $state(false)` is the pattern; ported code often copies the `const` form from older vanilla JS — use `let` for every `$state` field.

**EXIF read/write lives in `src/exif_helpers.rs`:** `read_exif_from_path` / `read_exif` / `build_exif_buffer` / `write_exif_to_image` are the only sanctioned way to touch EXIF — the old per-file copies had already drifted (`"jpg" | "jpeg"` vs `"jpeg"` format strings). New EXIF consumers must call these helpers, never hand-roll `exif::Reader::new()`; `write_exif_to_image` accepts `jpg`/`jpeg`/`png`.

**Frontend/backend extension lists are pinned by a parity test:** `mimetype_detector.rs::frontend_extension_lists_match_backend` parses `frontend/src/lib/constants.js` and asserts `VIDEO_EXTENSIONS`/`RAW_EXTENSIONS` match the backend detector (and `raw_processor::is_raw_file`) exactly, in both directions. Adding a media format means updating BOTH `constants.js` and the backend lists — the test fails otherwise.

**`kamadak-exif` is a cargo-machete false positive:** the crate's lib name is `exif` (used in `metadata_extractor`, `metadata_writer`, `image_editor`, `handlers_photo`, and now `exif_helpers`), so machete reports it as unused. Don't remove it; add `ignored = ["kamadak-exif"]` to `[package.metadata.cargo-machete]` if the report annoys.

**warp `test` feature is enabled for route smoke tests:** `handlers_static.rs` uses `warp::test::request()` (async — tests must be `#[tokio::test]` and `.reply(&routes).await`). Cargo.toml has `features = ["server", "multipart", "test"]`; don't remove the feature or the smoke test stops compiling.

**File-type helpers are in `utils.js`:** `isVideoFile`/`isRawFile`/`isCollagePhoto` live in `frontend/src/lib/utils.js`; components must import them, not re-define them (PhotoViewer and ViewerMetadata both had copies). `SwipeableViewer` still calls `this.viewer.isVideoFile` through the viewer's exposed API — keep that surface intact when refactoring.

**PhotoGrid loading is factored; keep the contracts:** `loadPhotos` delegates to `applyResetState` / `loadSemanticPage` / `loadRegularPage` / `appendPhotos`. `loadSemanticPage` returns `null` for stale responses and the caller no-ops (skip append AND the `lastLoadErrorAt` reset). The dedupe signature, semantic staleness guard, and retry-cooldown reset semantics from the earlier learnings are spread across these helpers — edits must preserve them.

**Rotation preserves EXIF via explicit carry:** `image::save` re-encodes from pixels and drops EXIF, so `rotate_image` reads EXIF from the ORIGINAL file before the pixel transform and writes it into the temp file with Orientation forced to 1 (`carry_exif_with_reset_orientation`). `Value::Unknown` fields are skipped (the experimental EXIF writer cannot serialize them; in kamadak-exif 0.6.1 the variant has three fields — pattern with `Value::Unknown(..)`). Regression test: `test_rotate_preserves_exif_in_file`.

**Hash re-key preserves `is_favorite`:** `Photo::create_or_update_with_transaction` captures `is_favorite` from the row being replaced (same `file_path`, different hash) and re-applies it to the fresh row. In-app rotation keys the row by the content hash while the next rescan re-derives the path hash, so without the carry every rotated photo would silently lose its favorite flag. Regression test: `test_create_or_update_preserves_favorite_across_hash_rekey`.

**Transcode cache never serves partial files:** `transcode_hevc_to_h264_with_timeout_and_path` writes to a sibling `*.mp4.tmp` and atomically renames on success; failure AND timeout remove the temp. `get_video_file` consults `get_transcode_status` — a `Failed`/`Timeout` state removes the stale file and serves the ORIGINAL video with `X-Transcode-Warning` (header only sent when a warning applies). The in-memory status store is capped at 128 entries (settled entries evicted first).

**Video file-serving contract:** 0-byte file → 416 with `Content-Range: bytes */0` (no `file_size - 1` math); unsatisfiable ranges and `start > end` → 416 (never clamped/404); suffix ranges `bytes=-N` supported; multi-range ignored → full 200; no-range GET streams via `tokio::fs::File` + `tokio_util::io::ReaderStream` (`tokio-util` with the `io` feature is a direct dep), never `std::fs::read`. HEAD mirrors exist on file routes (photo file, video, static assets) replying headers + empty body — HEAD must never trigger transcoding.

**Collage timestamps parse two formats:** `Collage::from_row` accepts RFC3339 AND SQLite `YYYY-MM-DD HH:MM:SS` (produced by `datetime('now')` / `CURRENT_TIMESTAMP`). Previously `accepted_at`/`rejected_at` were ALWAYS None in API responses and `created_at` serialized as fetch time. `accept()` also updates `thumbnail_path` (the thumbnail moves next to the accepted file).

**Search quick filters are prefix-limited:** SearchBar suggestions only offer `type:`/`location:`/`is_favorite:` — the backend tokenizer supports exactly those prefixes. `camera:`/`date:`/`has:` suggestions were REMOVED (they silently routed to semantic search and could never filter). A bare `location:` token (empty city) is skipped in the tokenizer instead of emitting `LIKE '%%'`.

**Model cache path `../data` is load-bearing:** `semantic_search.rs` resolves the CLIP cache as `data_path.join("../data/models")` — with the default `./data` it normalizes to `./data/models`, but the E2E suite runs the server with `TURBO_PIX_DATA_PATH=test-e2e-data`, so `../data/models` lands in the repo-root `./data/models` (shared cache, pre-downloaded by `cargo run -- --download-models`). "Simplifying" it to `data_path.join("models")` makes the E2E server hang at startup trying to download the model into the sandboxed dir. Do not "fix" the `..`.
