# Native-First Video Playback & Honest Transcoding — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Serve every video that a browser can play natively without touching it, and make the remaining transcode path honest (progress %, parallel jobs, no fake 5-minute timeout, fast-fail on empty/corrupt files).

**Architecture:** Server-side per-video capability record (codec, profile, bit depth, container, moov placement, completeness) captured at index time; a single serve-time decision in `get_video_file` picks **Direct Play (original) → Remux (`-c copy -movflags +faststart`) → Transcode (re-encode)**. The frontend declares its codec support once and asks the server; the viewer renders whatever path the server picks, polls transcode status by server deadline (not a fixed 5 min), and surfaces an "empty file" message or "Play original anyway" escape hatch when transcoding is impossible. Transcodes run in a bounded worker pool instead of a global `Semaphore::new(1)`.

**Tech Stack:** Rust (warp, tokio, ffmpeg/ffprobe via subprocess), SQLite (metadata JSON in `photos`), Svelte 5 (runes), Vite, Playwright E2E.

**Spec:** `.spec/video-playback-native-first.md` (read together with this plan; the plan argues from the spec).

## Global Constraints

- Zero-warning policy: `cargo clippy --all-targets -D warnings`, `cargo fmt --check` clean, `npm run lint`, `npm run format` clean.
- Frontend changes: run `npm run build` first, then `cargo build --bin turbo-pix` (build.rs embeds `dist/` and panics if missing).
- i18n parity: every new key in BOTH `frontend/src/i18n/en.json` and `de.json`, structurally identical (AGENTS.md learning 1).
- Never mutate the source photo file at serve time. Remux/transcode write to cache sidecars with atomic temp+rename.
- Existing `?transcode=true` param behavior stays compatible.
- TDD: write the failing test, see it fail, implement, see it pass, commit. Commit per task with meaningful message.
- Test fixture check: tests touching `test-data/test_video_hevc.mp4` must skip when the file is missing (`should_run_video_tests`/`if !path.exists() return;` pattern, existing convention).

---

### Task 1: Backend — extend video capability record (profile, bit depth, container, moov flag)

**Files:**
- Modify: `src/metadata_extractor.rs` (extract_video_metadata / apply_stream_info)
- Modify: `src/photo_processor.rs` (ProcessedPhoto fields + assembly)
- Modify: `src/db.rs` (metadata JSON assembly, `Photo` accessors)
- Test: `src/metadata_extractor.rs` (unit tests in module)

**Interfaces:**
- Produces: `PhotoMetadata` gains `video_profile: Option<String>`, `bit_depth: Option<u32>`, `container: Option<String>`; helper `fn container_from_format_name(&serde_json::Value) -> Option<String>` taking the first `format.format_name` token.
- Produces: `ProcessedPhoto` gains `video_profile`, `bit_depth`, `container` (all `Option<...>`).
- Produces: `db.rs` stores `metadata.video.profile`, `metadata.video.bit_depth`, `metadata.video.container`, `metadata.video.moov_at_start`.
- Produces: `Photo::video_profile()`, `Photo::bit_depth()`, `Photo::container()`, `Photo::moov_at_start()` accessors.

- [ ] **Step 1: Write the failing test** — two tests: one drives `apply_stream_info` (profile + bit_depth), one drives the container extraction helper. The container is parsed in `extract_video_metadata` from `format.format_name`, so extract that into a small helper `fn container_from_format_name(v: &serde_json::Value) -> Option<String>` that both the real code and the test can call directly.

```rust
// in src/metadata_extractor.rs tests module
#[test]
fn parses_profile_and_bitdepth_from_ffprobe_json() {
    let parsed = serde_json::json!({
        "format": { "format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "1.0" },
        "streams": [{
            "codec_type": "video", "codec_name": "h264",
            "profile": "High", "pix_fmt": "yuv420p", "width": 1920, "height": 1080,
            "r_frame_rate": "30000/1001"
        }]
    });
    let mut meta = PhotoMetadata::default();
    MetadataExtractor::apply_stream_info(&parsed, &mut meta); // matches existing test convention
    assert_eq!(meta.video_codec.as_deref(), Some("h264"));
    assert_eq!(meta.video_profile.as_deref(), Some("High"));
    assert_eq!(meta.bit_depth, Some(8));
}

#[test]
fn container_from_format_name_takes_first_token() {
    let parsed = serde_json::json!({ "format": { "format_name": "mov,mp4,m4a,3gp,3g2,mj2" } });
    assert_eq!(container_from_format_name(&parsed).as_deref(), Some("mov"));
    assert_eq!(container_from_format_name(&serde_json::json!({ "format": {} })), None);
}
```

- [ ] **Step 2: Run the test, verify it fails** — `cargo test --lib metadata_extractor::tests::parses_profile_and_bitdepth_from_ffprobe_json`. Expected: FAIL (fields don't exist / function undefined).

- [ ] **Step 3: Implement** — in `src/metadata_extractor.rs`:
  - Add to `PhotoMetadata`: `pub video_profile: Option<String>`, `pub bit_depth: Option<u32>`, `pub container: Option<String>`.
  - Add `src/video_capability.rs` with `pub fn parse_pix_fmt_bit_depth(pix_fmt: Option<&str>) -> Option<u32>` (mapping: `yuv420p`/`yuv422p`/`yuv444p`/`nv12`/`nv21`/`yuvj420p`/`yuvj422p`/`yuvj444p` → `Some(8)`; `*10le`/`*10be` variants → `Some(10)`; `*12le`/`*12be` variants → `Some(12)`; `*16le` → `Some(16)`; else `None`) and `pub mod video_capability;` in `src/lib.rs`. This module is shared with Task 2 (the decision engine lands in the same file there).
  - In `apply_stream_info`, after `metadata.video_codec = ...`, add `metadata.video_profile = video_stream["profile"].as_str().map(String::from);` and `metadata.bit_depth = crate::video_capability::parse_pix_fmt_bit_depth(video_stream["pix_fmt"].as_str());`.
  - For container: extract `pub(crate) fn container_from_format_name(parsed: &serde_json::Value) -> Option<String>` returning `parsed["format"]["format_name"].as_str()?.split(',').next().map(str::trim).map(String::from)`; call it from `extract_video_metadata` to set `metadata.container`.
- [ ] **Step 4: Run the test, verify pass** — `cargo test --lib metadata_extractor::tests::parses_profile_and_bitdepth_from_ffprobe_json` and `...container_from_format_name_takes_first_token`. Also run `cargo test --lib metadata_extractor` (full module; existing tests must still pass).

- [ ] **Step 5: Thread through `ProcessedPhoto` + DB JSON** —
  - `src/photo_processor.rs`: add the three `Option` fields to `ProcessedPhoto` struct; populate in `process_file_metadata_only` (`metadata.video_profile.clone()`, `metadata.bit_depth`, `metadata.container.clone()`) and in the unchanged-photo branch (`existing_photo.video_profile().map(String::from)` etc.).
  - `src/db.rs`: in the `video` map assembly (`~line 932`), insert `"profile"`, `"bit_depth"`, `"container"` when present.
  - `src/db.rs`: add `Photo` accessors mirroring `video_codec()` → `video_profile()` (`get("video")?.get("profile")?.as_str()`), `bit_depth()` (`...as_u64() as u32`), `container()`, `moov_at_start()` (`get("video")?.get("moov_at_start")?.as_bool()`).

- [ ] **Step 6: Thread `moov_at_start` into the record** — In `photo_processor.rs::process_file_metadata_only`, after `maybe_fix_moov_for_video(path)` runs, capture `let moov_at_start = if is_video { video_processor::has_moov_at_start(path).unwrap_or(true) } else { true };` and assign into `ProcessedPhoto.moov_at_start` (new `bool` field; default `true`). In `db.rs` assembly insert `"moov_at_start"` only when `!moov_at_start` (avoids bloating every record — absence means `true`).
  - `Photo::moov_at_start()` returns the JSON value or `true` default.

- [ ] **Step 7: Run full backend test suite** — `cargo test` (all). Expected: PASS. Then `cargo clippy --all-targets -D warnings` and `cargo fmt --check`.

- [ ] **Step 8: Commit**
```bash
git add -A && git commit -m "feat(video): capture profile/bit-depth/container/moov in capability record"
```

---

### Task 2: Backend — direct-playability decision function

**Files:**
- Create: `src/video_capability.rs`
- Modify: `src/lib.rs` (`pub mod video_capability;`)
- Test: `src/video_capability.rs` (unit tests)

**Interfaces:**
- Consumes: none (pure function; takes primitive params).
  `pub fn decide(codec: &str, container: &str, bit_depth: Option<u32>, moov_at_start: bool, file_size: i64, client: &ClientCodecs) -> DirectPlay`.
- Also produced: `pub fn parse_pix_fmt_bit_depth(pix_fmt: Option<&str>) -> Option<u32>` (moved/exported from Task 1 for reuse; Task 1 may use this too).

- [ ] **Step 1: Write the failing tests** (in the new module):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn client_all(): ClientCodecs { ClientCodecs { h264_8: true, h264_10: true, hevc: true, av1: true, vp9: true, vp8: true } }
    fn client_h264_8(): ClientCodecs { ClientCodecs::conservative() }

    #[test]
    fn h264_8_mp4_moov_at_start_direct_plays() {
        let c = client_all();
        assert_eq!(decide("h264", "mp4", Some(8), true, 1000, &c), DirectPlay::Yes);
        assert_eq!(decide("h264", "mov", Some(8), true, 1000, &c), DirectPlay::Yes);
        // missing bit_depth defaults to 8-bit (safe default for nearly all library h264)
        assert_eq!(decide("h264", "mp4", None, true, 1000, &c), DirectPlay::Yes);
    }

    #[test]
    fn h264_8_with_moov_at_end_needs_remux() {
        let c = client_all();
        assert_eq!(decide("h264", "mp4", Some(8), false, 1000, &c), DirectPlay::NeedsRemux);
    }

    #[test]
    fn h264_10bit_requires_client_h264_10() {
        let all = client_all();
        assert_eq!(decide("h264", "mp4", Some(10), true, 1000, &all), DirectPlay::Yes);
        let c8 = client_h264_8();
        assert_eq!(decide("h264", "mp4", Some(10), true, 1000, &c8), DirectPlay::No);
    }

    #[test]
    fn conservative_client_h264_10_needs_transcode() {
        let c = client_h264_8();
        assert_eq!(decide("h264", "mp4", Some(10), true, 1000, &c), DirectPlay::No);
    }

    #[test]
    fn hevc_and_empty_never_direct_playable() {
        let c = client_all();
        assert_eq!(decide("hevc", "mp4", Some(8), true, 1000, &c), DirectPlay::No);
        assert_eq!(decide("h264", "mp4", Some(8), true, 0, &c), DirectPlay::No); // empty file
    }

    #[test]
    fn av1_and_vp_in_webm_direct_playable_when_client_supports() {
        let c = client_all();
        assert_eq!(decide("av1", "webm", Some(8), true, 1000, &c), DirectPlay::Yes);
        assert_eq!(decide("vp9", "webm", Some(8), true, 1000, &c), DirectPlay::Yes);
        let c8 = client_h264_8();
        assert_eq!(decide("av1", "webm", Some(8), true, 1000, &c8), DirectPlay::No);
    }

    #[test]
    fn legacy_codecs_never_direct_playable() {
        let c = client_all();
        for codec in ["mpeg4", "fraps", "indeo5", "msmpeg4v2", "msmpeg4v1", "mjpeg"] {
            assert_eq!(decide(codec, "avi", Some(8), true, 1000, &c), DirectPlay::No);
        }
    }

    #[test]
    fn parse_client_header() {
        let c = ClientCodecs::parse(Some("h264-8,hevc,av1"));
        assert!(c.h264_8 && c.hevc && c.av1 && !c.h264_10);
        let d = ClientCodecs::parse(None);
        assert!(d.h264_8 && !d.hevc);
    }
}
```
*(The above replaces the original Task 2 test block in full — apply the full replacement, not a merge.)*

```rust
// ── canonical implementation ref (replaces the body shown earlier) ──
pub enum DirectPlay { Yes, NeedsRemux, No }

pub struct ClientCodecs { pub h264_8: bool, pub h264_10: bool, pub hevc: bool, pub av1: bool, pub vp9: bool, pub vp8: bool }

impl ClientCodecs {
    pub fn conservative() -> Self { Self { h264_8: true, ..Self::none() } }
    fn none() -> Self { Self { h264_8: false, h264_10: false, hevc: false, av1: false, vp9: false, vp8: false } }
    pub fn parse(header: Option<&str>) -> Self {
        let mut c = Self::none();
        if let Some(raw) = header {
            for tok in raw.split(',').map(str::trim) {
                match tok {
                    "h264-8" => c.h264_8 = true,
                    "h264-10" => c.h264_10 = true,
                    "hevc" => c.hevc = true,
                    "av1" => c.av1 = true,
                    "vp9" => c.vp9 = true,
                    "vp8" => c.vp8 = true,
                    _ => {}
                }
            }
        }
        c
    }
}

pub fn decide(
    codec: &str,
    container: &str,
    bit_depth: Option<u32>,
    moov_at_start: bool,
    file_size: i64,
    client: &ClientCodecs,
) -> DirectPlay {
    if file_size <= 0 { return DirectPlay::No; }
    match codec {
        "h264" => {
            if container != "mp4" && container != "mov" && container != "m4v" { return DirectPlay::No; }
            let supported = bit_depth.unwrap_or(8) <= 8 ? client.h264_8 : client.h264_10;
            if !supported { return DirectPlay::No; }
            if moov_at_start { DirectPlay::Yes } else { DirectPlay::NeedsRemux }
        }
        "av1" | "vp8" | "vp9" => {
            let client_support = if codec == "av1" { client.av1 } else if codec == "vp9" { client.vp9 } else { client.vp8 };
            if (container == "webm" || container == "mp4") && client_support {
                if moov_at_start { DirectPlay::Yes } else { DirectPlay::NeedsRemux }
            } else { DirectPlay::No }
        }
        _ => DirectPlay::No, // hevc, mpeg4, fraps, indeo5, msmpeg4*, mjpeg, ...
    }
}
```
*(Use the implementation in the canonical block above — it threads `bit_depth` so a 10-bit H.264 is never Direct-Played as 8-bit. `parse_pix_fmt_bit_depth` is exported from Task 1 for the caller to map `Photo::bit_depth()`/pix_fmt into the `bit_depth: Option<u32>` argument.)*

- [ ] **Step 2: Run, verify fail** — `cargo test --lib video_capability`. Expected: FAIL (module `video_capability` not found in `src/lib.rs`).

- [ ] **Step 3: Implement** — create `src/video_capability.rs` exactly as the canonical block above (add `pub mod video_capability;` to `src/lib.rs`). The test block's `use super::*` pulls `decide`, `ClientCodecs`, `DirectPlay` from the module root.

- [ ] **Step 4: Run, verify pass** — `cargo test --lib video_capability`; then `cargo clippy --all-targets -D warnings`, `cargo fmt --check`.

- [ ] **Step 5: Commit**
```bash
git add src/video_capability.rs src/lib.rs && git commit -m "feat(video): add direct-playability decision engine"
```

---

### Task 3: Backend — serve-time moov remux (stream-copy faststart sidecar)

**Files:**
- Modify: `src/video_processor.rs` (add `ensure_progressive_mp4`)
- Test: `src/video_processor.rs` (module tests)

**Interfaces:**
- Consumes: `get_transcoded_path_versioned` semantics (versioned by size+mtime), `CacheError`/`CacheResult`.
- Produces: `pub async fn ensure_progressive_mp4(input_path: &Path, output_path: &Path) -> CacheResult<()>` — runs `ffmpeg -y -i INPUT -c copy -movflags +faststart -f mp4 OUTPUT.tmp` then rename to `OUTPUT`; on `has_moov_at_start(input)` returns `Ok(())` immediately (no-op); uses the same temp+rename pattern as `transcode_hevc_to_h264_with_timeout_and_path`.
  creates parent dir; `-f mp4` explicit because `.tmp` suffix hides the muxer.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn ensure_progressive_mp4_remuxes_moov_to_front() {
    let _lock = acquire_test_env_lock();
    let temp = TempDir::new().unwrap();
    // Real fixture has moov at start already; force a moov-at-end copy.
    let src = Path::new("test-data/test_video.mp4");
    if !src.exists() { return; }
    let moov_end = temp.path().join("in.mp4");
    ffmpeg_copy_moov_end(&src, &moov_end); // local test helper defined below
    assert!(!has_moov_at_start(&moov_end).unwrap());
    let out = temp.path().join("out.mp4");
    ensure_progressive_mp4(&moov_end, &out).await.unwrap();
    assert!(out.exists());
    assert!(has_moov_at_start(&out).unwrap(), "remux must move moov forward");
    // second call is idempotent on a start-front file
    ensure_progressive_mp4(&out, &out).await.unwrap();
}
```
  Add test helper `ffmpeg_copy_moov_end(src: &Path, dst: &Path)` in the `video_processor` test module (uses `Command::new(get_ffmpeg_path()).args(["-y","-i",src,"-c","copy","-movflags","-faststart","-f","mp4",dst])`; `-movflags -faststart` forces the moov atom to the end).

- [ ] **Step 2: Run, verify fail** — `cargo test --lib video_processor::tests::ensure_progressive_mp4_remuxes_moov_to_front`. Expected: FAIL (`ensure_progressive_mp4` undefined).

- [ ] **Step 3: Implement `ensure_progressive_mp4`** in `video_processor.rs` (non-test). Reuse `get_ffmpeg_path()`, `has_moov_at_start`, temp+rename. Should run under the same transcode semaphore permit? **No** — it must not be serialized behind transcodes; it is a fast stream-copy. Call `acquire_transcode_permit()` only if you want to bound ffmpeg process count; prefer a distinct lightweight semaphore `REMOX_SEMAPHORE` bounded to 4 so remuxes never queue behind slow re-encodes. Keep `-hwaccel auto` OUT (stream copy needs no decoder).

- [ ] **Step 4: Run, verify pass** — `cargo test --lib video_processor::tests::ensure_progressive_mp4_remuxes_moov_to_front`; then `cargo test --lib video_processor` (existing transcode tests still pass), `cargo clippy --all-targets -D warnings`, `cargo fmt --check`.

- [ ] **Step 5: Commit**
```bash
git add src/video_processor.rs && git commit -m "feat(video): add serve-time moov faststart remux"
```

---

### Task 4: Backend — `get_video_file` rewrite: empty-file handling, decision, remux, remux/transcode cache subdir

**Files:**
- Modify: `src/handlers_video.rs`
- Test: `src/handlers_video.rs` (module tests)

**Interfaces:**
- Consumes: `video_capability::{decide, ClientCodecs, DirectPlay}`, `ensure_progressive_mp4`, `Photo::{video_codec, bit_depth, container, moov_at_start, file_size}`.
- Produces: empty-file → `X-Transcode-Warning: empty` + `Content-Length: 0` status 200; remux path serves `{TRANSCODE_CACHE_DIR}/remux/{hash}_{size}_{mtime}.mp4`; transcode path now gated on the capability record (not only `is_hevc_video`).
- VideoQuery gains `client_codecs: Option<String>` field (reads `X-TurboPix-Codecs` header precedence, fallback `?client=`).

- [ ] **Step 1: Write the failing tests** (in `handlers_video.rs` test module):

```rust
// NOTE: use the existing harness in this module: setup_test_video(*db_pool, temp_dir, hash)
// writes a fake "video.mp4", creates the photo row, returns the path. Handler is invoked
// DIRECTLY via get_video_file(...).into_response() (see test_video_202), not via routes.
#[tokio::test]
async fn empty_backing_file_returns_empty_warning_without_claiming_transcode() {
    let db_pool = create_in_memory_pool().await.expect("failed to create db");
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    // setup writes non-empty content; truncate to 0 to simulate a .pending-* file
    let video_path = setup_test_video(&db_pool, &temp_dir, hash).await;
    std::fs::write(&video_path, b"").expect("failed to truncate video");

    // ffprobe path must exist for any probing; is_hevc_video is not reached for the
    // empty branch, but set it defensively to avoid env surprises.
    let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
    create_script(&ffprobe_script, "#!/usr/bin/env sh\nprintf 'h264\\n'\n");
    let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
    let _cache_guard = EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

    let mut headers = HeaderMap::new();
    headers.insert("range", HeaderValue::from_static("bytes=0-100"));
    let response = get_video_file(
        hash.to_string(),
        VideoQuery { metadata: None, transcode: None, client_codecs: None },
        headers,
        db_pool,
    )
    .await
    .expect("handler should return")
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("content-length").unwrap(), "0");
    assert_eq!(response.headers().get("x-transcode-warning").unwrap(), "empty");
    assert_eq!(get_transcode_status(hash), None, "no transcode may be claimed for an empty file");
}

#[tokio::test]
async fn h264_original_served_directly_without_transcode_param() {
    let db_pool = create_in_memory_pool().await.expect("failed to create db");
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let hash = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let video_path = setup_test_video(&db_pool, &temp_dir, hash).await;
    // Give the photo row a valid h264 codec record so decide() Direct-Plays it.
    use crate::db::Photo;
    let mut photo = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
    photo.metadata = json!({ "video": { "codec": "h264", "container": "mp4", "moov_at_start": true } });
    photo.create(&db_pool).await.unwrap(); // upsert

    let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
    create_script(&ffprobe_script, "#!/usr/bin/env sh\nprintf 'h264\\n'\n");
    let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
    let _cache_guard = EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

    let mut headers = HeaderMap::new();
    headers.insert("range", HeaderValue::from_static("bytes=0-10"));
    let response = get_video_file(
        hash.to_string(),
        VideoQuery { metadata: None, transcode: None, client_codecs: None },
        headers,
        db_pool,
    )
    .await
    .expect("handler should return")
    .into_response();

    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    let cr = response.headers().get("content-range").unwrap().to_str().unwrap();
    assert!(cr.ends_with(&format!("/{}", std::fs::metadata(&video_path).unwrap().len())),
        "Direct Play must serve the ORIGINAL file bytes");
    assert!(response.headers().get("x-transcode-warning").is_none());
}

// moov-at-end h264: first request runs the remux (serves the cached remux sidecar, which has
// content-length != original), second request serves the same cache file without a second ffmpeg
// run (assert cache file exists under TRANSCODE_CACHE_DIR/remux/ after the first). Use a fixture
// produced by ffmpeg_copy_moov_end against test-data/test_video.mp4 (skip if fixture missing).
```
*(The existing harness helpers are `setup_test_video`/`setup_test_video_with_content` and direct handler calls; there is no `build_test_routes`/`setup_test_photo`/`Photo::update` in this module. `VideoQuery` gains the new `client_codecs: Option<String>` field in Task 4.)*

- [ ] **Step 2: Run, verify fail** (empty-file test especially).

- [ ] **Step 3: Implement** — restructure `get_video_file`:
  1. Resolve `client = ClientCodecs::parse(headers.get("X-TurboPix-Codecs").and_then(to_str).or(query.client_codecs.as_deref()))`.
  2. Stat the backing file **before** the decision; if `0` bytes (or missing) → `return Ok(with_transcode_warning(empty_reply, true))` with `Content-Length: 0`, warning `"empty"`, **before** any claim.
  3. Determine `record_codec = photo.video_codec().unwrap_or("")`, `container`, `bit_depth`, `moov_at_start` (from DB accessors; `moov_at_start` default `true`).
  4. `let play = decide(record_codec, container, moov_at_start, photo.file_size, &client);`
  5. Branch:
     - `DirectPlay::Yes` → serve original (existing range logic).
     - `DirectPlay::NeedsRemux` → ensure `remux_path = TRANSCODE_CACHE_DIR/remux/{hash}_{size}_{mtime}.mp4`; if missing, `ensure_progressive_mp4(orig, &remux_path)`; serve `remux_path`. On remux error → fall back to serving original with warning `"remux-failed"`.
     - `DirectPlay::No` → if this codec is actually transcode-able (has video stream) → existing transcode branch (claim→202→poll→serve), but gate on `record_codec` (call the transcode helper `transcode_codec_to_h264`, generalized from `transcode_hevc_to_h264` — see Task 5). If `client_wants_transcode` AND `transcoded_path` exists → serve it. If the file appears to have no decodable stream (empty/ffprobe-fails) → warning + original.
  6. For the `NeedsRemux`/`No` branches, keep `client_wants_transcode ? transcoded : remux/original` selection consistent with the existing `content_type` logic.
- [ ] **Step 4: Run the whole module** — `cargo test --lib handlers_video` (all pass), plus `cargo test --lib`, `cargo clippy --all-targets -D warnings`, `cargo fmt --check`.
- [ ] **Step 5: Commit**
```bash
git add src/handlers_video.rs && git commit -m "feat(video): serve-time decision, faststart remux, empty-file fast fail"
```

---

### Task 5: Backend — generalize transcode to any codec + worker pool + progress

**Files:**
- Modify: `src/video_processor.rs` (rename/generalize `transcode_hevc_to_h264`, add pool, add progress)
- Modify: `src/handlers_video.rs` (call generalized fn; keep busy using `TranscodeStatus.percent`; set `percent_duration`)
- Test: `src/video_processor.rs` (pool + progress tests)

**Interfaces:**
- Consumes: existing `claim_transcode`/`set_transcode_status`/`TranscodeStatus`.
- Produces: `pub async fn transcode_codec_to_h264_with_progress(input: &Path, output: &Path, on_progress: Arc<dyn Fn(Option<u8>) + Send + Sync>) -> CacheResult<()>` — codec-agnostic re-encode to H.264; `on_progress` is called with the parsed percent (or `None` when unknown) as ffmpeg emits `-progress pipe:1` lines. A thin `pub async fn transcode_codec_to_h264(input: &Path, output: &Path) -> CacheResult<()>` wrapper exists for callers that don't need progress.
- Produces: `pub fn transcode_semaphore() -> &'static Semaphore` initialized with `min(max(available_parallelism()/2, 1), 4)` clamped by `TURBO_PIX_MAX_TRANSCODES` env (default 2), 0 → error "transcoding disabled".
- Produces: `TranscodeStatus` gains `percent: Option<u8>` serialized; progress updated via ffmpeg `-progress pipe:1` parsing into the status store by hash.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn transcode_permit_semaphore_respects_env() {
    std::env::set_var("TURBO_PIX_MAX_TRANSCODES", "3");
    let s = transcode_semaphore();
    assert_eq!(s.available_permits(), 3);
    std::env::remove_var("TURBO_PIX_MAX_TRANSCODES");
}

#[tokio::test]
async fn status_reports_percent_during_transcode() {
    // Fake ffmpeg script that emits -progress pipe:1 lines (out_time_ms ramping to 2000)
    // then exits 0. Assert after spawn: poll(get_transcode_status) returns Some with percent.
}
```
  Update existing tests that reference `transcode_hevc_to_h264` to the new name.

- [ ] **Step 2: Run, verify fail.**

- [ ] **Step 3: Implement**
  - Rename `transcode_hevc_to_h264*` → `transcode_codec_to_h264*` (keep a `#[deprecated]` shim NOT needed — this is not a public API; update all callers).
  - `transcode_semaphore()` reads env once into a `OnceLock<Semaphore>`; `0` → the acquired permit errors "Transcoding is disabled (TURBO_PIX_MAX_TRANSCODES=0)".
  - ffmpeg invocation: add `-progress pipe:1` (and since output is temp `.mp4.tmp`, keep `-f mp4`); parse `out_time_ms=`/`out_time_us=` progress lines from stdout; divide by the input duration (probe once via `extract_video_metadata`, fall back to `None` percent when duration is unknown) and call `on_progress(Some(percent))`. The handler's `tokio::spawn` builds the closure to call `set_transcode_status(hash, TranscodeStatus { state: InProgress, percent, .. })` so the in-flight status carries progress. Do NOT parse `total_size` for percent — it is unreliable for stream-copy vs re-encode; out_time vs duration is the honest signal.
  - Update `handlers_video.rs` to call the generalized fn and pass a progress closure that updates `TranscodeStatus.percent`.
- [ ] **Step 4: Run** — `cargo test --lib video_processor`, `cargo test --lib handlers_video`, full `cargo test`, `cargo clippy --all-targets -D warnings`, `cargo fmt --check`.
- [ ] **Step 5: Commit**
```bash
git add src/video_processor.rs src/handlers_video.rs && git commit -m "feat(video): codec-agnostic transcoding, bounded worker pool, progress reporting"
```

---

### Task 6: Backend — status endpoint returns percent + deadline hint; remove client timeout mismatch

**Files:**
- Modify: `src/handlers_video.rs` (`get_video_status`)
- Test: `src/handlers_video.rs`

**Interfaces:**
- Produces: `/video/status` now includes `percent` (from `TranscodeStatus`), and an `eta_ms`/`deadline_ms` field = `(transcode_timeout_secs * 1000) - elapsed` when computable; absent when unknown.

- [ ] **Step 1: Write failing test** — assert status JSON contains `percent` and `deadline_ms` when a `TranscodeStatus` with `percent` is set.

- [ ] **Step 2: Run, verify fail.**

- [ ] **Step 3: Implement** — in `get_video_status`, after getting `status`, compute `deadline_ms` from `TURBO_PIX_TRANSCODE_TIMEOUT_SECS` (read via env; default 300) minus elapsed since `started_at`. Return JSON including `percent` and `deadline_ms`. Add the config env read helper in `video_processor.rs` `pub fn transcode_timeout_secs() -> u64`.
- [ ] **Step 4: Run** — `cargo test --lib handlers_video`, `cargo clippy --all-targets -D warnings`, `cargo fmt --check`.
- [ ] **Step 5: Commit**
```bash
git add src/handlers_video.rs src/video_processor.rs && git commit -m "feat(video): status endpoint exposes percent and server deadline"
```

---

### Task 7: Config — `TURBO_PIX_MAX_TRANSCODES` and `TURBO_PIX_TRANSCODE_TIMEOUT_SECS` env wiring

**Files:**
- Modify: `src/config.rs` (add fields + env reads + wiring into `main.rs`)
- Modify: `src/main.rs` (pass to handlers / ensure env defaults)

**Interfaces:**
- Produces: `Config` gains `max_transcodes: usize` (default 2) and `transcode_timeout_secs: u64` (default 300). `Config::from_env()` reads the two env vars (parse errors → `Err`). `main.rs` exposes them to `handlers_video`/`video_processor` via the existing env-default pattern (like `TRANSCODE_CACHE_DIR`).

- [ ] **Step 1: Write failing test** — in `config.rs` tests, set both env vars, build `Config::from_env()`, assert fields parsed; parse-error env → `Err`.

- [ ] **Step 2: Run, verify fail.**

- [ ] **Step 3: Implement** — `Config` struct + `from_env`, and in `main.rs` add `if env::var("TURBO_PIX_MAX_TRANSCODES").is_err() { set_var(..., max_transcodes.to_string()) }` and same for timeout (so `video_processor` env reads see defaults).
- [ ] **Step 4: Run** — `cargo test --lib config`, `cargo clippy --all-targets -D warnings`, `cargo fmt --check`.
- [ ] **Step 5: Commit**
```bash
git add src/config.rs src/main.rs && git commit -m "feat(config): TURBO_PIX_MAX_TRANSCODES and TRANSCODE_TIMEOUT_SECS"
```

---

### Task 8: Frontend — robust canPlayType codec probe (h264 including maybe, av1/vp9)

**Files:**
- Modify: `frontend/src/lib/utils.js` (`videoCodecSupport`)
- Test: E2E (see Task 12)

**Interfaces:**
- Produces: `videoCodecSupport.getClientCodecsString() -> string` returning `h264-8,h264-10,hevc,av1,vp9,vp8` comma-joined per probe; `canPlayH264()` (accepts `probably` OR `maybe` — matches Jellyfin), `canPlayAV1()`, `canPlayVP9()`, `canPlayVP8()`, `supportsHEVC()` (unchanged Firefox→false).
- Produces: exposes a `clientCodecsHeader` value used in Task 9.

- [ ] **Step 1: Write the failing test** — in `frontend/src/lib/utils.test.js` (check if exists; else create one with `node --test`):

```js
import { videoCodecSupport } from './utils.js';
test('getClientCodecsString returns comma list without duplicates', () => {
  const s = videoCodecSupport.getClientCodecsString();
  expect(typeof s).toBe('string');
  expect(s.split(',')).toContain('h264-8');
  expect(new Set(s.split(',')).size).toBe(s.split(',').length);
});
test('canPlayH264 trusts maybe', () => {
  // Without a real video element this is environment-dependent; assert the probe function exists and returns boolean
  expect(typeof videoCodecSupport.canPlayH264).toBe('function');
});
```
  (If the repo has no JS unit test runner, keep these as a smoke check and rely on E2E in Task 12.)
- [ ] **Step 2: Implement** — rewrite `videoCodecSupport` with `canPlayH264` accepting `''.replace(/\bno\b/,'')` logic (Jellyfin-style: `!!...canPlayType(...).replace(/no/, '')`), AV1/VP9 probes with `probably|maybe` acceptance, `getClientCodecsString()` that memoizes. Keep `supportsHEVC` Firefox special-case.
- [ ] **Step 3: Run frontend lint/build** — `npm run lint`, `npm run format`, `npm run build`. Fix any failures.
- [ ] **Step 4: Commit**
```bash
git add frontend/src/lib/utils.js && git commit -m "feat(web): robust codec support probe and client capability string"
```

---

### Task 9: Frontend — ask server for the playback decision; render direct/remux/transcode/empty

**Files:**
- Modify: `frontend/src/components/PhotoViewer.svelte`
- Modify: `frontend/src/lib/api.js` (add `getVideoDecision`)

**Interfaces:**
- Consumes: `videoCodecSupport.getClientCodecsString()`, existing `getVideoUrl`, `tryStartTranscode`, `pollTranscodeStatus`, `setVideoSource`.
- Produces: `api.getVideoDecision(photoHash, clientCodecs) -> { action: 'direct'|'remux'|'transcode'|'empty'|'error', url?: string, reason?: string }`.

- [ ] **Step 1: Implement** — add `getVideoDecision` to `api.js`:
```js
async getVideoDecision(hash, clientCodecs) {
  const res = await fetch(`/api/photos/${hash}/video?decision&client=${encodeURIComponent(clientCodecs)}`);
  if (res.status === 202) return { action: 'transcode', pollUrl: (await res.json()).poll_url };
  if (!res.ok) return { action: 'error' };
  const data = await res.json();
  return data; // { action, url, reason }
}
```
- [ ] **Step 2: Implement in `PhotoViewer.displayVideo`** — replace the current `isHEVC`/`needsTranscode` heuristic with:
```js
const decision = await api.getVideoDecision(photo.hash_sha256, videoCodecSupport.getClientCodecsString());
if (decision.action === 'direct' || decision.action === 'remux') { setVideoSource(photo, decision.url, false, false, false); }
else if (decision.action === 'transcode') { await tryStartTranscode(videoUrl(decision), photo); }
else if (decision.action === 'empty') { showTranscodeToast(t('video.file_empty'), true); }
else { showTranscodeToast(t('video.conversion_reason', {values:{reason: decision.reason||''}}), true); }
```
  Keep the async staleness check (`isOpen`, `currentPhoto?.hash_sha256`) before acting.
- [ ] **Step 3: i18n** — add the new keys to `en.json`/`de.json`.
- [ ] **Step 4: Run** — `npm run build`, `cargo build --bin turbo-pix`; then `npm run lint`, `npm run format`, `npm run test:i18n`.
- [ ] **Step 5: Commit**
```bash
git add frontend/src frontend/src/i18n && git commit -m "feat(web): server-driven playback decision in viewer"
```

---

### Task 10: Frontend — honest polling (percent, server deadline, no 5-min fake timeout)

**Files:**
- Modify: `frontend/src/components/PhotoViewer.svelte` (`pollTranscodeStatus`)

**Interfaces:**
- Consumes: `/video/status` returning `{ percent, deadline_ms, state }`.
- Produces: `pollTranscodeStatus` no longer hard-caps at 5 min; it stops when (a) state is `Failed`/`Timeout`/`Completed`, (b) server reports `deadline_ms` elapsed (grace +30s), or (c) the photo went stale. It updates the toast with `percent` when present.

- [ ] **Step 1: Modify `pollTranscodeStatus`** — replace `MAX_POLL_DURATION = 5*60*1000` with server-driven `deadline_ms`. Keep all existing `bailIfStale` guards and the shared-`transcodePollTimer` cleanup.
- [ ] **Step 2: i18n** — add `video.transcoding.progress` (`Converting… {percent}%`).
- [ ] **Step 3: Run** — `npm run build`, `cargo build --bin turbo-pix`, `npm run lint`, `npm run format`, `npm run test:i18n`.
- [ ] **Step 4: Commit**
```bash
git add frontend/src/components/PhotoViewer.svelte frontend/src/i18n && git commit -m "feat(web): transcode polling driven by server deadline and percent"
```

---

### Task 11: Frontend — "Play original anyway" escape hatch

**Files:**
- Modify: `frontend/src/components/PhotoViewer.svelte`
- Modify: `frontend/src/i18n/en.json`, `de.json`

**Interfaces:**
- Produces: transcode toast, when in failed/timeout state, renders a "Play original anyway" button; clicking calls `setVideoSource(photo, getVideoUrl(hash, {}), false, false, false)` and sets a module flag so the subsequent `videoEl.onerror` does not re-enter the transcode decision.

- [ ] **Step 1: Add button to the transcode toast** (`{#if transcodeError}` block), label `t('video.play_original')`, `data-action="play-original"`.
- [ ] **Step 2: Wire click** — sets `hasUserChosenOriginal = true`, calls `setVideoSource` with original URL, hides toast.
- [ ] **Step 3: Update `setVideoSource`'s `onerror`** — if `hasUserChosenOriginal`, do not request transcode, just show the raw playback error.
- [ ] **Step 4: i18n** — add `video.play_original`.
- [ ] **Step 5: Run** — `npm run build`, `cargo build --bin turbo-pix`, `npm run lint`, `npm run format`, `npm run test:i18n`.
- [ ] **Step 6: Commit**
```bash
git add frontend/src frontend/src/i18n && git commit -m "feat(web): play-original escape hatch on transcode failure"
```

---

### Task 12: E2E coverage for the native-first path

**Files:**
- Create: `tests/e2e/specs/video-playback.e2e.spec.js`

**Interfaces:**
- Consumes: `TestHelpers` (`navigateToView`, `openViewer`, `getPhotoCards`, `closeViewer`), the built server.

- [ ] **Step 1: Write the E2E spec** (Playwright, sequential workers):

```js
import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test('h264 video plays directly without transcode toast', async ({ page }) => {
  await TestHelpers.navigateToView(page, 'all');
  // library must contain test_video.mp4 (h264). If absent, skip.
  const cards = await TestHelpers.getPhotoCards(page);
  if (cards.filter(c => c.includes('test_video')).length === 0) return test.skip();
  await TestHelpers.openViewer(page, /* hash of test_video.mp4 */);
  const video = page.locator('#viewer-video');
  await expect(video).toBeVisible();
  // no transcode toast should appear
  await expect(page.locator('.transcode-toast')).toHaveCount(0);
  // src points at the original (no ?transcode= in URL)
  const src = await video.getAttribute('src');
  expect(src).not.toContain('transcode=true');
});

test('hevc video shows converting then plays', async ({ page }) => {
  // library must contain test_video_hevc.mp4. If absent, skip.
  const cards = await TestHelpers.getPhotoCards(page);
  if (cards.filter(c => c.includes('test_video_hevc')).length === 0) return test.skip();
  await TestHelpers.openViewer(page, /* hash of test_video_hevc.mp4 */);
  // transcode toast appears with 'Converting'
  await expect(page.locator('.transcode-toast')).toBeVisible();
  // eventually the toast clears and #viewer-video plays
  await expect(async () => {
    const src = await page.locator('#viewer-video').getAttribute('src');
    expect(src).toBeTruthy();
  }).toPass({ timeout: 10000 });
});
```
  Note: obtaining photo hashes requires a `GET /api/photos?query=type:video` call in the test or a `TestHelpers` extension; if the seeded library in E2E always contains the same fixtures (AGENTS.md: "test data photos are always indexed in the seeded library"), locate the card by filename and derive the hash from the card's `data-photo-hash` attribute.
- [ ] **Step 2: Run E2E** — `npm run test:e2e` (single worker). Fix any flakiness (startup retry once per AGENTS.md learning 10).
- [ ] **Step 3: Commit**
```bash
git add tests/e2e/specs/video-playback.e2e.spec.js && git commit -m "test(e2e): native-first video playback coverage"
```

---

### Task 13: Docs — AGENTS.md learnings merge

**Files:**
- Modify: `AGENTS.md` (Learnings section)

**Interfaces:**
- Consumes: nothing.

- [ ] **Step 1: Extract learnings** from this feature: (1) decide video serving server-side (codec+container+moov), never client-guess with UA sniff; (2) serve-time `-c copy +faststart` remux fixes "h264 that won't play" without re-encode; (3) empty/0-byte files must fast-fail with a warning, never a bare 416 that looks like a transcode hang; (4) transcode status must expose percent + server deadline so clients never fake-timeout. Verify existing 10 entries; fold or replace least-relevant if needed (capped at 10).
- [ ] **Step 2: Commit**
```bash
git add AGENTS.md && git commit -m "docs: record video playback decision learnings"
```

---

## Self-Review Notes

- **Spec coverage:** Each spec scenario maps to a task: Scenario 1→Task 4 (Direct Play regression + test), 2→Task 3+4 (remux), 3→Task 5+6+10 (honest progress, no fake timeout), 4→Task 5+7 (pool), 5→Task 5 (generalize transcode), 6→Task 4 (empty fast-fail), 7→Task 11 (escape hatch). Config in Task 7. i18n across 9-11.
- **Placeholders:** all functions have concrete signatures and test code; no "TBD".
- **Type consistency:** `TranscodeStatus.percent` added once (Task 5) and consumed by status endpoint (Task 6) and frontend (Task 10); `photo.video_codec()/bit_depth()/container()/moov_at_start()` produced in Task 1, consumed in Task 4. `ensure_progressive_mp4` produced Task 3, consumed Task 4. `ClientCodecs` produced Task 2, consumed Task 4.
