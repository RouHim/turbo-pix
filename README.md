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

TurboPix uses sensible defaults and requires minimal configuration:

| Environment Variable     | Default    | Description                               |
| ------------------------ | ---------- | ----------------------------------------- |
| `TURBO_PIX_PHOTO_PATHS`  | `./photos` | Comma-separated list of photo directories |
| `TURBO_PIX_DATA_PATH`    | `./data`   | Data directory for database and cache     |
| `TURBO_PIX_PORT`         | `18473`    | Server port                               |
| `RUST_LOG`               | `info`     | Log level (trace, debug, info, warn)      |

**Derived Paths from DATA_PATH:**
- Database: `{DATA_PATH}/database/turbo-pix.db`
- Thumbnails: `{DATA_PATH}/cache/thumbnails`

**Built-in Defaults:**
- Server binds to `127.0.0.1` (localhost only)
- Thumbnail sizes: 200px, 400px, 800px

## Architecture

TurboPix is built with modern Rust technologies for maximum performance:

- **Web Framework**: [Warp 0.4.2](https://github.com/seanmonstar/warp) - Fast, composable web framework
- **Database**: SQLite with R2D2 connection pooling
- **Image Processing**: Rust `image` crate with EXIF metadata extraction
- **Async Runtime**: Tokio for high-performance async I/O
- **Logging**: Standard logging with `log` and `env_logger`

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
