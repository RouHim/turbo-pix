# Native-First Video Playback with On-the-Fly Conversion — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every video plays natively whenever the requesting client can decode it (direct or lossless remux); when a conversion is truly required it starts streaming within seconds, stays seekable, and never blocks on a whole-file transcode.

**Architecture:** Three layers change together. (1) *Capability truth*: missing/stale `photos.metadata.video` facts are derived from the file once at decision time and persisted, so the decision stops treating "unknown container" as "unplayable". (2) *Decision*: a pure `plan()` function returns `Direct | StreamRemux | StreamAudio | StreamTranscode` from the resolved record plus the client's declared codecs (video **and** audio). (3) *Delivery*: a new `/video/stream` endpoint pipes fragmented MP4 out of ffmpeg (`-movflags frag_keyframe+empty_moov+default_base_moof`, `-frag_duration 1s`) straight into an HTTP chunked response; the browser appends those chunks to a `MediaSource` `SourceBuffer` and uses `timestampOffset` to map seek-restarts onto the true timeline. The existing whole-file transcode survives only as the cache-fill producer and the "play original anyway" escape hatch.

**Tech Stack:** Rust (warp, tokio, sqlx/SQLite, tokio-util), ffmpeg/ffprobe CLI, Svelte 5 runes, Media Source Extensions (MSE). No new Rust crates, no new npm packages.

**Spec:** `.spec/native-video-playback.md` — read it before starting; the plan argues from it.

## Global Constraints

Copied from the spec; every task implicitly includes this section.

- First frame within **5 seconds** of the play action on a LAN client for any remux/conversion (FR-005), measured at a 1920×1080 viewport.
- Seeking (start / middle / near end) resumes playback within **3 seconds**, and the reported total duration equals the source duration (FR-006).
- No secure-context-only APIs: **MSE + the media element only** — never WebCodecs (`VideoDecoder` needs HTTPS; the LAN origin is plain HTTP).
- In-scope clients: Chrome, Firefox, Edge on Windows, Linux, Android. Safari/iOS/macOS and TV browsers are out of scope.
- Playback never modifies source files; every derived artifact lives in the cache keyed by `hash + file_size + mtime_millis` (FR-013).
- Conversion concurrency stays bounded by `TURBO_PIX_MAX_TRANSCODES`; requests beyond the limit wait in a user-visible state instead of failing, and no retry loop may spawn unbounded encoder processes (FR-011).
- Only the first/default audio track is considered (FR-007); no track-selection UI.
- Zero-byte / `.pending-*` sources keep the current 200-empty + `X-Transcode-Warning: empty` behaviour and never claim a conversion slot (FR-014); photo endpoints and library metadata stay untouched (FR-014).
- Cached conversion/remux results are reused when present but **never gate** the first playback (FR-010).
- Build order is mandatory: `npm run build` before `cargo build --bin turbo-pix` (`build.rs` panics without `dist/`).
- Every new visible string lands in **both** `frontend/src/i18n/en.json` and `frontend/src/i18n/de.json`; `npm run test:i18n` must pass.
- Rust gates: `cargo fmt --check`, `cargo clippy --all-targets -D warnings`; iterator chains over loops; `Result<T, E>` with `?`; no dead code in any commit.
- CSS tokens (no hardcoded values), component-scoped styles, `aria-*` and `prefers-reduced-motion` are first-class.
- One commit per task; no behavioural changes in cleanup commits.

## Review Focus

Failure modes the spec implies whose tests are not obvious; each line names the task that pins it.

1. **Very short sources and boundary seeks** — a file shorter than one fragment, and seeks at the last keyframe or past `duration - 1s`, must produce a valid non-empty stream instead of a deadlock or an empty body. (Task 2 real-ffmpeg test on the 0.3 s `test-data/test_video.mp4`; Task 3 E2E.)
2. **Optimistic capability declaration** (client claims HEVC/AC-3 it cannot really decode) must self-heal to a stronger mode automatically, with one escalation per mode so it can never loop. (Task 6.)
3. **Missing / wrong duration metadata** must still yield a usable timeline: the decision reports a duration derived from the file, and the player falls back to the media element's own `durationchange` when it is absent. (Tasks 1, 2, 3.)
4. **Multiple audio tracks, or none at all** — only the first audio track is mapped, and a video with no audio plays without audio errors in every mode (`-map 0:a:0?` matching nothing). (Tasks 2, 3; fixture in Task 7.)
5. **Saturation plus repeated retries** — simultaneous stream requests must never spawn unbounded ffmpeg processes: live processes are bounded by `TURBO_PIX_MAX_TRANSCODES`, waiters are visible, and a retry storm cannot exceed the bound. (Tasks 2, 4.)

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `src/video_probe.rs` **(new)** | Derive missing capability facts from the file, persist them, return a resolved record. | 1 |
| `src/db.rs` **(modify)** | `Photo::persist_metadata_patch` (RFC 7396 `json_patch`) + `Photo::persist_duration_if_missing`. | 1 |
| `src/video_capability.rs` **(modify)** | Container families, client codec set incl. audio, `plan()` decision engine. | 1, 3 |
| `src/video_stream.rs` **(new)** | ffmpeg fMP4 argument builder, permit-queued spawn, supervising task, output MIME table. | 2 |
| `src/video_processor.rs` **(modify)** | Whole-file transcode args: first-track mapping + audio handling; reusable spawn helper; `ffmpeg_available()` test helper. | 1, 5 |
| `src/handlers_video.rs` **(modify)** | Decision JSON v2, `/video/stream` handler, cached-artifact fast path, cache-fill hook, mode escalation clamp. | 2, 3, 5, 6 |
| `src/handlers_photo.rs` **(modify)** | Route registration for `GET /api/photos/:hash/video/stream`. | 2 |
| `src/lib.rs` **(modify)** | Register `video_probe`, `video_stream`. | 1, 2 |
| `frontend/src/lib/video/msePlayer.js` **(new)** | MSE session: init segment, backpressure pump, `timestampOffset` seek restart, teardown. | 3 |
| `frontend/src/lib/utils.js` **(modify)** | Audio capability probes folded into `getClientCodecsString()`; `getVideoUrl` carries `client`. | 3, 8 |
| `frontend/src/lib/api.js` **(modify)** | `getVideoDecision` passes through `mode`/`mime`/`duration`/`cached`. | 3 |
| `frontend/src/components/PhotoViewer.svelte` **(modify)** | Stream playback, buffering/waiting states, seek, escalation ladder, teardown on close. | 3, 4, 6 |
| `frontend/src/i18n/{en,de}.json` **(modify)** | `video.stream.buffering`, `video.stream.waiting`. | 3 |
| `tests/e2e/setup/global-setup.js` **(modify)** | Seed the new matrix fixtures into the E2E library. | 7 |
| `test-data/*` **(new fixtures)** | Long, MKV, AC-3, AVI/MPEG-4, 10-bit, no-audio, multi-track, moov-at-end videos. | 2, 7 |
| `tests/e2e/specs/video-streaming.e2e.spec.js` **(new)** | First-frame, seek, reuse, saturation, self-heal E2E. | 3, 5, 6, 7 |

Ground rules:

- All work happens inside the feature worktree; run every command from its root.
- Rust test filters: `cargo test <filter>`; Playwright: `npm run test:e2e -- <spec-file>`.
- Rust tests that shell out to ffmpeg follow the existing pattern: guard on fixture + binary availability, mutate env only through `TestEnvGuard`/`EnvVarGuard` (`src/handlers_video.rs:731-800`).

---

### Task 1: Capability record derivation and persistence

**Files:**
- Create: `src/video_probe.rs`
- Modify: `src/lib.rs` (add `pub mod video_probe;` after `pub mod video_processor;`)
- Modify: `src/video_capability.rs` (add `ContainerFamily` after `parse_pix_fmt_bit_depth`, `src/video_capability.rs:14-22`)
- Modify: `src/db.rs` (two methods in `impl Photo`, next to `find_by_hash` at `src/db.rs:431`)
- Modify: `src/video_processor.rs` (extract `pub(crate) fn ffmpeg_available()` from `should_run_video_tests`, `src/video_processor.rs:1187-1220`)
- Test: `#[cfg(test)] mod tests` in `src/video_probe.rs`; one test in `src/db.rs`'s test module

**Interfaces:**
- Consumes: `crate::metadata_extractor::container_from_format_name` (`pub(crate)`, `src/metadata_extractor.rs:56`), `crate::video_capability::parse_pix_fmt_bit_depth` (`src/video_capability.rs:14`), `crate::video_processor::{get_ffprobe_path, has_moov_at_start, ffmpeg_available}`.
- Produces:
  - `pub struct ResolvedCapabilities { codec: String, container: Option<String>, family: ContainerFamily, bit_depth: Option<u32>, audio_codec: Option<String>, moov_at_start: bool, duration_secs: Option<f64>, probed: bool }`
  - `pub fn record_is_complete(photo: &Photo) -> bool`
  - `pub async fn resolve(pool: &DbPool, photo: &Photo) -> ResolvedCapabilities`
  - `pub(crate) fn parse_capabilities_from_ffprobe(parsed: &serde_json::Value) -> CapabilityPatch`
  - `pub const CAPABILITY_VERSION: u64 = 1;`
  - `Photo::persist_metadata_patch(pool, hash, patch) -> Result<(), sqlx::Error>`
  - `Photo::persist_duration_if_missing(pool, hash, duration_secs) -> Result<(), sqlx::Error>`
  - `ContainerFamily::{from_record, has_moov_layout}`

- [ ] **Step 1: Add `ContainerFamily` to `src/video_capability.rs`**

```rust
/// Container family after normalising the ffprobe `format_name` token and the
/// file extension. ffprobe reports `"matroska,webm"` for BOTH `.mkv` and
/// `.webm`, so the extension is the only reliable discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerFamily {
    Mp4,
    Matroska,
    Webm,
    Avi,
    MpegTs,
    Ogg,
    Other,
}

impl ContainerFamily {
    pub fn from_record(container: Option<&str>, file_name: &str) -> Self {
        let ext = file_name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match container.unwrap_or_default().to_ascii_lowercase().as_str() {
            "mp4" | "mov" | "m4v" | "m4a" | "3gp" | "3g2" | "mj2" => Self::Mp4,
            "webm" => Self::Webm,
            "matroska" | "mkv" => {
                if ext == "webm" {
                    Self::Webm
                } else {
                    Self::Matroska
                }
            }
            "avi" => Self::Avi,
            "mpegts" => Self::MpegTs,
            "ogg" => Self::Ogg,
            _ => match ext.as_str() {
                "mp4" | "mov" | "m4v" => Self::Mp4,
                "mkv" => Self::Matroska,
                "webm" => Self::Webm,
                "avi" => Self::Avi,
                "ts" | "m2ts" => Self::MpegTs,
                "ogv" | "ogg" => Self::Ogg,
                _ => Self::Other,
            },
        }
    }

    /// Matroska-family containers carry no `moov` atom; only MP4-family files
    /// have a progressive-playback layout that can be wrong.
    pub fn has_moov_layout(self) -> bool {
        matches!(self, Self::Mp4)
    }
}
```

- [ ] **Step 2: Write the failing tests for the ffprobe parser**

Create `src/video_probe.rs` with the tests below plus the stubs they need to compile (`CapabilityPatch`, `parse_capabilities_from_ffprobe` returning `CapabilityPatch::default()`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ffprobe_json() -> Value {
        json!({
            "format": { "format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "12.5" },
            "streams": [
                { "codec_type": "video", "codec_name": "h264", "pix_fmt": "yuv420p",
                  "disposition": { "attached_pic": 0 } },
                { "codec_type": "audio", "codec_name": "ac3",
                  "disposition": { "attached_pic": 0 } }
            ]
        })
    }

    #[test]
    fn parses_container_codec_bitdepth_audio_and_duration() {
        let patch = parse_capabilities_from_ffprobe(&ffprobe_json());
        assert_eq!(patch.codec.as_deref(), Some("h264"));
        assert_eq!(patch.container.as_deref(), Some("mov"));
        assert_eq!(patch.bit_depth, Some(8));
        assert_eq!(patch.audio_codec.as_deref(), Some("ac3"));
        assert_eq!(patch.duration_secs, Some(12.5));
    }

    #[test]
    fn skips_attached_cover_pictures_and_higher_bit_depths() {
        let parsed = json!({
            "format": { "format_name": "matroska,webm" },
            "streams": [
                { "codec_type": "video", "codec_name": "mjpeg",
                  "disposition": { "attached_pic": 1 } },
                { "codec_type": "video", "codec_name": "h264", "pix_fmt": "yuv420p10le",
                  "disposition": { "attached_pic": 0 } }
            ]
        });
        let patch = parse_capabilities_from_ffprobe(&parsed);
        assert_eq!(patch.codec.as_deref(), Some("h264"));
        assert_eq!(patch.bit_depth, Some(10));
        assert_eq!(patch.audio_codec, None);
        assert_eq!(patch.duration_secs, None);
    }

    #[test]
    fn container_family_discriminates_mkv_from_webm() {
        assert_eq!(ContainerFamily::from_record(Some("matroska"), "a.mkv"), ContainerFamily::Matroska);
        assert_eq!(ContainerFamily::from_record(Some("matroska"), "a.webm"), ContainerFamily::Webm);
        assert_eq!(ContainerFamily::from_record(Some("mov"), "a.mov"), ContainerFamily::Mp4);
        assert_eq!(ContainerFamily::from_record(None, "a.mp4"), ContainerFamily::Mp4);
        assert_eq!(ContainerFamily::from_record(None, "a.avi"), ContainerFamily::Avi);
        assert_eq!(ContainerFamily::from_record(Some("weird"), "a.bin"), ContainerFamily::Other);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib video_probe`
Expected: FAIL — `codec`/`container`/`bit_depth` are `None` because the parser is a stub.

- [ ] **Step 4: Implement the parser and the resolver**

```rust
//! Serve-time derivation and persistence of a video's playback capability
//! record.
//!
//! Legacy rows (indexed before the capability extractor existed) carry no
//! container / bit-depth / moov facts. Deciding from those records routes every
//! video into conversion — the dominant defect the spec targets. This module
//! probes the file once, persists what it learns into `photos.metadata.video`,
//! and returns the resolved facts.

use std::path::Path;

use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::db::Photo;
use crate::metadata_extractor::container_from_format_name;
use crate::video_capability::{parse_pix_fmt_bit_depth, ContainerFamily};
use crate::video_processor::{get_ffprobe_path, has_moov_at_start};

/// Bumped whenever the set of persisted capability facts changes; its presence
/// is the "record complete, never probe again" marker.
pub const CAPABILITY_VERSION: u64 = 1;

#[derive(Debug, Default, PartialEq)]
pub struct CapabilityPatch {
    pub codec: Option<String>,
    pub container: Option<String>,
    pub bit_depth: Option<u32>,
    pub audio_codec: Option<String>,
    pub duration_secs: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCapabilities {
    pub codec: String,
    pub container: Option<String>,
    pub family: ContainerFamily,
    pub bit_depth: Option<u32>,
    pub audio_codec: Option<String>,
    pub moov_at_start: bool,
    pub duration_secs: Option<f64>,
    /// True when this call had to probe the file (incomplete record).
    pub probed: bool,
}

/// Complete = a previous probe/scan wrote the version marker and a codec.
/// An absent `moov_at_start` key is "never probed", NOT "true".
pub fn record_is_complete(photo: &Photo) -> bool {
    let Some(video) = photo.metadata.get("video") else {
        return false;
    };
    video.get("capability_version").and_then(Value::as_u64) == Some(CAPABILITY_VERSION)
        && video
            .get("codec")
            .and_then(Value::as_str)
            .is_some_and(|c| !c.is_empty())
}

/// First non-attached video stream and first non-attached audio stream — the
/// only streams the playback decision may consider (spec: first/default audio
/// track only).
pub(crate) fn parse_capabilities_from_ffprobe(parsed: &Value) -> CapabilityPatch {
    let stream_field = |kind: &str, field: &str| {
        parsed["streams"]
            .as_array()?
            .iter()
            .find(|s| {
                s["codec_type"].as_str() == Some(kind)
                    && s["disposition"]["attached_pic"].as_i64() != Some(1)
            })
            .and_then(|s| s[field].as_str())
            .map(str::to_string)
    };

    CapabilityPatch {
        codec: stream_field("video", "codec_name"),
        container: container_from_format_name(parsed),
        bit_depth: parse_pix_fmt_bit_depth(stream_field("video", "pix_fmt").as_deref()),
        audio_codec: stream_field("audio", "codec_name"),
        duration_secs: parsed["format"]["duration"]
            .as_str()
            .and_then(|d| d.parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d > 0.0),
    }
}

/// Blocking probe: one ffprobe pass plus (MP4-family only) the moov layout
/// probe. `None` when ffprobe cannot run or emits no JSON.
fn probe_file(path: &Path) -> Option<(CapabilityPatch, bool)> {
    let output = std::process::Command::new(get_ffprobe_path())
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let parsed: Value = serde_json::from_slice(&output.stdout).ok()?;
    let patch = parse_capabilities_from_ffprobe(&parsed);
    let family = ContainerFamily::from_record(
        patch.container.as_deref(),
        path.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
    );
    let moov_at_start = if family.has_moov_layout() {
        has_moov_at_start(path).unwrap_or(true)
    } else {
        true
    };
    Some((patch, moov_at_start))
}

impl ResolvedCapabilities {
    pub fn from_record(photo: &Photo) -> Self {
        Self {
            codec: photo.video_codec().unwrap_or_default().to_string(),
            container: photo.container().map(str::to_string),
            family: ContainerFamily::from_record(photo.container(), &photo.filename),
            bit_depth: photo.bit_depth(),
            audio_codec: photo.audio_codec().map(str::to_string),
            moov_at_start: photo.moov_at_start(),
            duration_secs: photo.duration.filter(|d| *d > 0.0),
            probed: false,
        }
    }

    fn merged(photo: &Photo, patch: &CapabilityPatch, moov_at_start: bool) -> Self {
        let container = patch
            .container
            .clone()
            .or_else(|| photo.container().map(str::to_string));
        Self {
            codec: patch
                .codec
                .clone()
                .unwrap_or_else(|| photo.video_codec().unwrap_or_default().to_string()),
            family: ContainerFamily::from_record(container.as_deref(), &photo.filename),
            container,
            bit_depth: patch.bit_depth.or_else(|| photo.bit_depth()),
            audio_codec: patch
                .audio_codec
                .clone()
                .or_else(|| photo.audio_codec().map(str::to_string)),
            moov_at_start,
            duration_secs: patch
                .duration_secs
                .or_else(|| photo.duration.filter(|d| *d > 0.0)),
            probed: true,
        }
    }
}

/// Resolve the capability facts for one request. Complete records are used
/// as-is (no filesystem access); incomplete records are probed once and the
/// result is persisted, so later decisions are cheap.
pub async fn resolve(pool: &SqlitePool, photo: &Photo) -> ResolvedCapabilities {
    if record_is_complete(photo) {
        return ResolvedCapabilities::from_record(photo);
    }

    let path = Path::new(&photo.file_path).to_path_buf();
    let probed = tokio::task::spawn_blocking(move || probe_file(&path))
        .await
        .ok()
        .flatten();

    let Some((patch, moov_at_start)) = probed else {
        log::warn!(
            "Capability probe failed; deciding from the stored record only: {}",
            photo.filename
        );
        return ResolvedCapabilities::from_record(photo);
    };

    let mut video = serde_json::Map::new();
    video.insert("capability_version".to_string(), json!(CAPABILITY_VERSION));
    if let Some(codec) = &patch.codec {
        video.insert("codec".to_string(), json!(codec));
    }
    if let Some(container) = &patch.container {
        video.insert("container".to_string(), json!(container));
    }
    if let Some(bit_depth) = patch.bit_depth {
        video.insert("bit_depth".to_string(), json!(bit_depth));
    }
    // Explicit null documents "asked the file, there is no audio track" — which
    // is different from "never probed".
    video.insert("audio_codec".to_string(), json!(patch.audio_codec));
    video.insert("moov_at_start".to_string(), json!(moov_at_start));

    if let Err(e) =
        Photo::persist_metadata_patch(pool, &photo.hash_sha256, &json!({ "video": video })).await
    {
        log::warn!("Persisting derived video capabilities failed: {e}");
    }
    if let Some(duration) = patch.duration_secs {
        if let Err(e) =
            Photo::persist_duration_if_missing(pool, &photo.hash_sha256, duration).await
        {
            log::warn!("Persisting derived video duration failed: {e}");
        }
    }

    ResolvedCapabilities::merged(photo, &patch, moov_at_start)
}
```

- [ ] **Step 5: Add the persistence methods to `impl Photo` in `src/db.rs`**

```rust
    /// Merge-patch the stored `photos.metadata` JSON in one statement (SQLite
    /// `json_patch` implements RFC 7396), so concurrent capability writes
    /// cannot tear each other's metadata and unrelated keys survive untouched.
    pub async fn persist_metadata_patch(
        pool: &DbPool,
        hash: &str,
        patch: &serde_json::Value,
    ) -> Result<(), sqlx::Error> {
        let patch = patch.to_string();
        sqlx::query("UPDATE photos SET metadata = json_patch(metadata, ?1) WHERE hash_sha256 = ?2")
            .bind(patch)
            .bind(hash)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Fill in a duration the indexer never captured; never overwrites a
    /// positive stored value.
    pub async fn persist_duration_if_missing(
        pool: &DbPool,
        hash: &str,
        duration_secs: f64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE photos SET duration = ?1 \
             WHERE hash_sha256 = ?2 AND (duration IS NULL OR duration <= 0)",
        )
        .bind(duration_secs)
        .bind(hash)
        .execute(pool)
        .await?;
        Ok(())
    }
```

Test in `src/db.rs`'s test module (build the `Photo` literal inline like `handlers_video.rs:800-836` if no factory exists):

```rust
    #[tokio::test]
    async fn persist_metadata_patch_merges_without_clobbering() {
        let pool = create_in_memory_pool().await.expect("pool");
        let mut photo = test_photo("hash-patch");
        photo.metadata = json!({ "camera": { "make": "Canon" } });
        photo.create(&pool).await.expect("create");

        Photo::persist_metadata_patch(
            &pool,
            "hash-patch",
            &json!({ "video": { "capability_version": 1, "codec": "h264" } }),
        )
        .await
        .expect("patch");

        let stored = Photo::find_by_hash(&pool, "hash-patch").await.unwrap().unwrap();
        assert_eq!(stored.metadata["camera"]["make"], "Canon");
        assert_eq!(stored.metadata["video"]["codec"], "h264");
        assert_eq!(stored.metadata["video"]["capability_version"], 1);
    }
```

- [ ] **Step 6: Add `ffmpeg_available()` and the end-to-end resolve test**

Extract from `should_run_video_tests` (`src/video_processor.rs:1187-1220`) so both share one PATH scan:

```rust
/// True when both ffmpeg and ffprobe are runnable. Test helper — production
/// startup already fails fast via `verify_ffmpeg_available`.
pub(crate) fn ffmpeg_available() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|bin| {
        std::process::Command::new(bin)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}
```

```rust
    /// Minimal `Photo` row for resolver tests; mirrors the literal used by the
    /// `handlers_video` test module (`src/handlers_video.rs:800-836`).
    fn test_photo(hash: &str) -> Photo {
        Photo {
            hash_sha256: hash.to_string(),
            file_path: String::new(),
            filename: "video.mp4".to_string(),
            file_size: 0,
            mime_type: Some("video/mp4".to_string()),
            taken_at: None,
            width: None,
            height: None,
            orientation: None,
            duration: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: None,
            is_favorite: Some(false),
            semantic_vector_indexed: Some(false),
            metadata: json!({}),
            date_modified: chrono::Utc::now(),
            date_indexed: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn resolve_derives_and_persists_legacy_records_once() {
        let fixture = Path::new("test-data/test_video.mp4");
        if !fixture.exists() || !crate::video_processor::ffmpeg_available() {
            eprintln!("skipping: fixture or ffmpeg unavailable");
            return;
        }
        // Real ffprobe runs here: hold the shared env lock so another test
        // module's fake FFPROBE_PATH cannot break this test (codebase
        // convention; see src/metadata_extractor.rs:801-803).
        let _env_lock = crate::video_processor::tests::acquire_test_env_lock();
        let pool = crate::db::create_in_memory_pool().await.expect("pool");
        let mut photo = test_photo("hash-resolve");
        photo.file_path = fixture.to_string_lossy().into_owned();
        photo.filename = "test_video.mp4".to_string();
        photo.metadata = json!({});
        photo.create(&pool).await.expect("create");

        let first = resolve(&pool, &photo).await;
        assert!(first.probed, "an incomplete record must probe the file");
        assert_eq!(first.codec, "h264");
        assert_eq!(first.container.as_deref(), Some("mov"));
        assert_eq!(first.bit_depth, Some(8));
        assert!(first.moov_at_start);
        assert!(first.duration_secs.is_some_and(|d| d > 0.0));

        let stored = Photo::find_by_hash(&pool, "hash-resolve").await.unwrap().unwrap();
        assert_eq!(stored.metadata["video"]["capability_version"], 1);
        assert!(stored.duration.is_some_and(|d| d > 0.0));

        let second = resolve(&pool, &stored).await;
        assert!(!second.probed, "a complete record must not probe again");
        assert_eq!(second.codec, "h264");
    }
```

- [ ] **Step 7: Run the tests**

Run: `cargo test --lib video_probe && cargo test --lib persist_metadata_patch && cargo test --lib video_capability`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/video_probe.rs src/lib.rs src/db.rs src/video_capability.rs src/video_processor.rs
git commit -m "feat(video): derive and persist missing playback capability facts"
```

---

### Task 2: Streaming endpoint — fragmented MP4 out of ffmpeg

**Files:**
- Create: `src/video_stream.rs`
- Modify: `src/lib.rs` (add `pub mod video_stream;`)
- Modify: `src/handlers_video.rs` (`StreamQuery`, `stream_video`)
- Modify: `src/handlers_photo.rs` (register the route with the other video routes, `src/handlers_photo.rs:1171-1198`)
- Create fixtures: `test-data/test_video_long.mp4`, `test-data/test_video_long.mkv`, `test-data/test_video_ac3.mp4`
- Test: `#[cfg(test)] mod tests` in `src/video_stream.rs`; handler tests in `src/handlers_video.rs`

**Interfaces:**
- Consumes: `crate::video_processor::{acquire_transcode_permit, get_ffmpeg_path, transcode_timeout_secs, format_binary_error}`.
- Produces:
  - `pub enum StreamMode { Remux, Audio, Transcode }` with `as_str() -> &'static str`, `from_query(&str) -> Option<Self>`
  - `pub fn build_args(mode: StreamMode, input: &Path, start_secs: f64) -> Vec<String>`
  - `pub const STREAM_QUEUE_WAIT_SECS_DEFAULT: u64 = 20;` / `pub fn stream_queue_wait_secs() -> u64` (env `TURBO_PIX_STREAM_QUEUE_WAIT_SECS`)
  - `pub enum StreamStartError { Busy, Disabled, Spawn(String) }`
  - `pub struct StreamHandle { mode, stdout, stderr, child, permit }`
  - `pub async fn start_stream(mode, input: &Path, start_secs: f64) -> Result<StreamHandle, StreamStartError>`
  - `pub async fn supervise(child: Child, stderr: ChildStderr) -> Result<(), String>`
  - `pub fn output_mime(mode: StreamMode, video_codec: &str, audio_codec: Option<&str>) -> String` — derived from the codecs the run actually emits (Transcode always H.264+AAC; Audio copies the video codec and re-encodes audio to AAC; Remux copies both)
  - `pub async fn stream_video(photo_hash: String, query: StreamQuery, headers: HeaderMap, db_pool: DbPool) -> Result<Box<dyn Reply>, Rejection>`

Why this shape: `ffmpeg -ss <start> -i <in> … -movflags frag_keyframe+empty_moov+default_base_moof -frag_duration 1000000 -f mp4 pipe:1` writes `ftyp`+`moov` immediately and never rewinds; MSE appends the chunks with `SourceBuffer.timestampOffset = start`. Verified end-to-end in Chromium against a 60 s fixture: 1.1–1.3 s to first frame, seek-to-35 s resumed in 1.1 s, and this held for transcode, MKV→MP4 copy-remux and AC-3→AAC audio modes. ffmpeg rebases output timestamps to 0 (checked in the moof `tfdt` boxes), which is exactly why the seek mapping lives in the client's `timestampOffset` and not in ffmpeg timestamp flags.

- [ ] **Step 1: Create the fixtures**

```bash
# 20s 320x180 h264+aac progressive MP4 — seek/reuse tests
ffmpeg -y -f lavfi -i testsrc2=size=320x180:rate=24:duration=20 \
  -f lavfi -i sine=frequency=440:duration=20 \
  -c:v libx264 -preset ultrafast -g 48 -pix_fmt yuv420p -c:a aac -shortest \
  -movflags +faststart test-data/test_video_long.mp4

# same content in Matroska — remux-stream tests
ffmpeg -y -i test-data/test_video_long.mp4 -c copy test-data/test_video_long.mkv

# h264 video + AC-3 audio, MP4 — audio-only conversion tests
ffmpeg -y -i test-data/test_video_long.mp4 -c:v copy -c:a ac3 -b:a 192k \
  test-data/test_video_ac3.mp4
```

Commit all three (≈300 KB each). The commands are repeated in a comment header in `tests/e2e/specs/video-streaming.e2e.spec.js` when it lands in Task 3.

- [ ] **Step 2: Write the failing `build_args` / mode tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcode_args_reencode_into_fragmented_mp4() {
        let joined = build_args(StreamMode::Transcode, Path::new("/in.mp4"), 0.0).join(" ");
        assert!(joined.contains("-c:v libx264"));
        assert!(joined.contains("-preset veryfast"));
        assert!(joined.contains("-g 48"));
        assert!(joined.contains("-c:a aac"));
        assert!(joined.contains("-map 0:v:0 -map 0:a:0?"));
        assert!(joined.contains("-movflags frag_keyframe+empty_moov+default_base_moof"));
        assert!(joined.contains("-frag_duration 1000000"));
        assert!(joined.ends_with("-f mp4 pipe:1"));
        assert!(!joined.contains("-ss"), "start 0 must not emit a seek flag");
    }

    #[test]
    fn audio_mode_copies_video_and_reencodes_audio() {
        let joined = build_args(StreamMode::Audio, Path::new("/in.mp4"), 12.5).join(" ");
        assert!(joined.contains("-ss 12.500 -i /in.mp4"));
        assert!(joined.contains("-c:v copy"));
        assert!(joined.contains("-c:a aac"));
        assert!(!joined.contains("libx264"));
    }

    #[test]
    fn remux_mode_copies_everything() {
        let joined = build_args(StreamMode::Remux, Path::new("/in.mkv"), 3.0).join(" ");
        assert!(joined.contains("-c copy"));
        assert!(!joined.contains("libx264"));
    }

    #[test]
    fn mode_parsing_rejects_unknown_tokens() {
        assert_eq!(StreamMode::from_query("remux"), Some(StreamMode::Remux));
        assert_eq!(StreamMode::from_query("audio"), Some(StreamMode::Audio));
        assert_eq!(StreamMode::from_query("transcode"), Some(StreamMode::Transcode));
        assert_eq!(StreamMode::from_query("direct"), None);
        assert_eq!(StreamMode::from_query(""), None);
    }

    #[test]
    fn output_mime_follows_the_codecs_the_run_emits() {
        // Transcode always re-encodes: H.264 + AAC.
        assert_eq!(
            output_mime(StreamMode::Transcode, "hevc", Some("ac3")),
            "video/mp4; codecs=\"avc1.42E01E,mp4a.40.2\""
        );
        // Remux copies the video codec verbatim.
        assert_eq!(
            output_mime(StreamMode::Remux, "hevc", Some("aac")),
            "video/mp4; codecs=\"hvc1.1.6.L93.B0,mp4a.40.2\""
        );
        // Audio mode copies the video codec but re-encodes audio to AAC.
        assert_eq!(
            output_mime(StreamMode::Audio, "hevc", Some("ac3")),
            "video/mp4; codecs=\"hvc1.1.6.L93.B0,mp4a.40.2\""
        );
        // Remux copies BOTH: the MIME must name the copied audio codec.
        assert_eq!(
            output_mime(StreamMode::Remux, "h264", Some("ac3")),
            "video/mp4; codecs=\"avc1.42E01E,ac-3\""
        );
        // A silent source (`-map 0:a:0?` matches nothing) must declare a
        // video-only codec string, or the client's SourceBuffer rejects the
        // init segment.
        assert_eq!(
            output_mime(StreamMode::Transcode, "h264", None),
            "video/mp4; codecs=\"avc1.42E01E\""
        );
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test --lib video_stream --no-run`
Expected: compile error — module/type not defined.

- [ ] **Step 4: Implement `src/video_stream.rs`**

```rust
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::sync::SemaphorePermit;

use crate::video_processor::{acquire_transcode_permit, format_binary_error, get_ffmpeg_path, transcode_timeout_secs};

pub const STREAM_QUEUE_WAIT_SECS_DEFAULT: u64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMode {
    Remux,
    Audio,
    Transcode,
}

impl StreamMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Remux => "remux",
            Self::Audio => "audio",
            Self::Transcode => "transcode",
        }
    }

    pub fn from_query(value: &str) -> Option<Self> {
        match value {
            "remux" => Some(Self::Remux),
            "audio" => Some(Self::Audio),
            "transcode" => Some(Self::Transcode),
            _ => None,
        }
    }
}

/// How long a stream request waits for a conversion slot before answering
/// 503 + Retry-After (the client keeps its waiting state visible and retries).
/// Slots are `TURBO_PIX_MAX_TRANSCODES` permits, so waiters can never become
/// unbounded ffmpeg processes.
pub fn stream_queue_wait_secs() -> u64 {
    match std::env::var("TURBO_PIX_STREAM_QUEUE_WAIT_SECS") {
        Ok(raw) => raw.trim().parse::<u64>().unwrap_or_else(|_| {
            log::warn!("Invalid TURBO_PIX_STREAM_QUEUE_WAIT_SECS '{raw}', using default");
            STREAM_QUEUE_WAIT_SECS_DEFAULT
        }),
        Err(_) => STREAM_QUEUE_WAIT_SECS_DEFAULT,
    }
}

/// ffmpeg arguments for one stream run.
///
/// `-ss` is an *input* option (fast seek); ffmpeg rebases the output timeline
/// to zero and the client maps it back with `SourceBuffer.timestampOffset`.
/// Fragmented output flushes `ftyp`+`moov` before the first frame, so the
/// client can create its SourceBuffer immediately; `-frag_duration 1000000`
/// keeps fragments at ≤1 s even for stream copies whose keyframes are far
/// apart. `-map 0:a:0?` tolerates sources with no audio track.
pub fn build_args(mode: StreamMode, input: &Path, start_secs: f64) -> Vec<String> {
    let mut args: Vec<String> = vec!["-v".into(), "error".into(), "-nostdin".into()];
    if mode == StreamMode::Transcode {
        args.extend(["-hwaccel".into(), "auto".into()]);
    }
    if start_secs > 0.0 {
        args.extend(["-ss".into(), format!("{start_secs:.3}")]);
    }
    args.extend(["-i".into(), input.to_string_lossy().into_owned()]);
    args.extend(["-map".into(), "0:v:0".into(), "-map".into(), "0:a:0?".into()]);
    match mode {
        StreamMode::Transcode => args.extend(
            [
                "-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p",
                "-profile:v", "main", "-g", "48", "-keyint_min", "48", "-sc_threshold", "0",
                "-c:a", "aac", "-b:a", "160k", "-ac", "2",
            ]
            .iter()
            .map(|s| s.to_string()),
        ),
        StreamMode::Audio => args.extend(
            ["-c:v", "copy", "-c:a", "aac", "-b:a", "160k", "-ac", "2"]
                .iter()
                .map(|s| s.to_string()),
        ),
        StreamMode::Remux => args.extend(["-c", "copy"].iter().map(|s| s.to_string())),
    }
    args.extend(
        [
            "-movflags",
            "frag_keyframe+empty_moov+default_base_moof+omit_tfhd_offset",
            "-frag_duration",
            "1000000",
            "-f",
            "mp4",
            "pipe:1",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    args
}

/// Output MIME for the SourceBuffer the client must create — derived from the
/// codecs this run actually emits, never from the mode alone. Transcode always
/// emits H.264+AAC; Audio mode copies the video codec and re-encodes audio to
/// AAC; Remux copies both, so a wrong MIME would make the client drop exactly
/// the track it declared support for.
pub fn output_mime(mode: StreamMode, video_codec: &str, audio_codec: Option<&str>) -> String {
    let video = match (mode, video_codec) {
        (StreamMode::Transcode, _) => "avc1.42E01E",
        (_, "hevc") => "hvc1.1.6.L93.B0",
        (_, "av1") => "av01.0.15M.08",
        (_, "vp9") => "vp09.00.10.08",
        (_, "vp8") => "vp08.00.10.08",
        _ => "avc1.42E01E",
    };
    let audio = match (mode, audio_codec) {
        // `-map 0:a:0?` emits no audio for a silent source: declaring one here
        // would make the client's SourceBuffer reject the init segment
        // ("Initialization segment misses expected aac track").
        (_, None | Some("")) => None,
        (StreamMode::Transcode | StreamMode::Audio, _) | (_, Some("aac")) => Some("mp4a.40.2"),
        (_, Some("opus")) => Some("opus"),
        (_, Some("mp3")) => Some("mp4a.6B"),
        (_, Some("ac3")) => Some("ac-3"),
        (_, Some("eac3")) => Some("ec-3"),
        (_, Some("dts")) => Some("dts"),
        (_, Some("flac")) => Some("flac"),
        (_, Some(_)) => Some("mp4a.40.2"),
    };
    match audio {
        Some(audio) => format!("video/mp4; codecs=\"{video},{audio}\""),
        None => format!("video/mp4; codecs=\"{video}\""),
    }
}

#[derive(Debug)]
pub enum StreamStartError {
    /// No conversion slot within the queue wait.
    Busy,
    /// `TURBO_PIX_MAX_TRANSCODES=0`.
    Disabled,
    Spawn(String),
}

#[derive(Debug)]
pub struct StreamHandle {
    pub mode: StreamMode,
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
    pub child: Child,
    pub permit: SemaphorePermit<'static>,
}

pub async fn start_stream(
    mode: StreamMode,
    input: &Path,
    start_secs: f64,
) -> Result<StreamHandle, StreamStartError> {
    let permit = match tokio::time::timeout(
        Duration::from_secs(stream_queue_wait_secs()),
        acquire_transcode_permit(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        Ok(Err(_disabled)) => return Err(StreamStartError::Disabled),
        Err(_) => return Err(StreamStartError::Busy),
    };

    let ffmpeg = get_ffmpeg_path();
    let mut child = Command::new(&ffmpeg)
        .args(build_args(mode, input, start_secs))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| StreamStartError::Spawn(format_binary_error("ffmpeg", &ffmpeg, &e)))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| StreamStartError::Spawn("ffmpeg stdout pipe unavailable".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| StreamStartError::Spawn("ffmpeg stderr pipe unavailable".to_string()))?;

    Ok(StreamHandle { mode, stdout, stderr, child, permit })
}

/// Watch one ffmpeg child to completion. Killing at the deadline keeps a stuck
/// encoder from pinning a slot forever. Dropping the response body closes
/// stdout, which makes ffmpeg exit on EPIPE — that is what bounds client
/// disconnects.
pub async fn supervise(mut child: Child, stderr: ChildStderr) -> Result<(), String> {
    use tokio::io::AsyncReadExt;

    let stderr_task = tokio::spawn(async move {
        let mut reader = stderr;
        let mut buf = String::new();
        let _ = reader.read_to_string(&mut buf).await;
        buf
    });

    let deadline = Duration::from_secs(transcode_timeout_secs());
    let status = match tokio::time::timeout(deadline, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            let _ = child.kill().await;
            let _ = stderr_task.await;
            return Err(format!("ffmpeg wait failed: {e}"));
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = stderr_task.await;
            return Err(format!("stream timed out after {}s", deadline.as_secs()));
        }
    };
    let stderr_text = stderr_task.await.unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    let tail = stderr_text
        .lines()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ");
    Err(format!("ffmpeg exited with status {status}: {tail}"))
}
```

- [ ] **Step 5: Run the unit tests**

Run: `cargo test --lib video_stream`
Expected: PASS (5 tests).

- [ ] **Step 6: Add the real-ffmpeg integration tests**

```rust
    #[tokio::test]
    async fn streams_hevc_fixture_as_fragmented_mp4() {
        // Real ffmpeg is spawned: hold the shared env lock (codebase
        // convention) so another module's fake FFMPEG_PATH cannot break it.
        let _lock = crate::video_processor::tests::acquire_test_env_lock();
        let fixture = Path::new("test-data/test_video_hevc.mp4");
        if !fixture.exists() || !crate::video_processor::ffmpeg_available() {
            eprintln!("skipping: fixture or ffmpeg unavailable");
            return;
        }
        use tokio::io::AsyncReadExt;

        let handle = start_stream(StreamMode::Transcode, fixture, 0.0)
            .await
            .expect("stream must start");
        let mut head = vec![0u8; 64 * 1024];
        let read = handle.stdout.read(&mut head).await.expect("read head");
        head.truncate(read);
        assert!(head.windows(4).any(|w| w == b"ftyp"), "fMP4 must start with ftyp");
        assert!(head.windows(4).any(|w| w == b"moov"), "empty_moov must be written up front");
        let _ = supervise(handle.child, handle.stderr).await;
    }

    #[tokio::test]
    async fn boundary_seek_on_a_sub_second_fixture_still_produces_bytes() {
        let _lock = crate::video_processor::tests::acquire_test_env_lock();
        let fixture = Path::new("test-data/test_video.mp4"); // 0.3 s
        if !fixture.exists() || !crate::video_processor::ffmpeg_available() {
            return;
        }
        use tokio::io::AsyncReadExt;

        let handle = start_stream(StreamMode::Transcode, fixture, 0.2)
            .await
            .expect("a seek near the end must still start");
        let mut buf = vec![0u8; 4096];
        let read = handle.stdout.read(&mut buf).await.expect("read");
        assert!(read > 0, "boundary seeks must produce a valid stream head");
        let _ = supervise(handle.child, handle.stderr).await;
    }

    #[tokio::test]
    async fn remux_stream_from_matroska_produces_mp4_fragments() {
        let _lock = crate::video_processor::tests::acquire_test_env_lock();
        let fixture = Path::new("test-data/test_video_long.mkv");
        if !fixture.exists() || !crate::video_processor::ffmpeg_available() {
            return;
        }
        use tokio::io::AsyncReadExt;

        let handle = start_stream(StreamMode::Remux, fixture, 5.0)
            .await
            .expect("remux stream must start");
        let mut head = vec![0u8; 64 * 1024];
        let read = handle.stdout.read(&mut head).await.expect("read head");
        head.truncate(read);
        assert!(head.windows(4).any(|w| w == b"ftyp"));
        assert!(head.windows(4).any(|w| w == b"moov"));
        let _ = supervise(handle.child, handle.stderr).await;
    }
```

- [ ] **Step 7: Run the integration tests**

Run: `cargo test --lib video_stream -- --nocapture`
Expected: PASS.

- [ ] **Step 8: Add the HTTP handler and route**

In `src/handlers_video.rs`:

```rust
#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    /// Seconds into the source; the client restarts the stream here on seek.
    pub start: Option<f64>,
    /// Requested mode. Task 3 derives it from the decision; Task 6 lets the
    /// client escalate only.
    pub mode: Option<String>,
    /// Client codec declaration, same tokens as `?client=` on the decision.
    pub client: Option<String>,
}

/// Upper bound for `?start=` so a hostile client cannot ask for an absurd
/// offset; sources longer than a day are out of scope.
const MAX_STREAM_START_SECS: f64 = 86_400.0;

pub async fn stream_video(
    photo_hash: String,
    query: StreamQuery,
    headers: HeaderMap,
    db_pool: DbPool,
) -> Result<Box<dyn Reply>, Rejection> {
    let photo = match Photo::find_by_hash(&db_pool, &photo_hash).await {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {e}"),
            }))
        }
    };

    let source = Path::new(&photo.file_path);
    let source_size = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);
    if source_size == 0 {
        let response = warp::reply::with_status(Vec::<u8>::new(), StatusCode::OK);
        let response = warp::reply::with_header(response, "content-length", "0");
        let response = warp::reply::with_header(response, "x-transcode-warning", "empty");
        return Ok(Box::new(response));
    }

    let requested = query.mode.as_deref().unwrap_or("transcode");
    let Some(mode) = StreamMode::from_query(requested) else {
        return Err(reject::custom(NotFoundError));
    };
    let start = query.start.unwrap_or(0.0).clamp(0.0, MAX_STREAM_START_SECS);

    let handle = match start_stream(mode, source, start).await {
        Ok(handle) => handle,
        Err(StreamStartError::Busy) => {
            let response = warp::reply::with_status(
                warp::reply::json(&json!({ "error": "no conversion slot available" })),
                StatusCode::SERVICE_UNAVAILABLE,
            );
            return Ok(Box::new(warp::reply::with_header(response, "retry-after", "2")));
        }
        Err(StreamStartError::Disabled) => {
            let response = warp::reply::with_status(
                warp::reply::json(&json!({ "error": "conversion disabled" })),
                StatusCode::SERVICE_UNAVAILABLE,
            );
            return Ok(Box::new(warp::reply::with_header(response, "retry-after", "5")));
        }
        Err(StreamStartError::Spawn(message)) => {
            log::error!("Stream spawn failed: {message}");
            return Ok(Box::new(warp::reply::with_status(
                warp::reply::json(&json!({ "error": message })),
                StatusCode::INTERNAL_SERVER_ERROR,
            )));
        }
    };

    let mode = handle.mode;
    let video_codec = photo.video_codec().unwrap_or("").to_string();
    let audio_codec = photo.audio_codec().map(str::to_string);
    let duration = crate::video_probe::resolve(&db_pool, &photo).await.duration_secs;

    let StreamHandle { stdout, stderr, child, permit, .. } = handle;
    let hash = photo.hash_sha256.clone();
    tokio::spawn(async move {
        match supervise(child, stderr).await {
            Ok(()) => log::debug!("Stream finished for {hash}"),
            Err(reason) => log::warn!("Stream failed for {hash} ({}): {reason}", mode.as_str()),
        }
        drop(permit);
    });

    let body = tokio_util::io::ReaderStream::new(stdout);
    let response = warp::reply::stream(body);
    let response = warp::reply::with_status(response, StatusCode::OK);
    let response = warp::reply::with_header(response, "content-type", "video/mp4");
    let response = warp::reply::with_header(response, "cache-control", "no-store");
    let response = warp::reply::with_header(response, "x-turbopix-mode", mode.as_str());
    let response = warp::reply::with_header(
        response,
        "x-turbopix-mime",
        output_mime(mode, &video_codec, audio_codec.as_deref()),
    );
    let response: Box<dyn Reply> = match duration {
        Some(secs) => Box::new(warp::reply::with_header(
            response,
            "x-turbopix-duration",
            format!("{secs:.3}"),
        )),
        None => Box::new(response),
    };
    Ok(response)
}
```

Imports to extend in `src/handlers_video.rs:64-69`:

```rust
use crate::video_stream::{output_mime, start_stream, supervise, StreamMode, StreamQuery, StreamStartError, StreamHandle};
```

Register the route in `src/handlers_photo.rs` right after the status route (`src/handlers_photo.rs:1191-1198`) and `.or(...)` it into the combined routes (`src/handlers_photo.rs:1260-1263`):

```rust
    let api_photo_video_stream = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("video"))
        .and(warp::path("stream"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<StreamQuery>())
        .and(warp::header::headers_cloned())
        .and(with_db(db_pool.clone()))
        .and_then(stream_video);
```

Also import `stream_video` and `StreamQuery` next to the existing `get_video_file`/`get_video_status` imports (`src/handlers_photo.rs:10`).

- [ ] **Step 9: Add the handler tests (fake ffmpeg, permit exhaustion)**

Follow the existing helpers `setup_test_video`, `create_script`, `EnvVarGuard`, `collect_response_body` (`src/handlers_video.rs:731-870`):

```rust
    #[tokio::test]
    async fn stream_endpoint_serves_fragmented_bytes_with_mode_header() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let hash = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        setup_test_video(&db_pool, &temp_dir, hash).await;

        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg.sh");
        create_script(
            &ffmpeg_script,
            "#!/usr/bin/env sh\nprintf '\\000\\000\\000\\030ftypiso5'\nprintf '\\000\\000\\000\\010moov'\n",
        );
        let _ffmpeg_guard = EnvVarGuard::set("FFMPEG_PATH", ffmpeg_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let response = stream_video(
            hash.to_string(),
            StreamQuery { start: Some(0.0), mode: Some("remux".to_string()), client: None },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("stream should reply");
        let response = response.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-turbopix-mode"], "remux");
        assert_eq!(response.headers()["content-type"], "video/mp4");

        let body = collect_response_body(response).await;
        assert!(body.windows(4).any(|w| w == b"ftyp"));
    }

    #[tokio::test]
    async fn stream_endpoint_returns_503_when_pool_is_saturated() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let hash = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        setup_test_video(&db_pool, &temp_dir, hash).await;

        let _wait_guard = EnvVarGuard::set("TURBO_PIX_STREAM_QUEUE_WAIT_SECS", "0");

        // Hold every permit so the request cannot be served. This must not rely
        // on TURBO_PIX_MAX_TRANSCODES: the semaphore is a OnceLock sized by the
        // first caller in the process.
        let semaphore = crate::video_processor::transcode_semaphore();
        let mut held = Vec::new();
        while let Ok(permit) = semaphore.try_acquire() {
            held.push(permit);
        }
        assert!(!held.is_empty(), "at least one permit must be acquirable");

        let response = stream_video(
            hash.to_string(),
            StreamQuery { start: Some(0.0), mode: Some("transcode".to_string()), client: None },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("busy reply");
        let response = response.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["retry-after"], "2");

        drop(held);
    }
```

- [ ] **Step 10: Run the handler tests and the backend suite**

Run: `cargo test --lib handlers_video && cargo test`
Expected: PASS.

- [ ] **Step 11: Smoke the endpoint against a running server**

```bash
npm run build
nohup cargo run > /tmp/turbopix.log 2>&1 &
until curl -sf http://localhost:18473/health > /dev/null; do sleep 1; done
HASH=$(curl -s 'http://localhost:18473/api/photos?q=type:video&limit=1' | jq -r '.photos[0].hash_sha256')
curl -s -D- -o /tmp/stream.bin "http://localhost:18473/api/photos/$HASH/video/stream?mode=remux&start=0" | head -12
head -c 32 /tmp/stream.bin | xxd
pkill -f 'target/debug/turbo-pix'
```

Expected: `200`, `transfer-encoding: chunked`, `x-turbopix-mode: remux`, and the body head contains `ftyp`/`moov`.

- [ ] **Step 12: Commit**

```bash
git add src/video_stream.rs src/lib.rs src/handlers_video.rs src/handlers_photo.rs \
  test-data/test_video_long.mp4 test-data/test_video_long.mkv test-data/test_video_ac3.mp4
git commit -m "feat(video): add fragmented-MP4 streaming endpoint"
```

---

### Task 3: Decision contract v2 + MSE playback (the interface flip)

The decision JSON and the player that consumes it are one contract, so both sides change in this one commit.

**Files:**
- Modify: `src/video_capability.rs` (replace `DirectPlay`/`decide` with `Delivery`/`plan`; add audio codecs to `ClientCodecs`)
- Modify: `src/handlers_video.rs` (decision JSON v2, direct/stream routing, client-carrying URLs)
- Create: `frontend/src/lib/video/msePlayer.js`
- Modify: `frontend/src/lib/utils.js` (audio probes; `getVideoUrl` carries `client`)
- Modify: `frontend/src/lib/api.js` (pass through the new decision fields)
- Modify: `frontend/src/components/PhotoViewer.svelte` (stream playback, buffering/waiting states, teardown)
- Modify: `frontend/src/i18n/en.json`, `frontend/src/i18n/de.json`
- Test: rewritten `src/video_capability.rs` tests, updated `src/handlers_video.rs` decision tests, new `tests/e2e/specs/video-streaming.e2e.spec.js`

**Interfaces:**
- Consumes: Task 1's `ResolvedCapabilities`, Task 2's `StreamMode`, `output_mime`, `start_stream`, `/video/stream` route.
- Produces:
  - `pub enum Delivery { Direct, StreamRemux, StreamAudio, StreamTranscode }`
  - `pub fn plan(caps: &ResolvedCapabilities, client: &ClientCodecs) -> Delivery`
  - `pub struct AudioCodecs { aac, opus, mp3, flac, ac3, eac3, dts, vorbis: bool }`; `ClientCodecs` gains `pub audio: AudioCodecs`
  - Decision JSON: `{ action: "direct"|"stream"|"empty"|"error", url, mode, mime, duration, cached, reason }`. For `stream`, `url` is `/api/photos/{hash}/video/stream?client=…` — it carries **neither** `start` nor `mode`; the player appends `mode=<decision.mode>…` and `start=<seconds>` itself.
  - `frontend/src/lib/video/msePlayer.js`: `mseSupported(mime) -> boolean`, `createStreamPlayer(videoEl, { streamUrl, mime, duration, onState, onError }) -> { start(seconds), destroy() }`
  - i18n keys `video.stream.buffering`, `video.stream.waiting`

Decision table (encode exactly this):

| Source | Client declaration | Delivery |
|---|---|---|
| h264 ≤8-bit in mp4/mov with moov at start, audio aac or none | h264-8 + aac | `Direct` |
| h264 in mp4/mov, moov at end | h264-8 + aac | `StreamRemux` |
| h264/hevc/av1/vp9 in Matroska (`.mkv`) | video + audio declared | `StreamRemux` |
| vp8/vp9/av1 (+opus/vorbis) in Webm | declared | `Direct` |
| video declared, audio not declared (ac3/dts/…) | video yes, audio no | `StreamAudio` |
| video not declared (hevc without hevc, h264 10-bit without h264-10, legacy codecs) or container unknown | — | `StreamTranscode` |

- [ ] **Step 1: Replace the decision engine tests**

Replace the whole `#[cfg(test)] mod tests` in `src/video_capability.rs` (`src/video_capability.rs:132-253`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::video_probe::ResolvedCapabilities;

    fn caps(
        codec: &str,
        container: &str,
        bit_depth: Option<u32>,
        audio: Option<&str>,
        moov: bool,
    ) -> ResolvedCapabilities {
        ResolvedCapabilities {
            codec: codec.to_string(),
            container: Some(container.to_string()),
            family: ContainerFamily::from_record(Some(container), "video.mp4"),
            bit_depth,
            audio_codec: audio.map(str::to_string),
            moov_at_start: moov,
            duration_secs: Some(10.0),
            probed: true,
        }
    }

    fn client_all() -> ClientCodecs {
        ClientCodecs::parse(Some(
            "h264-8,h264-10,hevc,av1,vp9,vp8,aac,opus,mp3,flac,ac3,eac3,dts,vorbis",
        ))
    }

    /// What a plain Chrome/Firefox desktop really declares.
    fn web_client() -> ClientCodecs {
        ClientCodecs::parse(Some("h264-8,aac,opus,mp3,vorbis"))
    }

    #[test]
    fn h264_mp4_with_audio_plays_directly() {
        assert_eq!(
            plan(&caps("h264", "mov", Some(8), Some("aac"), true), &web_client()),
            Delivery::Direct
        );
        assert_eq!(plan(&caps("h264", "mp4", None, None, true), &web_client()), Delivery::Direct);
    }

    #[test]
    fn moov_at_end_remuxes_losslessly() {
        assert_eq!(
            plan(&caps("h264", "mov", Some(8), Some("aac"), false), &web_client()),
            Delivery::StreamRemux
        );
    }

    #[test]
    fn h264_in_matroska_remuxes_to_mp4() {
        assert_eq!(
            plan(&caps("h264", "matroska", Some(8), Some("aac"), true), &web_client()),
            Delivery::StreamRemux
        );
    }

    #[test]
    fn webm_vp9_plays_directly_only_when_declared() {
        let webm = ResolvedCapabilities {
            family: ContainerFamily::Webm,
            ..caps("vp9", "webm", Some(8), Some("opus"), true)
        };
        assert_eq!(plan(&webm, &web_client()), Delivery::Direct);
        let no_vp9 = ClientCodecs::parse(Some("h264-8,aac"));
        assert_eq!(plan(&webm, &no_vp9), Delivery::StreamTranscode);
    }

    #[test]
    fn hevc_direct_plays_only_when_declared() {
        let hevc = caps("hevc", "mov", Some(8), Some("aac"), true);
        assert_eq!(plan(&hevc, &client_all()), Delivery::Direct);
        assert_eq!(plan(&hevc, &web_client()), Delivery::StreamTranscode);
    }

    #[test]
    fn undeclared_audio_converts_audio_only() {
        let ac3 = caps("h264", "mov", Some(8), Some("ac3"), true);
        assert_eq!(plan(&ac3, &web_client()), Delivery::StreamAudio);
        assert_eq!(plan(&ac3, &client_all()), Delivery::Direct);
    }

    #[test]
    fn ten_bit_h264_needs_declared_support() {
        let ten_bit = caps("h264", "mov", Some(10), Some("aac"), true);
        assert_eq!(plan(&ten_bit, &web_client()), Delivery::StreamTranscode);
        assert_eq!(plan(&ten_bit, &client_all()), Delivery::Direct);
    }

    #[test]
    fn legacy_and_unknown_sources_convert() {
        assert_eq!(
            plan(&caps("mpeg4", "avi", Some(8), Some("mp3"), true), &client_all()),
            Delivery::StreamTranscode
        );
        assert_eq!(plan(&caps("", "", None, None, true), &client_all()), Delivery::StreamTranscode);
        assert_eq!(
            plan(&caps("hevc", "matroska", Some(8), Some("aac"), true), &web_client()),
            Delivery::StreamTranscode
        );
    }

    #[test]
    fn empty_or_missing_declaration_falls_back_to_conservative() {
        assert_eq!(ClientCodecs::parse(Some("")), ClientCodecs::conservative());
        assert_eq!(ClientCodecs::parse(None), ClientCodecs::conservative());
        assert_eq!(
            ClientCodecs::parse(Some("bogus-token")),
            ClientCodecs::none(),
            "a declaration with no recognized token is an empty capability set, not h264-8"
        );
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib video_capability --no-run`
Expected: compile errors for `Delivery`, `plan`, `ClientCodecs::audio`, `ClientCodecs::none`.

- [ ] **Step 3: Implement the engine**

In `src/video_capability.rs`, replace `DirectPlay`, `ClientCodecs`, `decide`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Direct,
    StreamRemux,
    StreamAudio,
    StreamTranscode,
}

/// Containers a media element may be handed as-is.
fn direct_container_ok(family: ContainerFamily) -> bool {
    matches!(family, ContainerFamily::Mp4 | ContainerFamily::Webm)
}

/// Video codecs that can be copied (never re-encoded) into an MP4 output.
fn copyable_into_mp4(codec: &str) -> bool {
    matches!(codec, "h264" | "hevc" | "av1" | "vp9")
}

fn video_supported(codec: &str, bit_depth: Option<u32>, client: &ClientCodecs) -> bool {
    match codec {
        "h264" => {
            if bit_depth.unwrap_or(8) <= 8 {
                client.h264_8
            } else {
                client.h264_10
            }
        }
        "hevc" => client.hevc,
        "av1" => client.av1,
        "vp9" => client.vp9,
        "vp8" => client.vp8,
        _ => false,
    }
}

fn audio_supported(codec: Option<&str>, client: &ClientCodecs) -> bool {
    match codec {
        None | Some("") => true,
        Some("aac") => client.audio.aac,
        Some("opus") => client.audio.opus,
        Some("mp3") => client.audio.mp3,
        Some("flac") => client.audio.flac,
        Some("ac3") => client.audio.ac3,
        Some("eac3") => client.audio.eac3,
        Some("dts") => client.audio.dts,
        Some("vorbis") => client.audio.vorbis,
        Some(_) => false,
    }
}

/// The single playback decision (spec FR-001..FR-004, FR-007).
pub fn plan(caps: &ResolvedCapabilities, client: &ClientCodecs) -> Delivery {
    let video_ok = video_supported(&caps.codec, caps.bit_depth, client);
    let audio_ok = audio_supported(caps.audio_codec.as_deref(), client);

    if video_ok && audio_ok {
        // Same container class the browser understands, and — for MP4-family —
        // a progressive layout.
        let layout_ok = !caps.family.has_moov_layout() || caps.moov_at_start;
        if direct_container_ok(caps.family) && layout_ok {
            return Delivery::Direct;
        }
        if copyable_into_mp4(&caps.codec) {
            return Delivery::StreamRemux;
        }
    }
    if video_ok && copyable_into_mp4(&caps.codec) {
        // Video is fine; the container or the audio track is not.
        return if audio_ok { Delivery::StreamRemux } else { Delivery::StreamAudio };
    }
    Delivery::StreamTranscode
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AudioCodecs {
    pub aac: bool,
    pub opus: bool,
    pub mp3: bool,
    pub flac: bool,
    pub ac3: bool,
    pub eac3: bool,
    pub dts: bool,
    pub vorbis: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientCodecs {
    pub h264_8: bool,
    pub h264_10: bool,
    pub hevc: bool,
    pub av1: bool,
    pub vp9: bool,
    pub vp8: bool,
    pub audio: AudioCodecs,
}

impl ClientCodecs {
    pub fn none() -> Self {
        Self {
            h264_8: false,
            h264_10: false,
            hevc: false,
            av1: false,
            vp9: false,
            vp8: false,
            audio: AudioCodecs::default(),
        }
    }

    pub fn conservative() -> Self {
        Self { h264_8: true, ..Self::none() }
    }

    /// Unknown tokens are ignored. A missing OR blank declaration yields the
    /// conservative baseline; a declaration that parses to nothing at all
    /// yields an empty capability set (the client explicitly claimed nothing).
    pub fn parse(header: Option<&str>) -> Self {
        let Some(raw) = header else {
            return Self::conservative();
        };
        if raw.trim().is_empty() {
            return Self::conservative();
        }
        let mut c = Self::none();
        for tok in raw.split(',').map(str::trim) {
            match tok {
                "h264-8" => c.h264_8 = true,
                "h264-10" => c.h264_10 = true,
                "hevc" => c.hevc = true,
                "av1" => c.av1 = true,
                "vp9" => c.vp9 = true,
                "vp8" => c.vp8 = true,
                "aac" => c.audio.aac = true,
                "opus" => c.audio.opus = true,
                "mp3" => c.audio.mp3 = true,
                "flac" => c.audio.flac = true,
                "ac3" => c.audio.ac3 = true,
                "eac3" => c.audio.eac3 = true,
                "dts" => c.audio.dts = true,
                "vorbis" => c.audio.vorbis = true,
                _ => {}
            }
        }
        c
    }
}
```

- [ ] **Step 4: Run the engine tests**

Run: `cargo test --lib video_capability`
Expected: PASS (9 tests). `cargo test --lib handlers_video --no-run` now fails — Step 5 fixes the handler.

- [ ] **Step 5: Rewrite the handler decision + routing**

Replace the decision section of `get_video_file` (`src/handlers_video.rs:186-232`) and the delivery match (`:238-265`):

```rust
    // Capability record resolution (Task 1) + the single decision engine.
    let caps = crate::video_probe::resolve(&db_pool, &photo).await;
    let delivery = crate::video_capability::plan(&caps, &client);

    let client_param = headers
        .get("X-TurboPix-Codecs")
        .and_then(|v| v.to_str().ok())
        .or(query.client_codecs.as_deref())
        .unwrap_or_default();

    let video_url = if client_param.is_empty() {
        format!("/api/photos/{}/video", photo_hash)
    } else {
        format!("/api/photos/{}/video?client={}", photo_hash, urlencoding(client_param))
    };
    let stream_base = if client_param.is_empty() {
        format!("/api/photos/{}/video/stream", photo_hash)
    } else {
        format!("/api/photos/{}/video/stream?client={}", photo_hash, urlencoding(client_param))
    };
    let mode = match delivery {
        Delivery::Direct => None,
        Delivery::StreamRemux => Some(StreamMode::Remux),
        Delivery::StreamAudio => Some(StreamMode::Audio),
        Delivery::StreamTranscode => Some(StreamMode::Transcode),
    };

    if query
        .decision
        .as_deref()
        .is_some_and(|v| v.is_empty() || v == "true")
    {
        let response = match delivery {
            Delivery::Direct => json!({
                "action": "direct",
                "url": video_url,
                "mode": null,
                "mime": null,
                "duration": caps.duration_secs,
                "cached": false,
                "reason": null,
            }),
            _ => {
                let mode = mode.expect("stream deliveries carry a mode");
                json!({
                    "action": "stream",
                    "url": stream_base,
                    "mode": mode.as_str(),
                    "mime": output_mime(mode, &caps.codec, caps.audio_codec.as_deref()),
                    "duration": caps.duration_secs,
                    "cached": false,
                    "reason": null,
                })
            }
        };
        return Ok(Box::new(warp::reply::json(&response)));
    }

    let (file_to_serve, warning) = match delivery {
        Delivery::Direct => (video_path.to_path_buf(), None),
        Delivery::StreamRemux | Delivery::StreamAudio | Delivery::StreamTranscode => {
            // A byte request that is not the stream endpoint means the client
            // wants a file. Serve the original; the client falls back to the
            // legacy ?transcode=true flow if it cannot use the stream.
            if client_wants_transcode {
                // Escape hatch / cache path (Task 5 wires the cached fast path).
                return serve_whole_file_transcode(&photo).await;
            }
            (video_path.to_path_buf(), None)
        }
    };
```

Do NOT keep the `return ...` sketch literally: first extract the current `DirectPlay::No` arm body (`src/handlers_video.rs:330-497`) into a helper `async fn serve_whole_file_transcode(photo: &Photo) -> Result<Box<dyn Reply>, Rejection>` that recomputes the versioned cache path internally (the code at `src/handlers_video.rs:315-328`), then call it from the `StreamRemux`/`StreamAudio`/`StreamTranscode` arm when `client_wants_transcode` is set. Task 5 splits the spawn block out of it as `spawn_whole_file_transcode` so the cache-fill hook reuses it. The old `NeedsRemux` faststart-sidecar arm is kept for the `StreamRemux` case **only when** the sidecar already exists (cache reuse is Task 5; until then the remux stream is used):

```rust
        Delivery::StreamRemux => {
            let cache_dir = std::env::var("TRANSCODE_CACHE_DIR")
                .unwrap_or_else(|_| "./data/cache/transcoded".to_string());
            let sidecar = remux_sidecar_path(
                &cache_dir,
                &photo.hash_sha256,
                photo.file_size,
                photo.date_modified.timestamp_millis(),
            );
            if sidecar.exists() {
                (sidecar, None)
            } else {
                (video_path.to_path_buf(), None)
            }
        }
```

Add `fn urlencoding(value: &str) -> String` locally (percent-encode `,` → `%2C` and space) rather than pulling a new crate; a tiny hand-rolled encoder for the comma/space-safe charset is enough because capability tokens are `[a-z0-9,-]`:

```rust
/// Capability strings are `[a-z0-9, -]`; escaping the comma keeps the value
/// unambiguous in a query string without a new dependency.
fn urlencoding(value: &str) -> String {
    value.replace(',', "%2C").replace(' ', "")
}
```

Also update the `?metadata=true` unchanged branch (it stays as-is) and delete the old imports of `decide`/`DirectPlay` (`src/handlers_video.rs:64`).

- [ ] **Step 6: Update the handler tests for the new decision JSON**

Rewrite `decision_endpoint_reports_direct_and_transcode_actions` (`src/handlers_video.rs:955-1026`) to assert:
- h264/mp4/8-bit/moov-start with `?client=h264-8,aac` → `{"action":"direct","url":"/api/photos/<hash>/video?client=h264-8%2Caac"}`.
- Same file with `?client=h264-8,hevc` but a record without `capability_version` → the handler probes; keep the fake ffprobe returning an mp4 container so the derived record still decides `direct`.
- mpeg4/avi with `?client=h264-8` → `{"action":"stream","mode":"transcode","mime":"video/mp4; codecs=\"avc1.42E01E,mp4a.40.2\"","url": ".../video/stream?client=...&start=0"}`.
- hevc/mp4/moov-start with `?client=hevc,aac` → `direct`; with `?client=h264-8,aac` → `stream`/`transcode`.
- h264/mp4/moov-end → `stream`/`remux`.
- ac3 audio + `?client=h264-8,aac` → `stream`/`audio`.

Use the existing `setup_test_video_with_content` helper plus a new module-level helper that patches the capability record through the real persistence path:

```rust
    /// Write a complete capability record (Task 1's `record_is_complete`
    /// contract) so the handler decides without probing the filesystem.
    async fn set_video_record(db_pool: &DbPool, hash: &str, video: serde_json::Value) {
        let patch = json!({ "video": video });
        Photo::persist_metadata_patch(db_pool, hash, &patch)
            .await
            .expect("capability patch");
    }
```

Task 5 reuses this helper. Task 1's `record_is_complete` requires `capability_version`, so every seeded record in these tests includes it.

- [ ] **Step 7: Run the handler tests**

Run: `cargo test --lib handlers_video`
Expected: PASS.

- [ ] **Step 8: Write the new frontend module `frontend/src/lib/video/msePlayer.js`**

```js
/**
 * Media Source Extensions playback for server-side streaming conversions.
 *
 * The server pipes fragmented MP4 from ffmpeg; the browser appends the chunks
 * to a SourceBuffer. ffmpeg rebases every stream run to timestamp 0, so a
 * seek is a *new* stream run whose segments are placed on the real timeline
 * with `SourceBuffer.timestampOffset`.
 */

/** @param {string} mime @returns {boolean} */
export function mseSupported(mime) {
  return typeof MediaSource !== 'undefined' && MediaSource.isTypeSupported(mime);
}

function once(target, event) {
  return new Promise((resolve) => target.addEventListener(event, resolve, { once: true }));
}

/**
 * @param {HTMLVideoElement} videoEl
 * @param {{streamUrl: string, mime: string, duration: number|null, onState: (state: string) => void, onError: (error: Error) => void}} options
 */
export function createStreamPlayer(videoEl, { streamUrl, mime, duration, onState, onError }) {
  let mediaSource = null;
  let sourceBuffer = null;
  let controller = null;
  let destroyed = false;
  let restartTimer = null;

  const state = (value) => {
    if (!destroyed) onState?.(value);
  };

  function urlFor(seconds) {
    const separator = streamUrl.includes('?') ? '&' : '?';
    return `${streamUrl}${separator}start=${seconds.toFixed(3)}`;
  }

  function isBuffered(seconds) {
    if (!sourceBuffer || sourceBuffer.buffered.length === 0) return false;
    for (let i = 0; i < sourceBuffer.buffered.length; i += 1) {
      if (seconds >= sourceBuffer.buffered.start(i) && seconds <= sourceBuffer.buffered.end(i)) {
        return true;
      }
    }
    return false;
  }

  async function pump(reader, start) {
    for (;;) {
      const { done, value } = await reader.read();
      if (destroyed) return;
      if (done) {
        try {
          if (mediaSource?.readyState === 'open') mediaSource.endOfStream();
        } catch {
          /* the source may already be closed */
        }
        state('ended');
        return;
      }
      if (sourceBuffer.updating) await once(sourceBuffer, 'updateend');
      if (destroyed) return;
      sourceBuffer.timestampOffset = start;
      sourceBuffer.appendBuffer(value);
      if (sourceBuffer.buffered.length > 0) state('buffering');
    }
  }

  async function start(seconds) {
    if (destroyed) return;
    controller?.abort();
    if (restartTimer !== null) clearTimeout(restartTimer);
    try {
      mediaSource?.endOfStream?.();
    } catch {
      /* already closed */
    }

    mediaSource = new MediaSource();
    videoEl.src = URL.createObjectURL(mediaSource);
    await once(mediaSource, 'sourceopen');
    if (destroyed) return;

    if (typeof duration === 'number' && Number.isFinite(duration) && duration > 0) {
      mediaSource.duration = duration;
    }
    sourceBuffer = mediaSource.addSourceBuffer(mime);
    sourceBuffer.timestampOffset = seconds;

    controller = new AbortController();
    const response = await fetch(urlFor(seconds), { signal: controller.signal });
    if (!response.ok || !response.body) {
      throw new Error(`stream HTTP ${response.status}`);
    }
    // Show "waiting for a free conversion slot" when the server holds the
    // request open because every worker is busy.
    restartTimer = setTimeout(() => state('waiting'), 1500);
    const reader = response.body.getReader();
    // First bytes arrived: we are buffering, not waiting.
    const first = await reader.read();
    if (restartTimer !== null) clearTimeout(restartTimer);
    if (destroyed || first.done) return;
    state('buffering');
    sourceBuffer.appendBuffer(first.value);
    videoEl.currentTime = seconds;
    videoEl.play().catch(() => {});
    state('playing');
    try {
      await pump(reader, seconds);
    } catch (error) {
      if (!destroyed && error.name !== 'AbortError') onError?.(error);
    }
  }

  // Seeking outside the buffered range restarts the stream at the target.
  const onSeeking = () => {
    const target = videoEl.currentTime;
    if (!isBuffered(target)) start(target).catch((error) => !destroyed && onError?.(error));
  };
  videoEl.addEventListener('seeking', onSeeking);

  function destroy() {
    destroyed = true;
    if (restartTimer !== null) clearTimeout(restartTimer);
    controller?.abort();
    videoEl.removeEventListener('seeking', onSeeking);
    try {
      mediaSource?.endOfStream?.();
    } catch {
      /* already closed */
    }
    if (videoEl.src.startsWith('blob:')) URL.revokeObjectURL(videoEl.src);
  }

  return { start, destroy };
}
```

Notes for the implementer:
- `state('playing')` fires before decoding proves out — the `onError`/`video.error` path in `PhotoViewer` (Task 6) is what catches a genuinely undecodable stream.
- The pump deliberately does not await `updateend` after the last append of an aborted stream: aborting the fetch rejects `reader.read()` with `AbortError`, which the catch swallows.

- [ ] **Step 9: Wire the viewer + capability declaration**

`frontend/src/lib/utils.js`:
- Add audio probes next to the existing video ones (`frontend/src/lib/utils.js:318-420`):

```js
  /** Audio-codec probes; tokens mirror the server's ClientCodecs audio set. */
  audioProbeTokens() {
    const mp4 = (codec) => `audio/mp4; codecs="${codec}"`;
    const webm = (codec) => `audio/webm; codecs="${codec}"`;
    const probes = [
      ['aac', mp4('mp4a.40.2')],
      ['opus', webm('opus')],
      ['mp3', 'audio/mpeg'],
      ['flac', 'audio/flac'],
      ['ac3', mp4('ac-3')],
      ['eac3', mp4('ec-3')],
      ['dts', mp4('dts')],
      ['vorbis', webm('vorbis')],
    ];
    return probes.filter(([, mime]) => this.canPlayType(mime)).map(([token]) => token);
  },
```

- Extend `getClientCodecsString()` (`frontend/src/lib/utils.js:401-412`) to append `this.audioProbeTokens()` before joining.
- `getVideoUrl(photoHash, { transcode, clientCodecs })` (add the option) appends `client=<encoded>`; every call site passes `videoCodecSupport.getClientCodecsString()`, which fixes the serve-time re-decision with the conservative baseline (`src/handlers_video.rs:141-146` treats a missing declaration as h264-8 only).

`frontend/src/lib/api.js`: `getVideoDecision` already returns the parsed JSON; keep the 202 branch (legacy whole-file path) and let the new fields flow through:

```js
    const data = await res.json();
    return {
      action: data.action,
      url: data.url,
      mode: data.mode ?? null,
      mime: data.mime ?? null,
      duration: typeof data.duration === 'number' ? data.duration : null,
      cached: Boolean(data.cached),
      reason: data.reason ?? null,
    };
```

`frontend/src/components/PhotoViewer.svelte`:
- Module-level import: `import { createStreamPlayer, mseSupported } from '../lib/video/msePlayer.js';`
- New state: `let streamPlayer = $state(null);` (or a plain `let`, per the component's existing style) and `let streamFallbackUsed = false;`
- In `displayVideo` (`frontend/src/components/PhotoViewer.svelte:684-734`) replace the `direct`/`remux`/`transcode` handling:

```js
    if (decision.action === 'direct') {
      setVideoSource(photo, decision.url, true);
      return;
    }
    if (decision.action === 'stream') {
      if (!decision.mime || !mseSupported(decision.mime)) {
        // Legacy fallback: whole-file conversion for browsers without MSE for
        // this codec; keeps FR-012's escape hatch intact.
        const legacy = getVideoUrl(photo.hash_sha256, {
          transcode: true,
          clientCodecs: videoCodecSupport.getClientCodecsString(),
        });
        if (await tryStartTranscode(legacy, photo)) return;
        setVideoSource(photo, legacy, false);
        return;
      }
      await playStream(photo, decision);
      return;
    }
    if (decision.action === 'empty') {
      showTranscodeToast(get(t)('video.file_empty', { default: 'This video file is empty or still being synced.' }), true);
      return;
    }
    showTranscodeToast(
      get(t)('video.conversion_reason', {
        values: { reason: decision.reason || '' },
        default: 'Could not convert this video: {reason}',
      }),
      true
    );
```

and add:

```js
  async function playStream(photo, decision, modeOverride = null) {
    if (!videoEl) return;
    destroyStreamPlayer();
    hasUserChosenOriginal = false;
    const mode = modeOverride || decision.mode;
    const separator = decision.url.includes('?') ? '&' : '?';
    // The decision URL carries neither `mode` nor `start`: the player always
    // appends `start=<seconds>` itself (msePlayer.urlFor), and `mode` is the
    // server-authorized mode the player may only escalate.
    const streamUrl = `${decision.url}${separator}mode=${mode}`;
    showTranscodeToast(
      get(t)('video.stream.buffering', { default: 'Video is being prepared for playback…' })
    );
    streamPlayer = createStreamPlayer(videoEl, {
      streamUrl,
      mime: decision.mime,
      duration: decision.duration,
      onState: (state) => {
        if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
        if (state === 'waiting') {
          showTranscodeToast(
            get(t)('video.stream.waiting', { default: 'Waiting for a free conversion slot…' })
          );
        } else if (state === 'playing' || state === 'ended') {
          hideTranscodeToast();
        }
      },
      onError: (error) => onStreamError(photo, decision, error),
    });
    videoEl.dataset.photoHash = photo.hash_sha256;
    videoEl.style.display = 'block';
    videoEl.classList.add('loaded');
    if (imageEl) imageEl.style.display = 'none';
    swipeableViewer?.reset();
    try {
      await streamPlayer.start(0);
      streamPlayer = null; // destroyed by the next playStream/destroy call
    } catch (error) {
      onStreamError(photo, decision, error);
    }
  }

  function destroyStreamPlayer() {
    if (!streamPlayer) return;
    streamPlayer.destroy();
    streamPlayer = null;
  }
```

`onStreamError` is implemented in Task 6 (escalation ladder); for this task it must at minimum hide the toast and show `video.transcoding.failed` plus the play-original escape:

```js
  function onStreamError(photo, decision, error) {
    log.warn?.('stream playback failed', error);
    if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
    destroyStreamPlayer();
    showTranscodeToast(
      get(t)('video.transcoding.failed', { default: 'Video conversion failed' }),
      true
    );
  }
```

- Call `destroyStreamPlayer()` in the viewer close path next to the existing poll teardown (`frontend/src/components/PhotoViewer.svelte:463-477`) and at the top of `displayVideo` when switching photos.

- [ ] **Step 10: Add the i18n keys to both dictionaries**

`frontend/src/i18n/en.json` (inside the existing `"video"` object, `frontend/src/i18n/en.json:282-292`):

```json
    "stream": {
      "buffering": "Video is being prepared for playback…",
      "waiting": "Waiting for a free conversion slot…"
    },
```

`frontend/src/i18n/de.json`:

```json
    "stream": {
      "buffering": "Video wird für die Wiedergabe vorbereitet…",
      "waiting": "Warte auf einen freien Konvertierungsplatz…"
    },
```

- [ ] **Step 11: Build and run the frontend + i18n gates**

```bash
npm run test:i18n
npm run lint
npm run build
cargo build --bin turbo-pix
```

Expected: all clean (zero lint errors, i18n parity, embedded build).

- [ ] **Step 12: Write the E2E spec for streaming playback**

`tests/e2e/specs/video-streaming.e2e.spec.js`:

```js
import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

/**
 * Fixtures (see test-data/, generated with ffmpeg):
 *   test_video_long.mp4  20 s h264+aac progressive
 *   test_video_long.mkv  20 s h264+aac Matroska
 *   test_video_ac3.mp4   20 s h264 + AC-3
 *   test_video_hevc.mp4  2 s hevc
 */

async function findVideoByFilename(page, filename) {
  const response = await page.request.get('/api/photos?q=type:video&limit=200');
  expect(response.ok()).toBeTruthy();
  const data = await response.json();
  const photo = (data.photos || []).find((p) => p.filename === filename);
  expect(photo, `${filename} must be seeded and indexed`).toBeTruthy();
  return photo;
}

async function openVideo(page, photo) {
  await TestHelpers.navigateToView(page, 'videos');
  await TestHelpers.waitForPhotosToLoad(page);
  await page.locator(TestHelpers.selectors.photoCard(photo.hash_sha256)).click();
  await TestHelpers.verifyViewerOpen(page);
}

function videoHandle(page) {
  return page.locator(TestHelpers.selectors.viewerVideo);
}

test.describe('On-the-fly streaming playback', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('HEVC plays while converting: first frame within 5s, true duration', async ({ page }) => {
    const hevc = await findVideoByFilename(page, 'test_video_hevc.mp4');
    await openVideo(page, hevc);

    const video = videoHandle(page);
    await expect(video).toBeVisible();
    const started = Date.now();
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 5000 }
    );
    expect(Date.now() - started).toBeLessThan(5000);

    const duration = await video.evaluate((el) => el.duration);
    expect(duration).toBeGreaterThan(1.5); // source duration is 2 s
    expect(duration).toBeLessThan(2.5);

    // The conversion notice must not survive the first frames.
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('Matroska h264 remuxes losslessly and seeks within 3s', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    await openVideo(page, mkv);

    const video = videoHandle(page);
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );

    const seekStart = Date.now();
    await video.evaluate((el) => {
      el.currentTime = 15;
    });
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.currentTime >= 15 && el.readyState >= 2;
      },
      null,
      { timeout: 3000 }
    );
    expect(Date.now() - seekStart).toBeLessThan(3000);
  });

  test('AC-3 audio converts without re-encoding video', async ({ page }) => {
    const ac3 = await findVideoByFilename(page, 'test_video_ac3.mp4');
    await openVideo(page, ac3);

    const response = await page.request.get(
      `/api/photos/${ac3.hash_sha256}/video?decision&client=h264-8,aac`
    );
    const decision = await response.json();
    expect(decision.action).toBe('stream');
    expect(decision.mode).toBe('audio');

    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );
  });

  test('h264 video still plays directly without any conversion notice', async ({ page }) => {
    const h264 = await findVideoByFilename(page, 'test_video.mp4');
    await openVideo(page, h264);
    const video = videoHandle(page);
    await expect(video).toBeVisible();
    const src = await video.getAttribute('src');
    expect(src).toContain(`/api/photos/${h264.hash_sha256}/video`);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });
});
```

- [ ] **Step 13: Seed the new fixtures in the E2E library**

Extend `seedTestMedia` (`tests/e2e/setup/global-setup.js:63-133`) to copy `test_video_long.mp4`, `test_video_long.mkv`, `test_video_ac3.mp4` into `test-e2e-data/photos` exactly like the two existing videos (`global-setup.js:113-130`), keeping the existing pinned `utimes` behaviour.

- [ ] **Step 14: Run the E2E spec and the backend suite**

```bash
npm run test:e2e -- video-streaming.e2e.spec.js
cargo test
```

Expected: 4 E2E tests pass; backend suite green (the legacy `transcoding.e2e.spec.js` assertions are updated in Task 5 when the whole-file path is demoted — if it fails here only because the HEVC flow no longer shows a blocking toast, update its assertions in this task to the new streaming behaviour).

- [ ] **Step 15: Commit**

```bash
git add src/video_capability.rs src/handlers_video.rs frontend/src/lib/video/msePlayer.js \
  frontend/src/lib/utils.js frontend/src/lib/api.js frontend/src/components/PhotoViewer.svelte \
  frontend/src/i18n/en.json frontend/src/i18n/de.json tests/e2e/specs/video-streaming.e2e.spec.js \
  tests/e2e/setup/global-setup.js
git commit -m "feat(video): stream converted/remuxed video via MSE and the new decision contract"
```

---

### Task 4: Saturation is visible, never fatal (FR-011, SC-006)

**Files:**
- Modify: `frontend/src/components/PhotoViewer.svelte` (`video.stream.waiting` path: retry loop on 503)
- Modify: `frontend/src/lib/video/msePlayer.js` (surface the HTTP status so the viewer can distinguish `503` from a real error)
- Test: `tests/e2e/specs/video-streaming.e2e.spec.js` (route-stubbed saturation), `src/handlers_video.rs` (already covered by Task 2 Step 9 — re-run only)

**Interfaces:**
- Consumes: Task 2's `503 + Retry-After: 2`, Task 3's `createStreamPlayer`, `playStream`.
- Produces: `createStreamPlayer` throws `StreamHttpError` with `{ status }`; `PhotoViewer` retries `503` forever while the viewer stays on the photo, showing `video.stream.waiting`.

- [ ] **Step 1: Write the failing E2E test**

Append to `tests/e2e/specs/video-streaming.e2e.spec.js`:

```js
  test('saturated conversions wait visibly and then start', async ({ page }) => {
    test.setTimeout(60_000);
    const hevc = await findVideoByFilename(page, 'test_video_hevc.mp4');

    // Answer the first two stream requests with 503 + Retry-After, then let the
    // real request through: this is exactly what a full worker pool looks like.
    let refusals = 0;
    await page.route('**/video/stream*', async (route) => {
      if (refusals < 2) {
        refusals += 1;
        await route.fulfill({
          status: 503,
          headers: { 'retry-after': '1', 'content-type': 'application/json' },
          body: JSON.stringify({ error: 'no conversion slot available' }),
        });
        return;
      }
      await route.continue();
    });

    await openVideo(page, hevc);
    await expect(page.locator('.transcode-toast')).toContainText('Waiting for a free conversion slot', {
      timeout: 10_000,
    });

    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );
    expect(refusals).toBe(2);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });
```

- [ ] **Step 2: Run to verify it fails**

Run: `npm run test:e2e -- video-streaming.e2e.spec.js -g "saturated"`
Expected: FAIL — the viewer treats 503 as a hard error (no waiting text, no retry).

- [ ] **Step 3: Implement the status-aware retry**

`frontend/src/lib/video/msePlayer.js`:

```js
export class StreamHttpError extends Error {
  constructor(status) {
    super(`stream HTTP ${status}`);
    this.name = 'StreamHttpError';
    this.status = status;
  }
}
```

and in `start()` replace `throw new Error(`stream HTTP ${response.status}`)` with `throw new StreamHttpError(response.status)`.

`frontend/src/components/PhotoViewer.svelte`, inside `playStream` (Task 3 Step 9), wrap the start call:

```js
    try {
      await streamPlayer.start(0);
      streamPlayer = null;
    } catch (error) {
      if (error?.status === 503) {
        // Worker pool saturated: keep the waiting state and retry until the
        // viewer moves on. Never give up on the user here.
        const retryMs = 1500;
        const photoHash = photo.hash_sha256;
        scheduleStreamRetry(() => {
          if (!isOpen || currentPhoto?.hash_sha256 !== photoHash) return;
          playStream(photo, decision, modeOverride);
        }, retryMs);
        return;
      }
      onStreamError(photo, decision, error);
    }
```

with a single-flight helper so retries cannot stack:

```js
  let streamRetryTimer = null;

  function scheduleStreamRetry(callback, delayMs) {
    if (streamRetryTimer !== null) return;
    streamRetryTimer = setTimeout(() => {
      streamRetryTimer = null;
      callback();
    }, delayMs);
  }
```

and clear it in `destroyStreamPlayer()`:

```js
  function destroyStreamPlayer() {
    if (streamRetryTimer !== null) {
      clearTimeout(streamRetryTimer);
      streamRetryTimer = null;
    }
    if (!streamPlayer) return;
    streamPlayer.destroy();
    streamPlayer = null;
  }
```

- [ ] **Step 4: Run the E2E test again**

Run: `npm run test:e2e -- video-streaming.e2e.spec.js -g "saturated"`
Expected: PASS.

- [ ] **Step 5: Prove the server bound cannot be exceeded**

Add to `src/video_stream.rs` tests:

```rust
    #[tokio::test]
    async fn repeated_refused_requests_spawn_no_processes() {
        // With every permit held, N attempts must all return Busy and leave the
        // permit count unchanged — a retry storm cannot create encoder processes.
        let semaphore = crate::video_processor::transcode_semaphore();
        let mut held = Vec::new();
        while let Ok(permit) = semaphore.try_acquire() {
            held.push(permit);
        }
        let _wait_guard = crate::video_processor::tests::TestEnvGuard::set("TURBO_PIX_STREAM_QUEUE_WAIT_SECS", "0");
        for _ in 0..5 {
            let err = start_stream(StreamMode::Transcode, Path::new("/nonexistent.mp4"), 0.0)
                .await
                .unwrap_err();
            assert!(matches!(err, StreamStartError::Busy));
        }
        assert_eq!(semaphore.available_permits(), 0);
        drop(held);
    }
```

`TestEnvGuard::set` does not exist yet — `src/video_processor.rs` tests expose `acquire_test_env_lock()` and a struct that resets on drop. Add a helper in that test module:

```rust
    /// Set an env var for the duration of a test (poisoning-safe, serialised by
    /// the module's env lock).
    pub(crate) struct TestEnvGuard {
        key: String,
        original: Option<String>,
        _lock: ...,
    }

    impl TestEnvGuard {
        pub(crate) fn set(key: &str, value: &str) -> Self { /* same shape as handlers_video's EnvVarGuard */ }
    }
```

Reuse the existing lock; do not invent a second env-mutation mechanism.

- [ ] **Step 6: Run the backend tests**

Run: `cargo test --lib video_stream && cargo test --lib handlers_video`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/lib/video/msePlayer.js frontend/src/components/PhotoViewer.svelte \
  src/video_stream.rs tests/e2e/specs/video-streaming.e2e.spec.js
git commit -m "feat(video): keep saturated conversion requests waiting instead of failing"
```

---

### Task 5: Reuse and cache-fill (FR-010, SC-007)

**Files:**
- Modify: `src/handlers_video.rs` (cached-artifact fast path in the decision; cache-fill hook after a full stream)
- Modify: `src/video_processor.rs` (whole-file transcode args: first-track mapping + audio handling)
- Test: `src/handlers_video.rs` tests, `tests/e2e/specs/video-streaming.e2e.spec.js`

**Interfaces:**
- Consumes: Task 3's `plan`/decision JSON, `get_transcoded_path_versioned`, `claim_transcode`, `transcode_codec_to_h264_with_progress`.
- Produces:
  - Decision returns `{"action":"direct","url":"/api/photos/{hash}/video?transcode=true&client=…","cached":true}` when a completed whole-file artifact exists.
  - `fn spawn_cache_fill(mode: StreamMode, photo: &Photo)` (in `handlers_video.rs`) — after a successful full stream (`start == 0`), produce the artifact that matches what the user actually watched, so the cached copy is never worse than the stream it replaces: `StreamTranscode` → the whole-file libx264+AAC conversion (bounded by `claim_transcode`); `StreamAudio` → a whole-file `-c:v copy -c:a aac` conversion; `StreamRemux` → the lossless faststart sidecar via `ensure_progressive_mp4` (never a re-encode). Superseded-by-ruling text: "the whole-file conversion" was mode-blind and would serve a lossy re-encode to later opens of an audio/remux source (FR-001).
  - Whole-file transcode ffmpeg args map `0:v:0` / `0:a:0?` and pick `-c:a copy` only for AAC/MP3 audio, else `-c:a aac -b:a 160k -ac 2`.

- [ ] **Step 1: Write the failing cache-hit test**

In `src/handlers_video.rs` tests (pattern of `test_video_cache_hit`, `src/handlers_video.rs:1256`), add:

```rust
    #[tokio::test]
    async fn decision_prefers_a_completed_cached_transcode() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let hash = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        let video_path = setup_test_video_with_content(&db_pool, &temp_dir, hash, b"fake-video-data").await;
        set_video_record(&db_pool, hash, json!({
            "codec": "mpeg4", "container": "avi", "bit_depth": 8,
            "audio_codec": "mp3", "moov_at_start": true, "capability_version": 1
        })).await;

        let cache_dir = temp_dir.path().join("transcoded");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let cached = crate::video_processor::get_transcoded_path_versioned(
            &cache_dir,
            hash,
            std::fs::metadata(&video_path).unwrap().len() as i64,
            0,
        );
        // The version folds in the photo's size+mtime; write the photo row's
        // values into the file stats so the helper matches (see test_video_cache_hit).
        std::fs::write(&cached, b"cached-transcode").unwrap();

        let _cache_guard = EnvVarGuard::set("TRANSCODE_CACHE_DIR", cache_dir.to_str().unwrap());
        let response = get_video_file(
            hash.to_string(),
            VideoQuery { metadata: None, transcode: None, client_codecs: Some("h264-8,aac".into()), decision: Some("true".into()) },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("decision reply");
        let body = collect_response_body(response.into_response()).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["action"], "direct");
        assert_eq!(json["cached"], true);
        assert!(json["url"].as_str().unwrap().contains("transcode=true"));
    }
```

Copy the exact version-matching mechanics from the existing `test_video_cache_hit` (`src/handlers_video.rs:1256-1310`); it already solves the size+mtime alignment.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib decision_prefers_a_completed_cached_transcode`
Expected: FAIL — the decision returns `stream` today.

- [ ] **Step 3: Implement the cache-hit fast path**

In the decision branch of `get_video_file` (Task 3 Step 5), before building the stream response:

```rust
        let cache_dir = std::env::var("TRANSCODE_CACHE_DIR")
            .unwrap_or_else(|_| "./data/cache/transcoded".to_string());
        let cached_path = get_transcoded_path_versioned(
            Path::new(&cache_dir),
            &photo.hash_sha256,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        // A completed artifact is only trusted when its status is not a stale
        // failure (same rule as the serve path, src/handlers_video.rs:499-509).
        let cached_ok = cached_path.exists()
            && !matches!(
                get_transcode_status(&photo.hash_sha256).map(|s| s.state),
                Some(TranscodeState::Failed | TranscodeState::Timeout)
            );
```

and use it in the `Delivery::Direct` arm as well as the streaming arms:

```rust
            _ if cached_ok => json!({
                "action": "direct",
                "url": format!("{video_url}{}transcode=true",
                    if video_url.contains('?') { "&" } else { "?" }),
                "mode": null,
                "mime": null,
                "duration": caps.duration_secs,
                "cached": true,
                "reason": null,
            }),
```

- [ ] **Step 4: Implement the cache-fill hook**

The stream handler's supervising task knows when a `start == 0` run finished cleanly. Pass that information through: extend the spawn in `stream_video` (Task 2 Step 8) to call the hook:

```rust
    if start <= 0.5 {
        let photo_for_fill = photo.clone();
        let db_for_fill = db_pool.clone();
        tokio::spawn(async move {
            match supervise(child, stderr).await {
                Ok(()) => spawn_cache_fill(&db_for_fill, &photo_for_fill).await,
                Err(reason) => log::warn!("Stream failed for {hash} ({}): {reason}", mode.as_str()),
            }
            drop(permit);
        });
    } else {
        tokio::spawn(async move { /* supervise + log only */ drop(permit); });
    }
```

`Photo` derives `Clone` (used elsewhere as `photo.clone()`); if it does not, wrap the fields the hook needs (hash, file path, size, mtime) in a small owned struct instead.

```rust
/// Fill the whole-file cache after a successful full playthrough so the next
/// open starts like a native play (FR-010). Bounded by the same claim/pool as
/// every other conversion, so this cannot spawn a second encoder for a hash
/// that is already being converted.
async fn spawn_cache_fill(db_pool: &DbPool, photo: &Photo) {
    let cache_dir = std::env::var("TRANSCODE_CACHE_DIR")
        .unwrap_or_else(|_| "./data/cache/transcoded".to_string());
    let output = get_transcoded_path_versioned(
        Path::new(&cache_dir),
        &photo.hash_sha256,
        photo.file_size,
        photo.date_modified.timestamp_millis(),
    );
    if output.exists() {
        return;
    }
    if !matches!(claim_transcode(&photo.hash_sha256), TranscodeClaim::Started) {
        return;
    }
    spawn_whole_file_transcode(db_pool.clone(), photo.clone(), output);
}
```

`spawn_whole_file_transcode` is the `tokio::spawn` block extracted out of `serve_whole_file_transcode` (originally `src/handlers_video.rs:400-510`: status callbacks + old-version cleanup + Completed/Failed status writes), now shared by the escape-hatch route and the cache-fill hook.

- [ ] **Step 5: Fix the whole-file audio handling**

In `src/video_processor.rs:922-947`, replace the fixed `-c:a copy` args with a mode-dependent tail:

```rust
        // Audio that the source carries and the cache can keep is copied;
        // anything else becomes AAC so the cached file is playable everywhere.
        let mut audio_args: Vec<&str> = match audio_codec {
            Some("aac") | Some("mp3") => vec!["-c:a", "copy"],
            _ => vec!["-c:a", "aac", "-b:a", "160k", "-ac", "2"],
        };
```

`transcode_codec_to_h264_with_timeout_and_path` needs the input audio codec: probe it once with `extract_video_metadata`-style ffprobe, or accept it as a parameter from the caller (which already resolved capabilities). Prefer the parameter: add `audio_codec: Option<String>` to `transcode_codec_to_h264_with_progress` / the inner fns and pass `caps.audio_codec` from `spawn_whole_file_transcode`. Add `-map 0:v:0 -map 0:a:0?` to the same args.

Test (fake ffmpeg, existing pattern `test_transcode_happy_path`, `src/video_processor.rs:1816`):

```rust
    #[test]
    fn transcode_args_keep_aac_but_convert_ac3() {
        let keep = build_transcode_args_for_test("aac");           // helper returning the Vec<String>
        assert!(keep.join(" ").contains("-c:a copy"));
        let convert = build_transcode_args_for_test("ac3");
        assert!(convert.join(" ").contains("-c:a aac"));
        assert!(convert.join(" ").contains("-map 0:v:0 -map 0:a:0?"));
    }
```

Refactor the ffmpeg arg construction into `fn build_transcode_args(input: &Path, output: &Path, audio_codec: Option<&str>, with_progress: bool) -> Vec<String>` so both the production path and the test use the same builder.

- [ ] **Step 6: Write the failing E2E test for reuse**

Append to `tests/e2e/specs/video-streaming.e2e.spec.js`:

```js
  test('a previously converted video starts without a blocking conversion', async ({ page }) => {
    test.setTimeout(120_000);
    const hevc = await findVideoByFilename(page, 'test_video_hevc.mp4');
    await openVideo(page, hevc);
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );
    // Give the background cache-fill a moment to finish (it is bounded by the
    // same worker pool; the fixture is 2 s of video).
    await page.waitForTimeout(5000);
    const status = await page.request.get(`/api/photos/${hevc.hash_sha256}/video?decision&client=h264-8,aac`);
    const decision = await status.json();
    expect(decision.action).toBe('direct');
    expect(decision.cached).toBe(true);

    // Reopen: playback must start within 2 s and never show a conversion toast.
    await TestHelpers.closeViewer(page);
    const started = Date.now();
    await page.locator(TestHelpers.selectors.photoCard(hevc.hash_sha256)).click();
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 2000 }
    );
    expect(Date.now() - started).toBeLessThan(2000);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });
```

- [ ] **Step 7: Run the backend and E2E tests**

```bash
cargo test --lib handlers_video && cargo test --lib video_processor
npm run test:e2e -- video-streaming.e2e.spec.js
npm run test:e2e -- transcoding.e2e.spec.js
```

Expected: PASS. Update `transcoding.e2e.spec.js` in this task: the HEVC flow no longer polls to completion — the second test becomes "HEVC starts streaming: the buffering toast is replaced by playing frames", and `video-playback.e2e.spec.js`'s decision test asserts the new `action`/`mode` fields.

- [ ] **Step 8: Commit**

```bash
git add src/handlers_video.rs src/video_processor.rs tests/e2e/specs/transcoding.e2e.spec.js \
  tests/e2e/specs/video-playback.e2e.spec.js tests/e2e/specs/video-streaming.e2e.spec.js
git commit -m "feat(video): reuse cached conversions and fill the cache after a full stream"
```

---

### Task 6: Failure handling and automatic fallback (FR-009, FR-012, SC-008)

**Files:**
- Modify: `src/handlers_video.rs` (mode escalation clamp on `/video/stream`)
- Modify: `frontend/src/components/PhotoViewer.svelte` (escalation ladder)
- Test: `src/handlers_video.rs`, `tests/e2e/specs/video-streaming.e2e.spec.js`

**Interfaces:**
- Consumes: Task 3's `plan`/decision, Task 4's `StreamHttpError`.
- Produces:
  - `fn escalated(plan: Delivery, requested: Option<StreamMode>) -> StreamMode` in `handlers_video.rs` (server never downgrades, only escalates).
  - Client ladder `remux → audio → transcode → original`, one step per failure, maximum 3 automatic attempts, then the error state with `Play original anyway`.

- [ ] **Step 1: Write the failing escalation test**

```rust
    #[test]
    fn requested_mode_can_only_escalate_the_planned_mode() {
        use crate::video_capability::Delivery;
        assert_eq!(
            escalated(Delivery::StreamRemux, Some(StreamMode::Transcode)),
            StreamMode::Transcode
        );
        assert_eq!(
            escalated(Delivery::StreamTranscode, Some(StreamMode::Remux)),
            StreamMode::Transcode,
            "a client hint must never downgrade a genuinely required conversion"
        );
        assert_eq!(
            escalated(Delivery::StreamAudio, None),
            StreamMode::Audio
        );
    }
```

- [ ] **Step 2: Implement the clamp and use it**

```rust
/// The client may ask for a stronger mode (self-heal after a failed remux);
/// it may never talk the server down from a conversion the plan requires.
fn escalated(plan: Delivery, requested: Option<StreamMode>) -> StreamMode {
    let planned = match plan {
        Delivery::StreamRemux => StreamMode::Remux,
        Delivery::StreamAudio => StreamMode::Audio,
        Delivery::Direct | Delivery::StreamTranscode => StreamMode::Transcode,
    };
    match requested {
        Some(StreamMode::Transcode) => StreamMode::Transcode,
        Some(StreamMode::Audio) if planned != StreamMode::Transcode => StreamMode::Audio,
        _ => planned,
    }
}
```

In `stream_video`, replace the Task 2 `StreamMode::from_query(requested)` selection with the plan-based one (resolve capabilities → `plan()` → `escalated(...)`). The `mode` query param stays optional: unknown values are ignored rather than 404'd, so an old client cannot break playback.

- [ ] **Step 3: Write the failing E2E test for self-healing**

```js
  test('a failed remux stream escalates one step and recovers', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    const requestedModes = [];
    await page.route('**/video/stream*', async (route) => {
      const mode = new URL(route.request().url()).searchParams.get('mode');
      requestedModes.push(mode);
      if (mode === 'remux') {
        // Simulate a client that cannot decode the remuxed stream: the server
        // answers, the browser rejects the bytes.
        await route.fulfill({ status: 200, headers: { 'content-type': 'video/mp4' }, body: 'not-mp4' });
        return;
      }
      await route.continue();
    });

    await openVideo(page, mkv);
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );

    // Exactly one escalation step (remux → audio), never a loop back to remux.
    expect(requestedModes).toEqual(['remux', 'audio']);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('an exhausted ladder shows the error and keeps the original playable', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    const requestedModes = [];
    await page.route('**/video/stream*', async (route) => {
      requestedModes.push(new URL(route.request().url()).searchParams.get('mode'));
      await route.fulfill({ status: 200, headers: { 'content-type': 'video/mp4' }, body: 'not-mp4' });
    });

    await openVideo(page, mkv);
    await expect(page.locator('.transcode-toast')).toContainText('Video conversion failed', {
      timeout: 30_000,
    });
    expect(requestedModes).toEqual(['remux', 'audio', 'transcode']);
    // The escape hatch is offered instead of a dead end.
    await expect(page.locator('[data-action="play-original"]')).toBeVisible();
  });
```

- [ ] **Step 4: Implement the client ladder**

In `PhotoViewer.svelte`:

```js
  const STREAM_LADDER = ['remux', 'audio', 'transcode'];

  function nextStreamMode(currentMode) {
    const index = STREAM_LADDER.indexOf(currentMode);
    return index >= 0 && index < STREAM_LADDER.length - 1 ? STREAM_LADDER[index + 1] : null;
  }

  /// `attemptedMode` is the mode the FAILED attempt used — never
  /// `decision.mode`, or an escalation to `audio` would fall back to `remux`
  /// on its own failure and loop forever.
  function onStreamError(photo, decision, attemptedMode, error) {
    if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
    destroyStreamPlayer();
    const nextMode = nextStreamMode(attemptedMode);
    if (nextMode) {
      playStream(photo, decision, nextMode);
      return;
    }
    showTranscodeToast(
      get(t)('video.transcoding.failed', { default: 'Video conversion failed' }),
      true
    );
  }
```

`playStream` passes the mode it is about to use into every `onStreamError` call it can reach: `onStreamError(photo, decision, mode, error)` inside `playStream`'s `onError` callback and its `catch`. There is no `streamEscalationExhausted` flag — the ladder itself terminates (`transcode` has no successor), so the maximum is three attempts per photo session. `modeOverride` (Task 3) is what carries the escalated mode.

Also update the `videoEl.onerror` handler so the final failure keeps the escape hatch visible:

```js
      showToast(
        get(t)('notifications.error', { default: 'Error' }),
        get(t)('video.transcoding.failed', { default: 'Video conversion failed' }),
        'error'
      );
```

is followed by `showTranscodeToast(..., true)` so the `data-action="play-original"` button (`frontend/src/components/PhotoViewer.svelte:1589-1597`) is rendered.

- [ ] **Step 5: Run the tests**

```bash
cargo test --lib handlers_video
npm run test:e2e -- video-streaming.e2e.spec.js
npm run test:e2e -- video-playback.e2e.spec.js
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/handlers_video.rs frontend/src/components/PhotoViewer.svelte tests/e2e/specs/video-streaming.e2e.spec.js
git commit -m "feat(video): self-heal failed streams through an escalation ladder"
```

---

### Task 7: Capability matrix fixtures and end-to-end verification (SC-003..SC-005)

**Files:**
- Create fixtures in `test-data/`: `test_video_10bit.mp4`, `test_video_noaudio.mp4`, `test_video_multitrack.mp4`, `test_video_moov_end.mp4`, `test_video_legacy.avi`
- Modify: `tests/e2e/setup/global-setup.js` (seed them)
- Modify: `tests/e2e/specs/video-streaming.e2e.spec.js` (matrix + timing + duration assertions)
- Test: the same spec, plus `src/video_probe.rs` / `src/video_capability.rs` unit tests for the new codecs

**Interfaces:**
- Consumes: everything above.
- Produces: fixtures + specs that prove SC-003/SC-004/SC-005 and the Review Focus items 3 and 4.

- [ ] **Step 1: Generate the remaining fixtures**

```bash
# 10-bit h264 (High 10) — must transcode for clients without h264-10
ffmpeg -y -f lavfi -i testsrc2=size=320x180:rate=24:duration=10 \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p10le -profile:v high10 \
  test-data/test_video_10bit.mp4

# h264 with no audio track at all
ffmpeg -y -f lavfi -i testsrc2=size=320x180:rate=24:duration=10 \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p -an test-data/test_video_noaudio.mp4

# two audio tracks (aac + ac3): only the first/default track may be considered
ffmpeg -y -i test-data/test_video_long.mp4 -i test-data/test_video_ac3.mp4 \
  -map 0:v:0 -map 0:a:0 -map 1:a:0 -c:v copy -c:a copy -shortest \
  test-data/test_video_multitrack.mp4

# progressive-less MP4: moov at the end (no +faststart)
ffmpeg -y -i test-data/test_video_long.mp4 -c copy test-data/test_video_moov_end.mp4

# legacy MPEG-4/AVI rip
ffmpeg -y -f lavfi -i testsrc2=size=320x180:rate=24:duration=10 \
  -f lavfi -i sine=frequency=440:duration=10 \
  -c:v mpeg4 -q:v 6 -c:a libmp3lame -shortest test-data/test_video_legacy.avi
```

Verify each with `ffprobe -v error -show_entries stream=codec_name,codec_type,pix_fmt -of csv=p=0 <file>` and record the output in the commit message.

- [ ] **Step 2: Seed the fixtures**

Extend `seedTestMedia` (`tests/e2e/setup/global-setup.js:63-133`) with the five new files (same copy + utimes pattern as `global-setup.js:113-130`). Pin distinct dates so the videos view's sort order stays stable for the existing specs that click the first card.

- [ ] **Step 3: Add the matrix E2E tests**

Append to `tests/e2e/specs/video-streaming.e2e.spec.js`:

```js
  const matrix = [
    // file, client declaration, expected action/mode
    ['test_video.mp4', 'h264-8,aac', 'direct', null],
    ['test_video_moov_end.mp4', 'h264-8,aac', 'stream', 'remux'],
    ['test_video_long.mkv', 'h264-8,aac', 'stream', 'remux'],
    ['test_video_ac3.mp4', 'h264-8,aac', 'stream', 'audio'],
    ['test_video_10bit.mp4', 'h264-8,aac', 'stream', 'transcode'],
    ['test_video_legacy.avi', 'h264-8,aac', 'stream', 'transcode'],
    ['test_video_hevc.mp4', 'h264-8,hevc,aac', 'direct', null],
    ['test_video_hevc.mp4', 'h264-8,aac', 'stream', 'transcode'],
    ['test_video_noaudio.mp4', 'h264-8,aac', 'direct', null],
  ];

  for (const [filename, client, action, mode] of matrix) {
    test(`decision matrix: ${filename} with [${client}] → ${action}/${mode}`, async ({ page }) => {
      const photo = await findVideoByFilename(page, filename);
      const response = await page.request.get(
        `/api/photos/${photo.hash_sha256}/video?decision&client=${encodeURIComponent(client)}`
      );
      expect(response.ok()).toBeTruthy();
      const decision = await response.json();
      expect(decision.action).toBe(action);
      if (mode) expect(decision.mode).toBe(mode);
      if (action === 'stream') {
        expect(decision.mime).toContain('video/mp4');
        expect(decision.duration).toBeGreaterThan(0);
      }
    });
  }
```

- [ ] **Step 4: Add the duration and no-audio playback checks**

```js
  test('multi-track and silent sources play without audio errors', async ({ page }) => {
    test.setTimeout(120_000);
    for (const filename of ['test_video_multitrack.mp4', 'test_video_noaudio.mp4']) {
      const photo = await findVideoByFilename(page, filename);
      await openVideo(page, photo);
      await page.waitForFunction(
        () => {
          const el = document.querySelector('#viewer-video');
          return el && el.currentTime > 0 && !el.error;
        },
        null,
        { timeout: 30_000 }
      );
      const error = await videoHandle(page).evaluate((el) => el.error?.code ?? null);
      expect(error).toBeNull();
      await TestHelpers.closeViewer(page);
    }
  });
```

- [ ] **Step 5: Run the whole E2E suite and the backend suite**

```bash
npm run build
cargo build --bin turbo-pix
npm run test:e2e
cargo test
```

Expected: all green. Cold-cache note: with an empty `./data/models` the E2E health check times out — pre-seed with `./target/debug/turbo-pix --download-models` first (AGENTS.md learning 10).

- [ ] **Step 6: Commit**

```bash
git add test-data tests/e2e/setup/global-setup.js tests/e2e/specs/video-streaming.e2e.spec.js
git commit -m "test(video): cover the playback capability matrix end to end"
```

---

### Task 8: Cleanup, docs and learnings

**Files:**
- Modify: `frontend/src/lib/utils.js` (remove the dead `clientCodecsHeader` getter and the stale comments referencing it)
- Modify: `README.md` if it documents environment variables (add `TURBO_PIX_STREAM_QUEUE_WAIT_SECS`, default 20 s)
- Modify: `AGENTS.md` (Learnings section — fold, do not append)
- Modify: `.spec/native-video-playback.md` (tick the acceptance criteria that are now covered, if the spec tracks status)

- [ ] **Step 1: Remove the dead code**

`clientCodecsHeader` (`frontend/src/lib/utils.js:414-420`) has zero call sites and its comment claims the header is used — it is not. Remove the getter and fix the comments at `frontend/src/lib/utils.js:308-312` and `:393-397` to say the capability string travels as the `?client=` query parameter (the header path remains server-side for other clients).

Verify: `grep -rn "clientCodecsHeader" frontend tests` returns nothing; `npm run lint` clean.

- [ ] **Step 2: Fold the new learnings into `AGENTS.md`**

The Learnings section is capped at 10 entries — fold, never append. Concrete folds:

- Entry 7 (native-first video serving): add (a) missing capability facts mean "probe the file once, persist `capability_version`, then decide" — never let an absent container select conversion; (b) ffmpeg fMP4 streams rebase timestamps to 0, so seeks are implemented with `SourceBuffer.timestampOffset`, not ffmpeg flags; (c) the `/video/stream` endpoint is permit-gated by `transcode_semaphore()` and answers `503 + Retry-After` when saturated; waiters must stay user-visible; (d) MSE is the only secure-context-free seekable path (WebCodecs needs HTTPS).
- Entry 1 (i18n): note the two new `video.stream.*` keys.

Re-read every entry for staleness while in there (the repo requires verifying entries still hold).

- [ ] **Step 3: Full gates**

```bash
cargo fmt --check
cargo clippy --all-targets -D warnings
npm run lint && npm run format:check 2>/dev/null || npm run lint
npm run test:i18n
npm run build && cargo build --bin turbo-pix
cargo test
npm run test:e2e
```

Expected: every command clean. Report the exact command list and results in the final summary.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/utils.js AGENTS.md README.md .spec/native-video-playback.md
git commit -m "docs(video): fold streaming learnings and drop dead capability-header code"
```

---

## Self-Review

**Spec coverage**

| Requirement | Task |
|---|---|
| FR-001 direct/remux without re-encode | 1, 2, 3 |
| FR-002 derive + persist missing metadata | 1 |
| FR-003 decision over codec/container/bit-depth/layout/audio/client | 3 |
| FR-004 full client declaration incl. HEVC + honor it | 3 |
| FR-005 first frame ≤ 5 s, non-blocking notice | 2, 3 (E2E timing) |
| FR-006 true duration + seek ≤ 3 s | 2, 3 (E2E seek) |
| FR-007 audio-only conversion | 3, 5 (whole-file args) |
| FR-008 notice only while genuinely converting | 3 |
| FR-009 self-heal on wrong declaration | 6 |
| FR-010 reuse without gating first playback | 5 |
| FR-011 bounded concurrency, visible waiting | 2, 4 |
| FR-012 actionable errors, no partial artifacts, escape hatch | 2 (supervisor cleanup), 6 |
| FR-013 no source modification by playback paths | 2, 5 (all outputs are cache artifacts) |
| FR-014 empty/`.pending-*` + photo endpoints unchanged | 2 (early return), 3 (no changes to non-video handlers) |
| SC-001/002 (census) | 1 + 3 matrix E2E |
| SC-003/004 (timings) | 3, 7 |
| SC-005 (platform matrix) | 7 (Chromium E2E) + manual Firefox/Edge/Android pass listed in the final report |
| SC-006 (saturation) | 4 |
| SC-007 (reuse ≤ 2 s) | 5 |
| SC-008 (failure + original playable) | 6 |

**Known gaps, stated honestly**

- Safari/iOS are explicitly out of scope; nothing in this plan verifies them.
- Firefox/Edge/Android cannot be exercised by this repo's Chromium-only Playwright setup; Task 7's matrix runs in Chromium and the remaining browsers are a manual verification step recorded in the final report. MSE + fMP4 (avc1/mp4a) is supported across the in-scope engines.
- The scan-time `fix_moov_atom` (`src/video_processor.rs:615-668`) still rewrites source files in place during indexing. That is an *indexing* behaviour outside this spec's "playback decisions and delivery only" boundary; the plan deliberately does not change it and instead guarantees that every playback-time path writes only cache artifacts. Flag it to the user as a follow-up if FR-013 is read strictly.
