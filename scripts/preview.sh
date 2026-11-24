#!/usr/bin/env bash
# Rebuild the single turbo-pix binary. The frontend must be built first:
# build.rs embeds dist/ into the binary and panics when it is missing.
set -euo pipefail
cd "$(dirname "$0")/.."
npm run build
# build.rs embeds the offline place dataset and panics when it is missing.
./scripts/fetch-geodata.sh
# The face pipeline links a static ONNX Runtime; the build fails without it.
eval "$(./scripts/fetch-ort.sh --export)"
cargo build --bin turbo-pix
printf '\nBuilt binary: %s\n' "$(pwd)/target/debug/turbo-pix"
