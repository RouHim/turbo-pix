# Feature Specification: Native-First Video Playback with On-the-Fly Conversion

**Created**: 2026-09-19
**Status**: Approved
**Input**: Video playback always shows "Video wird für die Wiedergabe konvertiert…"; play natively whenever possible, and when a conversion is truly required, do it on the fly without waiting, working on Chrome/Firefox/Edge across Windows/Linux/Android.

## Goal

Videos in the library must play natively whenever the requesting client can actually decode them: either directly, or after a lossless remux when only the container or the file layout blocks progressive playback. A full pre-conversion that blocks playback must disappear from the normal path; the rare unavoidable conversion must start playing within seconds while it is still being produced, and must remain fully seekable. Scope boundary: playback decisions and delivery only — no player-UI redesign, no change to library/thumbnail/metadata features, no Safari/iOS support, no resolution or quality-tier changes.

## User Scenarios

### Scenario 1 - A compatible video plays immediately (P1)

A user opens a video whose streams the browser decodes (the common H.264/AAC MP4 case). Playback starts right away; no conversion notice is shown, and no full-file processing happens before the first frame.

**Acceptance**

1. Given a library video whose video and audio streams the requesting client can decode, When the user opens it in the viewer, Then the first frame is displayed without a conversion notice and without waiting for the whole file.
2. Given the same video whose stored capability record lacks container, bit-depth, or faststart information, When the playback decision is made, Then the missing information is derived from the file itself and the video still plays directly.
3. Given a video whose streams are decodable but whose file is not progressively playable (or uses an unsupported container such as MKV/WebM), When the user opens it, Then it is remuxed losslessly (stream copy, no re-encode) and the first frame appears within the same start-time bound as a direct play.

### Scenario 2 - On-the-fly conversion when nothing else works (P1)

A user opens a video whose streams the client cannot decode (for example HEVC on Firefox or on a Linux Chrome without hardware decoding, an MPEG-4/DivX/MSMPEG4 legacy rip, or a 10-bit stream the client cannot handle). Playback begins within seconds while the server converts on the fly; a buffering/conversion state is visible until frames flow; the timeline shows the real total duration from the start.

**Acceptance**

1. Given a video no in-scope client browser can decode, When the user presses play, Then video frames are displayed within 5 seconds and a visible buffering/conversion state is shown until then.
2. Given such a video, When the user drags the scrubber to an arbitrary position (start, middle, near end), Then playback resumes at that position within 3 seconds and the displayed duration equals the source duration.
3. Given the conversion is still running, When the user pauses, seeks, or leaves and reopens the video, Then no state is lost, no duplicate conversion is charged against the concurrency limit needlessly, and a previously produced result is reused when present.

### Scenario 3 - Capabilities are honored per client (P2)

The same file behaves differently depending on what the requesting browser can really decode: a device with hardware HEVC decoding plays the HEVC file natively, a browser without it gets the converted stream; the decision is made per request, not per file.

**Acceptance**

1. Given an HEVC file and a client that declares HEVC decoding support, When the user opens it, Then it is played natively (direct or remux, no re-encode).
2. Given the same file and a client that does not declare HEVC support, When the user opens it, Then it is converted on the fly under the start-time bound.
3. Given a client whose declared capability turns out to be wrong (the browser fails to play the selected stream), When playback fails, Then the system falls back to conversion automatically without user action and without a blocking round trip.

### Scenario 4 - Audio-only incompatibility (P2)

A file whose video stream the client decodes but whose audio track it cannot (AC-3, E-AC-3, DTS) must not be re-encoded in full: the video data is passed through untouched while only the audio is converted.

**Acceptance**

1. Given an H.264 video with an AC-3 audio track, When a client that cannot decode AC-3 but can decode H.264 opens it, Then the video stream is not re-encoded and audio is converted, with playback under the start-time bound.
2. Given a file whose audio track is empty or missing, When the user opens it, Then video plays without audio errors.

### Scenario 5 - Conversion failure is recoverable (P2)

When a conversion or remux fails, times out, or is refused because the concurrency limit is saturated, the user gets an actionable error and can still attempt to play the original — never a silent permanent stall.

**Acceptance**

1. Given a conversion that fails or exceeds its deadline, When the failure is detected, Then an error state is shown and the "play original anyway" option remains available.
2. Given more simultaneous conversion requests than the configured limit allows, When the extra requests arrive, Then they wait (visible to the user) instead of failing, and they start once a slot frees up.
3. Given a failed or timed-out conversion, When the same video is requested again, Then no partial or stale artifact is served, and a retry is possible after the retry cooldown.

## Functional Requirements

- **FR-001**: A video whose streams the requesting client can decode MUST be delivered without re-encoding — either directly or as a lossless remux.
- **FR-002**: Incomplete or missing stored decision metadata (container, bit depth, faststart/seek layout, audio codec) MUST NOT by itself select the conversion path; the missing properties MUST be derived from the file before the decision, and the derived values MUST be persisted so later decisions are cheap.
- **FR-003**: The playback decision MUST account for video codec, container, bit depth, file layout (progressively playable vs. not), audio codec, and the client's declared decoding capabilities.
- **FR-004**: The client MUST declare every codec/profile/container combination it can actually decode, including HEVC and higher bit depths where available, and the server MUST honor that declaration — including direct play for HEVC/AV1/VP9 when declared.
- **FR-005**: When a conversion is unavoidable, the first video frame MUST be presented within 5 seconds of the play action on a LAN client, with a visible buffering/conversion state in the meantime; no variant of the default path may require the whole file to be processed first.
- **FR-006**: Streaming conversions and remuxes MUST expose the true total duration before processing completes, and MUST support seeking to any position with playback resuming within 3 seconds.
- **FR-007**: When only the audio stream is incompatible, the video stream MUST be passed through unmodified and only audio MUST be converted.
- **FR-008**: The conversion notice ("Video wird für die Wiedergabe konvertiert…") MUST NOT appear for any video that starts natively, and, when a conversion is genuinely running, MUST be presented as a non-blocking buffering state that is replaced by playback as soon as frames flow.
- **FR-009**: A wrong capability declaration MUST self-heal: when the browser fails to play the selected stream, the system MUST fall back to the converted path automatically, without user action and without restarting the decision round trip.
- **FR-010**: Conversion and remux results MUST be reusable for repeat playback so that a previously produced result starts like a native play; reuse MUST NOT gate the first playback of an as-yet unconverted video.
- **FR-011**: Concurrent conversions MUST remain bounded by the configured limit; requests beyond the limit MUST wait in a user-visible state rather than fail, and a freed slot MUST start a waiting request.
- **FR-012**: Failed, timed-out, or empty sources MUST produce an actionable error state, MUST NOT leave partial artifacts that later requests could serve, and MUST keep the "play original" escape hatch available.
- **FR-013**: Source files MUST never be modified; all derived (remuxed/converted) outputs live in cache storage keyed by source identity plus content version.
- **FR-014**: Existing non-playback behavior MUST be preserved: zero-byte/`.pending-*` files keep their current "empty / still being synced" handling, and photo (non-video) endpoints and library metadata are unaffected.

## Key Entities

- **Video Capability Record**: per-video facts driving the decision — video codec, container, bit depth, file layout (progressively playable or not), audio codec, duration, file size and modification time (content version).
- **Client Capability Profile**: the codec/container capabilities a requesting client declares for the current session; varies per browser, OS, and hardware.
- **Playback Decision**: the resolved delivery mode for one request — direct play, lossless remux, or conversion (whole file or audio-only) — together with the reason it was chosen.
- **Playback Session**: one active delivery job — mode, progress, current seek position, state (queued, running, failed, timed out, completed), and the cache artifact it produced.

## Edge Cases

- Zero-byte or `.pending-*` sources: keep the current empty-file response path, and do not claim a conversion slot for them.
- Stale records claiming a non-progressive layout although the file is already progressively playable: serve the playable original instead of a nonexistent artifact.
- Records missing container/bit-depth entirely (the dominant case observed in the live library): must resolve to direct/remux, never to conversion.
- Duration is unknown or wrong in stored metadata: the decision and the timeline must still produce a usable duration.
- Files with multiple audio tracks: only the first/default track is considered; no track-selection UI is introduced.
- Very short videos (shorter than one conversion segment) and seeks near the start/end boundaries must not deadlock or return empty streams.
- Non-faststart files large enough that a whole-file remux would exceed the start-time bound: the remux path must stream just like the conversion path.
- Plain HTTP over LAN: the solution MUST NOT depend on secure-context-only browser APIs (WebCodecs requires HTTPS; the media-element and MSE paths do not).
- Server has no hardware acceleration: conversion must fall back to software encoding rather than failing.
- Concurrency saturation plus repeated user retries: must not spawn unbounded encoder processes.

## Research Notes

- `http://trueserver:18473/` library census (2026-09-19, 1400-photo sample, 42 videos): 38 of 42 videos (90%) decide `transcode`; every one of the 38 has no stored container metadata, while all 4 videos that decide `direct` have one (`container: "mov"`). Codec distribution: 31 H.264, 11 HEVC — the failure is a metadata/decision bug, not an exotic-codec problem.
- https://www.chromium.org/audio-video/ — HEVC playback in Chromium is limited to Google Chrome and requires hardware support, so desktop Linux Chrome generally cannot decode HEVC; Matroska appears in Chromium's container list but is not a general cross-browser path.
- https://caniuse.com/hevc — 16.94% full + 76.59% partial global HEVC support (Aug 2026): HEVC capability differs per client and must be probed, not assumed.
- https://developer.mozilla.org/en-US/docs/Web/Media/Guides/Formats/Video_codecs — HEVC is an MP4-only codec on the web; container/codec combinations like H.264-in-Matroska need a remux rather than a re-encode.
- https://developer.mozilla.org/en-US/docs/Web/Media/Guides/Formats/Audio_codecs — browser audio support (AAC, AC-3, DTS) is platform-dependent (Firefox relies on OS codecs), which makes audio a separate dimension of the decision.
- https://developer.mozilla.org/en-US/docs/Web/API/VideoDecoder — WebCodecs is secure-context-only and therefore unusable on TurboPix's LAN HTTP origin.
- https://developer.mozilla.org/en-US/docs/Web/API/Media_Source_Extensions_API — MSE provides the segment-based streaming path that works on insecure origins and is the viable basis for seekable on-the-fly playback.

## Assumptions

- Capability probing stays client-side and declared to the server (media-element `canPlayType` / MediaCapabilities); the server's decision remains authoritative for the delivery mode.
- Conversion/remux outputs may be cached and reused for replays, but caching must never gate the first playback.
- Only the first/default audio track is considered; audio-track selection UI is out of scope.
- The existing pre-conversion cache and "play original anyway" escape hatch remain as fallbacks rather than as the default path.
- Conversion concurrency remains bounded by the existing configurable limit, with a software-encoder fallback when no hardware acceleration is present.
- In-scope clients are Chrome, Firefox, and Edge on Windows, Linux, and Android; Safari/iOS/macOS and TV browsers are out of scope.
- Videos are served over unauthenticated LAN HTTP; no secure-context-only APIs may be required.

## Success Criteria

- **SC-001**: On a representative sample of the real library (≥1000 photos), at least 95% of videos that a Chrome-class client can decode resolve to direct play or remux (baseline measured 2026-09-19: 4 of 42 = 9.5%).
- **SC-002**: For that same class of videos, zero occurrences of the conversion notice occur in an end-to-end run (baseline: conversion notice on 90% of sampled videos).
- **SC-003**: For videos that do require remux or conversion (e.g. non-progressive H.264, HEVC, MPEG-4 fixtures), the first frame is displayed within 5 seconds (p95) of the play action at a 1920×1080 client viewport.
- **SC-004**: During a streamed conversion/remux, seeking to an arbitrary position (near start, middle, near end) resumes playback within 3 seconds, and the reported total duration equals the source duration.
- **SC-005**: The browser/platform matrix (Chrome, Firefox, Edge on Windows, Linux, Android) plays the fixture set — progressive H.264 MP4, H.264 MKV, HEVC MP4, MPEG-4/AVI — with HEVC direct-playing wherever hardware decoding is declared and converted elsewhere; the H.264 fixtures never show the conversion notice.
- **SC-006**: With the concurrency limit saturated, additional simultaneous requests all eventually start playing, none fail, and each waiting request shows a user-visible waiting state.
- **SC-007**: Re-opening a previously converted video starts playback within 2 seconds without a blocking conversion flow.
- **SC-008**: A forced conversion failure surfaces an actionable error and the original remains playable through the escape hatch; no partial artifact is served on retry.
