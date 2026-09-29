# Feature Specification: Playable Originals Are Never Converted

**Created**: 2026-09-28
**Status**: Approved
**Input**: Opening a video shows a note that it must be converted, yet the original plays fine when requested directly — why convert a video the browser can already play?

## Goal

TurboPix currently decides "can this client play this file?" from a *guessed* declaration (a user-agent veto plus `canPlayType` answers) and, when that guess is negative, sends the viewer straight into a conversion stream. The user then discovers that the very same browser plays the original without any processing — the app's own "play the original anyway" escape hatch proves the guess was wrong. Every conversion (container remux, audio exchange, full re-encode) must instead be triggered by an observed failure of the original: the browser tries the source first, and only a real decode error or the absence of a first decoded frame within a bounded window starts the conversion. Scope: the client-side playback behaviour and the capability declaration it sends; the server's playback planning, the capability record, and the conversion implementations themselves stay as they are.

## User Scenarios

### Scenario 1 - A browser that can play the source plays the source (P1)

A HEVC/4K video is opened in a browser that can decode HEVC. Today the viewer asks the server, is told "transcode", and starts a conversion stream — a notice plus a wasted ffmpeg job for a file the browser can already play.

**Acceptance**
1. Given a video whose codec and container this browser can actually decode, When the viewer opens it, Then the original file is requested and playback starts from it, with no conversion job spawned, no conversion status entry, and no cache artifact for that hash.
2. Given the server's fast playback decision proposes a conversion rung (because the declared capability set under-reports), When the viewer opens the video, Then the original is still attempted first, and a first decoded frame within the grace window keeps playback on the original with no conversion started.
3. Given the original played successfully, When the user closes and opens the video again, Then playback starts on the original again and no conversion notice ever appears.

### Scenario 2 - A genuinely unsupported codec converts, but only after a demonstrated failure (P1)

A browser without a decoder for the file's codec opens the same style of video: conversion is correct here — it just must not be assumed in advance.

**Acceptance**
1. Given a video this browser cannot decode, When the viewer opens it, Then the original is attempted and the conversion (the rung the server planned) starts only after the attempt failed by media error or by producing no first decoded frame within the grace window.
2. Given the conversion has started, When the user watches the notice, Then the notice only appears from that moment on — never before a conversion job is running.
3. Given the original failed for a video in this session, When the user reopens that video, Then the planned conversion starts immediately, without repeating the grace window.
4. Given a genuinely unsupported video, When the user chooses to play the original anyway, Then the original is requested as today and the player does not loop back into conversion.

### Scenario 3 - The capability declaration stops being a guess (P2)

Firefox runs on the user's machine and plays HEVC natively (Firefox 134+ on Windows, 136+ on macOS, 137+ on Linux). TurboPix nevertheless refuses to declare HEVC support because of a user-agent rule inherited from Jellyfin, so every HEVC video is converted.

**Acceptance**
1. Given a browser whose platform decoder handles HEVC, When the capability declaration is built, Then HEVC is declared from the browser's real capability answer, independent of the user-agent string.
2. Given a capability query answers "unsupported" for a codec, but a real playback of a file of that codec succeeds, When the next capability declaration is built, Then the successful playback wins and the codec is declared.
3. Given a codec was verified in this browser, When any video of that codec is opened later (including after a browser restart), Then the server's decision answers direct play from the first request and no grace window and no conversion notice occur.

### Scenario 4 - Honest notices (P2)

The "waiting for a free conversion slot" text currently also comes from a slow-start timer, so a video that simply takes a moment to produce its first bytes claims to be queued for a busy conversion pool.

**Acceptance**
1. Given a conversion job is waiting for a free worker slot, When the player waits, Then the queued notice is shown and the escape hatch is offered.
2. Given a slot was granted and the run is merely slow to produce its first bytes, Then the queued/saturated-pool notice is never shown; a "preparing" state is shown instead.
3. Given a video plays from the original, Then no conversion, preparing, or queued notice appears at all.

## Functional Requirements

- **FR-001**: On opening a video, the viewer attempts the original file before any conversion rung is started, regardless of the playback decision the server returns. Only files the server reports as empty or incomplete are exempt.
- **FR-002**: No conversion job of any rung (container/faststart remux, audio exchange, full re-encode) may start until the original attempt has been evaluated as failed per FR-003.
- **FR-003**: The original attempt is failed when either the media element reports a decode/format error, or no first decoded video frame has been observed within the grace window (default 5 s, within a 3-5 s range). Whichever signal arrives first ends the attempt.
- **FR-004**: The grace window must never expire while the original's delivery is still making progress: as long as the original is still delivering data/first-frame progress, the window is extended, so a slow but working network is never classified as "cannot play".
- **FR-005**: The capability declaration must be derived from the browser's real capability answers for the actual codec/container, including platform/hardware decoders; a user-agent-based veto for a codec is not permitted.
- **FR-006**: The capability declaration must include every codec proven playable by an actual playback in that browser (FR-007), even when a capability query claims otherwise.
- **FR-007**: A successful original playback records a persistent, browser-local verification that this browser can play that codec. The verification is written only from an actual successful playback, never from a capability query and never from the declaration itself.
- **FR-008**: The server's playback decision remains a fast, side-effect-free hint: it starts no conversion, it supplies the rung to use after a failure, and it may never replace the original attempt of FR-001.
- **FR-009**: A video whose original attempt failed in the current session starts its planned conversion directly when reopened in that session, without repeating the grace window; the per-codec verification of FR-007 stays untouched by that failure.
- **FR-010**: If a video fails despite a recorded verification, the conversion path takes over immediately for that video only; one failing file must never disable direct play for other files of the same codec.
- **FR-011**: Conversion-related notices appear only while a conversion job is actually pending or running. No text may claim that a video must be converted before such a job exists.
- **FR-012**: The queued notice may only be shown when the server really held the request for a free worker slot; a granted-but-slow run shows the preparing state instead.
- **FR-013**: A direct playback leaves no conversion traces: no conversion process is spawned, no conversion status entry is created, and no conversion cache artifact is written for that hash.
- **FR-014**: All new or changed user-visible strings are added to both language bundles (parity enforced by the i18n integrity test) and use the existing icon set; no hardcoded text and no emojis.
- **FR-015**: The existing escape hatch to play the original while a conversion is pending or has failed stays available.

## Key Entities

- **Video capability record** (per video: codec, bit depth, container, layout/moov position, audio codec): read-only input to the server's plan.
- **Capability declaration** (per browser: the codec tokens the client claims for this request): what the server plans against.
- **Playback verification** (per browser, persisted: codecs that demonstrably played): upgrades the declaration; produced only by real playback.
- **Playback attempt** (per viewer open: the original attempt with its grace window and its verdict playable/unplayable).
- **Playback decision** (direct / remux / audio / transcode): a side-effect-free hint that names the rung to use after a failed attempt.
- **Conversion job** (the rung actually started, with its progress/queue state): the only state that may be communicated as "converting".

## Edge Cases

- Empty, 0-byte and still-syncing files are never attempted and never converted; the existing "file is empty or still being synced" message stays.
- MP4 sources whose metadata cannot be reached before the window (moov not at the start, or layout unknown) end the attempt and take the container-remux rung, which is a copy, not a re-encode.
- Video decodable but audio not playable/copiable: the attempt fails, the audio rung runs; a source the browser can play in full must never be converted.
- Profile/bit-depth differences (e.g. 8-bit vs 10-bit HEVC, High-10 H.264): the verification is keyed to the token that was actually verified, so a file of a different profile that fails converts without clearing the verified token.
- Stale playback: a verdict arriving after the user switched photos or closed the viewer must not affect the new playback (existing staleness discipline).
- Seeks or pauses while the attempt is pending must not be counted as a failure signal.
- Conversions disabled (worker pool 0): after a genuine failure the user sees the existing message and escape hatch, never a hang.
- Choosing "play original anyway" for a genuinely unsupported file stays possible and must not re-enter the conversion loop.
- A browser whose capability query answers "unsupported" for a codec the platform decoder handles must end up declaring it after the first successful playback, without a manual cache reset.

## Research Notes

- Live probe of the reported video (2026-09-28): `GET /api/photos/af1e8b9c…/video?decision&client=h264-8` answers `{"action":"stream","mode":"transcode"}`, while `?client=…,hevc,aac` answers `{"action":"direct"}` — the plan follows the declaration, so a wrong declaration converts a playable file.
- `ffprobe` of that file: HEVC Main (`hvc1`), level 5.2, 3840x2160 at 60 fps, 8-bit yuv420p, AAC-LC stereo, `moov` at the start, 119 MB / 22.6 s — nothing in the file itself requires processing.
- https://www.firefox.com/en-US/firefox/136.0/releasenotes/ and https://www.phoronix.com/news/Firefox-137-Beta — HEVC playback is enabled by default in Firefox 134 (Windows), 136 (macOS) and 137 (Linux/Android); a user-agent veto for HEVC is therefore stale and produces exactly the reported over-conversion.
- Local throwaway probe (headless Chromium 153 via Playwright, clip served from /tmp, nothing added to the repository): `canPlayType` for `hvc1.1.6.*` answers `""`, `MediaSource.isTypeSupported` false, `mediaCapabilities.decodingInfo` `supported:false`, and the original errors immediately (`MEDIA_ELEMENT_ERROR: Format error`, code 4) — so for a truly unsupported codec the direct attempt resolves within milliseconds and delays the conversion by practically nothing.
- Code anchors: `frontend/src/lib/utils.js` (user-agent HEVC veto plus memoized declaration), `frontend/src/components/PhotoViewer.svelte` (goes straight into the conversion stream; escape hatch gated on error/queued state), `frontend/src/lib/video/msePlayer.js` (1.5 s slow-start timer emits the queued state), `src/video_capability.rs::plan()` (direct play requires a declared codec and `moov_at_start == Some(true)`), `src/handlers_video.rs` (a plain byte request for any non-direct delivery serves the original — which is why the escape hatch works).

## Assumptions

- Approved direction: the original is tried first for every rung; conversion starts only after a real failure; failure means media error or no first decoded frame within 3-5 s; a proven codec is remembered persistently per browser and codec; notices appear only once a job runs.
- Grace window default 5 s (the upper end of the approved 3-5 s range).
- "Fast backend probe upfront" means the existing side-effect-free decision call: it stays, starts nothing, and never replaces the original attempt.
- The verification memory lives in the browser (no new server state) and does not modify a video's capability record.
- The failed-video fact is session-scoped rather than persisted, so a transient failure never condemns a file permanently.
- The "play original anyway" escape hatch is kept as-is.
- Server-side planning, the capability record and the conversion implementations are unchanged; only the declaration becomes truthful and the client stops converting files it can play.

## Success Criteria

- **SC-001**: For every video in the library whose codec the browser can decode, opening it plays the original; the number of conversion jobs started for such videos is 0 (observed across a sampled set covering at least three codecs).
- **SC-002**: The reported video (HEVC 4K60 `hvc1` + AAC) opens in a HEVC-capable browser with no conversion notice and no conversion job, starting playback from the original.
- **SC-003**: On a browser that cannot decode the codec, the conversion starts within 1 s after a media error, and no later than 1 s after the grace window when no error is raised.
- **SC-004**: After one successful original playback, reopening a video of the same codec in that browser — including after a browser restart — shows no grace window, no conversion notice and no conversion job.
- **SC-005**: With a deliberately throttled original delivery, no conversion starts while data keeps arriving.
- **SC-006**: No conversion, preparing or queued notice is ever shown without a conversion job present, and the queued wording appears only when the server really held the request for a free slot.
- **SC-007**: Empty/incomplete files still show their existing message without a playback attempt, and genuinely unsupported files still convert and play (no regression).
