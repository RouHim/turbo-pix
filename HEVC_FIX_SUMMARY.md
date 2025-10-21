# HEVC Transcoding Fix Summary

## Problem
HEVC videos were not transcoding to H.264 for browsers without HEVC support. No error messages were visible, and videos simply wouldn't play.

## Root Causes Identified

1. **Wrong FFmpeg Encoder**: Code used `libopenh264` which is NOT available in standard ffmpeg builds
2. **Poor Error Visibility**: Errors logged as warnings, easy to miss
3. **Missing Error Details**: FFmpeg stderr/stdout not captured in logs
4. **Non-persistent Cache**: Transcoded videos stored in `/tmp`, lost on restart

## Fixes Applied

### 1. FFmpeg Encoder Change (src/video_processor.rs)
**Before:**
```rust
"-c:v", "libopenh264",
"-b:v", "5M",
```

**After:**
```rust
"-c:v", "libx264",        // Universal H.264 encoder
"-preset", "fast",        // Encoding speed
"-crf", "23",             // Quality (18-28, lower=better)
```

**Why:** `libx264` is available in virtually all ffmpeg builds, including the static build from johnvansickle.com that the container uses.

### 2. Enhanced Error Logging (src/handlers_video.rs)
**Before:**
```rust
log::warn!("Transcoding failed for {}, falling back to original: {}", ...);
```

**After:**
```rust
log::error!("Transcoding FAILED for {}: {}", ...);
log::error!("Falling back to original HEVC video. Client may not be able to play it.");
log::error!("To fix: Ensure ffmpeg has HEVC decoder and libx264 encoder...");
```

**Why:** Errors now use `log::error!` making them impossible to miss in logs.

### 3. FFmpeg Error Capture (src/video_processor.rs)
Added logging of ffmpeg's stdout and stderr when transcoding fails:
```rust
log::error!("FFmpeg stderr: {}", stderr);
log::error!("FFmpeg stdout: {}", stdout);
```

**Why:** Allows debugging of ffmpeg failures.

### 4. Persistent Transcoding Cache (docker-compose.yml)
**Before:** Used `/tmp/turbo-pix` (ephemeral)

**After:**
```yaml
environment:
  - TRANSCODE_CACHE_DIR=/data/cache/transcoded
```

**Why:** Transcoded videos persist across container restarts, saving CPU and time.

## How to Test

### Step 1: Rebuild the Container
```bash
podman-compose build
# or
podman compose build
```

### Step 2: Run the Test Script
```bash
./test_hevc_fix.sh
```

This script will:
1. Stop old container
2. Start new container with fixes
3. Verify ffmpeg has required codecs
4. Monitor logs for transcoding activity

### Step 3: Manual Testing
1. Open http://localhost:18473 in a browser that doesn't support HEVC (e.g., Firefox on Linux)
2. Navigate to `PXL_20251018_124956882.mp4`
3. Try to play the video

### Step 4: Check Logs
Monitor the logs while playing the video:
```bash
podman-compose logs -f turbopix
# or
podman compose logs -f turbopix
```

**Expected Success Logs:**
```
INFO Transcoding HEVC video to H.264: PXL_20251018_124956882.mp4 (hash: b...)
INFO Transcoding completed successfully: PXL_20251018_124956882.mp4 -> /data/cache/transcoded/...mp4
```

**Expected Error Logs (if ffmpeg missing codecs):**
```
ERROR Transcoding FAILED for PXL_20251018_124956882.mp4: ffmpeg transcode exited with status ...
ERROR FFmpeg stderr: [detailed error message]
ERROR Falling back to original HEVC video. Client may not be able to play it.
```

## Debugging Tools

### debug_hevc.sh
Comprehensive diagnostic script:
```bash
./debug_hevc.sh
```

This checks:
- FFmpeg version
- Available HEVC decoders
- Available H.264 encoders
- Cache directory structure
- Recent transcoding logs
- Provides manual test command

### Manual FFmpeg Test
Test transcoding manually inside the container:
```bash
podman-compose exec turbopix /ffmpeg \
  -hwaccel auto \
  -i /photos/PXL_20251018_124956882.mp4 \
  -c:v libx264 \
  -preset fast \
  -crf 23 \
  -c:a copy \
  -movflags +faststart \
  -y /tmp/test_transcode.mp4
```

If this fails, check the error message to understand what's wrong.

## Expected Outcomes

### Success Case
1. Browser detects no HEVC support
2. Client requests video with `?transcode=true`
3. Server transcodes HEVC → H.264 using libx264
4. Transcoded file cached in `/data/cache/transcoded/`
5. Video plays in browser
6. Subsequent plays use cached version (fast)

### Failure Case (if ffmpeg lacks codecs)
1. Browser detects no HEVC support
2. Client requests video with `?transcode=true`
3. Server tries to transcode but ffmpeg fails
4. **ERROR logs visible** (unlike before!)
5. Server serves original HEVC with warning header
6. Client detects warning and shows error message to user

## If Still Not Working

### Check FFmpeg Codecs
```bash
podman-compose exec turbopix /ffmpeg -decoders | grep hevc
podman-compose exec turbopix /ffmpeg -encoders | grep libx264
```

Both should show results. If not, the static ffmpeg build might be incomplete.

### Check Cache Permissions
```bash
podman-compose exec turbopix ls -la /data/cache
```

Should show writable directories.

### Enable Debug Logging
Edit `docker-compose.yml`:
```yaml
environment:
  - RUST_LOG=debug  # Changed from info
```

Then rebuild and restart.

## Performance Notes

- **First play**: Takes time to transcode (CPU intensive)
- **Subsequent plays**: Instant (cached)
- **Cache location**: `./data/cache/transcoded/` (persistent)
- **Cache cleanup**: Manual for now (automatic cleanup can be added later)

## Browser HEVC Support

HEVC is supported in:
- Safari (macOS/iOS)
- Edge (Windows with HEVC codec pack)
- Chrome (Android, some Windows systems)

HEVC is NOT supported in:
- Firefox (all platforms)
- Chrome on Linux
- Many older browsers

The transcoding system automatically handles this!
