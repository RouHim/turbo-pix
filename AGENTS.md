# Agent Guidelines for TurboPix

## Build/Test/Lint Commands Backend

- `cargo run` - Start the application
- `cargo build` - Build the project
- `cargo test` - Run all tests
- `cargo test <test_name>` - Run specific test
- `cargo check` - Check for compilation errors without building
- `cargo clippy` - Run linter (Clippy) for code quality
- `cargo fmt` - Format code according to Rust standards

## Test/Lint/Format Commands Frontend

- `npm run lint` - Run linter (ESLint) for code quality
- `npm run format` - Format code using Prettier

## Test-Driven Development (TDD)

### Test Infrastructure

- **Helpers**: `create_test_db_pool()`, `Photo::new_test_photo()` in `src/db.rs`
- **Sample data**: `photos/sample_with_exif.jpg` for EXIF testing, `photos/test_video.mp4` for video processing tests
- **Video tests**: Require `RUN_VIDEO_TESTS=1` env var, `ffmpeg` and `ffprobe` installed, and sample video file present
- **Pattern**: Unit tests in `#[cfg(test)]` modules, integration in `tests/` dir

## E2E Testing with Browser Automation

### Test Setup

- Start application in the background using `nohup` and continue
- Wait until the app gets up by checking the health endpoint `curl --retry 5 --retry-delay 2 http://localhost:8080/health`
- **IMPORTANT**: Use available MCP servers for browser automation (check for `mcp__` prefixed tools)
  - Prefer Playwright MCP server if available (no additional dependencies)
  - Only use Puppeteer MCP as fallback (avoids adding puppeteer to package.json)
  - Do NOT install browser automation tools as project dependencies
- Navigate to `http://localhost:18473` for testing
- Finally don't forget to kill the app process

### Key E2E Test Scenarios

- **Photo Grid Loading**: Verify photos load and display correctly
- **Search Functionality**: Test search queries and result filtering
- **Photo Viewer**: Test image opening, navigation, and metadata display
- **Thumbnail Generation**: Verify thumbnails render properly
- **Responsive Design**: Test on different viewport sizes
- **API Endpoints**: Verify `/api/photos`, `/api/search`, `/thumbnails/*` responses

## Post-Feature Testing Protocol

**TDD-Enhanced Testing Workflow:**

1. **Start with failing tests** - Write tests for new feature before implementation
2. **Implement to pass tests** - Build minimal functionality to satisfy test requirements
3. **Start application** with `cargo run` for manual verification
4. **Run full test suite** - `cargo test` to ensure no regressions
5. **E2E validation** - Use Puppeteer to test complete user workflows
6. **Visual regression testing** - Take screenshots and verify UI changes
7. **Performance verification** - Check application still processes photos efficiently
8. **Browser console check** - Verify no JavaScript errors introduced

**Test-First Feature Implementation:**

- Define API contracts in tests before coding endpoints
- Test error conditions before implementing error handling
- Verify edge cases through unit tests before integration testing

## Code Style & Conventions

- **Modules**: Use single-file modules for simplicity (completed module flattening)
- **Imports**: Group std, external crates, then local modules with blank lines between
- **Structs**: Use PascalCase, derive Debug/Clone/Serialize/Deserialize as needed
- **Functions**: Use snake_case, make async where needed for web handlers
- **Error Handling**: Use `Result<T, E>` types, propagate errors with `?` operator
- **Dependencies**: Use specific versions in Cargo.toml, prefer stable crates
- **Config**: Load from environment variables with sensible defaults
- **Database**: Use connection pooling (r2d2), implement CRUD in separate modules
- **Logging**: Use `log` crate with `env_logger` for standard logging
- **JSON**: Use `serde_json::json!` macro for responses, consistent error format
- **Web handlers**: Return `Result<HttpResponse>`, use proper HTTP status codes
- **Clean Code**: KISS principle, avoid unnecessary complexity, comment non-obvious logic, remove dead code
- **SOLID principles**: Single Responsibility, Open/Closed, Liskov Substitution, Interface Segregation, Dependency Inversion

## Code Quality Standards

- **Zero warnings policy**: Maintain zero clippy warnings and compilation warnings
- **Test coverage**: All tests must pass before commits
- **Code review required**: Perform code review before committing
- **Utility preservation**: Use `#[allow(dead_code)]` for potentially useful functions rather than deletion

## Project Context & Development Guidelines

### Project Status

- **Non-production application**: TurboPix is a development/personal project, not a production system
- **No legacy compatibility requirements**: Since this isn't serving production users, we can make breaking changes freely
- **Database can be recreated**: No need to preserve existing data or maintain migration compatibility
- **Modern approach preferred**: Can use latest APIs and remove fallback mechanisms without concern

## Development Workflow & Common Pitfalls

### Static File Development

- **Incremental rebuild for static files**: `rm target/debug/turbo-pix && cargo build` after modifying `/static/` files
- **Full clean rebuild**: `cargo clean && cargo build` (slower, recompiles all dependencies)
- Rust embeds static files at compile-time using `include_str!()` macro - changes require recompilation
- Delete binary or incremental cache: `rm -rf target/debug/turbo-pix target/debug/incremental/turbo_pix-*`
- Use browser dev tools to verify latest changes are served
- **Common pitfall**: Modified static files won't be served until recompilation forces re-embedding
- **Why incremental doesn't detect changes**: Rust build system doesn't track files referenced by `include_str!()` macros

### JavaScript Debugging

- Access app objects via browser console (e.g., `window.turboPixApp`)
- Test both functional behavior and UI state synchronization
- **Common bug pattern**: Functions update app state but forget to sync UI elements
- **Example**: setSortBy() bug where photos reordered correctly but select dropdown value didn't update

### UI State Management

- Always sync DOM elements after app state changes
- **Pattern**: `appState.value = newValue; domElement.value = newValue;`
- **Fix pattern**: After state changes, explicitly update corresponding DOM elements
- Verify both functional behavior AND visual UI state changes in testing

## Debugging Guidelines & Common Issues

### Anti-Pattern: Development Code in Production Logic

- **Avoid**: Hardcoded test paths, fallback logic, or development shortcuts in production code paths
- **Key insight**: Remove all development convenience code before deployment - it often masks real bugs

### Video Playback Debugging

- **DOM structure**: Videos appear as photo cards in grid but play in `#viewer-video` element
- **Grid selector**: Use `[data-photo-id]` for targeting specific photos in grid, not `.photo-grid-item`
- **API testing**: Test both GET and HEAD requests - different error patterns may emerge
- **Browser state**: Refresh page between E2E tests to avoid caching/state interference
- **Database verification**: Check `mime_type` field consistency for media type detection

### E2E Testing Best Practices

- **Selector strategy**: Use data attributes (`[data-photo-id="176"]`) for reliable element targeting
- **API verification**: Test endpoints directly before browser automation testing
- **Response validation**: Check both status codes and content length for file serving
- **Screenshot documentation**: Capture working states for regression comparison
- **Process cleanup**: Always kill background processes after testing to avoid port conflicts

### Backend Bug Investigation Workflow

1. **API endpoint testing**: Verify raw API responses with curl/browser dev tools
2. **Database state verification**: Check data consistency and field values
3. **Hardcoded path detection**: Search for fallback logic that might override dynamic behavior
4. **Documentation**: Update AGENTS.md with lessons learned and debugging insights, keep it short and to the point

## Rust Code Style Preferences

- **Prefer iterator chains over for loops**: Use `.iter().filter_map().next()` instead of nested loops and conditionals
- **Use arrays over vecs for known sizes**: `[A, B, C]` instead of `vec![A, B, C]` (avoids heap allocation)

## CLIP Semantic Search Architecture

CLIP-based semantic search for natural language photo queries in 100+ languages.

**Model**: nllb-clip-base-siglip__v1 (~2GB, 512-dim embeddings, cosine threshold 0.7)
**Config**: `CLIP_ENABLE=true`, `CLIP_MODEL_PATH=./models/clip`
**API**: `GET /api/search/clip?q={query}` → text encoding → sqlite-vec similarity search
**Auto-indexing**: Embeddings generated during photo indexing (startup + midnight rescan), images only

**Key Files**:
- `clip_encoder.rs`: ONNX inference (384x384 visual, 77-token textual)
- `scheduler.rs`: Async embedding generation (tokio::spawn)
- `db.rs`: `save_embedding()`, `search_by_clip_embedding()` with sqlite-vec

**ONNX Specifics** (nllb-clip-base-siglip__v1):
- Visual: input=`"image"` `[1,3,384,384]`, output=`[0]` (not named)
- Textual: input=`"text"` `[1,77]` i32, output=`[0]` (not named)

**Limitations**: Hardcoded threshold (0.7), mutex bottleneck, no batching/caching