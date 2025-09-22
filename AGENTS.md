# Agent Guidelines for TurboPix

## Build/Test/Lint Commands

- `cargo run` - Start the application
- `cargo build` - Build the project
- `cargo test` - Run all tests
- `cargo test <test_name>` - Run specific test
- `cargo check` - Check for compilation errors without building
- `cargo clippy` - Run linter (Clippy) for code quality
- `cargo fmt` - Format code according to Rust standards

## Test-Driven Development (TDD)

### TDD Cycle

1. **Write failing test** → 2. **Implement minimal code** → 3. **Refactor** → 4. **Run full suite**

### TDD Commands

```bash
cargo test <test_name> --lib  # Run specific test
cargo test                   # Full test suite (target: 77+ passing)
```

### Test Infrastructure

- **Helpers**: `create_test_db_pool()`, `Photo::new_test_photo()` in `src/db.rs`
- **Sample data**: `photos/sample_with_exif.jpg` for EXIF testing
- **Pattern**: Unit tests in `#[cfg(test)]` modules, integration in `tests/` dir

## E2E Testing with Puppeteer

### Test Setup

- Start application with `cargo run` (default port: 18473)
- Use Puppeteer MCP server for browser automation
- Navigate to `http://localhost:18473` for testing

### Key E2E Test Scenarios

- **Photo Grid Loading**: Verify photos load and display correctly
- **Search Functionality**: Test search queries and result filtering
- **Photo Viewer**: Test image opening, navigation, and metadata display
- **Thumbnail Generation**: Verify thumbnails render properly
- **Responsive Design**: Test on different viewport sizes
- **API Endpoints**: Verify `/api/photos`, `/api/search`, `/thumbnails/*` responses

### Post-Feature Testing Protocol

**TDD-Enhanced Testing Workflow:**

1. **Start with failing tests** - Write tests for new feature before implementation
2. **Implement to pass tests** - Build minimal functionality to satisfy test requirements
3. **Start application** with `cargo run` for manual verification
4. **Run full test suite** - `cargo test` to ensure no regressions (target: 77+ tests passing)
5. **E2E validation** - Use Puppeteer to test complete user workflows
6. **Visual regression testing** - Take screenshots and verify UI changes
7. **Performance verification** - Check application still processes photos efficiently
8. **Browser console check** - Verify no JavaScript errors introduced

**Test-First Feature Implementation:**

- Define API contracts in tests before coding endpoints
- Test error conditions before implementing error handling
- Verify edge cases through unit tests before integration testing

## Code Style & Conventions

- **Modules**: Use `mod.rs` files for module organization
- **Imports**: Group std, external crates, then local modules with blank lines between
- **Structs**: Use PascalCase, derive Debug/Clone/Serialize/Deserialize as needed
- **Functions**: Use snake_case, make async where needed for web handlers
- **Error Handling**: Use `Result<T, E>` types, propagate errors with `?` operator
- **Dependencies**: Use specific versions in Cargo.toml, prefer stable crates
- **Config**: Load from environment variables with sensible defaults
- **Database**: Use connection pooling (r2d2), implement CRUD in separate modules
- **Logging**: Use `tracing` crate for structured logging
- **JSON**: Use `serde_json::json!` macro for responses, consistent error format
- **Web handlers**: Return `Result<HttpResponse>`, use proper HTTP status codes

## Project Context & Development Guidelines

### Project Status

- **Non-production application**: TurboPix is a development/personal project, not a production system
- **No legacy compatibility requirements**: Since this isn't serving production users, we can make breaking changes freely
- **Database can be recreated**: No need to preserve existing data or maintain migration compatibility
- **Modern approach preferred**: Can use latest APIs and remove fallback mechanisms without concern

### Development Philosophy

- **Clean, simple code over backwards compatibility**
- **Remove unused/legacy code aggressively**
- **Use modern APIs without fallbacks** (e.g., navigator.share() only)
- **Direct schema initialization over complex migration systems**
- **Focus on code clarity and maintainability**

## Project Status & Cleanup History

### Legacy/Compatibility Code Removal (Completed)

- **Database migrations removed**: Replaced migration system with direct schema initialization
- **Compatibility `city` field removed**: Eliminated redundant column from database schema, Photo struct, SearchQuery struct, and all SQL operations
- **JavaScript modernization**: Removed fallback clipboard mechanisms, using modern `navigator.share()` API only
- **Code comments cleaned**: Removed all migration-related and compatibility comments

### Current Issues

- **One remaining warning**: Unused `db_pool` field in `ThumbnailService` struct (src/cache/thumbnails.rs)

### Next Steps Identified

- Fix unused field warning in ThumbnailService
- Consider module flattening: Convert directories to single files (indexer/, scheduler/, web/handlers/)
- Test application functionality after restructuring

## Development Workflow & Common Pitfalls

### Static File Development

- **Always run `cargo clean && cargo build`** after modifying files in `/static/`
- Rust embeds static files at compile-time using `include_str!()` macro - changes require recompilation
- Use browser dev tools to verify latest changes are served
- **Common pitfall**: Modified static files won't be served until recompilation forces re-embedding

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
