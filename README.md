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
    <i>A straightforward photo and video viewer—fast, simple, and nothing else.</i>
</p>

## Motivation

After migrating from cloud photo services to self-hosted solutions, I wanted a simple and fast photo and video gallery
that works self-hosted without the need to upload or move my existing photo directory structure in any way.

## How it works

TurboPix scans your photo directories at startup, reads metadata and generates CLIP embeddings for semantic search from
your photos, and stores everything in a local database. You can then browse photos by date, search for specific cameras
or filenames, and view detailed information about each photo. Thumbnails are generated on-the-fly and cached for faster
loading. Each night, TurboPix checks for new photos and updates the database accordingly.

## Differentiation

You might ask how this differs to existing solutions like Immich, Lychee, PhotoPrism, Piwigo, or others.
Here are some key differences:

- **Simplicity**: TurboPix focuses on showing your photos and nothing else.
- **Speed**: Written in Rust, TurboPix is optimized for performance and low resource usage.
- **Read-Only**: It does not reorder or move your existing photo files.

## Features

- **Photo / Video Browsing**: View your photo and video collection
- **Semantic Search**: Search photos by describing their content
- **Timeline View**: See photo density over time
- **Map View**: Plot every geo-located photo on an OpenStreetMap tile layer, clustered by location, and open
  photos straight from a place
- **Metadata Display**: View camera settings, date taken, and GPS coordinates
- **Dark Mode**: Switch between light and dark themes
- **Favorites**: Mark photos to find them later
- **Albums**: Group photos and videos into albums you create, rename, and delete
- **Collages**: Generate collage candidates from a selection and accept or reject them
- **Saved Searches**: Store a search and re-run it later
- **Batch Selection**: Select many items at once to favorite, export, delete, or add them to an album
- **Housekeeping**: Review duplicate and issue candidates and keep or delete them
- **Mobile Support**: Works on phones and tablets
- **Self-Hosted Container Support**: Runs on your own computer or as a container, no cloud services
- **RAW Support**: Supports various RAW image formats

## Run the application

> [!NOTE]  
> On the first run, the application may take some time to scan and index your photos, especially if you have a large
> collection.
> Subsequent runs will be much faster as only new or modified photos are processed.
> Also note that on the first run, the AI models for semantic search will be downloaded automatically into the data
> directory (~580MB).

### Docker

Docker Example:

```shell
docker run -p 18473:18473 \
        -v /path/to/pictures:/photos:ro \
        -v ./data:/data \
        rouhim/turbo-pix
```

Docker compose example:

```yaml
services:
  turbo-pix:
    image: rouhim/turbo-pix
    volumes:
      - /path/to/pictures:/photos:ro # could be mounted read only
      - ./data:/data
    ports:
      - '18473:18473'
```

### Native execution

### Prerequisites

- Node.js 26 and npm — builds the frontend bundle
- A stable Rust toolchain with `rustfmt` and `clippy` (see `rust-toolchain.toml`) — builds the binary
- ffmpeg and ffprobe on `PATH` — tested with 6.x from distro packages and 9.0 in the container image

### Installation

Releases are Linux musl builds for `x86_64` and `aarch64`
(`turbo-pix-x86_64-unknown-linux-musl`, `turbo-pix-aarch64-unknown-linux-musl`); there is no macOS or Windows build.
Pick the binary matching your architecture from the [releases page](https://github.com/RouHim/turbo-pix/releases):

```shell
# x86_64/aarch64 Linux — change the binary name below if you run on ARM64
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
TURBO_PIX_PHOTO_PATHS=/path/to/pictures,/path/to/videos \
TURBO_PIX_DATA_PATH=data \
./turbo-pix
```

Building from source requires the frontend bundle first: `npm install && npm run build` (writes dist/), then `cargo build --release`.

## Configuration

All configuration is done via environment variables:

| Name                               | Description                                                                                                                                                                                                                                          | Default value                                    | Required                      |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ | ----------------------------- |
| `TURBO_PIX_PHOTO_PATHS`            | Comma-separated list of photo directories to scan                                                                                                                                                                                                    | `./photos`                                       | Yes                           |
| `TURBO_PIX_DATA_PATH`              | Data directory for database, cache, and AI models                                                                                                                                                                                                    | `./data`                                         | No                            |
| `TURBO_PIX_PORT`                   | Port on which the application should listen                                                                                                                                                                                                          | `18473`                                          | No                            |
| `TURBO_PIX_HOST`                   | Address to bind. Loopback by default; set `0.0.0.0` for LAN/Docker access                                                                                                                                                                            | `127.0.0.1`                                      | No                            |
| `TURBO_PIX_ALLOWED_HOSTS`          | Comma-separated hostnames the `Host` header may carry (DNS-rebinding protection). Pinned automatically for loopback binds; **required when binding non-loopback** — the API is unauthenticated, and `docker-compose.yml` refuses to start without it | _(loopback names)_                               | Yes when binding non-loopback |
| `TURBO_PIX_LOCALE`                 | Default UI locale (`en`, `de`)                                                                                                                                                                                                                       | `en`                                             | No                            |
| `TURBO_PIX_NOMINATIM_URL`          | Base URL for the Nominatim geocoding service                                                                                                                                                                                                         | `https://nominatim.openstreetmap.org`            | No                            |
| `TURBO_PIX_STREAM_QUEUE_WAIT_SECS` | Seconds a `GET /api/photos/{hash}/video/stream` request waits for a free conversion slot before answering `503` + `Retry-After`                                                                                                                      | `20`                                             | No                            |
| `TURBO_PIX_TILE_URL`               | Raster tile endpoint (`{z}/{x}/{y}` placeholders) used by the Map view                                                                                                                                                                               | `https://tile.openstreetmap.org/{z}/{x}/{y}.png` | No                            |
| `TURBO_PIX_MAX_CACHE_SIZE_MB`      | Maximum thumbnail cache size in megabytes                                                                                                                                                                                                            | `1024`                                           | No                            |
| `TURBO_PIX_TRANSCODE_TIMEOUT_SECS` | Per-conversion ffmpeg timeout in seconds                                                                                                                                                                                                             | `300`                                            | No                            |
| `TURBO_PIX_MAX_TRANSCODES`         | Concurrent video conversions; `0` disables transcoding. Default scales with the CPU count (`min(max(nproc/2,1),4)`)                                                                                                                                  | _(nproc-derived)_                                | No                            |
| `FFMPEG_PATH`                      | ffmpeg binary to invoke                                                                                                                                                                                                                              | `ffmpeg`                                         | No                            |
| `FFPROBE_PATH`                     | ffprobe binary to invoke                                                                                                                                                                                                                             | `ffprobe`                                        | No                            |
| `TRANSCODE_CACHE_DIR`              | Root of the whole-file transcode cache                                                                                                                                                                                                               | `{DATA_PATH}/cache/transcoded`                   | No                            |
| `RUST_LOG`                         | Log level (trace, debug, info, warn, error). The binary calls `env_logger::init()` without a default filter, so a native run logs only `error`; the container image and `docker-compose.yml` set `info`                                              | `error` (native), `info` (container)             | No                            |

### Derived Paths

The following paths are automatically derived from `TURBO_PIX_DATA_PATH`:

- **Database**: `{DATA_PATH}/database/turbo-pix.db`
- **Thumbnails**: `{DATA_PATH}/cache/thumbnails/`
- **Transcode cache**: `{DATA_PATH}/cache/transcoded/`
- **AI Models**: `{DATA_PATH}/models/`

### AI Models

AI models are resolved (and downloaded when they are missing) when the application starts. If you want to download
them manually, you can run:

```shell
./turbo-pix --download-models
```

Or with Docker:

```shell
docker run --rm -v ./data:/data rouhim/turbo-pix --download-models
```

The CLIP weights (~580MB of RAM) are loaded on first use — typically the startup indexing run — and released again
after ten minutes without one, so a server that sits idle does not keep them resident. The first semantic search (or the
first indexing run that needs embeddings) after that pays a short reload; the rest of the application is unaffected.

## Supported Platforms

Only what this repository actually exercises:

- **OS**: Linux. Release binaries are static musl builds for `x86_64` and `aarch64`; macOS and Windows are neither built nor tested.
- **Browsers**: Chromium-based browsers. The Playwright suite runs a single Chromium project; Firefox and Safari are untested.
- **Toolchain**: Node.js 26 (CI and the container image) and a stable Rust toolchain (`rust-toolchain.toml`).
- **ffmpeg**: 6.x from distro packages, as CI installs it, and 9.0, the version baked into the container image.

## Supported Formats

### Images

- **Standard formats**: JPEG, PNG, GIF, WebP, BMP, TIFF
- **RAW formats**: CR2, CR3 (Canon), NEF, NRW (Nikon), ARW, SRF, SR2 (Sony), RAF (Fujifilm), ORF (Olympus), RW2 (
  Panasonic), DNG (Adobe), PEF (Pentax)

### Videos

- MP4, MOV, AVI, MKV, WebM, M4V

Playback is native-first and decided server-side per request: the server compares
each file's capabilities (codec, container, bit depth, layout) with the codecs the
browser declares and picks direct play, a lossless remux (container/layout only,
e.g. moving the `moov` atom so playback can start immediately), or a conversion.
Videos the browser cannot decode are streamed as fragmented MP4 straight out of
ffmpeg while they convert, so playback starts long before the conversion
finishes; seeking works through Media Source Extensions, which needs no secure
context (WebCodecs does, and TurboPix is usually served over plain HTTP on the
LAN). Conversions run in a bounded worker pool (`TURBO_PIX_MAX_TRANSCODES`): a
stream request that finds the pool full waits up to
`TURBO_PIX_STREAM_QUEUE_WAIT_SECS` for a slot and otherwise answers `503` with
`Retry-After` while the viewer keeps its waiting notice on screen. Converted and
remuxed artifacts are cached and reused on the next open, and the viewer offers
the original file as an escape hatch if a conversion keeps failing.

## Limitations

- **HEIC Support**: Limited due to [image-rs issue #1375](https://github.com/image-rs/image/issues/1375)
- **Live Photos**: iOS Live Photos are treated as separate image and video files

## Support

If you find TurboPix useful, consider [buying me a coffee](https://buymeacoffee.com/rouhim)

# Disclaimer

> [!NOTE]  
> This project is primarily developed using AI-assisted coding, but all code is manually reviewed by me
> and validated through multiple security gates including vulnerability scanning, linting, static code analysis, and
> automated testing to ensure quality and security.
