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

echo "Using: $COMPOSE"
echo ""

echo "=== Testing HEVC Transcoding Fix ==="
echo ""

# Stop and remove old container
echo "Stopping old container..."
$COMPOSE down

# Start new container
echo "Starting new container with fixes..."
$COMPOSE up -d

# Wait for container to be ready
echo "Waiting for server to start..."
sleep 5

# Check if server is up
echo "Checking server health..."
curl -f http://localhost:18473/health || {
    echo "ERROR: Server not responding!"
    if [ "$COMPOSE" = "podman-compose" ]; then
        CONTAINER_ID=$($COMPOSE ps 2>&1 | grep turbopix | awk '{print $1}')
        podman logs $CONTAINER_ID
    else
        $COMPOSE logs
    fi
    exit 1
}

echo ""
echo "=== Verifying FFmpeg Codec Support ==="
echo ""

# Get container ID for exec
if [ "$COMPOSE" = "podman-compose" ]; then
    CONTAINER_ID=$($COMPOSE ps 2>&1 | grep turbopix | awk '{print $1}')
    EXEC_CMD="podman exec $CONTAINER_ID"
else
    EXEC_CMD="$COMPOSE exec turbopix"
fi

echo "Checking HEVC decoders:"
$EXEC_CMD /ffmpeg -decoders 2>&1 | grep -i "hevc\|h265" || echo "  WARNING: No HEVC decoder found!"
echo ""

echo "Checking H.264 encoders:"
$EXEC_CMD /ffmpeg -encoders 2>&1 | grep -i "libx264" || echo "  WARNING: libx264 encoder not found!"
echo ""

echo "=== Testing Transcoding ==="
echo ""

# Follow logs in background
echo "Starting log monitoring (Ctrl+C to stop)..."
echo "Now:"
echo "  1. Open http://localhost:18473 in your browser"
echo "  2. Navigate to your HEVC video (PXL_20251018_124956882.mp4)"
echo "  3. Try to play it"
echo "  4. Watch the logs below for transcoding messages"
echo ""
echo "Look for these log messages:"
echo "  - 'Transcoding HEVC video to H.264: PXL_20251018_124956882.mp4'"
echo "  - 'Transcoding completed successfully'"
echo "  OR"
echo "  - 'Transcoding FAILED' (with error details)"
echo ""
echo "--- Container Logs ---"

if [ "$COMPOSE" = "podman-compose" ]; then
    podman logs -f $CONTAINER_ID
else
    $COMPOSE logs -f turbopix
fi
