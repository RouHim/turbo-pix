# TurboPix

A fast photo gallery that automatically organizes and displays your photos with smart thumbnails and search. Built with Rust and Warp for high performance.

## Features

- 🚀 **Fast Browsing**: Browse thousands of photos quickly
- 🖼️ **Smart Thumbnails**: Automatically creates thumbnails in multiple sizes
- 🔍 **Photo Search**: Find photos by name, date, or camera details
- 📱 **Works Everywhere**: Runs on desktop, tablet, and mobile
- 🏗️ **Easy Setup**: Run with Docker or install locally
- 📊 **Reliable**: Built-in health monitoring and performance tracking

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
   Visit http://localhost:18473

### Docker

```bash
# Build and run with Docker
docker-compose up --build

# Or manually:
docker build -t turbo-pix .
docker run -p 18473:18473 \
  -v /path/to/photos:/photos \
  -v ./data:/app/data \
  -e TURBO_PIX_PHOTO_PATHS=/photos \
  turbo-pix
```

## Configuration

| Environment Variable                 | Default               | Description                                 |
| ------------------------------------ | --------------------- | ------------------------------------------- |
| `TURBO_PIX_PHOTO_PATHS`              | `./photos`            | Comma-separated list of photo directories   |
| `TURBO_PIX_DB_PATH`                  | `./data/turbo-pix.db` | SQLite database path                        |
| `TURBO_PIX_PORT`                     | `18473`               | Server port                                 |
| `TURBO_PIX_HOST`                     | `0.0.0.0`             | Server host address                         |
| `TURBO_PIX_CACHE_PATH`               | `./data/cache`        | Cache directory path                        |
| `TURBO_PIX_THUMBNAIL_SIZES`          | `200,400,800`         | Comma-separated thumbnail sizes             |
| `TURBO_PIX_WORKERS`                  | `4`                   | Number of worker threads                    |
| `TURBO_PIX_MAX_CONNECTIONS`          | `100`                 | Maximum database connections                |
| `TURBO_PIX_CACHE_SIZE_MB`            | `512`                 | LRU cache size in MB                        |
| `TURBO_PIX_MEMORY_CACHE_SIZE`        | `1000`                | In-memory cache item count                  |
| `TURBO_PIX_MEMORY_CACHE_MAX_SIZE_MB` | `100`                 | In-memory cache size limit                  |
| `TURBO_PIX_SCAN_INTERVAL`            | `3600`                | Directory scan interval in seconds          |
| `TURBO_PIX_BATCH_SIZE`               | `1000`                | Processing batch size                       |
| `TURBO_PIX_METRICS_ENABLED`          | `true`                | Enable metrics collection                   |
| `TURBO_PIX_HEALTH_CHECK_PATH`        | `/health`             | Health check endpoint path                  |
| `RUST_LOG`                           | `info`                | Log level (trace, debug, info, warn, error) |

## Architecture

TurboPix is built with modern Rust technologies for maximum performance:

- **Web Framework**: [Warp 0.4.2](https://github.com/seanmonstar/warp) - Fast, composable web framework
- **Database**: SQLite with R2D2 connection pooling
- **Image Processing**: Rust `image` crate with EXIF metadata extraction
- **Async Runtime**: Tokio for high-performance async I/O
- **Logging**: Structured logging with `tracing`

### API Endpoints

- `GET /health` - Health check
- `GET /ready` - Readiness check
- `GET /api/photos` - List photos with pagination and search
- `GET /api/photos/{id}` - Get specific photo details
- `GET /api/photos/{id}/file` - Serve photo file
- `GET /api/photos/{id}/video` - Serve video file with metadata
- `GET /api/photos/{id}/metadata` - Get photo metadata only
- `PUT /api/photos/{id}` - Update photo (favorite status, etc.)
- `DELETE /api/photos/{id}` - Delete photo
- `GET /api/search` - Search photos
- `GET /api/search/suggestions` - Get search suggestions
- `GET /api/cameras` - List camera makes and models
- `GET /api/stats` - Get photo statistics
