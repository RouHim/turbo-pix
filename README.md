<p align="center">
  <img src="https://raw.githubusercontent.com/RouHim/turbo-pix/main/.github/readme/banner.svg" width="600">
</p>

<p align="center">
A fast, self-hosted photo and video viewer that reads your folders in place.
</p>


![TurboPix photo grid](.github/readme/screenshot.webp)

## Requirements

- Docker, or Linux on `x86_64`/`aarch64` for the release binary
- `ffmpeg` and `ffprobe` on `PATH` for a native install; the container ships both
- A trusted LAN: there is no login, and TurboPix must never be exposed publicly

## Install

```shell
docker run -p 18473:18473 \
  -v /path/to/pictures:/photos \
  -v ./data:/data \
  -e TURBO_PIX_ALLOWED_HOSTS=localhost,192.168.0.36 \
  rouhim/turbo-pix
```

The photo mount must be writable: TurboPix writes metadata edits, rotation and the scan's faststart rewrite into
your files, and deletion removes them, so a read-only mount answers those actions with `403` while browsing and
playback keep working.

`TURBO_PIX_ALLOWED_HOSTS` is a comma-separated list of every hostname or IP you use to reach TurboPix. The sample
allows `localhost` for the URL below and the LAN address `192.168.0.36`; replace the latter with your own. A request
whose `Host` is not on the list is rejected, so write actions such as favorites, edits and deletion fail with `403`
even though browsing still works.

Native alternative: download a release binary from the [releases page](https://github.com/RouHim/turbo-pix/releases),
create the data directory, and start the app.

```shell
LATEST_VERSION=$(curl -L -s -H 'Accept: application/json' https://github.com/RouHim/turbo-pix/releases/latest | \
sed -e 's/.*"tag_name":"\([^"]*\)".*/\1/') && \
curl -L -o turbo-pix https://github.com/RouHim/turbo-pix/releases/download/$LATEST_VERSION/turbo-pix-x86_64-unknown-linux-musl && \
chmod +x turbo-pix
```

Replace `x86_64` with `aarch64` on ARM64. Then:

```shell
mkdir data
TURBO_PIX_PHOTO_PATHS=/path/to/pictures,/path/to/videos \
TURBO_PIX_DATA_PATH=data \
./turbo-pix
```

## Usage

Open <http://localhost:18473>, or `http://<host>:18473` from another device. The first start scans your library and
downloads the search model (~580 MB) and the face models (~264 MB). Later starts only process new or changed files, and a
nightly rescan keeps the index current; start one from Housekeeping if you do not want to wait. The UI shows indexing
progress. Face recognition groups detected faces into people; rename or hide clusters there, or click a person to
filter the grid. Set `TURBO_PIX_FACE_RECOGNITION=false` to skip face indexing and hide the People view.

Configuration, supported formats and build instructions are in [DEVELOPMENT.md](DEVELOPMENT.md).

## Troubleshooting and help

- **No photos**: check `TURBO_PIX_PHOTO_PATHS` and restart with `RUST_LOG=info` to see the scan log.
- **Another device cannot connect**: set `TURBO_PIX_HOST=0.0.0.0` and `TURBO_PIX_ALLOWED_HOSTS` to the IP or hostname
  you use. The container already binds `0.0.0.0`.
- **Startup fails with `ffmpeg binary not found`**: the process checks both binaries before it serves anything, so
  install `ffmpeg` and `ffprobe` on `PATH`, or set `FFMPEG_PATH` / `FFPROBE_PATH` to their locations (native install
  only; the container ships both).
- **Slow first start**: indexing and the model download run in the background; the UI shows progress.
- Help: [open an issue](https://github.com/RouHim/turbo-pix/issues) or [buy me a coffee](https://buymeacoffee.com/rouhim).

> [!NOTE]
> This project is primarily developed using AI-assisted coding, but all code is manually reviewed by me and validated
> through multiple security gates including vulnerability scanning, linting, static code analysis, and automated testing
> to ensure quality and security.

## License

The TurboPix source code is [MIT](LICENSE). The optional face models are licensed for non-commercial use only and are
not covered by the MIT license; see [NOTICE](NOTICE).
