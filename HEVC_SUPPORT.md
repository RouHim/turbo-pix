# HEVC Video Support in TurboPix

## Issue

The video file `PXL_20251018_124956882.mp4` uses **HEVC (H.265) codec**, which has limited browser support. Most web browsers cannot play HEVC videos natively due to licensing restrictions.

## Root Cause

1. **Browser Limitation**: Modern browsers (Chrome, Firefox, Safari on non-Apple platforms) do not support HEVC playback without specific hardware or licensing
2. **FFmpeg Limitation on Fedora**: The default Fedora FFmpeg package has HEVC codec disabled due to patent/licensing concerns

## Solution Implemented

TurboPix now includes **intelligent HEVC handling** with client-side codec detection using the **Media Capabilities API**:

### Smart Codec Detection (Option 3)

- **Client-Side Detection**: Uses Media Capabilities API to check browser HEVC support
- **Conditional Transcoding**: Only transcodes when browser cannot play HEVC natively
- **Zero Overhead on Safari/Edge**: Browsers with native HEVC support get original files
- **Automatic Fallback**: Chrome/Firefox automatically get transcoded H.264 version
- **Caching**: Transcoded videos are cached to avoid repeated transcoding
- **Range Request Support**: Maintains HTTP range request support for seeking

### Code Changes

**Backend (Rust):**

1. **src/video_processor.rs**:
   - Added `is_hevc_video()` to detect HEVC codec using ffprobe
   - Added `transcode_hevc_to_h264()` for on-demand transcoding
   - Added `get_transcoded_path()` for cache management
   - Uses `libopenh264` encoder (available on most systems)

2. **src/handlers_video.rs**:
   - Added `transcode` query parameter to `VideoQuery` struct
   - Modified `get_video_file()` to respect client's transcode request
   - Only transcodes when: (1) client requests it AND (2) video is HEVC
   - Serves original HEVC files when client supports it

**Frontend (JavaScript):**

3. **static/js/utils.js**:
   - Added `videoCodecSupport` module with Media Capabilities API integration
   - `canPlayCodec()`: Tests browser support for specific codecs
   - `supportsHEVC()`: Convenience method for HEVC detection
   - Includes fallback to `canPlayType()` for older browsers
   - Caches results to avoid repeated API calls

4. **static/js/viewer.js**:
   - Modified `displayVideo()` to check HEVC support before loading
   - Detects video codec from photo metadata
   - Requests transcode only when browser doesn't support HEVC
   - Logs detection results for debugging

## System Requirements

### For Full HEVC Support

To enable HEVC transcoding, you need FFmpeg with HEVC support:

**On Fedora/RHEL:**

```bash
# Enable RPM Fusion repositories
sudo dnf install https://download1.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm
sudo dnf install https://download1.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-$(rpm -E %fedora).noarch.rpm

# Install full FFmpeg
sudo dnf swap ffmpeg-free ffmpeg --allowerasing
```

**On Debian/Ubuntu:**

```bash
sudo apt install ffmpeg
```

### Verification

Check if your FFmpeg supports HEVC:

```bash
ffmpeg -decoders | grep hevc
ffmpeg -encoders | grep 264
```

You should see:

- HEVC decoder (`hevc`, `hevc_cuvid`, `hevc_qsv`, or `hevc_vaapi`)
- H.264 encoder (`libx264`, `h264_vaapi`, or `libopenh264`)

## Configuration

Set the transcoding cache directory (optional):

```bash
export TRANSCODE_CACHE_DIR=/path/to/cache
```

Default: `/tmp/turbo-pix/transcoded/`

## How It Works

### Flow for HEVC Videos

**On Safari/Edge (HEVC-capable):**

```
1. User clicks video thumbnail
2. Client detects HEVC support → Yes! ✓
3. Request: GET /api/photos/{hash}/video (no transcode param)
4. Server: Serves original HEVC file
5. Result: Instant playback with hardware acceleration
```

**On Chrome/Firefox (No HEVC):**

```
1. User clicks video thumbnail
2. Client detects HEVC support → No ✗
3. Request: GET /api/photos/{hash}/video?transcode=true
4. Server: Checks for cached H.264 version
   - If cached: Serves transcoded file (instant)
   - If not cached: Transcodes on-demand (5-30s), then serves
5. Result: Plays H.264 version
```

### Media Capabilities API Detection

The client-side detection is highly accurate:

```javascript
const config = {
  type: 'file',
  video: {
    contentType: 'video/mp4; codecs="hvc1.1.6.L93.B0"',
    width: 3840,
    height: 2160,
    bitrate: 10000000,
    framerate: 30,
  },
};

const result = await navigator.mediaCapabilities.decodingInfo(config);
// Returns: { supported: boolean, smooth: boolean, powerEfficient: boolean }
```

## Performance Notes

**On HEVC-capable browsers (Safari, Edge with HW support):**

- **Zero overhead**: Original file served directly
- **Hardware decoding**: Maximum performance and battery efficiency
- **No transcoding**: Server CPU not used

**On non-HEVC browsers (Chrome, Firefox):**

- **First playback**: 5-30 seconds for transcoding (one-time cost)
- **Subsequent playback**: Instant (served from cache)
- **Cache location**: `/tmp/turbo-pix/transcoded/` (configurable)
- **Cache cleanup**: Currently manual (future: automatic LRU)

## Testing

Test video codec detection:

```bash
ffprobe -v error -select_streams v:0 -show_entries stream=codec_name -of default=noprint_wrappers=1:nokey=1 your_video.mp4
```

## Future Improvements

1. **Progressive Transcoding**: Start playback while transcoding continues
2. **Cache Management**: Automatic cleanup of old transcoded files
3. **Quality Presets**: User-selectable quality/size tradeoffs
4. **GPU Acceleration**: Use hardware encoders when available (VAAPI, NVENC, QSV)
5. **Client-side Detection**: Show progress indicator during first transcode

## Related Files

- `src/video_processor.rs` - Video processing and transcoding logic
- `src/handlers_video.rs` - HTTP video serving with transcode support
- `src/handlers_thumbnail.rs` - Video thumbnail generation
