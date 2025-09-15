# TurboPix

A fast, lightweight photo gallery built with Rust and Actix-web. Features automatic thumbnail generation, full-text search, and a responsive web interface.

## Features

- 🚀 **Fast**: Built with Rust for optimal performance
- 🖼️ **Smart Thumbnails**: Automatic generation with LRU caching
- 🔍 **Full-Text Search**: Search photos by filename, metadata, and EXIF data
- 📱 **Responsive**: Mobile-friendly web interface
- 🏗️ **Container-Ready**: Docker and Kubernetes support
- 📊 **Observability**: Health checks, metrics, and structured logging
- 🔒 **Secure**: Non-root container execution, minimal attack surface

## Quick Start

### Local Development

1. **Prerequisites**
   ```bash
   # Install Rust (1.75+)
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

   # Install dependencies (Linux)
   sudo apt-get update && sudo apt-get install -y \
     libsqlite3-dev \
     pkg-config \
     build-essential
   ```

2. **Clone and Run**
   ```bash
   git clone <repository-url>
   cd turbo-pix

   # Set photo directory and run
   TURBO_PIX_PHOTO_PATHS=/path/to/your/photos cargo run
   ```

3. **Open Browser**
   Visit http://localhost:8080

### Docker

```bash
# Build and run with Docker
docker-compose up --build

# Or manually:
docker build -t turbo-pix .
docker run -p 8080:8080 \
  -v /path/to/photos:/photos \
  -v ./data:/app/data \
  -e TURBO_PIX_PHOTO_PATHS=/photos \
  turbo-pix
```

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `TURBO_PIX_PHOTO_PATHS` | `./photos` | Comma-separated list of photo directories |
| `DATABASE_URL` | `./data/turbo-pix.db` | SQLite database path |
| `RUST_LOG` | `info` | Log level (trace, debug, info, warn, error) |
| `THUMBNAIL_SIZE` | `300` | Thumbnail size in pixels |
| `CACHE_SIZE_MB` | `100` | LRU cache size in MB |
| `PORT` | `8080` | Server port |

## API Endpoints

### Photos
- `GET /api/photos` - List photos with pagination
- `GET /api/photos/:id` - Get photo details
- `GET /api/photos/:id/thumbnail` - Get photo thumbnail

### Search
- `GET /api/search?q=query` - Search photos

### Health & Monitoring
- `GET /health` - Health check
- `GET /ready` - Readiness check
- `GET /metrics` - Prometheus metrics

### Static Files
- `GET /` - Main application
- `GET /static/*` - Static assets (CSS, JS)

## Development

### Build Commands
```bash
# Development build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Run linter
cargo clippy

# Format code
cargo fmt
```

### Project Structure
```
src/
├── main.rs              # Application entry point
├── config.rs            # Configuration management
├── lib.rs               # Library exports
├── db/                  # Database layer
│   ├── models.rs        # Data models
│   ├── schema.rs        # Database schema
│   ├── crud.rs          # CRUD operations
│   └── connection.rs    # Database connection
├── indexer/             # Photo indexing
│   ├── scanner.rs       # File system scanner
│   ├── processor.rs     # Metadata processor
│   └── metadata.rs      # EXIF/metadata extraction
├── cache/               # Caching layer
│   ├── memory.rs        # In-memory LRU cache
│   └── thumbnails.rs    # Thumbnail generation
├── scheduler/           # Background tasks
│   └── tasks.rs         # Task scheduler
└── web/                 # Web layer
    ├── handlers/        # HTTP handlers
    ├── routes.rs        # Route definitions
    └── middleware.rs    # Custom middleware
```

## Deployment

### Docker Compose (Recommended)
```yaml
version: '3.8'
services:
  turbopix:
    image: turbopix:latest
    ports:
      - "8080:8080"
    volumes:
      - ./data:/app/data
      - /path/to/photos:/photos:ro
    environment:
      - TURBO_PIX_PHOTO_PATHS=/photos
      - RUST_LOG=info
    restart: unless-stopped
```

## Monitoring

### Health Checks
- `/health` - Basic health check
- `/ready` - Readiness check (includes database connectivity)
- `/metrics` - Prometheus-compatible metrics

### Logging
Structured JSON logging with configurable levels. Logs include:
- Request/response details
- Error tracking with stack traces
- Performance metrics
- User interactions

## Performance

- **Indexing**: ~1000 photos/minute
- **Thumbnail Generation**: <1 second per photo
- **Search**: <100ms response time
- **Memory Usage**: ~50MB base + cache
- **Container Size**: ~50MB (Alpine Linux)

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Run tests: `cargo test`
5. Check formatting: `cargo fmt --check`
6. Run linter: `cargo clippy`
7. Submit a pull request

## License

MIT License - see LICENSE file for details