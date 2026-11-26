# TurboPix development

Build, test and data-layout details for TurboPix. The [README](README.md) covers installation and use.

## Build from source

TurboPix is a Rust binary that serves a Vite frontend embedded at compile time. `build.rs` embeds `dist/` and
`assets/geodata.txt`, so both must exist before `cargo build`.

Prerequisites:

- Node.js 26 and npm
- A stable Rust toolchain with `rustfmt` and `clippy` (see `rust-toolchain.toml`)
- `curl`, `unzip`, `tar` and `zstd` to fetch the place dataset and the static ONNX Runtime
- ffmpeg and ffprobe on `PATH` for video features and the E2E suite

Build sequence:

```shell
./scripts/fetch-geodata.sh
eval "$(./scripts/fetch-ort.sh --export)"
npm ci && npm run build
cargo build --release
```

`./scripts/preview.sh` runs the same steps and builds a debug binary. Rebuild the frontend before every Rust build
after a frontend change, otherwise `build.rs` embeds the previous `dist/`.

`fetch-geodata.sh` downloads the GeoNames `cities500` and `allCountries` dumps, filters them to populated places
(including `PPLX` quarters) and writes `assets/geodata.txt`; later runs reuse the file. `fetch-ort.sh` downloads the
pinned static ONNX Runtime archive from `RouHim/onnxruntime-static`, verifies its SHA-256 and prints or exports
`ORT_LIB_PATH`. Local builds use the x86_64 glibc archive, releases the x86_64 and aarch64 musl archives; the container
build fetches it in a separate stage. Pins live in `scripts/ort/<target>.env` and are updated by Renovate.

## Tests

```shell
cargo test
npm run test:unit
npm run test:i18n
npm run test:e2e
```

The E2E suite uses Playwright with a real backend. Global setup builds `target/debug/turbo-pix`, seeds
`test-e2e-data/` from `test-data/` and starts the server on port 18473. `TURBO_PIX_E2E_REUSE=1` targets a prepared
server instead of building and reseeding.

Lint and format gates:

```shell
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
npm run lint
npm run format:check
```

## Data directory

`TURBO_PIX_DATA_PATH` defaults to `./data` and contains:

- `database/turbo-pix.db`: the SQLite database
- `cache/thumbnails/`: the thumbnail cache, capped by `TURBO_PIX_MAX_CACHE_SIZE_MB`
- `cache/transcoded/`: the whole-file transcode cache (`TRANSCODE_CACHE_DIR` overrides it, with `copied/` and `remux/`
  namespaces)
- `cache/faces/{file_hash}/{face_id}.jpg`: face crops, one directory per photo
- `cache/xet/`: the hf-xet chunk cache for model downloads (`HF_XET_CACHE` overrides it)

AI models live at `<DATA_PATH>/../data/models`: `clip/` for CLIP, `face/` for SCRFD and ArcFace. With the default
`./data` and the container's `/data` this resolves to `<DATA_PATH>/models`.

## AI models

Models are resolved and downloaded when the application starts. `./turbo-pix --download-models` fetches them without
starting the server.

CLIP runs on ONNX Runtime: a vision encoder (~351.7 MB) and a text encoder (~254.1 MB) from the pinned
`Xenova/clip-vit-base-patch32` HuggingFace revision, verified by SHA-256. Both sessions (~600 MiB of RAM) load on first
use, typically the startup indexing run, and are released after ten minutes without one. The first semantic search, or
the first indexing run that needs embeddings, pays a short reload; the rest of the application is unaffected.

To seed an offline or air-gapped installation, place `vision.onnx`, `text.onnx` and `tokenizer.json` in
`<DATA_PATH>/../data/models/clip`. They are verified when loaded. A custom data path shares the sibling `data/models`
cache. After a runtime or model change that alters embeddings, the shipped migration resets the vector tables and photo
flags, so the next scan re-embeds the library.

An upgrade from the Candle build leaves the old `models--openai--clip-vit-base-patch32` safetensors and tokenizer
snapshot under `<DATA_PATH>/models`. It is no longer used and can be deleted to reclaim about 580 MB.

Face recognition uses SCRFD-2.5G (detector, ~3.3 MB) and antelopev2 R100 (recognition, ~260.7 MB) from the pinned
`immich-app` HuggingFace revisions, verified by SHA-256. The weights are licensed for non-commercial use only; see
[NOTICE](NOTICE). To seed an offline installation, place `detector.onnx` and `recognizer.onnx` at
`<DATA_PATH>/../data/models/face`. The face phase runs after semantic vectors, and `TURBO_PIX_FACE_RECOGNITION=false`
skips it.

## Code provenance

The project is developed with AI-assisted coding. All code is reviewed manually and checked by linting, static
analysis, vulnerability scanning and automated tests.
