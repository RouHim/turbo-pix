#!/bin/sh
# Fetch the pinned static ONNX Runtime used by the face pipeline.
#
# The archive is not committed: it is ~20 MB compressed / ~130 MB extracted
# and only needed while building. The per-target pin files live in
# scripts/ort/<target>.env and are updated by Renovate from the
# RouHim/onnxruntime-static release assets.
#
#   eval "$(./scripts/fetch-ort.sh --export)"
#   cargo test
#
# CI/Containerfile pass --target and DEST so the same recipe works for the
# host (glibc, tests) and the two musl release targets.
set -eu

usage() {
    cat >&2 <<'EOF'
usage: scripts/fetch-ort.sh [--target <triple>] [--export]

  --target <triple>  x86_64-unknown-linux-gnu (default: rustc host),
                     x86_64-unknown-linux-musl, aarch64-unknown-linux-musl
  --export           print `export ORT_LIB_PATH=...` instead of the path

Environment:
  DEST               extraction directory (default: vendor/ort/<target>)
EOF
    exit 2
}

TARGET=""
EXPORT=0
while [ $# -gt 0 ]; do
    case "$1" in
        --target)
            [ $# -ge 2 ] || usage
            TARGET=$2
            shift 2
            ;;
        --export)
            EXPORT=1
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            usage
            ;;
    esac
done

if [ -z "$TARGET" ]; then
    TARGET=$(rustc -vV | sed -n 's/^host: //p')
fi
[ -n "$TARGET" ] || { echo "could not determine host target" >&2; exit 1; }

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
DEST=${DEST:-"$ROOT/vendor/ort/$TARGET"}
PIN="$ROOT/scripts/ort/$TARGET.env"
if [ ! -f "$PIN" ]; then
    echo "unsupported target: $TARGET" >&2
    exit 2
fi

# shellcheck disable=SC1090
. "$PIN"
case "${ORT_TAG:-}" in
    v*)
        ORT_VERSION=${ORT_TAG#v}
        ;;
    *)
        echo "invalid ORT_TAG in $PIN" >&2
        exit 1
        ;;
esac
case "$ORT_VERSION" in
    ''|*[!0-9.]*)
        echo "invalid ORT_TAG in $PIN" >&2
        exit 1
        ;;
esac
case "${ORT_SHA:-}" in
    ''|*[!0-9a-f]*)
        echo "invalid ORT_SHA in $PIN" >&2
        exit 1
        ;;
esac
if [ "${#ORT_SHA}" -ne 64 ]; then
    echo "invalid ORT_SHA length in $PIN" >&2
    exit 1
fi

ASSET="onnxruntime-${ORT_VERSION}-${TARGET}-static.tar.zst"
URL="https://github.com/RouHim/onnxruntime-static/releases/download/${ORT_TAG}/${ASSET}"
LIB_DIR="$DEST/lib"
STAMP="$DEST/.ort-version"

if [ -f "$LIB_DIR/libonnxruntime.a" ] && [ "$(cat "$STAMP" 2>/dev/null || true)" = "$ORT_TAG" ]; then
    :
else
    case "$DEST" in
        ''|/|.)
            echo "refusing to clear output directory: $DEST" >&2
            exit 1
            ;;
    esac
    command -v zstd >/dev/null 2>&1 || { echo "zstd is required" >&2; exit 1; }

    TMP=$(mktemp -d)
    trap 'rm -rf "$TMP"' EXIT HUP INT TERM

    if command -v curl >/dev/null 2>&1; then
        curl -fL --retry 3 --retry-delay 2 -o "$TMP/ort.tar.zst" "$URL"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$TMP/ort.tar.zst" "$URL"
    else
        echo "curl or wget is required" >&2
        exit 1
    fi

    echo "$ORT_SHA  $TMP/ort.tar.zst" | sha256sum -c - >/dev/null

    rm -rf "$DEST"
    mkdir -p "$DEST"
    tar --zstd -xf "$TMP/ort.tar.zst" -C "$DEST"

    [ -f "$LIB_DIR/libonnxruntime.a" ] || {
        echo "libonnxruntime.a not found in $ASSET" >&2
        exit 1
    }
    printf '%s\n' "$ORT_TAG" > "$STAMP"

    rm -rf "$TMP"
    trap - EXIT HUP INT TERM
fi

if [ "$EXPORT" -eq 1 ]; then
    printf 'export ORT_LIB_PATH=%s\n' "$(printf '%s' "$LIB_DIR" | sed "s/'/'\\\\''/g; s/^/'/; s/$/'/")"
else
    printf '%s\n' "$LIB_DIR"
fi
