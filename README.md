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
- **Metadata Display**: View camera settings, date taken, and GPS coordinates
- **Dark Mode**: Switch between light and dark themes
- **Favorites**: Mark photos to find them later
- **Mobile Support**: Works on phones and tablets
- **Self-Hosted Container Support**: Runs on your own computer or as a container, no cloud services
- **RAW Support**: Supports various RAW image formats
- **Speedy**: Written in Rust for performance

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
      - /path/to/pictures:/photos:ro  # could be mounted read only
      - ./data:/data
    ports:
      - "18473:18473"
```

### Native execution

### Prerequisites

- ffmpeg

### Installation

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
TURBO_PIX_PHOTO_PATHS=/path/to/pictures,/path/to/videos \
TURBO_PIX_DATA_PATH=data \
./turbo-pix
```

## Configuration

All configuration is done via environment variables:

| Name                    | Description                                       | Default value | Required |
|-------------------------|---------------------------------------------------|---------------|----------|
| `TURBO_PIX_PHOTO_PATHS` | Comma-separated list of photo directories to scan | `./photos`    | Yes      |
| `TURBO_PIX_DATA_PATH`   | Data directory for database, cache, and AI models | `./data`      | No       |
| `TURBO_PIX_PORT`        | Port on which the application should listen       | `18473`       | No       |
| `TURBO_PIX_LOCALE`      | Default UI locale (`en`, `de`)                    | `en`          | No       |
| `RUST_LOG`              | Log level (trace, debug, info, warn, error)       | `info`        | No       |

### Derived Paths

The following paths are automatically derived from `TURBO_PIX_DATA_PATH`:

- **Database**: `{DATA_PATH}/database/turbo-pix.db`
- **Thumbnails**: `{DATA_PATH}/cache/thumbnails/`
- **AI Models**: `{DATA_PATH}/models/`

### AI Models

Normally AI models are downloaded automatically when you first use the semantic search feature. If you want to download
them manually, you can run:

```shell
./turbo-pix --download-models
```

Or with Docker:

```shell
docker run --rm -v ./data:/data rouhim/turbo-pix --download-models
```

## Supported Formats

### Images

- **Standard formats**: JPEG, PNG, GIF, WebP, BMP, TIFF
- **RAW formats**: CR2, CR3 (Canon), NEF, NRW (Nikon), ARW, SRF, SR2 (Sony), RAF (Fujifilm), ORF (Olympus), RW2 (
  Panasonic), DNG (Adobe), PEF (Pentax)

### Videos

- MP4, MOV, AVI, MKV, WebM, M4V

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
