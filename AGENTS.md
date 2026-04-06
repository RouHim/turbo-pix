# Agent Guidelines for TurboPix

## Project Context

Breaking changes are allowed, this application is not in production yet!
Breaking changes are allowed, this application is not in production yet!
Breaking changes are allowed, this application is not in production yet!
This means, no legacy support, no migration scripts, no backward compatibility.
Development/personal project - breaking changes acceptable, database and cache can be recreated!

## Development Commands

**Backend:** `cargo run` | `cargo test` | `cargo clippy` | `cargo fmt`  
**Frontend:** `npm run lint` | `npm run format`

## Code Style

**Backend / Rust:**

- Iterator chains over loops: `.iter().filter_map().next()`
- Arrays over vecs: `[A, B]` vs `vec![A, B]`
- Error handling: `Result<T, E>` with `?`
- Imports: std, external crates, local (blank lines between)
- Zero warnings policy

**Frondend / Vanilla Javascript:**

- `const` over `let` (no reassignment)
- Arrow functions: `() => {}` over `function() {}`
- Template literals: `` `string ${var}` `` over `'string ' + var`
- When adding visible text to the frontend, add them to the `i18n` translation system.

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
- when changing static files, we have to rebuild the binary (cargo build --bin turbo-pix)
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

**UI state desync:** Update both: `appState.value = x; domElement.value = x;`

**Video bugs:** Use `[data-photo-id]` selectors, test GET/HEAD requests, verify `mime_type` in DB

**Icons:** Do not use emojis, use feather icons instead

**Indexing phases:** When adding a new indexing phase to scheduler.rs, also update: (1) CANONICAL*PHASES in handlers_indexing.rs, (2) step div in static/index.html, (3) indexing_phase*\* keys in both i18n files (en + de). Add a regression test for the new phase.

**sqlite-vec:** Uses the vlasky/sqlite-vec community fork (git dep, not crates.io). Drop-in replacement API — same `sqlite3_vec_init`, `vec_distance_cosine`, `vec0` virtual table. Fork includes native musl fix, so no build-time sed patches needed in the Containerfile.

**Glassmorphism visibility:** `backdrop-filter` on CSS Grid children has no visible blur effect — the element must be `position: fixed` overlaying scrollable content for the blur to actually show. Header and sidebar need fixed positioning with content scrolling behind them.

**InfiniteScroll layout dependency:** `infiniteScroll.js:9` binds to `.main-content` as the scroll container (`scrollTop`, `scrollHeight`, `clientHeight`). Any layout refactor must keep `.main-content` as a scrollable element with `overflow-y: auto` — removing this breaks infinite scroll silently.

**Router static file order:** `router.js` must be listed in `handlers_static.rs` STATIC_FILES and loaded in `index.html` BEFORE `app.js` — `window.router` must exist when app.js initializes.

**Router month-without-year guard:** In `router.js buildUrl()`, `?month=` must be nested inside the `year !== null` check. Writing `?month=3` without `?year=` is semantically invalid and the restore path already ignores it.

**Router anti-loop pattern:** Components called from `onStateChange` (popstate) must accept `updateUrl=false` to skip re-pushing to history. Pattern: `applyFilter(updateUrl=true)` normally, `applyFilter(updateUrl=false)` from popstate handler — prevents infinite push loops.

**E2E port collision:** `npm run test:e2e` global-setup may pass health check against a stale dev server on 18473, then `cargo run` fails with "port in use". Always run `pkill -9 -f turbo-pix` before the test suite to ensure a clean port.

**i18n global name:** The app creates its translation manager as `window.i18nManager` (via `new window.I18nManager()` in `app.js`). `window.i18n` is never assigned — calling `window.i18n?.t()` silently falls back to hardcoded strings. Always use `window.i18nManager.t()` or `utils.t('key', 'fallback')`.

**i18n key format:** Translation keys in `data-i18n` HTML attributes must use the exact flat key from the dictionary (e.g., `ui.indexing_phase_discovering`), not dot-path sub-objects (e.g., `ui.indexing.discovering`). The i18nManager does flat lookup, not nested object traversal.

**Startup indexing isolation:** `src/main.rs:start_background_tasks()` must keep `run_startup_rescan()` on a dedicated `std::thread` with its own `tokio::runtime::Runtime`; moving startup indexing back onto the main async runtime starves HTTP requests and makes `/api/indexing/status` look hung.

**Indexing empty-state contract:** `static/js/photoGrid.js:showEmptyState()` must check `window.indexingStatus.isIndexing && !currentQuery` before treating `photos.length === 0` as a true empty state; otherwise first-run indexing regresses to a misleading “No Photos Found” screen.
