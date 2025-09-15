# Agent Guidelines for TurboPix

## Build/Test/Lint Commands
- `cargo run` - Start the application
- `cargo build` - Build the project
- `cargo test` - Run all tests
- `cargo test <test_name>` - Run specific test
- `cargo check` - Check for compilation errors without building
- `cargo clippy` - Run linter (Clippy) for code quality
- `cargo fmt` - Format code according to Rust standards

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