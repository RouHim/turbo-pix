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

Learnings are grouped by area. When merging new learnings, prefer folding them into an existing group/bullet over appending standalone entries.

### Frontend — state, routing & loading

- **State & routing:** the `route` store (`router.svelte.js`) and the `$state` stores (`state.svelte.js`) are the single source of truth — components render from them and must not mirror values into write-only fields. `$state` fields MUST use `let`, not `const` (`const x = $state(...)` trips eslint's `no-const-assign` in compiled output).
- **Router guards:** in `buildUrl()`, `?month=` must be nested inside the `year !== null` check (`?month` without `?year` is semantically invalid and the restore path ignores it). Components called from `popstate`/route `$effect` accept `updateUrl=false` to skip re-pushing history (anti-loop pattern: `applyFilter(updateUrl=true)` normally, `false` from popstate).
- **Route-restore `$effect` vs debounced writers:** TimelineSlider's sync effect must read `route.year`/`route.month` BEFORE a `dragInProgress` guard (set in the input handler, cleared in the 300ms-debounced callback) — a debounced-writer field read before the guard wipes the in-progress filter on every drag tick, and an early return that reads nothing replaces the effect's dependency set with `{}`, permanently unsubscribing it (Back/Forward never restores).
- **Infinite scroll layout dependency:** PhotoGrid binds `.main-content` (`scrollTop`/`scrollHeight`/`clientHeight`) — any layout refactor must keep it a scrollable element with `overflow-y: auto`, or infinite scroll dies silently.
- **Ported DOM helpers:** when porting vanilla `setField(id, value)`-style helpers to Svelte, drop the `id` parameter — a leftover id string is truthy and renders literally (e.g. "meta-filesize"). Grep ported call sites for two-arg calls to single-arg functions.
- **File-type helpers:** `isVideoFile`/`isRawFile`/`isCollagePhoto` live in `lib/utils.js` — import them, never re-define (PhotoViewer and ViewerMetadata both had copies). Keep `SwipeableViewer`'s `this.viewer.isVideoFile` surface intact.
- **PhotoGrid loading contracts:** `loadPhotos` delegates to `applyResetState` / `loadSemanticPage` / `loadRegularPage` / `appendPhotos`; `loadSemanticPage` returns `null` for stale responses and the caller no-ops (skip append AND the `lastLoadErrorAt` reset). The dedupe signature, semantic staleness guard and retry-cooldown reset semantics are spread across these helpers — edits must preserve them.
- **Load-more dedupe:** `lastLoadSignature` is set before the request and MUST be cleared in the error path, or every retry rebuilds the identical signature and is silently swallowed until reload. Scroll-triggered retries respect a 5s `LOAD_RETRY_COOLDOWN_MS` after a failure; manual retries bypass the cooldown; the success path resets `lastLoadErrorAt` so a recovered backend is retried immediately.

### Frontend — search

- **Query tokenizer (backend `db::search_photos`):** splits on whitespace and ANDs per-token. Only `type:`/`location:`/`is_favorite:` prefixes exist — SearchBar suggestions are limited to exactly those (`camera:`/`date:`/`has:` were removed; they silently routed to semantic search and could never filter). `location:` absorbs following words until the next prefix token; build the LIKE from the TRIMMED city (absorbed words accumulate a leading space); a bare `location:` contributes no filter (never `LIKE '%%'`).
- **Semantic latency:** `/api/search/semantic` takes ~3s server-side per query (embedding generation) even on tiny collections — E2E/manual checks must allow 5-8s after the request fires; the loading skeleton is not a hang.
- **Semantic staleness:** the semantic path needs its own AbortController signal AND a staleness guard: capture `queryAtStart` before `api.semanticSearch(...)` and bail on `queryAtStart !== currentQuery || signal.aborted` before pushing results (a ~3s embedding response can pollute a grid the user already navigated away from).
- **Semantic mode reset:** PhotoGrid exits `semanticSearchMode` when `route.view !== 'all'` — semantic results are unfilterable, so filtered views (favorites/videos) use the regular path with the view filter merged into the query (`cat` + Favorites → `q=cat is_favorite:true`). Returning to `all` with a non-prefix query must re-enable the mode (`isPrefixQuery` in utils.js) or the same URL (`/?q=cat`) degrades to text search after a view round-trip (the route-sync effect no-ops when `route.query === currentQuery`).

### i18n

- **Parity:** keys are dot-paths into nested JSON dictionaries; en.json and de.json MUST stay structurally identical — every new key lands in BOTH.
- **Integrity guard:** `tests/i18n-integrity.test.js` (`npm run test:i18n`, wired into the CI lint-format job) scans every `$t`/`get(t)` literal, template, and map key (Sidebar/SortControls `key:` fields, App's `titleKeys` object) in `frontend/src` against both dictionaries and fails listing ALL missing keys plus parity drift. Template `${…}` placeholders must be one of the test's `enums` map entries (`phase.id`, `monthKey`, `weekdayKey`) — a new template site needs its enum added. App's `titleFallbacks` are plain strings, not keys.
- **`$t`/`get(t)` options:** always pass `values` for keys containing `{...}` placeholders (omitting it returns the raw message); `values` must be INSIDE the options object — a third argument is silently dropped. `get(t)(key, 'fallback string')` does not apply the positional fallback — pass `{ default: '…' }`.

### Frontend — CSS & styling

- **Scoped styles beat global overrides:** global `@media`/`@container` rules and `@layer utilities` helpers of equal-or-lower specificity are outranked by scoped rules (scoped selectors carry the `svelte-*` hash) — responsive overrides MUST live in the component's scoped `<style>`, never in `app.css` where they silently no-op (examples: PhotoViewer's `.viewer-content` grid + `.viewer-sidebar` bottom-sheet, IndexingOrbit's ring, Header, Sidebar, App's `.content-header` ≤480px, explicit scoped `.foo.hidden { display: none }` for `@layer utilities` helpers).
- **Dead-CSS deletion is property-level:** a global media rule is only dead where the scoped rule sets the SAME properties. The global `.viewer-sidebar` mobile block (position/transform/height/z-index) is LIVE — the scoped block only sets transition/padding/box-shadow and the mobile-sidebar E2E asserts z-index:15; conversely `.viewer-content`/`.photo-grid` rules at ≤1200/≤1024 are fully shadowed and removable.
- **Glassmorphism:** `backdrop-filter` on CSS Grid children has no visible blur — the element must be `position: fixed` overlaying scrollable content. Header/sidebar fixed, content scrolls behind.
- **Lightning CSS:** Vite's default CSS minifier collapses adjacent `backdrop-filter` + `-webkit-backdrop-filter` pairs to the `-webkit-` form, which Chromium ignores (computed `backdrop-filter: none`) — `vite.config.js` must keep `build.cssMinify: false`.
- **Canvas colors:** read `--primary-color` via `getComputedStyle` (returns `oklch(...)`); build alpha variants by inserting ` / <alpha>` before the closing paren (`oklch(0.55 0.08 250 / 0.3)` is a valid `fillStyle`).
- **Skeleton tiles:** direct grid children of `.photo-grid` (same tracks as cards, zero layout shift) with `aspect-ratio: 1` + `min-width: 0` + `border-radius: 0` under the mobile `@container` block. Traps: (1) a nested `.loading-skeleton` grid mis-resolves its `1fr` tracks under intrinsic sizing; (2) an EMPTY box with `aspect-ratio: 1` gets a ~200px auto min-content contribution that inflates tracks — `min-width: 0` opts out; (3) the shared skeleton base in `app.css` must not set `height` — it's a DIFFERENT property than the scoped `aspect-ratio`, both apply, and the fixed height wins (portrait tiles where cards are square).
- **Icons:** feather only, no emojis. `Icon.svelte` imports per-icon SVGs via `feather-icons/dist/icons/<name>.svg?raw`; unregistered names render an EMPTY string silently (no build error, no console warning) — every `<Icon name>` including dynamic bindings MUST be registered in `frontend/src/components/Icon.svelte`; grep `name="` and `name={` usages against the map. The raw opening tag carries `class="feather feather-*"` — the runtime regex strips `width|height|class|aria-hidden` (browsers keep the FIRST duplicate attribute), so `:global(.feather)` never matches — size icons via `:global(svg)`; and `grep -c` against `dist/assets/index.js` counts raw-source lines, not rendered occurrences.

### Frontend — viewer & gestures

- **Viewer async staleness (event vs navigation split):** continuations that touch the URL or navigate MUST bail when `!isOpen` (close() clears the URL photo param but not `currentPhoto`, so an ungarded `replaceState` reopens the dismissed viewer via the route-sync effect) or when `currentPhoto?.hash_sha256 !== photoHash` (user swiped). Side-effect events (`photoUpdated`/`photoRemoved` dispatch, toast, local `photos` filter) fire BEFORE the guard — suppressing them leaves a grid card keyed by a dead hash. `acceptCollageFromViewer` closes only when `getNormalizedCollageId(currentPhoto) === collageId` (NORMALIZED — `collageId` may arrive as a number); else reset `isAcceptingCollage`/`isPendingCollage`.
- **Video-path staleness:** every async continuation in `PhotoViewer.svelte` must re-check `currentPhoto?.hash_sha256 === photo.hash_sha256` before acting — `displayVideo` (after the HEVC support probe and after the transcode `fetch`), `pollTranscodeStatus` (interval top AND after every `await`), `videoEl.onerror` (no retry/toast for a stale photo), and `displayImage`'s `img.onerror` (mirrors its existing `onload` guard).
- **Shared interval owner:** `PhotoViewer.pollTranscodeStatus` shares the module-level `transcodePollTimer` across polls — capture the interval id in a local `const`, `clearInterval` that, and only null the shared field (and hide the shared toast) when it still points at YOUR interval; a stale poll clearing it kills a newer photo's poll (promise never settles, spinner hangs, video never gets its transcode URL).
- **`startViewTransition` defers its callback:** it runs on the NEXT frame — if the viewer closes (Escape) in that window, `close()` removes classes never added and the deferred callback re-adds `active`/`fade-in` (viewer visibly open with `isOpen=false`, Escape appears dead). Guard the callback with `if (!isOpen) return;`.
- **Viewer metadata sidebar starts hidden:** `showSidebar` is `false` at open — `.viewer-sidebar` sits off-viewport (`width: 0; overflow: hidden`). Drive the `[title="View Details"]` (`metadata-btn`) toggle first, wait for `.viewer-sidebar.show`, then click `#metadata-edit-btn`.
- **Gesture manager touch scope:** `GestureManager` listens on a sub-element (`.viewer-main`) but `e.touches` is document-global — filter tracked touches with `this.element.contains(touch.target)`, or a simultaneous touch on chrome outside the element leaves a stale touch that breaks double-tap/pinch until unmount.
- **Actions bind before `$effect` handlers:** `use:gestures` registers touch listeners at element creation, SwipeableViewer's handlers in a mount `$effect` — on the pan-initiating touchmove the manager runs first with `activeGesture` still null and cannot `preventDefault`. The viewer-side handler must call `event.preventDefault()` itself when it calls `startPan()`.
- **Card-level keydown vs inner buttons:** card roots that own click/keyboard (HousekeepingCard) must NOT `preventDefault` bubbled Enter/Space from inner action buttons — guard with `if (e.target !== e.currentTarget) return;`. PhotoCard/CollagesView avoid the problem structurally: plain root + stretched `.photo-card-open-layer` (`position: absolute; inset: 0; z-index: 3; role="button"`) owns open with no focusable descendants; action buttons are siblings at z-index 15.
- **Focus trap is document-level:** keydown dispatches to `document.activeElement` and bubbles through ITS ancestors — a sheet-level Tab handler never fires when focus sits on `document.body` or the ring trigger; intercept Tab in the document keydown handler and pull outside focus back into the sheet.
- **`svelte-ignore` a11y comments trip eslint's `svelte/no-unused-svelte-ignore`:** the Vite build warns `a11y_no_static_element_interactions` but eslint's parser does not, so the ignore is "unused" to lint. Use `role="presentation"` on decorative containers instead (descendants stay accessible).
- **Playwright click stability vs CSS animations:** an animated `transform: scale(...)` (`.photo-viewer.fade-in`) keeps every descendant's bounding box moving, so `locator.click()` on viewer children never stabilizes. Keep viewer open/close animations opacity-only.

### Selection & batch

- **`selectionState` contract:** plain-object map (`selected[key] = true`), keys are `hash_sha256` (or `String(collage.id)` on the collages surface); `$state` reactivity for `Object.keys`/`delete`/index-assign is guaranteed in runes, Set `has()` is not. `orderedKeys` (visible keys in display order) is maintained by each view's `$effect` (PhotoGrid, HousekeepingView, CollagesView). Selection clears ONLY on `route.view`/`route.query` change (App.svelte effect with `untrack`; sort/year/month keep it) and on mode exit; `pruneSelection` runs where a list is wholesale-replaced or spliced; batch actions drop their applied keys explicitly; the bar auto-exits at 0; X/Escape are disabled while `busy`; `selectAllVisible` is visible-only by design.
- **Long-press selection:** the `longpress` action (`lib/longpress.js`) is touch-only, 500ms delay, 10px movement threshold (scroll cancels), suppresses contextmenu while armed and swallows the following click via a capture-phase listener (flag reset on the NEXT pointerdown). Card click handlers check `selectionMode` FIRST (before the `.card-action-btn` guard).
- **Batch window events:** `housekeepingKept` (`detail.hashes`) — HousekeepingView filters candidates + prunes; `photosReloadRequested` — PhotoGrid does `loadPhotos(true)`, dispatched by batch date-shift (backend returns hashes only). PhotoGrid must register `photosReloadRequested` with a NAMED handler (anonymous arrows can't be removed).
- **Batch API:** batch endpoints return `BatchResult`: `{applied, skipped (date-shift only), failed}` — `skipped`/`failed` are omitted when empty (`skip_serializing_if`); every handler loops per item and NEVER rejects the whole request (FR-011). The one non-200: `batch_export` replies 400 when ANY selected photo is unknown or its backing file is gone (checked up front). Validation via `handlers_photo::validate_hashes` (`pub(crate)`, photo + housekeeping; collages have their own `validate_collage_ids`): empty or >1000 → 400. `batch_date_shift` rejects `days == 0`.
- **Batch routes are literal-before-param:** `/api/photos/batch/*`, `/api/housekeeping/candidates/batch-remove`, and `/api/collages/batch-{accept,reject}` MUST be registered BEFORE the `{hash}`/`{id}`/`candidates/{hash}` routes — otherwise `batch-remove` matches `remove_housekeeping_candidate("batch-remove")` and `batch-accept` parses as `accept_collage(0)`. The collage `reject` route must use `with_db(db_pool.clone())` (batch routes come after it in source and would otherwise borrow a moved pool).

### Backend — metadata, EXIF & dates

- **taken_at extraction order:** ffprobe `format.tags.creation_time` → `com.apple.quicktime.creationdate` → `streams[].tags.creation_time` → `format.tags.date`/`date-{lang}`; then filename patterns (`%Y%m%d_%H%M%S`, `%Y%m%d%H%M%S`, `%Y-%m-%d-%H-%M-%S` full stem, plus shard `%Y%m%d`/`%Y-%m-%d` with optional adjacent time; years < 1990 rejected, consistent with `parse_video_creation_time`); then `apply_file_creation_fallback()` using `created().or_else(modified)`.
- **EXIF access:** all EXIF reads/writes go through `src/exif_helpers.rs` (`read_exif_from_path`/`read_exif`/`build_exif_buffer`/`write_exif_to_image`; the writer accepts `jpg`/`jpeg`/`png`) — never hand-roll `exif::Reader::new()` (per-file copies had already drifted).
- **`/exif` status codes:** photo without an EXIF segment (`exif::Error::NotFound`, normal for screenshots/generated images) → 404; genuine read/parse corruption → 500. No frontend consumer exists for /exif.
- **EXIF rationals need a finite guard:** `MetadataExtractor::rational_to_f64` rejects `denom == 0` and non-finite results at extraction time — NaN/±inf flows into serde_json's `json!` (`to_value(...).unwrap()`), which PANICS and aborts the entire rescan. Same class as the `apply_stream_info` `d != 0.0` guard.

### Backend — rotation & photo identity

- **Rotation on housekeeping candidates:** rewriting `photos.hash_sha256` violates `housekeeping_candidates.photo_hash`'s FK (`ON DELETE CASCADE`, no `ON UPDATE`) — delete the stale candidate row inside the SAME transaction as the PK rewrite (`image_editor::rotate_image`). `Photo::update_with_old_hash` therefore requires a `&mut sqlx::Transaction` (not a bare pool) and checks `SELECT changes()` — it fails loudly on a 0-row update (a stale snapshot would otherwise commit silently, leaving file/DB divergent and the returned hash 404ing). Regression test: `test_rotate_db_update_removes_housekeeping_candidate`.
- **Rotate re-reads the row under the lock:** `rotate_photo` acquires ROTATE_LOCK BEFORE `find_by_hash` — an overlapping rotate must read the row after the first committed, or it double-applies orientation from a stale snapshot.
- **Rotation preserves EXIF:** `image::save` re-encodes from pixels and drops EXIF — read EXIF from the ORIGINAL before the pixel transform, write it into the temp file with Orientation forced to 1 (`carry_exif_with_reset_orientation`). Skip `Value::Unknown` fields (the experimental writer can't serialize them; pattern with `Value::Unknown(..)`) and `In::THUMBNAIL` fields (writer emits a dangling/corrupt IFD1). EXIF writes are themselves atomic: `write_exif_to_image` writes a counter-unique sibling temp (`{stem}.exif_tmp.{n}`) and renames over the original — an in-place write truncates/corrupts on crash or ENOSPC. Warn, never silently drop: `rotate_image` `log::warn!`s when the original's EXIF can't be read (PNG pngext `Exif\0\0` chunks fail kamadak) and when the file's Orientation diverges from `photo.orientation` (a stale DB value would bake in an irreversible rotation). Regression test: `test_rotate_preserves_exif_in_file`.
- **Hash re-key preserves `is_favorite`:** `Photo::create_or_update_with_transaction` captures `is_favorite` from the row being replaced (same `file_path`, different hash) and re-applies it. NOTE: `hash_sha256` is a PATH-string hash, deliberately — identity (and favorites) stay stable across in-place edits (Lightroom re-export, sync overwrite). Because the hash never changes on byte edits, the thumbnail/transcode/collage caches fold a size+mtime CONTENT VERSION into their keys (`CacheKey.content_version`, `get_transcoded_path_versioned`, `build_collage_signature`); the rescan updates `file_size`/`file_modified`, rotating the cache key and regenerating. `clear_for_hash` removes every `{hash}_*` file (versioned and unversioned). Regression test: `test_create_or_update_preserves_favorite_across_hash_rekey`.
- **Thumbnails:** hash-keyed `{cache_dir}/{hash[..3]}/{hash}_{size}.{format}`; `CacheManager::clear_for_hash` (never the old flat `clear_for_path` scheme) is called on orphan deletion, photo deletion, and rotation (old hash). Writes are atomic (counter-unique sibling temp + rename) — a truncated cache file would otherwise be served forever. `ThumbnailGenerator::new` SEEDS its in-memory LRU index from disk (walking the hash subdirs) or `enforce_cache_limit` misses previous runs and the cache grows across restarts. Rotations are serialized by a global `LazyLock<tokio::sync::Mutex<()>>` with counter-unique temp names (`tmp.{n}.{ext}`).

### Backend — video serving & transcode

- **Transcode pipeline:** `video_processor::claim_transcode(hash)` consults AND inserts the `InProgress` status under the status-store lock (`TranscodeClaim::{Started, AlreadyInProgress, PreviouslyFailedOrTimedOut}`) — the global semaphore only serialized jobs, never prevented duplicate spawns. `get_video_file` checks `get_transcode_status` BEFORE spawning: `Failed`/`Timeout` → remove any leftover `*.mp4.tmp` and serve the ORIGINAL with `X-Transcode-Warning` (fresh failures; statuses older than `TRANSCODE_RETRY_COOLDOWN` (15 min) are re-claimable so a transient ffmpeg failure recovers without restart); `InProgress` → 202/poll_url without spawning; only absent/`Completed`-with-no-file spawns. `transcode_hevc_to_h264_with_timeout_and_path` writes to a sibling `*.mp4.tmp` and atomically renames on success; failure AND timeout remove the temp. Status store capped at 128 (settled evicted first; `InProgress` never evicted — soft cap).
- **Video file-serving contract:** plain GET of a 0-byte video → 200/len 0; Range against it → 416 `Content-Range: bytes */0`; unsatisfiable ranges and `start > end` → 416 (never clamped/404); suffix ranges `bytes=-N` supported; multi-range ignored → full 200. Both arms stream (`tokio::fs::File` + `ReaderStream` + `warp::reply::stream`; `tokio-util` with the `io` feature is a direct dep) — never `std::fs::read`. Content-length comes from a RE-STAT of the open handle (`file.metadata()`) — the file may shrink between stat and open, and an over-advertised length truncates the transfer. Streaming IS possible in warp 0.4 (`warp::reply::stream` + explicit content-length); `warp::fs::file` (construction-time path) and `warp::body::Body` (bytes-only From impls) are the dead ends.
- **HEAD mirror contracts:** photo-file HEAD reports the ACTUAL on-disk size (stat, not DB `file_size`), 404s when the backing file is gone, and never advertises `accept-ranges` (GET ignores Range). RAW HEAD reports `image/jpeg` with a documented content-length divergence (RAW source size; GET length needs transcoding). Static-asset HEAD omits `accept-ranges`. Video HEAD keeps it (video GET implements ranges). HEAD must never trigger transcoding.

### Backend — DB, scanning & cleanup

- **sqlx:** `Query::execute` needs no `Executor` import — passing `&mut *tx` (or `&mut **tx` for `&mut Transaction`) resolves via the inherent method's `E: Executor` bound; importing it is an unused-import warning. Applies to `db.rs` and `image_editor.rs` alike.
- **sqlite-vec:** uses the vlasky/sqlite-vec community fork (git dep, not crates.io) — drop-in API (`sqlite3_vec_init`, `vec_distance_cosine`, `vec0`), includes the native musl fix, so no Containerfile sed patches.
- **libsqlite3-sys constraint:** cannot bump independently while `sqlx = 0.8.x` is in use — `cargo tree -i libsqlite3-sys` resolves both through the same `0.30.x` native `sqlite3` link target; `0.37.x` causes a links conflict.
- **Orphan cleanup chunks via a temp table:** `delete_orphaned_photos` inserts scanned paths into a per-connection `TEMP TABLE scanned_paths` (multi-row `INSERT OR IGNORE` in chunks of 500) then runs `NOT IN (SELECT path FROM scanned_paths)` — one placeholder per file exceeds `SQLITE_MAX_VARIABLE_NUMBER` (32766) and silently killed nightly cleanup forever. All statements run on ONE held `pool.acquire()` connection in autocommit mode (chunk inserts never hold a write lock); `DROP TABLE IF EXISTS` before CREATE so a mid-way failure can't poison the pooled connection. Returns `(file_path, hash_sha256)` pairs so callers can clear hash-keyed caches.
- **Partial scans skip orphan cleanup:** `FileScanner::scan()` returns `scan_complete = false` when any root is missing or any directory read fails; `full_rescan_and_cleanup` then skips `delete_orphaned_photos` entirely (deleting rows for temporarily unreachable files would permanently lose favorites/manual metadata).
- **Scanner is cycle-safe:** `walk_directory` records canonicalized dir paths in a visited set (symlink loops no longer stack-overflow) with a depth cap of 64; uses `entry.file_type()` (no-follow) then `fs::metadata` for symlink targets; unreadable dirs flag a partial scan.
- **Decode concurrency caps:** metadata phase `calculate_optimal_metadata_concurrency()` = `min(4, cores)` (each task fully decodes a full-resolution image for blurhash; RAW demosaic ~8 bytes/pixel — the OOM risk the semantic phase avoids by capping at 2). RAW file serving, thumbnail generation, and collage generation share one 4-permit `RAW_DECODE_LIMIT`; collage uses `try_acquire()` (fn is sync) — on contention it ABORTS the collage with an Err, the caller skips the commit, and a skipped photo must never be `continue`d into the collage (the committed full-chunk signature would block nightly regeneration forever). Non-RAW photo files and collage images stream via `ReaderStream` + `warp::reply::stream` with a re-statted content-length.
- **Vacuum:** `vacuum_database` sets `PRAGMA busy_timeout = 1000` on its dedicated connection and RESTORES 30000 afterwards (a leaked 1s limit makes pooled API writes fail with SQLITE_BUSY); the scheduler logs a `warn!` skip on failure (a midnight VACUUM racing live writes must not block them).
- **Pagination tiebreaker:** `list_with_pagination`/`search_photos` ORDER BY `{sort} {dir}, hash_sha256 {dir}` — camera bursts share the identical EXIF second, and SQLite's tie order follows scan/rowid order which shifts when a background rescan inserts rows between page fetches (photos duplicate or skip across pages).
- **Race-free duplicate gate:** `INSERT ... ON CONFLICT DO NOTHING RETURNING id` against a unique index (COALESCEd columns so NULLs participate) — `fetch_optional` → `None` = conflict, then `SELECT ... WHERE query IS ?` (`IS` gives NULL==NULL equality) returns the existing row for the 409. No transaction needed.
- **Migrations are content-checksummed:** `sqlx::migrate!` records a SHA-384 of each file in `_sqlx_migrations.checksum` and verifies it on every startup — NEVER edit an applied migration (editing `20250101000005_create_indexes.sql` bricked migrated DBs with `VersionMismatch`); schema cleanups on shipped migrations belong in a NEW migration (`DROP INDEX IF EXISTS`). Repairing an already-applied DB is a one-row bookkeeping fix: `sqlite3 <db> "UPDATE _sqlx_migrations SET checksum = X'<sha384 of CURRENT file>' WHERE version = ..."`.
- **`kamadak-exif` is a cargo-machete false positive:** the crate's lib name is `exif` (used in metadata_extractor, metadata_writer, image_editor, handlers_photo, exif_helpers) — add `ignored = ["kamadak-exif"]` to `[package.metadata.cargo-machete]` if the report annoys.
- **warp `test` feature** is enabled for route smoke tests (`warp::test::request()`, async — `#[tokio::test]` + `.reply(&routes).await`; `features = ["server", "multipart", "test"]` in Cargo.toml); don't remove it or the smoke test stops compiling.

### Backend — semantic search internals

- **Model cache path `../data` is load-bearing:** `semantic_search.rs` resolves the CLIP cache as `data_path.join("../data/models")` — with the E2E data path (`TURBO_PIX_DATA_PATH=test-e2e-data`) that lands in the repo-root `./data/models` (shared cache, pre-downloaded). "Simplifying" to `data_path.join("models")` makes the E2E server hang at startup downloading the model. Do not "fix" the `..`.
- **Temp-frame counter is module-level:** `semantic_search.rs` uses ONE shared `static TEMP_FRAME_COUNTER: AtomicU64` for both `compute_video_semantic_vector` and `encode_video_vector` — per-function counters seeded at 0 collided on `$TMPDIR/turbopix_<pid>_<n>` and one `TempFrameDir` guard deleted the other's frames mid-flight.
- **Semantic score filter must live in SQL:** `SemanticSearchEngine::search` applies `WHERE 1.0 - (vec_distance_cosine(...) / 2.0) >= MIN_SIMILARITY_SCORE` BEFORE `LIMIT/OFFSET` — post-filtering a paginated window truncates results (a short page kills `hasMore` despite further valid matches). The pinned sqlite-vec only uses its KNN index with a `MATCH` constraint + `ORDER BY distance` (k capped at LIMIT+OFFSET 4096), so the query is a full-scan CTE that computes the distance ONCE per row (evaluating in both WHERE and ORDER BY would double the 2048-byte-blob dot-product cost). Handler clamps `limit` to 1..=200, `offset` ≤1M.

### Backend — HTTP & routes

- **warp `or` drops handler-level `reject::not_found()` on same-path method routes:** `CombineRejection` ignores a `Reason::NotFound` when a sibling route rejects with `MethodNotAllowed` — a PATCH/DELETE handler returning `reject::not_found()` for a missing id surfaces as 405. Same-path method-differentiated handlers (e.g. saved-searches PATCH/DELETE) must reply 404 directly: `warp::reply::with_status(warp::reply::json(&json!({...})), StatusCode::NOT_FOUND).into_response()`. Type note: `warp::reply()` returns an opaque `impl Reply`, so mixed arms must be converted via `.into_response()` (unify on `warp::reply::Response`).

### Backend — collages & export

- **Collage timestamps parse two formats:** `Collage::from_row` accepts RFC3339 AND SQLite `YYYY-MM-DD HH:MM:SS` (`datetime('now')`/`CURRENT_TIMESTAMP`). `accept()` also updates `thumbnail_path` (the thumbnail moves next to the accepted file).
- **Collage accept is idempotent:** `accept_collage` returns the existing file path when `accepted_at`/`rejected_at` are set — a double-submit (second tab) previously renamed an already-moved source and 500'd.
- **`zip` 7.2.0 is a direct dependency** (was transitive via candle-core): exports use `ZipWriter::new(std::fs::File)` + `zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)` (7.x names — the `FileOptions` alias is gone) + `start_file(name, options)` + `std::io::copy`. `ZipFile` from `archive.by_name()` borrows the `ZipArchive` mutably — read entries in scoped blocks. Archive building runs in `spawn_blocking`; the reply streams via `ReaderStream` with re-statted content-length + `content-disposition: attachment`. Temp archives live in `{data_path}/cache/export/turbo-pix-export-{YYYYMMDD-HHMMSS}[-n].zip`; the NEXT export sweeps `turbo-pix-export-*.zip` older than 1h.

### Backend — security

- **Posture:** the server binds `127.0.0.1` by default (`TURBO_PIX_HOST` overrides; the Containerfile/compose set `0.0.0.0` explicitly) because the API is unauthenticated and destructive. CORS is GONE — `warp_helpers::require_same_origin()` rejects any `Origin` whose hostname differs from the `Host` header (hostname-only comparison so the Vite dev proxy passes; bracketed IPv6 literals parse via the `[...]` part); requests without `Origin` (curl, scripts) pass; `Origin: null` is always rejected. `TURBO_PIX_ALLOWED_HOSTS` (comma-separated, default `["127.0.0.1", "localhost", "::1"]`) pins the Host header against DNS rebinding when set — armed whenever `TURBO_PIX_HOST` resolves to a loopback address, INCLUDING the string `"localhost"`; a non-loopback bind with an empty `allowed_hosts` logs a startup warning. Config tests must restore BOTH env vars. JSON bodies are capped at 1 MiB via `warp::body::content_length_limit` on the three JSON routes (favorite/metadata/rotate) — it CANNOT be global middleware: warp rejects requests missing Content-Length with 411 (handled explicitly in `handle_rejection`). `DatabaseError` responses are sanitized to a generic message (real error logged server-side).

### Indexing

- **Phases:** adding an indexing phase to scheduler.rs also requires updating (1) CANONICAL*PHASES in handlers_indexing.rs, (2) the PHASES array + phase UI in `IndexingOrbit.svelte`, (3) `indexing_phase_*` keys in BOTH i18n files, plus a regression test.
- **Startup isolation:** `start_background_tasks()` must keep `run_startup_rescan()` on a dedicated `std::thread` with its own `tokio::runtime::Runtime` — on the main async runtime it starves HTTP requests and makes `/api/indexing/status` look hung.
- **Empty-state contract:** PhotoGrid's template empty-state branch must check `indexingState.isIndexing && !currentQuery` before treating `photos.length === 0` as a true empty state — otherwise first-run indexing regresses to a misleading "No Photos Found" screen.

### Build & tooling

- **build.rs stale-dist guard:** walks `frontend/{index.html,src,public}` PLUS `vite.config.js`, `package.json`, `frontend/svelte.config.js` (emitting rerun-if-changed per entry) and panics when the newest source is newer than `dist/index.html` — a loud failure instead of silently embedding a stale bundle. Strict `>` comparison: equal mtimes count as fresh. CI must run `npm run build` before any cargo step that embeds dist/.
- **Frontend/backend extension lists are pinned by a parity test:** `mimetype_detector.rs::frontend_extension_lists_match_backend` parses `frontend/src/lib/constants.js` and asserts `VIDEO_EXTENSIONS`/`RAW_EXTENSIONS` match the backend detector (and `raw_processor::is_raw_file`) exactly, in both directions — adding a media format means updating BOTH.

### E2E & test infra

- **Video bug hunting:** use `[data-photo-id]` selectors, test GET/HEAD requests, verify `mime_type` in DB.
- **Server port races:** global-setup may pass the health check against a stale dev server, then `cargo run` fails "port in use" — global-setup kills stale servers itself via `pkill -9 -f 'target/(debug|release)/turbo-pix'` (only that deliberately narrow pattern; a broad `-f turbo-pix` match kills the Playwright runner itself). Consecutive `npx playwright test` invocations race the same way — let the previous run's teardown fully settle (or kill leftover servers) before the next run; a failed first run is infra, re-run once before suspecting app regressions.
- **Server log strips query strings:** the warp access log shows `GET /api/photos` even with `?page=1&limit=50&q=...` — when debugging request issues, trust the browser-side `page.on('request')`/`waitForResponse` URLs.
- **Seed determinism:** (1) global-setup seeds TWO pending collages — the accept test consumes one, so failed-accept and arrow-key-nav tests (≥1/≥2 cards) actually run. (2) The housekeeping candidate is inserted only after `/api/indexing/status` reports `is_complete` — the housekeeping phase runs LAST and starts with `DELETE FROM housekeeping_candidates` (seeding earlier raced a 2-4s window). (3) `test-data/test_video_hevc.mp4` has `creation_time` pinned to 2020-01-01 so `test_video.mp4` stays FIRST in taken_at-DESC (`videoCards[0]` assertions stable). (4) `test-data/sample_with_exif.jpg` (taken_at 2024-01-01) is seeded for the metadata EXIF test and targeted BY HASH (its 2011 date sorts it last). The HEVC test clears the per-run transcode cache only when status is not `InProgress` — deleting a running job's temp makes its final rename fail.
- **Seed/clear server state BEFORE `goto`:** mount-time fetches populate in-memory lists at page load, so a `beforeEach` that deletes via `page.request` after navigating leaves stale rows in the UI list. Order: `deleteAllSavedSearches(page)` → `goto` → wait. Playwright's `page.request` works before any navigation.
- **Test-env lock needs a drop-guarded depth wrapper:** `video_processor::tests::acquire_test_env_lock()` returns a `TestEnvGuard` whose `Drop` decrements the thread-local nesting depth — a manual `release_test_env_lock()` leaks the depth and later acquires silently no-op (the FFPROBE_PATH/FFMPEG_PATH race between test modules quietly returns). Regression test: `test_env_lock_guard_drop_resets_nesting_depth`.
- **Reduced-motion E2E contract is the inline `style.animation`:** `indexing-orbit.e2e.spec.js` polls `element.style.animation` — keep animation properties in the inline style binding (a CSS class toggle or `@media (prefers-reduced-motion)` rule leaves the inline style empty and fails the test).
