# Feature Specification: Native-First Video Playback & Honest Transcoding

**Created**: 2026-08-21
**Status**: Approved
**Input**: Video playback on the LAN instance (Firefox on Windows). Many videos that encode H.264/MP4 "should play natively" but either hang in perpetual "converting", error out, or never start; HEVC/H.265 files always route to a serialized, often-slow server transcode whose progress the UI cannot see and which it gives up on after 5 minutes.

## Understood User Request
Play every video in the library that the browser can actually play **natively** — without any server transcode — and make the remaining transcode path honest: show real progress, never loop forever, never claim "failed" while the server is still working, and handle the broken files (empty `.pending-*`, `moov`-at-end MP4s) with useful behavior instead of a dead spinner.

## Motivation
The current pipeline special-cases a single codec (HEVC) on the client and decides with a hardcoded user-agent sniff + an unreliable `mediaCapabilities` probe. The server never decides anything; it only reacts to `?transcode=true` and only for HEVC. This is exactly the antipattern the media-server world abandoned: the decision of "can this client play this file?" belongs to a single authoritative capability check, and "does this file need processing at all?" belongs to the server (codec + container + bit depth). Jellyfin, PhotoPrism, and Immich all converge on the same model: **Direct Play (serve original) → Direct Stream (container remux only) → Transcode (re-encode)** — prioritizing Direct Play to minimize server load and latency. TurboPix has progressive-download MP4 serving (fine for small phone videos) but lacks the decision layer and the honest lifecycle.

## Summary
Introduce a server-side, cached **video capability record** per video (codec, profile, bit depth, container, complete-file flag, moov-at-start flag) captured at index time and refreshed on rescan. At serve time, a single endpoint answers "what should this client see?" based on (a) the video's recorded capabilities and (b) the client's declared codec support, passed as a query/header. Serve the original when playable (Direct Play); remux (`-c copy -movflags +faststart`) when only the container/moov layout is wrong; transcode only when the codec/profile genuinely requires it. Make transcode state observable (progress % + `duration`/`size`), parallelize it beyond `1` job, drop the client's hard 5-minute poll cap, and fail fast with a human message on genuinely broken files. A "Play original anyway" escape hatch covers the long-tail "transcode hopeless" case.

Out of scope: HLS fragment streaming (a separate, later effort), scheduled pre-transcoding of the whole library (an orthogonal follow-up), and any change to the thumbnail/semantic pipeline.

## User Scenarios

### Scenario 1 — H.264 MP4 plays natively, zero server work (P1)
Rouven opens a `h264`/`aac` MP4 in Firefox. It is a normal, complete file with `moov` at the start. Today: it serves the original directly — this already works. Regression guard: the requested feature must never degrade this path to a transcode.

**Acceptance**
1. Given a `h264` MP4 that Firefox can play, When the viewer opens it, Then the `<video>` element loads the **original** file URL (no `?transcode=`), the request returns 206 with the original `video/mp4`, and the transcode toast/spinner **never** appears.
2. Given such a video, When served, Then no ffmpeg process is spawned and no transcode status entry is created for its hash.
3. Given such a video already cached originally, When reopened after a restart, Then it still plays directly (the capability record persists across restarts).

### Scenario 2 — A `h264` MP4 with `moov` at the end plays natively after a fast remux (P1)
Many older phone/WhatsApp MP4s have `moov` at the file end, so a browser cannot start streaming them until the whole file downloads, which reads as "won't play". Today `fix_moov_atom` runs only at scan time and rewrites the source; if indexing happened before we fixed this, or the file changed in place, serve-time still serves the broken layout.

**Acceptance**
1. Given a `h264`/`hevc` MP4 whose `moov` atom is not at the start, When the viewer requests the video, Then the server performs a **stream-copy remux** (`-c copy -movflags +faststart`) to a cached sidecar immediately (not a full re-encode), and serves the remuxed file; the video plays and clicks-to-start do not require a full download.
2. Given the remux succeeds, When the same hash is requested again, Then the cached remux is served, no new ffmpeg invocation runs.
3. Given a remux fails (corrupt file), When requested, Then the server falls back to serving the original bytes and sets `X-Transcode-Warning`, and the UI shows a useful message (not an infinite spinner).

### Scenario 3 — HEVC to Firefox: honest progress, no fake failure (P1)
Rouven (Firefox) opens one of the 2,090 HEVC videos. Firefox has no HEVC decoder, so transcoding is genuinely required. Today the transcode is serialized behind a global semaphore, has no progress signal, and the frontend stops polling after 5 minutes and shows "timed out" even when the server keeps working.

**Acceptance**
1. Given an HEVC video and a client that does not support HEVC, When the viewer opens it, Then the server claims the transcode slot and returns 202 + `poll_url` exactly as today, and the UI shows "Converting… N%".
2. Given the transcode is running, When the UI polls `/video/status`, Then the response includes a monotonic `percent` (or, when unknown, the same "in progress" state without decrementing), a stable `started_at`, and never reports `Failed`/`Timeout` while ffmpeg is alive.
3. Given the transcode takes longer than 5 minutes, When the UI polls, Then it **does not** give up at 5 minutes; it keeps polling while the server state remains `InProgress`, up to the server's actual transcode deadline (300s) + a bounded grace period, and only then reports timeout.
4. Given the transcode completes, When the UI next polls, Then it reads `Completed`, swaps the `<video>` source to the cached H.264 file, and the video plays.

### Scenario 4 — Concurrent transcodes, not one-at-a-time (P1)
Opening several HEVC videos in a row today queues them behind `Semaphore::new(1)`, so a single long job makes everything after it appear frozen.

**Acceptance**
1. Given N HEVC videos requested near-simultaneously, When each opens, Then up to `min(available_parallelism/2, 4)` distinct ffmpeg jobs run concurrently (configurable via `TURBO_PIX_MAX_TRANSCODES`, default `2`), and the rest wait in the queue rather than spawning duplicates.
2. Given two requests for the **same** hash, When both arrive during transcode, Then only one ffmpeg job runs and the second gets the 202/poll response (existing claim logic preserved).

### Scenario 5 — Non-HEVC-but-unsupported codecs get transcoded (P2)
The catalog contains `mpeg4` (215), `fraps` (30), `indeo5` (15), `msmpeg4v2` (6), `mjpeg` (3), `av1` (1), `vp8` (1), and `h264` High-10-bit content that Firefox cannot decode. Today these are treated as "not HEVC" and served as-is — an error screen or dead `<video>`.

**Acceptance**
1. Given a video whose recorded codec/profile is not directly playable by the client, When the viewer requests it, Then the server transcodes it to H.264 in the same slot lifecycle as HEVC (claim → 202/poll → serve), rather than serving an unplayable stream.
2. Given a video with a codec the server's ffmpeg cannot decode (e.g. a genuinely corrupt `.pending-*` file), When requested, Then the server returns the warning header with a specific reason and the UI shows "This file appears empty or incomplete" (localized), not "failed" — see Scenario 6.

### Scenario 6 — Empty / 0-byte / partial files fail fast, not forever (P2)
53 indexed rows are `.pending-*` temp files left by a phone sync app, **0 bytes on disk**. The server returns 416 for any range; the `<video>` fires `error`; the frontend neither transcodes nor tells the user anything useful.

**Acceptance**
1. Given a photo row whose backing file is 0 bytes (or missing), When the `/video` GET or range request arrives, Then the server returns `Content-Length: 0` with status 200 (or 410-style `X-Transcode-Warning: empty`), never a bare 416.
2. Given such a file, When the viewer opens it, Then the UI shows a single reliable message: "This video file is empty or still being synced" (localized), with no spinner, no fake transcode, no repeated retries.

### Scenario 7 — "Play original anyway" escape hatch (P2)
A transcoding failure (e.g. a corrupt file, or a 4K HEVC the transcode keeps timing out) currently dead-ends. The user should be able to try the original even if the browser may not play it.

**Acceptance**
1. Given a transcode failure/timeout has been reported for a video, When the user clicks "Play original anyway" on the transcode toast, Then the `<video>` source is set to the original URL and playback is attempted; if playback then fails, the original error message (not a transcode message) is shown.
2. Given "Play original anyway" returns to a video that then errors, When the error fires, Then it does **not** loop back to requesting another transcode (no infinite retry).

## Backend Design

### Video capability record (authoritative, server-side)
Extend the stored `metadata.video` JSON (already built in `db.rs:932-958` from `ProcessedPhoto`) with:
- `profile` (e.g. `High`, `Main`, `Constrained Baseline`, `High 4:4:4 Predictive`, `High 10`)
- `bit_depth` (from `pix_fmt`, e.g. `8`, `10`)
- `container` (from ffprobe `format.format_name` / extension)
- `moov_at_start: bool`
- `playable: bool` — resolved by a single server-side function `is_browser_direct_playable(codec, profile, bit_depth, container, moov_at_start, file_size)` (see below)

Recorded once at index time by extending the existing ffprobe probe in `metadata_extractor.rs` (already reads `codec_name`, `profile`, `pix_fmt`, `r_frame_rate`, `width/height`) and refreshed when the file's size/mtime changes (matching the existing versioned-cache invalidation model). `moov_at_start` is captured via the existing `has_moov_at_start` (cheap ffprobe trace) at index time and re-checked lazily at serve time.

### Direct-playability decision (the forwarded single source of truth)
A server function decides whether the **original file** can be served to a browser without any processing:
`playable := container in [mp4, mov]  and  moov_at_start  and  codec in browser_playable_codecs(bit_depth)  and  complete(file_size > 0)`

where `browser_playable_codecs(bd)` for the Web target set (Jellyfin codec table ≥ 2025):
- `h264` with `bit_depth == 8` (any profile: Baseline/Main/High/Constrained Baseline)
- `h264` with `bit_depth == 10` only when client reports HEVC-class support (this is rare and normally transcoded)
- `av1`, `vp8`, `vp9` when `container in [mp4, webm]`
- everything else (`hevc`, `mpeg4`, `fraps`, `indeo5`, `msmpeg4v1/v2`, `mjpeg`, `h264-10bit`) → not directly playable

**Client capability declaration.** The frontend sends its support in the request: a `X-TurboPix-Codecs` header or `?client=` query carrying a compact capability bitmask, e.g. `h264-8,h264-10,hevc,av1,vp9`. The server intersects this with the video record and picks the cheapest path. A missing header defaults to the conservative set (`h264-8` only — a plain-old-browser baseline), which keeps every client at least as capable as today's Firefox path.

### Serve-time flow (`get_video_file`)
1. Read the photo row + capability record.
2. If the file is 0 bytes/missing → `X-Transcode-Warning: empty`, serve nothing (does not claim a transcode).
3. If `moov_at_start == false` and codec is directly-playable → run cached **stream-copy remux** (`-c copy -movflags +faststart`) to `{transcode_cache}/remux/{hash}_{size}_{mtime}.mp4` (a separate `remux/` subdir from the transcode cache; atomic temp+rename, matching the existing transcode temp pattern), then serve the remux. This is cheap (~seconds, no re-encode) and fixes the largest class of "h264 that won't play". It fires **regardless of `?transcode=true`** — it is a serve-time streamability fix, not a codec conversion, and is independent of the `is_hevc_video` gate (which only guards the full re-encode path).
4. If the video is directly playable → serve original (206 range as today).
5. If the codec is not directly playable → transcode path (claim → 202/poll → serve) exactly as today, but:
   - claim uses the same `claim_transcode` lock, gated on the recorded codec (not just `is_hevc_video`).
   - transcode writes progress to the status store.
6. Audio-only or unrecognized-but-valid video → attempt transcode; if ffmpeg cannot decode, warning + fall back to original bytes.

### Transcode worker pool
- Replace the global `Semaphore::new(1)` with `Semaphore::new(min(available_parallelism()/2, 4))`, configurable via `TURBO_PIX_MAX_TRANSCODES` (default `2`). The claim/status store stays in-memory and keyed by hash (unchanged semantics), so duplicate-same-hash prevention is preserved at the claim layer, while the semaphore now bounds concurrent **distinct** jobs instead of serializing everything.
- Timeout stays 300s per job (unchanged), but the failure distinction stays `Failed` vs `Timeout`, and the retry cooldown (15 min) stays.
- Record per-job `percent` by running ffmpeg with `-progress pipe:1` and parsing `out_time_ms`/`total_size` against duration; when impossible (no duration), `percent: null` + state `InProgress` (the UI treats unknown as "running").

### Status endpoint shape (`get_video_status`)
Extend the existing `TranscodeStatus` JSON with:
```json
{ "state": "InProgress|Completed|Failed|Timeout",
  "hash": "...", "started_at": "...", "error": null,
  "percent": 42 }
```
`percent` is `null` when unknown; `Failed`/`Timeout` carry the human reason in `error`.

## Frontend Design

### Codec support declaration
In `frontend/src/lib/utils.js`, replace the `videoCodecSupport` heuristics with a deterministic, browser-native probe that mirrors Jellyfin's:
- `canPlayH264`: `video.canPlayType('video/mp4; codecs="avc1.42E01E, mp4a.40.2"')` → `"probably"` or `"maybe"` counts as supported (Jellyfin uses `!no`). The current code only trusts `"probably"`, which under-reports F. Keep the `mediaCapabilities` path as a secondary signal only for HEVC/AV1/VP9 resolution.
- `supportsHEVC` keeps the Firefox special-case (`false`) and the `hvc1.*`/`hev1.*` probe for others.
- New `serverVideoDecision(hash)` (see below) supersedes per-video guessing.

### Viewer flow (`PhotoViewer.svelte`)
- `displayVideo(photo)`:
  1. Ask `GET /api/photos/{hash}/video?decision` (or reuse the existing `?metadata=true` style) for the server's recommended path. Response: `{ action: "direct"|"remux"|"transcode"|"empty"|"error", url, reason }`.
  2. `direct` → set `<video>` src to the original URL.
  3. `remux` → set src to the remux URL (fast, likely already cached).
  4. `transcode` → the existing `tryStartTranscode` → `pollTranscodeStatus`, but polling now driven by `status.percent` and bounded by the server's real deadline (300s) + grace, not a fixed 5 minutes.
  5. `empty` → show the localized "file empty / still syncing" message, no spinner.
  6. `error` → show the localized reason.
- `pollTranscodeStatus`: keep the same stale-photo bailouts (memory entry: check hash before each await), but replace `MAX_POLL_DURATION = 5*60*1000` with a server-driven deadline. Read `percent` out of each poll to update the "Converting… N%" message; on `Failed`/`Timeout` show reason; **never** self-abort while state is `InProgress` and elapsed < 300s + 30s grace.
- `setVideoSource`: on `error` for a video that *was* transcoded, if the user had chosen "Play original anyway", do not re-enter the transcode decision.

### Escape hatch UI
When the transcode toast is in the failed/timeout state, add a button "Play original anyway" (localized `video.play_original`). Clicking it calls `displayVideo(photo, /*force*/ false)` with a flag that bypasses the decision step and sets the source to the original URL. i18n keys added to both `en.json` and `de.json` (parity required per AGENTS.md learning 1).

## i18n
New keys (must land in BOTH `frontend/src/i18n/en.json` and `de.json`, structurally identical):
- `video.transcoding.progress` → "Converting… {percent}%"
- `video.file_empty` → "This video file is empty or still being synced."
- `video.play_original` → "Play original anyway"
- `video.conversion_reason` → "Could not convert this video: {reason}"
Existing keys reused: `video.transcoding.started`, `video.transcoding.failed`, `video.transcoding.timeout`.

## Testing

### Backend unit tests
- `is_browser_direct_playable` table: h264-8 ✓, h264-10 ✗, hevc ✗, mpeg4 ✗, av1-in-webm ✓, empty file ✗, moov-at-end ✗.
- `claim_transcode` still dedupes same-hash concurrent claims (existing tests cover this; extend with a non-HEVC codec to prove the gate was generalized).
- Remux: given a `moov`-at-end h264 MP4 (fixture `test_video.mp4` can be re-created with `-movflags -faststart`), `ensure_progressive_mp4` writes a cached `-c copy -movflags +faststart` sidecar and subsequent requests serve the cache.
- Empty-file: `get_video_file` on a 0-byte backing file returns the empty/incomplete warning and creates no transcode status.
- Status `percent`: fake ffmpeg script (existing `fake_ffprobe.sh` harness pattern) emits `-progress` lines; assert `percent` is populated.

### Frontend / E2E
- `data-testid` stable selectors for the transcode toast and its "Play original" button.
- E2E (Playwright, sequential): open the HEVC fixture (`test-data/test_video_hevc.mp4`), assert the "Converting… N%" toast appears, then the video starts playing from the transcoded source (test asserts `src` contains a cached path or the original after completion).
- E2E: open a 0-byte `.pending-*`-style fixture, assert the "file empty / still syncing" message and that no transcode is attempted.

## Configuration
- New: `TURBO_PIX_MAX_TRANSCODES` (default `2`; bounds the worker pool `Semaphore`. `0` = no transcode permits → all transcodes immediately return "transcoding unavailable" (202-with-reason, never a silent infinite wait); `1` = exactly one job at a time, the historical behavior. Distinct from the claim-layer dedupe which is always on.)
- New: `TURBO_PIX_TRANSCODE_TIMEOUT_SECS` (default `300`; replaces the hardcoded 300s).

## Risks / Trade-offs
- **Backward compatibility**: The `?transcode=true` param stays supported and behaves as today (server decides via the same path); the new decision endpoint is additive. Existing clients that only send `?transcode=true` still work.
- **`moov` remux writes a sidecar** (not the source): matches the existing never-mutate-source cache philosophy (the scan-time `fix_moov_atom` already rewrites the source only at index; serve-time remux must NOT touch the original).
- **Server decision must not be stalcked by a client lying about codecs**: the conservative default (`h264-8` only) means an over-optimistic client can at worst cause the server to serve a file the client can't play — then the `<video>` `error` path catches it with the "Play original anyway" escape. No correctness hazard from a lying client.
- **HEVC transcoding cost is unchanged** — this does not make HEVC faster to *encode*; it makes the experience honest (progress, parallel jobs, no fake timeout). Real acceleration (HW encoders, scheduling) is explicitly out of scope.
