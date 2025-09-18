# Agent Guidelines for TurboPix

## Build/Test/Lint Commands

- `cargo run` - Start the application
- `cargo build` - Build the project
- `cargo test` - Run all tests
- `cargo test <test_name>` - Run specific test
- `cargo check` - Check for compilation errors without building
- `cargo clippy` - Run linter (Clippy) for code quality
- `cargo fmt` - Format code according to Rust standards

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

1. Start application with `cargo run`
2. Use Puppeteer to navigate to main page
3. Take screenshots for visual regression testing
4. Test new feature functionality end-to-end
5. Verify existing functionality still works
6. Check browser console for JavaScript errors

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

### Application Status

✅ **Application confirmed working** - TurboPix runs successfully with cleaned schema, processes 79 photos, server listening on 0.0.0.0:18473
