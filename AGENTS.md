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

2. **State & routing:** the `route` store (`router.svelte.js`) and `$state` stores (`state.svelte.js`) are the single source of truth — components render from them, never mirror values into write-only fields; `$state` fields MUST use `let` (const trips eslint's `no-const-assign`). Route-restore trap: TimelineSlider's sync effect must read `route.year`/`route.month` BEFORE the `dragInProgress` guard — reading a debounced-writer field first wipes the in-progress filter on every drag tick, and an early return that reads nothing replaces the effect's dependency set with `{}`, permanently unsubscribing it (Back/Forward never restores).

3. **Scoped styles beat global overrides:** global `@media`/`@container` rules and `@layer utilities` helpers of equal-or-lower specificity are outranked by scoped `svelte-*` selectors — responsive overrides MUST live in the component's scoped `<style>`, never in `app.css` where they silently no-op. Dead-CSS deletion is property-level: a global rule is only dead where the scoped rule sets the SAME properties. Keep `build.cssMinify: false` in `vite.config.js` (Lightning CSS collapses `backdrop-filter` pairs to the `-webkit-` form Chromium ignores).

4. **Viewer async staleness:** every async continuation in `PhotoViewer.svelte` must re-check `currentPhoto?.hash_sha256 === photo.hash_sha256` before acting (`displayVideo` after the HEVC probe AND after the transcode `fetch`, `pollTranscodeStatus` at interval top AND after every `await`, `onerror` handlers); continuations touching the URL/navigating must also bail when `!isOpen`. `pollTranscodeStatus` shares a module-level timer — capture the id in a local `const`, only clear/null the shared field when it still points at YOUR interval. Guard `startViewTransition` callbacks with `if (!isOpen) return;` (the callback runs on the next frame).

5. **Icon registry:** feather only, no emojis. An unregistered `<Icon name>` (including dynamic bindings) renders an EMPTY string silently — no build error, no console warning; every icon MUST be registered in `frontend/src/components/Icon.svelte`; grep `name="`/`name={` usages against the map. The runtime regex strips the raw `class="feather …"`, so `:global(.feather)` never matches — size icons via `:global(svg)`.

6. **Rotation & photo identity:** `image::save` re-encodes from pixels and drops EXIF — read EXIF from the ORIGINAL before the transform and write it into the temp file with Orientation forced to 1 (skip `Value::Unknown` and `In::THUMBNAIL` fields), via `src/exif_helpers.rs` (never hand-roll `exif::Reader::new()`); EXIF writes are atomic sibling-temp + rename. Rewriting `photos.hash_sha256` violates the `housekeeping_candidates` FK (no `ON UPDATE`) — delete the stale candidate row in the SAME transaction; `Photo::update_with_old_hash` takes a `&mut sqlx::Transaction` and checks `SELECT changes()`. `hash_sha256` is a PATH-string hash on purpose (favorites stay stable across in-place edits) — thumbnail/transcode/collage caches fold a size+mtime CONTENT VERSION into their keys, and `clear_for_hash` removes every `{hash}_*` file. Album membership (`album_members`) references `photos(hash_sha256)` AND `albums(id)` with `ON DELETE CASCADE` both ways — library removals and album deletions clean up membership for free, but only because sqlx enables FK enforcement; cascade claims need a dedicated test, never a pragma.

7. **Native-first video serving & codec-agnostic transcode:** playback decisions live SERVER-SIDE — `get_video_file` runs `decide()` over the capability record (codec/container/bit-depth/`moov_at_start`) against the client's declared codecs (`X-TurboPix-Codecs` header or `client` query param; missing → conservative h264-8). Appending `?decision` (bare or `=true`) returns JSON `{action: direct|remux|transcode|empty, url, reason}` — always 200, never 202; the 202/`poll_url` handshake belongs to the transcode spawn, not the decision probe. Transcode is codec-AGNOSTIC (`transcode_codec_to_h264`, not hevc-only) and gated by a worker pool: `transcode_semaphore()` is `OnceLock` and locks in the FIRST `transcode_max_pool()` it saw — `TURBO_PIX_MAX_TRANSCODES` (0 = disabled, checked before touching the semaphore; absent → `min(max(nproc/2,1),4)`), timeout via `TURBO_PIX_TRANSCODE_TIMEOUT_SECS` (default 300); no test reset hook, so tests assert `transcode_max_pool()` directly. `claim_transcode` still consults AND inserts `InProgress` under the status-store lock (the semaphore only serializes, never prevents duplicate spawns); `Failed`/`Timeout` remove leftover `*.mp4.tmp` and serve the original with `X-Transcode-Warning` (`TRANSCODE_RETRY_COOLDOWN` = 15 min). `/video/status` reports `percent` + `deadline_ms`; the Svelte poller stops on the server deadline (+30s grace), never a hard-coded client cap. Status store capped at 128 (`InProgress` never evicted).

8. **Migrations are content-checksummed:** `sqlx::migrate!` records a SHA-384 of each migration file and verifies it on every startup — NEVER edit an applied migration (bricks DBs with `VersionMismatch`); schema cleanups on shipped migrations belong in a NEW migration. Repairing an applied DB: `sqlite3 <db> "UPDATE _sqlx_migrations SET checksum = X'<sha384 of CURRENT file>' WHERE version = ..."`.

9. **Orphan cleanup chunks via a temp table:** `delete_orphaned_photos` inserts scanned paths into a per-connection `TEMP TABLE scanned_paths` (multi-row chunks of 500) then runs `NOT IN (SELECT path FROM scanned_paths)` — one placeholder per file exceeds `SQLITE_MAX_VARIABLE_NUMBER` (32766) and silently killed nightly cleanup. All statements on ONE held `pool.acquire()` connection; `DROP TABLE IF EXISTS` before CREATE. Partial scans (`scan_complete = false` when a root is missing / directory unreadable) skip orphan cleanup entirely — deleting rows for temporarily unreachable files would permanently lose favorites/manual metadata. Membership `IN (...)` lists chunk at the same 500; `INSERT OR IGNORE INTO album_members … SELECT … FROM photos WHERE hash IN (…)` makes add/create-from-selection robust to photos that left the library between selection and submit (unknown hashes silently skip).

10. **E2E — server port races:** global-setup kills stale servers via `pkill -9 -f 'target/(debug|release)/turbo-pix'` — ONLY that deliberately narrow pattern (a broad `-f turbo-pix` match kills the Playwright runner itself). Consecutive `npx playwright test` invocations race the same way: let the previous run's teardown fully settle before re-running; a failed first run is usually infra — re-run once before suspecting app regressions.
