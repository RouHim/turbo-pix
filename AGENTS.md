# Agent Guidelines for TurboPix

## Project Context

Breaking changes are allowed, this application is not in production yet!
This means, no legacy support, no migration scripts, no backward compatibility.
Development/personal project - breaking changes acceptable, database and cache can be recreated!

## Development Commands

**Backend:** `cargo run` | `cargo test` | `cargo clippy` | `cargo fmt`  
**Frontend:** `npm run build` (required before cargo build — embeds dist/) | `npm run lint` | `npm run format`  
**Preview:** `./scripts/preview.sh` — builds `dist/`, fetches ONNX Runtime, then builds the binary  
**Build prerequisite:** local Rust builds and E2E need `eval "$(./scripts/fetch-ort.sh --export)"`; see `DEVELOPMENT.md` → Build from source.

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

- Lint and format before committing
- Tests must pass
- Meaningful commit messages

# Development pattern

- TDD: Test Driven Development, write tests first, then implement the feature
- BDD: Behavior Driven Development, focus on the behavior of the application, use GIVEN, WHEN, THEN style
- E2E: End to End testing, test the application as a whole, use Playwright or Puppeteer
- **Avoid:** Hardcoded paths and fallback logic mask bugs
- When troubleshooting bugs, try to reproduce the bug first writing a test
- After finishing a Task (feature, bug fix, etc) extract relevant learnings from the session/task (if there are ones),
  and fold them into `docs/learnings.md`. Also verify all entries in the learnings section are still valid.

## Testing

- Test images and videos are located in `test-data/`

**Unit / i18n (node, no browser):**

```bash
npm run test:unit         # All tests/*.test.js (mse player, encoder hint, timeline aggregates, i18n)
npm run test:i18n         # i18n key parity + usage integrity only
```

Both run in CI's `lint-format` job.

**E2E:**

TurboPix uses Playwright for end-to-end testing with real backend integration.

**Quick Start:**

```bash
npm run test:e2e          # Run all tests
npm run test:e2e:reuse    # Target a prepared server on 18473; skips setup/teardown
npm run test:e2e:ui       # Interactive UI mode
npm run test:e2e:headed   # See browser
npm run test:e2e:debug    # Debug mode
npm run test:e2e:report   # View test report
```

`test:e2e:reuse` expects a seeded, healthy server on `TURBO_PIX_E2E_PORT` (default
18473); it does not build, reseed, or stop it.

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

- Start: `eval "$(./scripts/fetch-ort.sh --export)" && nohup cargo run &` + wait for `curl --retry 5 --retry-delay 2 http://localhost:18473/health`
- Test at `http://localhost:18473`
- Kill process after testing

## Learnings

Durable traps live in `docs/learnings.md` — read it before touching a matching area: i18n keys, state/routing, scoped CSS, viewer async staleness, icons/map config, rotation & photo identity, video serving/transcode, migrations, SQLite queries & the derived date/coordinate copy, E2E harness. New trap? Fold it into an existing entry there (capped at 10).

Mobile gestures: behavior and acceptance live in `.spec/mobile-gestures.md`; tests in `tests/e2e/specs/mobile-gestures.e2e.spec.js`.
