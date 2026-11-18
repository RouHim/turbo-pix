#!/usr/bin/env bash
# Rebuild the single turbo-pix binary. The frontend must be built first:
# build.rs embeds dist/ into the binary and panics when it is missing.
set -euo pipefail
cd "$(dirname "$0")/.."
npm run build
cargo build --bin turbo-pix
printf '\nBuilt binary: %s\n' "$(pwd)/target/debug/turbo-pix"
