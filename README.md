# TurboPix

A fast photo gallery that automatically organizes and displays your photos with smart thumbnails and search. Built with Rust and Warp for high performance.

## Features

- 🚀 **Fast Browsing**: Browse thousands of photos quickly
- 🖼️ **Smart Thumbnails**: Automatically creates thumbnails in multiple sizes
- 🔍 **Semantic Search**: Find photos using natural language (AI-powered CLIP)
- 🌍 **Multilingual**: Search in 100+ languages (English, German, Spanish, etc.)
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

| Environment Variable          | Default        | Description                               |
| ----------------------------- | -------------- | ----------------------------------------- |
| `TURBO_PIX_PHOTO_PATHS`       | `./photos`     | Comma-separated list of photo directories |
| `TURBO_PIX_DATA_PATH`         | `./data`       | Data directory for database and cache     |
| `TURBO_PIX_PORT`              | `18473`        | Server port                               |
| `TURBO_PIX_MAX_CACHE_SIZE_MB` | `1024`         | Maximum thumbnail cache size in MB        |
| `CLIP_ENABLE`                 | `false`        | Enable CLIP semantic search               |
| `CLIP_MODEL_PATH`             | `./models/clip`| Path to CLIP model files                  |
| `RUST_LOG`                    | `info`         | Log level (trace, debug, info, warn)      |

**Derived Paths from DATA_PATH:**
- Database: `{DATA_PATH}/database/turbo-pix.db`
- Thumbnails: `{DATA_PATH}/cache/thumbnails`

**Built-in Defaults:**
- Server binds to `0.0.0.0` (all interfaces)
- Thumbnail sizes: 200px, 400px, 800px

### CLIP Semantic Search Setup

CLIP enables searching photos using natural language in 100+ languages.

1. **Download CLIP models** (~600MB):
   ```bash
   bash scripts/download_clip_models.sh
   ```

2. **Enable CLIP**:
   ```bash
   export CLIP_ENABLE=true
   export CLIP_MODEL_PATH=./models/clip
   ```

3. **Search examples**:
   - `cat` - Find photos of cats
   - `Katze` - Same in German
   - `sunset beach` - Find beach sunset photos
   - `birthday party` - Find party photos

**Note**: First run will generate embeddings for all photos (may take time for large libraries).

## Architecture

TurboPix is built with modern Rust technologies for maximum performance:

- **Web Framework**: [Warp 0.4.2](https://github.com/seanmonstar/warp) - Fast, composable web framework
- **Database**: SQLite with R2D2 connection pooling
- **Vector Search**: [sqlite-vec](https://github.com/asg017/sqlite-vec) - Fast vector similarity search
- **ML Inference**: [ONNX Runtime](https://onnxruntime.ai/) - Cross-platform AI model inference
- **CLIP Model**: Multilingual CLIP (nllb-clip-base-siglip__v1) for semantic search
- **Image Processing**: Rust `image` crate with EXIF metadata extraction
- **Async Runtime**: Tokio for high-performance async I/O
- **Logging**: Standard logging with `log` and `env_logger`

### API Endpoints

**Health & Monitoring:**
- `GET /health` - Health check
- `GET /ready` - Readiness check with database connection test
- `GET /api/stats` - Photo library statistics

**Photos:**
- `GET /api/photos` - List photos with pagination
- `GET /api/photos/{hash}` - Get photo details
- `GET /api/photos/{hash}/file` - Serve original photo file
- `GET /api/photos/{hash}/thumbnail?size={size}` - Get thumbnail (small/medium/large)
- `GET /api/photos/{hash}/exif` - Get EXIF metadata
- `PUT /api/photos/{hash}/favorite` - Toggle favorite status
- `GET /api/photos/timeline` - Get photos grouped by date

**Video:**
- `GET /api/photos/{hash}/video` - Serve video file with range support

**Search:**
- `GET /api/search/clip?q={query}` - CLIP semantic search (natural language)

**Thumbnails:**
- `GET /api/thumbnails/hash/{hash}/{size}` - Direct thumbnail access
