# Agent Guidelines for TurboPix

## Project Context

Development/personal project - breaking changes acceptable, database can be recreated

## Development Commands

**Backend:** `cargo run` | `cargo test` | `cargo clippy` | `cargo fmt`  
**Frontend:** `npm run lint` | `npm run format`

## Code Style

**Rust:**

- Iterator chains over loops: `.iter().filter_map().next()`
- Arrays over vecs: `[A, B]` vs `vec![A, B]`
- Error handling: `Result<T, E>` with `?`
- Imports: std, external crates, local (blank lines between)
- Zero warnings policy

**General:**

- KISS + SOLID principles

## Testing

**TDD/BDD:** Write tests first (GIVEN, WHEN, THEN style)

**Helpers:**

- `create_test_db_pool()`, `Photo::new_test_photo()` in `src/db.rs`
- Video tests need `RUN_VIDEO_TESTS=1`, ffmpeg/ffprobe

**E2E:**

- Start: `nohup cargo run &` + wait for `curl --retry 5 --retry-delay 2 http://localhost:18473/health`
- Use Playwright MCP (preferred) or Puppeteer (fallback) - don't install as dependency - don't create manual e2e tests
- Test at `http://localhost:18473`
- Use `[data-photo-id]` selectors
- Kill process after testing

## Common Issues

**Static files not updating:** `rm target/debug/turbo-pix && cargo build` (embedded via `include_str!()`)

**UI state desync:** Update both: `appState.value = x; domElement.value = x;`

**Video bugs:** Use `[data-photo-id]` selectors, test GET/HEAD requests, verify `mime_type` in DB

**Avoid:** Hardcoded paths and fallback logic mask bugs
