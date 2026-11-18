#!/bin/sh
# Fetch the pinned static ONNX Runtime used by the face pipeline.
#
# The archive is not committed: it is ~10 MB compressed / ~100 MB extracted
# and only needed while building. Call this before cargo, then export the
# printed ORT_LIB_PATH:
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

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
DEST=${DEST:-"$ROOT/vendor/ort/$TARGET"}

case "$TARGET" in
    x86_64-unknown-linux-gnu)
        ASSET=onnxruntime-v1.23.2-ubuntu-22.04-amd64-static.7z
        SHA=63633f5cc56dcc98c410472fbfaf6c6eed7ab78a72f64ba02ad551877b931297
        ;;
    x86_64-unknown-linux-musl)
        ASSET=onnxruntime-v1.23.2-x86_64-linux-musl-static.7z
        SHA=23cb3535cd8d3fae3d3d46941721176579238e47b1affdf7ce1282ae117ef22a
        ;;
    aarch64-unknown-linux-musl)
        ASSET=onnxruntime-v1.23.2-aarch64-linux-musl-static.7z
        SHA=7637b228d77278bc4e7d86f278f0d107e47f7ad266e5c9d5814fd52ca581d35b
        ;;
    *)
        echo "unsupported target: $TARGET" >&2
        exit 2
        ;;
esac

LIB_DIR="$DEST/lib"
if [ ! -f "$LIB_DIR/libonnxruntime.a" ]; then
    TMP=$(mktemp -d)
    trap 'rm -rf "$TMP"' EXIT HUP INT TERM
    URL="https://github.com/RapidAI/OnnxruntimeBuilder/releases/download/1.23.2/$ASSET"

    if command -v curl >/dev/null 2>&1; then
        curl -fL --retry 3 --retry-delay 2 -o "$TMP/ort.7z" "$URL"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$TMP/ort.7z" "$URL"
    else
        echo "curl or wget is required" >&2
        exit 1
    fi

    echo "$SHA  $TMP/ort.7z" | sha256sum -c - >/dev/null

    mkdir -p "$TMP/extract"
    if command -v 7z >/dev/null 2>&1; then
        7z x -y -o"$TMP/extract" "$TMP/ort.7z" >/dev/null
    elif command -v bsdtar >/dev/null 2>&1; then
        bsdtar -xf "$TMP/ort.7z" -C "$TMP/extract"
    else
        echo "7z or bsdtar is required" >&2
        exit 1
    fi

    LIB=$(find "$TMP/extract" -type f -name libonnxruntime.a | head -n 1)
    [ -n "$LIB" ] || { echo "libonnxruntime.a not found in $ASSET" >&2; exit 1; }
    # The archive root holds `lib/` and `include/`; copy that root, not the
    # lib directory itself, so `$DEST/lib/libonnxruntime.a` is the stable path.
    SRC_ROOT=$(dirname "$(dirname "$LIB")")
    mkdir -p "$DEST"
    cp -a "$SRC_ROOT"/. "$DEST"/
    rm -rf "$TMP"
    trap - EXIT HUP INT TERM
fi

if [ "$EXPORT" -eq 1 ]; then
    printf 'export ORT_LIB_PATH=%s\n' "$(printf '%s' "$LIB_DIR" | sed "s/'/'\\\\''/g; s/^/'/; s/$/'/")"
else
    printf '%s\n' "$LIB_DIR"
fi
