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

**i18n integrity guard:** `tests/i18n-integrity.test.js` (run via `npm run test:i18n`, wired into the CI lint-format job) scans every `$t('…')`/`get(t)('…')` literal, `$t(\`…\`)`/`get(t)(\`…\`)` template, and map key (Sidebar/SortControls `key:` fields, App's `titleKeys` object) in `frontend/src` against BOTH dictionaries and fails listing ALL missing keys plus any en/de parity drift. Template `${…}` placeholders must be one of `phase.id` (IndexingOrbit PHASES), `monthKey`, `weekdayKey` (constants.js) — a new template site needs its enum added to the test's `enums` map or the guard fails. New i18n keys MUST land in both en.json and de.json. App's `titleFallbacks` are plain strings, not keys — the titleKeys extraction is scoped to the object for that reason.

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

**Icon.svelte renders nothing for unregistered names:** the ICONS map only contains explicitly imported feather SVGs; any `<Icon name>` not in the map silently renders an empty string (no build error, no console warning) — icon-only buttons become invisible. Every icon name passed to `<Icon>` (including dynamic `name={...}` bindings — toasts, suggestions, phase icons, theme toggle, transcode spinner) MUST be registered in `frontend/src/components/Icon.svelte`; grep `name="` and `name={` usages against the map before adding a new one. Also, the `class="feather"` attribute is stripped from the raw SVG, so `:global(.feather)` selectors never match — size icon elements via `:global(svg)`.

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

**Hash re-key preserves `is_favorite`:** `Photo::create_or_update_with_transaction` captures `is_favorite` from the row being replaced (same `file_path`, different hash) and re-applies it to the fresh row. In-app rotation keys the row by the content hash while the next rescan re-derives the path hash, so without the carry every rotated photo would silently lose its favorite flag. Regression test: `test_create_or_update_preserves_favorite_across_hash_rekey`. NOTE: `hash_sha256` is a PATH-string hash, deliberately — it keeps photo identity (and favorites) stable across in-place edits (Lightroom re-export, sync overwrite). Because the hash never changes on byte edits, the thumbnail/transcode/collage caches fold a size+mtime CONTENT VERSION into their keys (`CacheKey.content_version`, `get_transcoded_path_versioned`, `build_collage_signature`); the rescan updates `file_size`/`file_modified` when it notices the change, which rotates the cache key and regenerates — stale cache entries are otherwise served forever. `clear_for_hash` removes every `{hash}_*` file (versioned and unversioned).

**Transcode cache never serves partial files:** `transcode_hevc_to_h264_with_timeout_and_path` writes to a sibling `*.mp4.tmp` and atomically renames on success; failure AND timeout remove the temp. `get_video_file` consults `get_transcode_status` — a `Failed`/`Timeout` state removes the stale file and serves the ORIGINAL video with `X-Transcode-Warning` (header only sent when a warning applies). The in-memory status store is capped at 128 entries (settled entries evicted first).

**Video file-serving contract:** 0-byte file → 416 with `Content-Range: bytes */0` (no `file_size - 1` math); unsatisfiable ranges and `start > end` → 416 (never clamped/404); suffix ranges `bytes=-N` supported; multi-range ignored → full 200; no-range GET streams via `tokio::fs::File` + `tokio_util::io::ReaderStream` (`tokio-util` with the `io` feature is a direct dep), never `std::fs::read`. HEAD mirrors exist on file routes (photo file, video, static assets) replying headers + empty body — HEAD must never trigger transcoding.

**Collage timestamps parse two formats:** `Collage::from_row` accepts RFC3339 AND SQLite `YYYY-MM-DD HH:MM:SS` (produced by `datetime('now')` / `CURRENT_TIMESTAMP`). Previously `accepted_at`/`rejected_at` were ALWAYS None in API responses and `created_at` serialized as fetch time. `accept()` also updates `thumbnail_path` (the thumbnail moves next to the accepted file).

**Search quick filters are prefix-limited:** SearchBar suggestions only offer `type:`/`location:`/`is_favorite:` — the backend tokenizer supports exactly those prefixes. `camera:`/`date:`/`has:` suggestions were REMOVED (they silently routed to semantic search and could never filter). A bare `location:` token (empty city) is skipped in the tokenizer instead of emitting `LIKE '%%'`.

**Model cache path `../data` is load-bearing:** `semantic_search.rs` resolves the CLIP cache as `data_path.join("../data/models")` — with the default `./data` it normalizes to `./data/models`, but the E2E suite runs the server with `TURBO_PIX_DATA_PATH=test-e2e-data`, so `../data/models` lands in the repo-root `./data/models` (shared cache, pre-downloaded by `cargo run -- --download-models`). "Simplifying" it to `data_path.join("models")` makes the E2E server hang at startup trying to download the model into the sandboxed dir. Do not "fix" the `..`.

**`location:` search trims the absorbed city:** the tokenizer absorbs following words into the city string, which accumulates a leading space for `location: New York` — the LIKE pattern must be built from the TRIMMED city or it silently matches nothing. Bare `location:` (empty after trim) contributes no filter at all (not `LIKE '%%'`).

**Transcode spawn consults status first:** `get_video_file` checks `get_transcode_status` BEFORE spawning: `Failed`/`Timeout` → remove any leftover `*.mp4.tmp` and serve the ORIGINAL with `X-Transcode-Warning` (works even when no stale file exists); `InProgress` → return 202/poll_url WITHOUT spawning a second job; only absent/`Completed`-with-no-file spawns. The status-store cap (128) is a SOFT limit: eviction never removes `InProgress` entries (a polled status must not 404), so a burst can exceed the cap.

**0-byte and range responses:** a plain GET of a 0-byte video → 200 with content-length 0; only a Range request against it → 416 `Content-Range: bytes */0`. Both the no-range arm AND the range arm stream (`tokio::fs::File` + `ReaderStream` + `warp::reply::stream`) — never `std::fs::read`/`vec![0u8; n]` for the whole file. Content-length is derived from a RE-STAT of the open handle (`file.metadata()`) — the file may be replaced/shrunk between the pre-open stat and the open, and an over-advertised length truncates the transfer.

**`/exif` endpoint status codes:** `exif_helpers::read_exif*` now return `Result<Exif, exif::Error>` (original kamadak error preserved). A photo WITHOUT an EXIF segment (`exif::Error::NotFound`, normal for screenshots/generated images) → 404 via `get_photo_exif`; genuine read/parse corruption → 500. No frontend consumer exists for /exif.

**HEAD mirror contracts:** photo-file HEAD reports the ACTUAL on-disk size (stat, not DB `file_size`), 404s when the backing file is gone, and NEVER advertises `accept-ranges` (GET photo-file ignores Range). For RAW files HEAD reports `image/jpeg` (what GET serves) with a documented content-length divergence (RAW source size; GET length would need transcoding). Static-asset HEAD also omits `accept-ranges` (static GET ignores Range). Video HEAD keeps `accept-ranges` (video GET implements ranges).

**Temp-frame counter is module-level:** `semantic_search.rs` uses ONE shared `static TEMP_FRAME_COUNTER: AtomicU64` for both `compute_video_semantic_vector` and `encode_video_vector` — per-function counters seeded at 0 collided on the same `$TMPDIR/turbopix_<pid>_<n>` path, letting one `TempFrameDir` guard delete the other's frames mid-flight.

**Rotate EXIF carry warns, never silently drops:** `rotate_image` `log::warn!`s when the original's EXIF cannot be read (e.g. PNGs with the pngext `Exif\0\0`-prefixed chunk fail kamadak's reader) instead of silently writing the re-encoded file without EXIF, and warns when the file's EXIF Orientation diverges from `photo.orientation` (a stale DB value would bake in an irreversible rotation).

**Security posture (round 11):** the server binds `127.0.0.1` by default (`TURBO_PIX_HOST` overrides; the Containerfile/compose set `0.0.0.0` explicitly) because the API is unauthenticated and destructive. CORS is GONE — `warp_helpers::require_same_origin()` rejects any request whose `Origin` hostname differs from the `Host` header (hostname-only comparison so the Vite dev proxy passes; bracketed IPv6 literals parse via the `[...]` part — a naive ':' split would collapse every IPv6 host to `[` and make the check vacuous; `TURBO_PIX_ALLOWED_HOSTS` (comma-separated) pins the Host header itself against DNS rebinding when set); requests without `Origin` (curl, scripts) always pass; `Origin: null` is always rejected. JSON bodies are capped at 1 MiB via `warp::body::content_length_limit` on the three JSON routes (favorite/metadata/rotate) — it CANNOT be global middleware: warp rejects requests missing Content-Length with 411 (handled explicitly in `handle_rejection`). `DatabaseError` responses are sanitized to a generic message (real error logged server-side). Streaming IS possible in warp 0.4 via `warp::reply::stream(ReaderStream::new(tokio::fs::File))` + explicit content-length (re-stat the open handle) — `get_photo_file` and the video route both stream; `warp::fs::file` (construction-time path) and `warp::body::Body` (bytes-only From impls, sealed FilterBase) are the dead ends, NOT streaming per se. TRANSCODE_CACHE_DIR defaults to `{data_path}/cache/transcoded` (main.rs sets it from config; the old world-writable `/tmp/turbo-pix` is squat-able via symlinks).

**Orphan cleanup chunks via a temp table:** `delete_orphaned_photos` inserts the scanned paths into a per-connection `TEMP TABLE scanned_paths` (multi-row `INSERT OR IGNORE ... VALUES (?),(?)...` in chunks of 500) and runs `NOT IN (SELECT path FROM scanned_paths)` — a single NOT IN with one placeholder per file exceeds `SQLITE_MAX_VARIABLE_NUMBER` (32766) and silently killed nightly cleanup forever. All statements run on ONE held `pool.acquire()` connection (temp tables are per-connection) in autocommit mode so chunk inserts never hold a write lock. The table is `DROP TABLE IF EXISTS`-ed BEFORE the CREATE so a mid-way failure on a previous run (SQLITE_BUSY, I/O error) cannot poison the pooled connection with a leftover temp table (recurring "table already exists" failures until restart). Returns `(file_path, hash_sha256)` pairs so callers can clear hash-keyed caches.

**Partial scans skip orphan cleanup:** `FileScanner::scan()` returns a `scan_complete` flag that is `false` when any root is missing or any directory read fails; `full_rescan_and_cleanup` then skips `delete_orphaned_photos` entirely. Deleting rows for temporarily unreachable files (unmounted drive, permission change) would permanently lose favorites/manual metadata.

**Transcode claim is atomic:** `video_processor::claim_transcode(hash)` consults AND inserts the `InProgress` status under the status-store lock (`TranscodeClaim::{Started, AlreadyInProgress, PreviouslyFailedOrTimedOut}`), closing the check-then-act window where two concurrent identical requests both spawned ffmpeg jobs; the global semaphore only serialized the jobs, never prevented the duplicate spawn.

**Thumbnails are hash-keyed — clear by hash:** `ThumbnailGenerator::get_cache_path` writes `{cache_dir}/{hash[..3]}/{hash}_{size}.{format}`; `CacheManager::clear_for_hash(hash)` (replacing the old `clear_for_path` flat stem scheme that never matched and leaked thumbnails forever) is called on orphan deletion, photo deletion, and rotation (old hash). Rotations are serialized by a global `LazyLock<tokio::sync::Mutex<()>>` in the handler and use a counter-unique temp name (`tmp.{n}.{ext}`) so concurrent rotates cannot interleave temp writes/renames. `ThumbnailGenerator::new` SEEDS its in-memory LRU index from disk (walking the hash subdirs) — without that, files from previous runs are invisible to `enforce_cache_limit` and the cache grows across restarts regardless of `max_cache_size_mb`.

**Vacuum busy_timeout is per-connection:** `vacuum_database` sets `PRAGMA busy_timeout = 1000` on its dedicated connection and RESTORES 30000 afterwards — a leaked 1s limit would make ordinary API requests on that pooled connection fail with SQLITE_BUSY under >1s write contention.

**Semantic score filter must live in SQL:** `SemanticSearchEngine::search` applies `WHERE 1.0 - (vec_distance_cosine(...) / 2.0) >= MIN_SIMILARITY_SCORE` BEFORE `LIMIT/OFFSET` — post-filtering a paginated window truncates results (a short page kills `hasMore` even though further valid matches exist). The pinned sqlite-vec (v0.2.4-alpha) only uses its KNN index with a `MATCH` constraint + `ORDER BY distance` and caps k = LIMIT+OFFSET at 4096 (hard error beyond), so KNN is NOT used — the query is a full scan via a CTE that computes the distance ONCE per row (evaluating it in both WHERE and ORDER BY would double the 2048-byte-blob dot-product cost). Handler clamps `limit` to 1..=200 and `offset` to ≤1M.

**Rotate re-reads the row under the lock:** `rotate_photo` acquires ROTATE_LOCK BEFORE `find_by_hash` — a second overlapping rotate must read the row after the first committed, or it double-applies orientation from a stale snapshot and its `UPDATE ... WHERE hash_sha256 = old_hash` matches 0 rows. `Photo::update_with_old_hash` now checks `SELECT changes()` and fails loudly on a 0-row update (the stale snapshot would otherwise commit silently, leaving file/DB divergent and the returned hash 404ing).

**EXIF rationals need a finite guard:** `MetadataExtractor::rational_to_f64` rejects `denom == 0` and non-finite results at extraction time — a corrupt EXIF rational (0/0 → NaN, x/0 → ±inf) flows into serde_json's `json!` conversion (`to_value(...).unwrap()`), which PANICS on non-finite floats and aborts the entire rescan. Same class as the `apply_stream_info` `d != 0.0` guard.

**EXIF writes are atomic:** `write_exif_to_image` writes to a counter-unique sibling temp (`{stem}.exif_tmp.{n}`) and renames over the original — an in-place `fs::write` truncates/corrupts the original on crash or ENOSPC. `carry_exif_with_reset_orientation` (rotate) skips `In::THUMBNAIL` fields: the experimental kamadak writer drops JPEGInterchangeFormat/Length and emits a dangling/corrupt IFD1 block (mirrors `metadata_writer::should_copy_field`).

**File scanner is cycle-safe:** `walk_directory` records canonicalized dir paths in a visited set (symlink loops like `current -> ..` no longer stack-overflow the process) with a depth cap of 64, uses `entry.file_type()` (no-follow) and then `fs::metadata` for symlink targets, and flags unreadable dirs as a partial scan.

**Phase-1 decode concurrency is capped:** `calculate_optimal_metadata_concurrency()` = `min(4, cores)` for the metadata-scan JoinSet — each task fully decodes a full-resolution image for blurhash (RAW demosaic ~8 bytes/pixel), the same OOM risk the semantic phase avoids by capping at 2.

**Scheduled vacuum is busy-tolerant:** `vacuum_database` sets `PRAGMA busy_timeout = 1000` on its dedicated connection and the scheduler logs a `warn!` skip on failure — a midnight VACUUM racing live API writes otherwise either fails nightly or blocks all writes for its duration.

**build.rs guards bundle-shaping configs:** the stale-dist guard also tracks `vite.config.js`, `package.json`, and `frontend/svelte.config.js` — edits to those change `dist/` without touching `index.html`/`src`/`public` and would otherwise embed a stale bundle silently.

**E2E seed determinism:** (1) global-setup seeds TWO pending collages — the accept test consumes one, so the failed-accept and arrow-key-nav tests (which need ≥1/≥2 cards) actually run. (2) the housekeeping candidate is inserted only after `/api/indexing/status` reports `is_complete` — the housekeeping phase runs LAST and starts with `DELETE FROM housekeeping_candidates`, so seeding earlier raced a 2-4s window. (3) `test-data/test_video_hevc.mp4` (2s 320×240 libx265, `creation_time` pinned to 2020-01-01) drives the transcode poll test; the pinned date keeps `test_video.mp4` FIRST in taken_at-DESC so every existing `videoCards[0]` assertion stays stable. (4) `test-data/sample_with_exif.jpg` (Canon Make/Model EXIF, taken_at 2024-01-01) is seeded for the metadata EXIF test, which targets it BY HASH (its 2011 date sorts it last, so photos[0]-based tests are untouched). The HEVC test clears the per-run transcode cache at start but only when `/api/photos/{hash}/video/status` is not `InProgress` — deleting a running job's temp file makes its final rename fail and the retry's poll end in Failed.

**Transcode failures retry after a cooldown:** `claim_transcode` treats `Failed`/`Timeout` statuses older than `TRANSCODE_RETRY_COOLDOWN` (15 min) as re-claimable — a permanently blocked hash would otherwise never recover from a transient ffmpeg failure without a server restart. Fresh failures keep serving the original with `X-Transcode-Warning`.

**RAW decode concurrency is capped:** `get_photo_file`'s RAW arm holds a 4-permit `RAW_DECODE_LIMIT` semaphore across decode+encode (each decode transiently holds several full-resolution buffers). The SAME static semaphore guards `generate_thumbnail`'s RAW arm (`acquire().await` — its decode also full-res) and collage generation (`try_acquire()`, the fn is sync; on contention `create_collage_image` ABORTS the collage with an Err — the caller then skips the commit, and a skipped photo must never be `continue`d into the collage, because the committed full-chunk signature would block nightly regeneration forever and make the skip permanent). Non-RAW photo files and collage images stream via `tokio_util::io::ReaderStream` + `warp::reply::stream` with a re-statted content-length — never buffer whole files.

**Loopback bind activates the host pin:** the DNS-rebinding pin (`TURBO_PIX_ALLOWED_HOSTS` default `["127.0.0.1", "localhost", "::1"]`) is armed whenever `TURBO_PIX_HOST` resolves to a loopback address, INCLUDING the string `"localhost"` (it is also a loopback alias, so pinning only literal IPs would leave `localhost` unpinned); a non-loopback bind with an empty `allowed_hosts` logs a startup warning. Config tests must restore BOTH env vars after each case.

**Viewer post-await continuations: event vs. navigation split:** PhotoViewer async continuations that touch the URL or navigate MUST bail when `!isOpen` (close() clears the URL photo param but not `currentPhoto`, so an ungarded `replaceState` deterministically reopens the dismissed viewer via the route-sync effect) or when `currentPhoto?.hash_sha256 !== photoHash` (user swiped). Side-effect events (`photoUpdated`/`photoRemoved` dispatch, toast, local `photos` filter) fire BEFORE the guard — suppressing them leaves a grid card keyed by a dead hash (rotated) or a card that 404s (deleted). `acceptCollageFromViewer` closes only when `getNormalizedCollageId(currentPhoto) === collageId` (compare NORMALIZED ids — `collageId` may arrive as a number); else it resets `isAcceptingCollage`/`isPendingCollage`.

**Pagination needs a unique tiebreaker:** `list_with_pagination`/`search_photos` ORDER BY `{sort} {dir}, hash_sha256 {dir}` — camera bursts share the identical EXIF second, and SQLite's tie order follows scan/rowid order which shifts when a background rescan inserts rows between page fetches (photos then duplicate or skip across pages).

**Collage accept is idempotent:** `accept_collage` returns the existing file path when `accepted_at`/`rejected_at` are set — a double-submit (second tab) previously renamed an already-moved source and 500'd with a misleading message.

**Thumbnail cache writes are atomic:** `save_to_disk_cache` writes a counter-unique sibling temp and renames — a truncated file from a crash/ENOSPC would otherwise be served forever as a cache hit.

**Focus trap lives at document level:** keydown events dispatch to `document.activeElement` and bubble through ITS ancestors — a sheet-level Tab handler never fires when focus sits on `document.body` (clicked non-focusable content) or the ring trigger, so the aria-modal trap must intercept Tab in the document keydown handler and pull outside focus back into the sheet (also covers Firefox, where clicking non-focusable content leaves focus on that element).

**warp `or` drops handler-level `reject::not_found()` on same-path method routes:** `CombineRejection` ignores a `Reason::NotFound` when a sibling route rejects with `MethodNotAllowed` (reject.rs `combine`), so a PATCH/DELETE handler returning `reject::not_found()` for a missing id surfaces as 405 whenever the other method route on the same path rejects — warp's `or` chain combines ALL sibling rejections, not just the final one. Handlers on same-path method-differentiated routes (e.g. saved-searches PATCH/DELETE) must reply 404 directly: `warp::reply::with_status(warp::reply::json(&json!({...})), StatusCode::NOT_FOUND).into_response()`. Type note: `warp::reply()` returns an opaque `impl Reply`, so a handler mixing `WithStatus<Json>` and `WithStatus<impl Reply>` arms must convert every arm via `.into_response()` (unify on `warp::reply::Response`).

**Race-free duplicate gate:** `INSERT ... ON CONFLICT DO NOTHING RETURNING id` against a unique index (with `COALESCE`d columns so NULLs participate) is a check-then-act-free create: `fetch_optional` → `None` = conflict, then `SELECT ... WHERE query IS ? ...` (`IS` gives NULL==NULL equality) returns the existing row for the 409. No transaction needed.

**E2E specs that seed/clear server state must do it BEFORE `goto`:** the sidebar (and any mount-time fetch) populates its in-memory list at page load, so a `beforeEach` that deletes via `page.request` after navigating leaves stale rows in the UI list — the first save then `unshift`s onto them (saved-searches duplicate test saw 2 rows after 1 save). Order: `deleteAllSavedSearches(page)` → `goto` → wait. Playwright's `page.request` works before any navigation.

**Consecutive `npx playwright test` invocations race on the server port:** run N's teardown and run N+1's global-setup health check can interleave (stale server still bound → setup's `cargo run` fails with port in use → first tests die with `net::ERR_CONNECTION_REFUSED` at `goto`). Let the previous run's teardown fully settle (or kill leftover servers) before launching the next spec; a failed first run is infra, re-run once before suspecting app regressions.

**Batch endpoints return `BatchResult`, partial failure is 200:** `{applied: [ids], skipped: [ids] (date-shift only), failed: [{id, error}]}` — `skipped`/`failed` are omitted when empty (`skip_serializing_if`), so the frontend treats them as optional. Every batch handler loops per item (find → mutate → update) and NEVER rejects the whole request; failures are identified per item (FR-011). The one non-200: `batch_export` replies 400 with `{error, failed}` when ANY selected photo is unknown or its backing file is gone (checked up front, before building the archive). Validation is shared via `handlers_photo::validate_hashes` (`pub(crate)`, used by photo + housekeeping; collage has its own `validate_collage_ids`) — empty → 400, >1000 → 400. `batch_date_shift` rejects `days == 0` with 400.

**Batch routes must be literal-before-param:** the four `/api/photos/batch/*`, `/api/housekeeping/candidates/batch-remove`, and `/api/collages/batch-{accept,reject}` routes are registered BEFORE the `{hash}`/`{id}`/`candidates/{hash}` param routes. The photo batch routes can't be swallowed by `api_photo_get` (two extra segments), but `/api/housekeeping/candidates/batch-remove` WOULD match `remove_housekeeping_candidate("batch-remove")` and `/api/collages/batch-accept` WOULD parse as `accept_collage(0)` — the ordering rule is load-bearing there. The `reject` collage route must use `with_db(db_pool.clone())` — the batch routes come after it in source and would otherwise borrow a moved pool.

**`zip` 7.2.0 is a direct dependency (was transitive via candle-core):** exports use `zip::ZipWriter::new(std::fs::File)` + `zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)` (7.x name — the old `FileOptions` alias is gone) + `start_file(name, options)` + `std::io::copy`. `ZipFile` from `archive.by_name()` borrows the `ZipArchive` mutably — read entries in scoped blocks or collect bytes before the next `by_name`. Archive building runs in `spawn_blocking` (sync IO); the reply streams the finished file via `ReaderStream` with re-statted content-length + `content-disposition: attachment`. Temp archives live in `{data_path}/cache/export/turbo-pix-export-{YYYYMMDD-HHMMSS}[-n].zip`; the NEXT export sweeps `turbo-pix-export-*.zip` older than 1h (bounded disk, can never delete an in-flight archive).

**`selectionState` contract (frontend):** plain-object map (`selected[key] = true`), keys are `hash_sha256` (or `String(collage.id)` on the collages surface); `$state` reactivity for `Object.keys`/`delete`/index-assign is guaranteed in runes, Set `has()` is not. `orderedKeys` (current surface's visible keys in display order) is maintained by each view's `$effect` — PhotoGrid, HousekeepingView, CollagesView. Selection clears ONLY on `route.view`/`route.query` change (App.svelte effect with `untrack`; sort/year/month keep it — same result set) and on mode exit; `pruneSelection` runs where a list is wholesale-replaced (housekeeping/collages loads) or spliced (PhotoGrid `photoRemoved`/favorites-view `favoriteToggled`); batch actions drop their applied keys explicitly (views' prune is not guaranteed for items outside the loaded page). The bar auto-exits when the count reaches 0 (all selected items left the surface — favorites view unfavorite, collages accept/reject, delete, keep); X/Escape are disabled while `busy` so an in-flight action can't be orphaned. `selectAllVisible` is visible-only by design (FR-014).

**Long-press enters selection mode:** the `longpress` Svelte action (`frontend/src/lib/longpress.js`) is touch-only (`pointerType === 'touch'`), 500ms delay, 10px movement threshold (scroll gesture cancels), suppresses the contextmenu while armed and swallows the following click via a capture-phase listener (flag reset on the NEXT pointerdown so a long-press whose click never arrives can't eat a later tap). Card click handlers check `selectionMode` FIRST (before the `.card-action-btn` guard) so Enter/Space via the open layer toggles selection too.

**New window events for batch flows:** `housekeepingKept` (`detail.hashes`) — HousekeepingView filters candidates + prunes; `photosReloadRequested` — PhotoGrid does `loadPhotos(true)`, dispatched by batch date-shift because the backend returns only hashes, not updated photos (one reload keeps every surface + sort order consistent). PhotoGrid must register `photosReloadRequested` with a NAMED handler (anonymous arrow in addEventListener can't be removed).
