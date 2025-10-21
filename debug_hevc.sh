#!/bin/bash

# Detect if using podman or docker
if command -v podman-compose &> /dev/null; then
    COMPOSE="podman-compose"
elif command -v podman &> /dev/null && podman compose version &> /dev/null; then
    COMPOSE="podman compose"
elif command -v docker &> /dev/null; then
    COMPOSE="docker compose"
else
    echo "ERROR: Neither podman-compose nor docker compose found!"
    exit 1
fi

echo "=== TurboPix HEVC Transcoding Debugger ==="
echo "Using: $COMPOSE"
echo ""

# Get container name/ID (podman-compose vs podman compose have different syntax)
if [ "$COMPOSE" = "podman-compose" ]; then
    CONTAINER_ID=$($COMPOSE ps 2>&1 | grep turbopix | awk '{print $1}')
else
    CONTAINER_ID=$($COMPOSE ps -q turbopix 2>&1)
fi

if [ -z "$CONTAINER_ID" ]; then
    echo "ERROR: TurboPix container is not running!"
    echo "Start it with: $COMPOSE up -d"
    exit 1
fi

echo "Container ID: $CONTAINER_ID"
echo ""

# For podman-compose, we need to use podman exec directly
if [ "$COMPOSE" = "podman-compose" ]; then
    EXEC_CMD="podman exec $CONTAINER_ID"
else
    EXEC_CMD="$COMPOSE exec turbopix"
fi

echo "=== Checking ffmpeg capabilities ==="
$EXEC_CMD /ffmpeg -version 2>&1 | head -n 5
echo ""

echo "=== Checking ffmpeg decoders (looking for hevc/h265) ==="
$EXEC_CMD /ffmpeg -decoders 2>&1 | grep -i "hevc\|h265"
echo ""

echo "=== Checking ffmpeg encoders (looking for h264/libx264) ==="
$EXEC_CMD /ffmpeg -encoders 2>&1 | grep -i "h264\|libx264"
echo ""

echo "=== Checking /data directory permissions ==="
$EXEC_CMD ls -la /data 2>&1
echo ""

echo "=== Checking cache directory structure ==="
$EXEC_CMD ls -la /data/cache 2>&1
echo ""

echo "=== Checking if transcode cache directory exists ==="
$EXEC_CMD ls -la /data/cache/transcoded 2>&1 || echo "  (Directory will be created on first transcode)"
echo ""

echo "=== Checking container logs for transcoding errors ==="
if [ "$COMPOSE" = "podman-compose" ]; then
    podman logs $CONTAINER_ID 2>&1 | grep -i "transcode\|hevc\|ffmpeg" | tail -n 20
else
    $COMPOSE logs turbopix 2>&1 | grep -i "transcode\|hevc\|ffmpeg" | tail -n 20
fi
echo ""

echo "=== Testing HEVC video detection ==="
TEST_VIDEO="PXL_20251018_124956882.mp4"
echo "Looking for test video: $TEST_VIDEO"
# This would need to be run inside container with actual photo path
# $EXEC_CMD /ffprobe -v quiet -select_streams v:0 -show_entries stream=codec_name -of default=noprint_wrappers=1:nokey=1 /photos/$TEST_VIDEO

echo ""
echo "=== Manual transcoding test ==="
echo "Run this command to test manual transcoding:"
echo "$EXEC_CMD /ffmpeg -hwaccel auto -i /photos/$TEST_VIDEO -c:v libx264 -preset fast -crf 23 -c:a copy -movflags +faststart -y /tmp/test_transcode.mp4"
echo ""
echo "=== Done ==="
