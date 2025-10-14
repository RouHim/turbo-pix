<p align="center">
  <img src="https://raw.githubusercontent.com/RouHim/turbo-pix/main/.github/readme/banner.svg" width="600">
</p>

<p align="center">
    <a href="https://github.com/RouHim/turbo-pix/actions/workflows/ci.yml"><img src="https://github.com/RouHim/turbo-pix/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
    <a href="https://hub.docker.com/r/rouhim/turbo-pix"><img alt="Docker Pulls" src="https://img.shields.io/docker/pulls/rouhim/turbo-pix"></a>
    <a href="https://hub.docker.com/r/rouhim/turbo-pix/tags"><img alt="Docker Image Size (tag)" src="https://img.shields.io/docker/image-size/rouhim/turbo-pix/latest"></a>
    <a href="https://buymeacoffee.com/rouhim"><img alt="Donate me" src="https://img.shields.io/badge/-buy_me_a%C2%A0coffee-gray?logo=buy-me-a-coffee"></a>
</p>

<p align="center">
    <i>A self-hosted photo gallery with metadata extraction and search capabilities.</i>
</p>

## Motivation

After migrating from cloud photo services to self-hosted solutions, I wanted a simple photo gallery that works locally without depending on external services. TurboPix organizes photos by date and camera, lets you search through them, and keeps everything on your own hardware.

## How it works

TurboPix scans your photo directories at startup, reads metadata from your photos, creates thumbnails, and stores everything in a local database. You can then browse photos by date, search for specific cameras or filenames, and view detailed information about each photo.

## Features

- **Photo Browsing**: View your photo collection organized by date
- **Thumbnails**: Automatically creates smaller preview images
- **Search**: Find photos by filename or camera model
- **Metadata Display**: View camera settings, date taken, and GPS coordinates
- **Dark Mode**: Switch between light and dark themes
- **Video Playback**: View videos alongside your photos
- **Favorites**: Mark photos to find them later
- **Location View**: See where your photos were taken (if GPS data is available)
- **Semantic Search**: Find photos by describing what's in them (optional, requires AI models)
- **Mobile Support**: Works on phones and tablets
- **Self-Hosted**: Runs on your own computer, no cloud services
- **Docker Support**: Can be deployed as a container

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

TurboPix is built with Rust and uses the following libraries:

### Backend
- **Web Framework**: [Warp 0.3](https://github.com/seanmonstar/warp)
- **Database**: SQLite with [R2D2](https://github.com/sfackler/r2d2) connection pooling
- **Image Processing**: [image](https://github.com/image-rs/image) crate for EXIF/IPTC metadata
- **Video Processing**: [ffmpeg](https://ffmpeg.org/) for video metadata and thumbnails
- **AI/ML**: [tract](https://github.com/sonos/tract) ONNX runtime (optional, for semantic search)
- **Async Runtime**: [Tokio](https://tokio.rs/)
- **Logging**: Standard `log` and `env_logger`

### Frontend
- **Vanilla JavaScript**: No external frameworks
- **CSS3**: Responsive design with CSS Grid and Flexbox
- **BlurHash**: Placeholder images while loading
- **i18n**: English and German translations

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

TurboPix uses several techniques to improve performance:

- **Parallel Processing**: Processes multiple photos at once during indexing
- **Database Indexing**: Uses SQLite indexes for faster queries
- **Thumbnail Generation**: Creates thumbnails in WebP and JPEG formats
- **Lazy Loading**: Loads images only when you scroll to them
- **HTTP Caching**: Caches static files to reduce server load
- **BlurHash Placeholders**: Shows low-resolution previews while full images load

Tested on hardware ranging from Raspberry Pi to desktop computers with photo collections from a few hundred to tens of thousands of photos.

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
