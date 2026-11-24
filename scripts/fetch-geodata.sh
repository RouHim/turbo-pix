#!/bin/sh
# Build the offline place dataset used to resolve photo coordinates into place
# names.
#
# The dataset is not committed: it is derived from the upstream GeoNames dumps
# (`cities500.zip` plus the `allCountries.zip` PPLX rows, ~100 MB compressed)
# and only needed while building. `src/place_resolver.rs` embeds the derived
# file into the binary, so nothing is fetched at runtime.
#
#   ./scripts/fetch-geodata.sh           # build assets/geodata.txt (reused when valid)
#   ./scripts/fetch-geodata.sh --force   # rebuild from upstream
#   ./scripts/fetch-geodata.sh --validate <file>
#
# GeoNames data is licensed CC BY 4.0 (https://www.geonames.org); the
# attribution lives in README.md and in the map attribution line.
set -eu

BASE_URL="https://download.geonames.org/export/dump"
HEADER=$(printf '#turbo-pix-places-v1\tname\tlatitude\tlongitude\tfeature_class\tfeature_code\tcountry_code\tpopulation')
MIN_ROWS=390000
# Bump when the upstream dumps themselves change: CI caches assets/geodata.txt
# under a hash of this script, so editing it is what invalidates the cache.
DATASET_REVISION=1

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
OUT="${GEODATA_OUT:-"$ROOT/assets/geodata.txt"}"

WORK_DIR=""
cleanup() {
    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi
}
trap cleanup EXIT

usage() {
    echo "usage: $0 [--force] | --validate <file>" >&2
    exit 1
}

validate() {
    dataset_file=$1

    if [ ! -s "$dataset_file" ]; then
        echo "dataset missing or empty: $dataset_file" >&2
        exit 1
    fi

    first_line=$(head -n 1 "$dataset_file")
    if [ "$first_line" != "$HEADER" ]; then
        echo "dataset header missing or wrong: $dataset_file" >&2
        exit 1
    fi

    rows=$(( $(wc -l < "$dataset_file") - 1 ))
    if [ "$rows" -lt "$MIN_ROWS" ]; then
        echo "dataset too small: $dataset_file has $rows rows, expected at least $MIN_ROWS" >&2
        exit 1
    fi

    bad_rows=$(awk -F'\t' 'NR > 1 && NF != 7' "$dataset_file" | wc -l)
    if [ "$bad_rows" -ne 0 ]; then
        echo "dataset has $bad_rows rows with other than 7 columns: $dataset_file" >&2
        exit 1
    fi

    if ! awk -F'\t' '$1 == "Longerich" && $5 == "PPLX" && $6 == "DE" { found = 1 } END { exit !found }' "$dataset_file"; then
        echo "dataset anchor row missing (Longerich PPLX DE): $dataset_file" >&2
        exit 1
    fi

    if ! grep -q "$(printf 'K\303\270benhavn')" "$dataset_file"; then
        echo "dataset UTF-8 check failed (København missing): $dataset_file" >&2
        exit 1
    fi

    echo "dataset ok: $rows rows, $(wc -c < "$dataset_file") bytes"
}

build() {
    WORK_DIR=$(mktemp -d)

    curl -fL --retry 3 --retry-delay 2 --connect-timeout 15 \
        "$BASE_URL/cities500.zip" -o "$WORK_DIR/cities500.zip"
    curl -fL --retry 3 --retry-delay 2 --connect-timeout 15 \
        "$BASE_URL/allCountries.zip" -o "$WORK_DIR/allCountries.zip"

    unzip -p "$WORK_DIR/cities500.zip" > "$WORK_DIR/cities500.txt"
    # Districts (PPLX) of the big cities, deduplicated against cities500 in the
    # merge below. Column 8 is the feature code in the GeoNames dump layout.
    unzip -p "$WORK_DIR/allCountries.zip" | LC_ALL=C awk -F'\t' '$8 == "PPLX"' > "$WORK_DIR/pplx.txt"

    # geonameid, name, latitude, longitude, feature class, feature code,
    # country code, population — projected to the seven columns the resolver
    # reads, with cities500 rows winning over an allCountries duplicate.
    LC_ALL=C awk -F'\t' -v OFS='\t' '
        NR == FNR { seen[$1]++; print $2, $5, $6, $7, $8, $9, $15; next }
        !($1 in seen) { seen[$1]++; print $2, $5, $6, $7, $8, $9, $15 }
    ' "$WORK_DIR/cities500.txt" "$WORK_DIR/pplx.txt" > "$WORK_DIR/body.txt"

    { printf "%s\n" "$HEADER"; cat "$WORK_DIR/body.txt"; } > "$OUT.tmp"
    validate "$OUT.tmp"
    mv "$OUT.tmp" "$OUT"
}

case "${1:-}" in
    --validate)
        [ -n "${2:-}" ] || usage
        validate "$2"
        ;;
    --force)
        build
        ;;
    "")
        if [ -s "$OUT" ] && [ "$(head -n 1 "$OUT")" = "$HEADER" ]; then
            validate "$OUT"
        else
            build
        fi
        ;;
    *)
        usage
        ;;
esac
