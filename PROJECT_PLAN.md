# TurboPix - Fast Rust Photo Gallery

A blazing-fast, container-first photo gallery application inspired by Google Photos, built with Rust and vanilla JavaScript.

## 🎯 Project Goals

- **Performance First**: Lightning-fast photo browsing and indexing
- **Container Native**: CNCF-compliant, Kubernetes-ready
- **Existing Photo Support**: Works with existing photo collections
- **Google Photos UX**: Modern, responsive web interface
- **Minimal Dependencies**: Simple, maintainable codebase

## 🏗️ Architecture Overview

### Backend Stack
- **Web Framework**: Actix Web (high-performance HTTP server)
- **Database**: rusqlite with r2d2 connection pooling
- **Scheduling**: clockwerk for background tasks
- **Threading**: Pure Rust std::thread (no async complexity)
- **Image Processing**: image crate for thumbnails
- **EXIF Parsing**: kamadak-exif for metadata extraction

### Frontend Stack
- **JavaScript**: Vanilla ES6+ (no frameworks)
- **HTML**: Semantic HTML5
- **CSS**: CSS3 Grid/Flexbox, CSS Variables
- **Caching**: Browser native caching with HTTP headers

## 📦 Dependencies (Cargo.toml)

```toml
[package]
name = "turbo-pix"
version = "0.1.0"
edition = "2021"

[dependencies]
# Web framework
actix-web = "4"
actix-files = "0.6"

# Database
rusqlite = { version = "0.37", features = ["bundled"] }
r2d2 = "0.8.10"
r2d2_sqlite = "0.31"

# Scheduling
clockwerk = "0.4"

# Image processing & MIME
image = "0.24"
kamadak-exif = "0.5"
mime_guess = "2.0.5"

# Serialization
serde = { version = "1.0", features = ["derive"] }

# Utilities
lru = "0.12"
walkdir = "2"
md5 = "0.7"
chrono = { version = "0.4", features = ["serde"] }

# Logging (structured)
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json"] }
```

## 📁 Project Structure

```
turbo-pix/
├── Cargo.toml
├── PROJECT_PLAN.md
├── README.md
├── Containerfile
├── src/
│   ├── main.rs
│   ├── config.rs           # Environment configuration
│   ├── db/
│   │   ├── mod.rs
│   │   ├── connection.rs   # r2d2 pool setup
│   │   ├── models.rs       # Photo structs
│   │   ├── schema.rs       # SQL schemas
│   │   └── migrations.rs   # Migration runner
│   ├── indexer/
│   │   ├── mod.rs
│   │   ├── scanner.rs      # File system walker
│   │   ├── metadata.rs     # EXIF extraction
│   │   └── processor.rs    # Photo processing pipeline
│   ├── cache/
│   │   ├── mod.rs
│   │   ├── thumbnails.rs   # Thumbnail generation
│   │   └── memory.rs       # LRU cache
│   ├── web/
│   │   ├── mod.rs
│   │   ├── handlers/       # Route handlers
│   │   │   ├── mod.rs
│   │   │   ├── photos.rs   # Photo CRUD
│   │   │   └── search.rs   # Search endpoints
│   │   ├── routes.rs       # Route configuration
│   │   └── middleware.rs   # Logging, CORS, etc.
│   ├── scheduler/
│   │   ├── mod.rs
│   │   └── tasks.rs        # Background jobs
│   └── utils/
│       ├── mod.rs
│       └── image.rs        # Image utilities
├── static/                 # Web assets
│   ├── index.html          # Main SPA entry point
│   ├── css/
│   │   ├── main.css       # Base styles & CSS Grid
│   │   ├── components.css # Photo grid, viewer, modals
│   │   └── responsive.css # Mobile-first responsive
│   ├── js/
│   │   ├── app.js         # Main application controller
│   │   ├── api.js         # REST API client
│   │   ├── photoGrid.js   # Photo grid with lazy loading
│   │   ├── viewer.js      # Full-screen photo viewer
│   │   ├── search.js      # Search & filtering
│   │   └── utils.js       # Helper functions
│   └── icons/
│       └── *.svg          # SVG icons
└── migrations/            # SQL migration files
```

## 🗄️ Database Schema

```sql
-- Core photo metadata
CREATE TABLE photos (
    id INTEGER PRIMARY KEY,
    path TEXT UNIQUE NOT NULL,
    filename TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    mime_type TEXT NOT NULL,
    date_taken DATETIME,
    date_modified DATETIME NOT NULL,
    date_indexed DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    width INTEGER,
    height INTEGER,
    orientation INTEGER DEFAULT 1,
    camera_make TEXT,
    camera_model TEXT,
    iso INTEGER,
    aperture REAL,
    shutter_speed TEXT,
    focal_length REAL,
    gps_latitude REAL,
    gps_longitude REAL,
    location_name TEXT,
    hash_md5 TEXT UNIQUE,
    thumbnail_path TEXT,
    has_thumbnail BOOLEAN DEFAULT FALSE
);

-- Performance indexes
CREATE INDEX idx_photos_date_taken ON photos(date_taken);
CREATE INDEX idx_photos_path ON photos(path);
CREATE INDEX idx_photos_hash ON photos(hash_md5);
CREATE INDEX idx_photos_location ON photos(gps_latitude, gps_longitude);
```

## ⚙️ Configuration (Environment Variables)

```bash
# Core Settings
TURBO_PIX_PORT=8080
TURBO_PIX_HOST=0.0.0.0

# Photo Directories (comma-separated)
TURBO_PIX_PHOTO_PATHS=/photos,/archive

# Database & Cache
TURBO_PIX_DB_PATH=/data/turbo-pix.db
TURBO_PIX_CACHE_PATH=/data/cache
TURBO_PIX_THUMBNAIL_SIZES=200,400,800

# Performance
TURBO_PIX_WORKERS=4
TURBO_PIX_MAX_CONNECTIONS=100
TURBO_PIX_CACHE_SIZE_MB=512

# Indexing
TURBO_PIX_SCAN_INTERVAL=3600
TURBO_PIX_BATCH_SIZE=1000

# Observability
RUST_LOG=info
TURBO_PIX_METRICS_ENABLED=true
TURBO_PIX_HEALTH_CHECK_PATH=/health
```

## 🚀 Container Strategy

### Dockerfile
```dockerfile
FROM rust:1.75-alpine AS builder
WORKDIR /app
COPY . .
RUN apk add --no-cache musl-dev
RUN cargo build --release

FROM alpine:3.18
RUN apk add --no-cache ca-certificates
COPY --from=builder /app/target/release/turbo-pix /usr/local/bin/
COPY --from=builder /app/static /usr/local/share/turbo-pix/static
EXPOSE 8080
USER 1000:1000
ENTRYPOINT ["turbo-pix"]
```

### CNCF Compliance
- ✅ **12-Factor App**: All config via environment variables
- ✅ **Health Checks**: `/health` and `/ready` endpoints
- ✅ **Metrics**: Prometheus-compatible `/metrics` endpoint
- ✅ **Structured Logging**: JSON logs for observability
- ✅ **Graceful Shutdown**: SIGTERM handling
- ✅ **Stateless**: All state in mounted volumes
- ✅ **Security**: Non-root user, minimal attack surface

## 🎨 Frontend Features

### Google Photos-inspired UI
- **Responsive Grid**: CSS Grid masonry layout
- **Infinite Scroll**: Lazy loading with Intersection Observer
- **Full-screen Viewer**: Slideshow mode with navigation
- **Search & Filter**: By date, location, camera metadata
- **Touch Gestures**: Swipe navigation for mobile
- **Keyboard Navigation**: Arrow keys, space, escape

### Performance Optimizations
- **Virtual Scrolling**: Efficient rendering of large collections
- **Thumbnail Preloading**: Background image loading
- **HTTP Caching**: Browser-native image caching
- **Progressive Enhancement**: Works without JavaScript

## 📋 Implementation Phases

### Phase 1 - Foundation (Days 1-3) ✅ COMPLETE
- [x] Project setup with Cargo.toml
- [x] Database schema & r2d2 pool setup
- [x] Basic Actix Web server with static file serving
- [x] Photo model and basic CRUD operations
- [x] Environment configuration system

### Phase 2 - Indexing (Days 4-6) ✅ COMPLETE
- [x] File system scanner with walkdir
- [x] EXIF metadata extraction with kamadak-exif
- [x] MD5 hash calculation for duplicate detection
- [x] clockwerk scheduler for periodic scans
- [x] Migration system for database updates

### Phase 3 - Thumbnails & Cache (Days 7-9)
- [x] Multi-size thumbnail generation (200px, 400px, 800px)
- [x] LRU memory cache implementation
- [x] Background thumbnail processing with std::thread
- [x] Thumbnail serving via Actix Web handlers
- [x] Cache cleanup and management

### Phase 4 - REST API (Days 10-12) ✅ COMPLETE
- [x] Photos API endpoints (GET, POST, PUT, DELETE)
- [x] Search API with filtering by date, metadata
- [x] Pagination and sorting support
- [x] Error handling and validation

### Phase 5 - Frontend (Days 13-17)
- [ ] HTML5 base template with responsive design
- [ ] CSS Grid photo layout with masonry style
- [ ] Vanilla JS photo grid with lazy loading
- [ ] Full-screen photo viewer with navigation
- [ ] Search interface with live filtering
- [ ] Mobile touch gestures and responsive design

### Phase 6 - Observability (Days 18-20)
- [ ] Health check endpoints (/health, /ready)
- [ ] Prometheus metrics collection
- [ ] Structured JSON logging with tracing
- [ ] Graceful shutdown handling
- [ ] Performance monitoring

### Phase 7 - Container & Deployment (Days 21-22)
- [ ] Multi-stage Dockerfile optimization
- [ ] Kubernetes deployment manifests
- [ ] Docker Compose for local development
- [ ] CI/CD pipeline setup
- [ ] Documentation and README

## 🔧 Development Commands

```bash
# Development
cargo run
cargo test
cargo clippy
cargo fmt

# Container
docker build -t turbo-pix .
docker run -p 8080:8080 -v /photos:/photos turbo-pix

# Production
cargo build --release
./target/release/turbo-pix
```

## 📊 Performance Targets

Based on reference project benchmarks:

### Indexing Performance
- **80k photos**: ~6 seconds (SSD storage)
- **6k photos**: ~38 seconds (SD card)
- **Memory usage**: < 512MB during indexing

### Runtime Performance
- **Photo grid loading**: < 1 second
- **Thumbnail generation**: < 100ms per image
- **Search response**: < 50ms
- **Memory usage**: < 256MB runtime

## 🎯 Success Criteria

- [ ] Indexes existing photo collections without modification
- [ ] Google Photos-like user experience
- [ ] Container-native deployment (Kubernetes ready)
- [ ] Sub-second photo browsing performance
- [ ] Mobile-responsive design
- [ ] CNCF compliance (health checks, metrics, logging)
- [ ] Minimal resource usage (< 256MB RAM runtime)

---

*This document serves as the living specification for TurboPix development. Update as implementation progresses.*
