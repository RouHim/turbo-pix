# TurboPix development

Build, configuration, format and architecture details for TurboPix. The
[README](README.md) covers installing and running the app.

## Project links

- CI: <https://github.com/RouHim/turbo-pix/actions/workflows/ci.yml>
- Container images: <https://hub.docker.com/r/rouhim/turbo-pix> (also [tags](https://hub.docker.com/r/rouhim/turbo-pix/tags))
- Releases: <https://github.com/RouHim/turbo-pix/releases>

## How it works

TurboPix scans your photo directories at startup, reads metadata and generates CLIP embeddings for semantic search from
your photos, and stores everything in a local database. You can then browse photos by date, search for specific cameras
or filenames, and view detailed information about each photo. Thumbnails are generated on-the-fly and cached for faster
loading. Each night, TurboPix checks for new photos and updates the database accordingly; the same full cycle can be
started on demand from the Housekeeping view (or `POST /api/indexing/scan`). Run one instance per data directory: the
scan lock coordinates triggers inside a process, not across processes.

## Capabilities

- **Photo / Video Browsing**: View your photo and video collection
- **Semantic Search**: Search photos by describing their content
- **Timeline View**: See photo density over time
- **Map View**: Plot every geo-located photo on an OpenStreetMap tile layer, clustered by location, and open
  photos straight from a place
- **Metadata Display**: View camera settings, date taken, and GPS coordinates
- **Dark Mode**: Switch between light and dark themes
- **Favorites**: Mark photos to find them later
- **Albums**: Group photos and videos into albums you create, rename, and delete
- **Collages**: Generate collage candidates from your recent photo days — at least ten photos on a day,
  within the last 30 days — then accept or reject them
- **Saved Searches**: Store a search and re-run it later
- **Batch Selection**: Select many items at once to favorite, export, delete, or add them to an album
- **People**: Group detected faces into people, rename or hide clusters, and filter the grid by person
- **Housekeeping**: Review duplicate and issue candidates and keep or delete them
- **Mobile Support**: Works on phones and tablets
- **Self-Hosted Container Support**: Runs on your own computer or as a container, no cloud services
- **RAW Support**: Supports various RAW image formats

## Build from source

### Prerequisites

- Node.js 26 and npm - builds the frontend bundle
- A stable Rust toolchain with `rustfmt` and `clippy` (see `rust-toolchain.toml`) - builds the binary
- `7z` (p7zip) and `curl` - fetch the pinned static ONNX Runtime
- ffmpeg and ffprobe on `PATH` - tested with 8.x (8.0.1, as CI installs it) and 9.0 in the container image

### Build

Building from source requires the frontend bundle first: `npm install && npm run build` (writes `dist/`). The face
pipeline links a static ONNX Runtime archive, so fetch it and export its path before the Rust build:

```shell
eval "$(./scripts/fetch-ort.sh --export)"
cargo build --release
```

The helper downloads the pinned archive (x86_64 glibc for local builds, x86_64/aarch64 musl for releases) and verifies
its SHA-256. The container build does this in a separate stage.

## Docker Compose

```yaml
services:
  turbo-pix:
    image: rouhim/turbo-pix
    environment:
      TURBO_PIX_HOST: 0.0.0.0
      TURBO_PIX_ALLOWED_HOSTS: localhost,192.168.0.36
    volumes:
      - /path/to/pictures:/photos # must be writable: edits, rotation and deletion write in place
      - ./data:/data
    ports:
      - '18473:18473'
```

The repository's `docker-compose.yml` sets `TURBO_PIX_HOST=0.0.0.0` and requires `TURBO_PIX_ALLOWED_HOSTS` to be set to
the hostname or IP you use to reach TurboPix; see the comments in that file.

## Configuration

All configuration is done via environment variables:

| Name                               | Description                                                                                                                                                                                                                                          | Default value                                    | Required                      |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ | ----------------------------- |
| `TURBO_PIX_PHOTO_PATHS`            | Comma-separated list of photo directories to scan                                                                                                                                                                                                    | `./photos`                                       | Yes                           |
| `TURBO_PIX_DATA_PATH`              | Data directory for database, cache, and AI models                                                                                                                                                                                                    | `./data`                                         | No                            |
| `TURBO_PIX_PORT`                   | Port on which the application should listen                                                                                                                                                                                                          | `18473`                                          | No                            |
| `TURBO_PIX_HOST`                   | Address to bind. Loopback by default; set `0.0.0.0` for LAN/Docker access                                                                                                                                                                            | `127.0.0.1`                                      | No                            |
| `TURBO_PIX_ALLOWED_HOSTS`          | Comma-separated hostnames the `Host` header may carry (DNS-rebinding protection). Pinned automatically for loopback binds; required by `docker-compose.yml`, otherwise strongly recommended — a native binary only logs a startup warning            | _(loopback names)_                               | Yes for compose               |
| `TURBO_PIX_LOCALE`                 | Default UI locale (`en`, `de`)                                                                                                                                                                                                                       | `en`                                             | No                            |
| `TURBO_PIX_NOMINATIM_URL`          | Base URL for the Nominatim geocoding service                                                                                                                                                                                                         | `https://nominatim.openstreetmap.org`            | No                            |
| `TURBO_PIX_STREAM_QUEUE_WAIT_SECS` | Seconds a `GET /api/photos/{hash}/video/stream` request waits for a free conversion slot before answering `503` + `Retry-After`                                                                                                                      | `20`                                             | No                            |
| `TURBO_PIX_TILE_URL`               | Raster tile endpoint (`{z}/{x}/{y}` placeholders) used by the Map view                                                                                                                                                                               | `https://tile.openstreetmap.org/{z}/{x}/{y}.png` | No                            |
| `TURBO_PIX_FACE_RECOGNITION`       | Enable face detection/recognition and the People view. `false`/`0`/`no`/`off` skips the phase and hides the People surface; existing data is retained                                                                                                | `true`                                           | No                            |
| `TURBO_PIX_MAX_CACHE_SIZE_MB`      | Maximum thumbnail cache size in megabytes                                                                                                                                                                                                            | `1024`                                           | No                            |
| `TURBO_PIX_TRANSCODE_TIMEOUT_SECS` | Per-conversion ffmpeg timeout in seconds                                                                                                                                                                                                             | `300`                                            | No                            |
| `TURBO_PIX_MAX_TRANSCODES`         | Concurrent video conversions; `0` disables transcoding. Default scales with the CPU count (`min(max(nproc/2,1),4)`)                                                                                                                                  | _(nproc-derived)_                                | No                            |
| `FFMPEG_PATH`                      | ffmpeg binary to invoke                                                                                                                                                                                                                              | `ffmpeg`                                         | No                            |
| `FFPROBE_PATH`                     | ffprobe binary to invoke                                                                                                                                                                                                                             | `ffprobe`                                        | No                            |
| `TRANSCODE_CACHE_DIR`              | Root of the whole-file transcode cache                                                                                                                                                                                                               | `{DATA_PATH}/cache/transcoded`                   | No                            |
| `RUST_LOG`                         | Log level (trace, debug, info, warn, error). The binary calls `env_logger::init()` without a default filter, so a native run logs only `error`; the container image and `docker-compose.yml` set `info`                                              | `error` (native), `info` (container)             | No                            |

### Derived paths

The following paths are automatically derived from `TURBO_PIX_DATA_PATH`:

- **Database**: `{DATA_PATH}/database/turbo-pix.db`
- **Thumbnails**: `{DATA_PATH}/cache/thumbnails/`
- **Transcode cache**: `{DATA_PATH}/cache/transcoded/`
- **AI Models**: `{DATA_PATH}/models/` (CLIP) and `{DATA_PATH}/models/face/` (SCRFD + ArcFace)
- **Face crops**: `{DATA_PATH}/cache/faces/{file_hash}/{face_id}.jpg` (one directory per photo)
- **Xet cache**: `{DATA_PATH}/cache/xet/` (hf-xet chunk cache for model downloads; `HF_XET_CACHE` overrides it)

### AI models

AI models are resolved (and downloaded when they are missing) when the application starts. If you want to download
them manually, you can run:

```shell
./turbo-pix --download-models
```

Or with Docker:

```shell
docker run --rm -v ./data:/data rouhim/turbo-pix --download-models
```

The CLIP weights (~580MB of RAM) are loaded on first use - typically the startup indexing run - and released again
after ten minutes without one, so a server that sits idle does not keep them resident. The first semantic search (or the
first indexing run that needs embeddings) after that pays a short reload; the rest of the application is unaffected.

Face recognition uses SCRFD-2.5G (detector, ~3.3 MB) and antelopev2 R100 (recognition, ~260.7 MB), downloaded from the
pinned `immich-app` HuggingFace revisions and verified by SHA-256. The weights are licensed for non-commercial use only;
see `NOTICE`. To seed an offline/air-gapped installation, place the two files at
`{DATA_PATH}/models/face/detector.onnx` and `{DATA_PATH}/models/face/recognizer.onnx`; they are verified when loaded.
The face phase runs after semantic vectors. `TURBO_PIX_FACE_RECOGNITION=false` skips it entirely.

## Supported platforms

Only what this repository actually exercises:

- **OS**: Linux. Release binaries are static musl builds for `x86_64` and `aarch64`; macOS and Windows are neither built nor tested.
- **Browsers**: Chromium-based browsers. The Playwright suite runs a single Chromium project; Firefox and Safari are untested.
- **Toolchain**: Node.js 26 (CI and the container image) and a stable Rust toolchain (`rust-toolchain.toml`).
- **ffmpeg**: 8.x (8.0.1 from distro packages, as CI installs it) and 9.0, the version baked into the container image.

## Supported formats

### Images

- **Standard formats**: JPEG, PNG, GIF, WebP, BMP, TIFF
- **RAW formats**: CR2, CR3 (Canon), NEF, NRW (Nikon), ARW, SRF, SR2 (Sony), RAF (Fujifilm), ORF (Olympus), RW2 (
  Panasonic), DNG (Adobe), PEF (Pentax)

### Videos

- MP4, MOV, AVI, MKV, WebM, M4V

## Limitations

- **HEIC Support**: Limited due to [image-rs issue #1375](https://github.com/image-rs/image/issues/1375)
- **Live Photos**: iOS Live Photos are treated as separate image and video files
