<p align="center">
  <h1 align="center">TurboPix</h1>
</p>

<p align="center">
    <a href="https://github.com/RouHim/turbo-pix/actions/workflows/ci.yml"><img src="https://github.com/RouHim/turbo-pix/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
    <a href="https://hub.docker.com/r/rouhim/turbo-pix"><img alt="Docker Pulls" src="https://img.shields.io/docker/pulls/rouhim/turbo-pix"></a>
    <a href="https://hub.docker.com/r/rouhim/turbo-pix/tags"><img alt="Docker Image Size (tag)" src="https://img.shields.io/docker/image-size/rouhim/turbo-pix/latest"></a>
    <a href="https://buymeacoffee.com/rouhim"><img alt="Donate me" src="https://img.shields.io/badge/-buy_me_a%C2%A0coffee-gray?logo=buy-me-a-coffee"></a>
</p>

<p align="center">
    <i>A blazingly fast, self-hosted photo gallery with smart organization, EXIF metadata extraction, and AI-powered features.</i>
</p>

## Motivation

After migrating from cloud photo services to self-hosted solutions, I wanted a photo gallery that combines the speed of modern web technologies with the power of Rust. TurboPix provides instant photo browsing, intelligent organization by date and camera, and semantic search capabilities - all while being completely self-hosted and privacy-focused.

## How it works

TurboPix scans your photo directories at startup, extracts comprehensive EXIF metadata, generates optimized thumbnails, and builds a searchable database. The metadata is stored in a structured JSON format for efficient querying. Photos are automatically organized by date, camera model, and can be searched semantically using AI-powered embeddings. The web interface is designed for speed, with lazy loading, BlurHash placeholders, and responsive image delivery.

## Features

- **Blazingly Fast**: Built with Rust and async I/O for instant photo browsing
- **Smart Thumbnails**: Automatically generates WebP and JPEG thumbnails in multiple sizes (200px, 400px, 800px)
- **Powerful Search**: Search by filename, camera make/model, or semantic content
- **Rich Metadata**: Comprehensive EXIF extraction including camera settings, GPS location, and video codec information
- **Modern UI**: Responsive design with BlurHash placeholders, dark mode, and gesture support
- **Video Support**: Playback and metadata extraction for common video formats
- **Favorites**: Mark and filter your favorite photos
- **GPS Location**: Display and search photos by location
- **AI-Powered**: Semantic search using local embeddings (optional)
- **Mobile-First**: Optimized for mobile devices with touch gestures and adaptive layouts
- **Privacy-Focused**: Completely self-hosted, no external services required
- **Easy Deployment**: Docker support with multi-architecture images

## Run the application

### Native execution

Download the latest release for your system from the [releases page](https://github.com/RouHim/turbo-pix/releases):

```shell
# Assuming you run a x86/x64 system, if not adjust the binary name to download
LATEST_VERSION=$(curl -L -s -H 'Accept: application/json' https://github.com/RouHim/turbo-pix/releases/latest | \
sed -e 's/.*"tag_name":"\([^"]*\)".*/\1/') && \
curl -L -o turbo-pix https://github.com/RouHim/turbo-pix/releases/download/$LATEST_VERSION/turbo-pix-x86_64-unknown-linux-musl && \
chmod +x turbo-pix
```

Create a folder to store the application data:

```shell
mkdir data
```

Start the application with:

```shell
TURBO_PIX_PHOTO_PATHS=/path/to/pictures \
TURBO_PIX_DATA_PATH=data \
./turbo-pix
```

> Since the binary is compiled [completely statically](https://github.com/rust-cross/rust-musl-cross), there are no dependencies on system libraries like glibc.

### Docker

Docker Example:

```shell
docker run -p 18473:18473 \
        -v /path/to/pictures:/photos:ro \
        -v ./data:/data \
        -e TURBO_PIX_PHOTO_PATHS=/photos \
        rouhim/turbo-pix
```

Docker compose example:

```yaml
services:
  turbo-pix:
    image: rouhim/turbo-pix
    volumes:
      - /path/to/pictures:/photos:ro  # mount read only
      - ./data:/data
    ports:
      - "18473:18473"
    environment:
      TURBO_PIX_PHOTO_PATHS: /photos
      RUST_LOG: info
```

## Configuration

All configuration is done via environment variables:

| Name                          | Description                                               | Default value | Required |
|-------------------------------|-----------------------------------------------------------|---------------|----------|
| `TURBO_PIX_PHOTO_PATHS`       | Comma-separated list of photo directories to scan         | `./photos`    | Yes      |
| `TURBO_PIX_DATA_PATH`         | Data directory for database, cache, and AI models        | `./data`      | No       |
| `TURBO_PIX_PORT`              | Port on which the application should listen              | `18473`       | No       |
| `RUST_LOG`                    | Log level (trace, debug, info, warn, error)              | `info`        | No       |

### Derived Paths

The following paths are automatically derived from `TURBO_PIX_DATA_PATH`:

- **Database**: `{DATA_PATH}/database/turbo-pix.db`
- **Thumbnails**: `{DATA_PATH}/cache/thumbnails/`
- **AI Models**: `{DATA_PATH}/models/`

### AI Models

TurboPix can use local AI models for semantic search. To download models:

```shell
./turbo-pix --download-models
```

Or with Docker:

```shell
docker run --rm -v ./data:/data rouhim/turbo-pix --download-models
```

## API Endpoints

TurboPix provides a RESTful API for programmatic access:

### Photo Management
- `GET /api/photos` - List photos with pagination, sorting, and filtering
  - Query params: `limit`, `offset`, `sort`, `order`, `q` (search), `year`, `month`
- `GET /api/photos/{hash}` - Get specific photo details
- `PUT /api/photos/{hash}` - Update photo (e.g., favorite status)
- `DELETE /api/photos/{hash}` - Delete photo

### Media Access
- `GET /api/photos/{hash}/file` - Serve photo file with caching
- `GET /api/photos/{hash}/thumbnail` - Serve optimized thumbnail
  - Query params: `size` (small/medium/large), `format` (webp/jpeg)
- `GET /api/photos/{hash}/video` - Serve video file with metadata

### Search & Discovery
- `GET /api/search` - Search photos by filename, camera, or metadata
- `GET /api/search/semantic` - AI-powered semantic search
- `GET /api/cameras` - List all camera makes and models
- `GET /api/timeline` - Get photo timeline density data

### System
- `GET /health` - Health check endpoint
- `GET /ready` - Readiness check with database validation
- `GET /api/stats` - Photo statistics (total, by type, by camera)

## Architecture

TurboPix is built with modern Rust technologies for maximum performance and reliability:

### Backend Stack
- **Web Framework**: [Warp 0.3](https://github.com/seanmonstar/warp) - Fast, composable async web framework
- **Database**: SQLite with [R2D2](https://github.com/sfackler/r2d2) connection pooling
- **Image Processing**: [image](https://github.com/image-rs/image) crate with EXIF/IPTC metadata extraction
- **Video Processing**: [ffmpeg](https://ffmpeg.org/) integration for video metadata and thumbnails
- **AI/ML**: [tract](https://github.com/sonos/tract) ONNX runtime for semantic search embeddings
- **Async Runtime**: [Tokio](https://tokio.rs/) for high-performance async I/O
- **Logging**: Standard logging with `log` and `env_logger`

### Frontend Stack
- **Pure Vanilla JavaScript**: No framework overhead, instant loading
- **CSS3**: Modern responsive design with CSS Grid and Flexbox
- **BlurHash**: Progressive image loading with low-quality placeholders
- **Progressive Enhancement**: Works without JavaScript for basic functionality
- **Internationalization**: Multi-language support (EN, DE)

### Data Model

Photos are stored with a clean separation between computational and informational metadata:

**Core Fields** (19 fields):
- Identification: `hash_sha256`, `file_path`, `filename`, `file_size`, `mime_type`
- Computational: `taken_at`, `width`, `height`, `orientation`, `duration`
- UI State: `thumbnail_path`, `has_thumbnail`, `blurhash`, `is_favorite`
- Metadata: JSON blob with nested structure
- System: `file_modified`, `date_indexed`, `created_at`, `updated_at`

**Metadata JSON Structure**:
```json
{
  "camera": {
    "make": "Canon",
    "model": "EOS R5",
    "lens_make": "Canon",
    "lens_model": "RF 24-70mm F2.8L IS USM"
  },
  "settings": {
    "iso": 1600,
    "aperture": 2.8,
    "shutter_speed": "1/250",
    "focal_length": 50.0,
    "exposure_mode": "manual",
    "metering_mode": "spot",
    "white_balance": "auto",
    "flash_used": false,
    "color_space": "sRGB"
  },
  "location": {
    "latitude": 52.5200,
    "longitude": 13.4050
  },
  "video": {
    "codec": "h264",
    "audio_codec": "aac",
    "bitrate": 5000,
    "frame_rate": 30.0
  }
}
```

## Performance

TurboPix is optimized for speed at every layer:

- **Fast Indexing**: Parallel photo processing with rayon
- **Efficient Queries**: SQLite with JSON extraction and proper indexing
- **Optimized Thumbnails**: Pre-generated WebP/JPEG in multiple sizes
- **Lazy Loading**: Only load images when they enter the viewport
- **HTTP Caching**: Aggressive caching headers for static assets
- **BlurHash Placeholders**: Instant visual feedback while loading

Tested on various hardware from Raspberry Pi to desktop workstations with collections ranging from hundreds to tens of thousands of photos.

## Limitations

- **HEIC Support**: Limited due to [image-rs issue #1375](https://github.com/image-rs/image/issues/1375)
- **RAW Formats**: Basic support; some proprietary formats may not be fully supported
- **Live Photos**: iOS Live Photos are treated as separate image and video files

## Development

### Prerequisites

```bash
# Install Rust (1.75+)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install dependencies (Linux)
sudo apt-get update && sudo apt-get install -y \
  libsqlite3-dev \
  pkg-config \
  build-essential \
  ffmpeg

# Install Node.js for frontend development
curl -fsSL https://deb.nodesource.com/setup_21.x | sudo -E bash -
sudo apt-get install -y nodejs
```

### Build and Run

```bash
# Clone repository
git clone https://github.com/RouHim/turbo-pix.git
cd turbo-pix

# Install npm dependencies
npm install

# Build and run
cargo run

# Run tests
cargo test

# Run linters
cargo clippy
npm run lint

# Format code
cargo fmt
npm run format
```

### Project Structure

```
turbo-pix/
├── src/
│   ├── main.rs              # Application entry point
│   ├── db.rs                # Database layer with Photo entity
│   ├── db_schema.rs         # Database schema definitions
│   ├── handlers_*.rs        # HTTP request handlers
│   ├── indexer.rs           # Photo indexing and processing
│   ├── metadata_extractor.rs # EXIF/video metadata extraction
│   ├── scheduler.rs         # Background tasks
│   ├── semantic_search.rs   # AI-powered search
│   └── photo_processor.rs   # Thumbnail generation
├── static/
│   ├── js/                  # Frontend JavaScript
│   ├── css/                 # Styles
│   └── index.html          # Main HTML
├── container-data/
│   └── Containerfile       # Docker build
└── test-data/              # Test photos and videos
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

See [LICENSE](LICENSE) file for details.

## Support

If you find TurboPix useful, consider [buying me a coffee](https://buymeacoffee.com/rouhim)
