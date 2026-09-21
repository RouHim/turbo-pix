# Feature Specification: GPU-Accelerated ffmpeg Transcoding

**Created**: 2026-09-21
**Status**: Approved
**Input**: "lets use ffmpeg with gpu if available for transcoding"

## Goal
TurboPix re-encodes video server-side in two places: cached whole-file conversions and the live MSE streaming transcoder. Both currently pin the CPU encoder `libx264`. On a host that has a usable hardware video encoder those jobs should run on the GPU, so conversions finish materially faster and the CPU is left free — while every existing playback decision, cache artifact, status transition and client contract stays exactly as it is. Scope boundary: encoder selection is automatic and opportunistic. There is no operator configuration, no new client protocol, no change to which files are playable, and no change to the CPU path when no GPU encoder is usable.

## User Scenarios

### Scenario 1 - Hardware-accelerated conversion on a GPU host (P1)
The host exposes a working hardware H.264 encoder. A user opens a video the client cannot play natively; the server converts it; the artifact appears sooner than with the CPU encoder and plays exactly as before.

**Acceptance**
1. Given a host where the startup probe accepted a hardware encoder, When a cached whole-file conversion completes, Then the artifact is a valid H.264/AAC MP4 and the completed job reports a hardware encoder, not `libx264`.
2. Given that host and a 1080p HEVC source, When the same conversion runs on hardware and on the software encoder, Then hardware wall-clock is at most half the software baseline.
3. Given a live transcode stream on that host, When the client seeks and continues playback, Then seeking, per-response MIME negotiation, buffering and the escalation ladder behave exactly as with the software encoder.

### Scenario 2 - No usable hardware encoder (P1)
The host has no GPU, or has one whose encoder ffmpeg cannot use (missing device node, driver, encoder build, or permissions). The feature must be invisible: same artifacts, same latency class, no client-visible warnings.

**Acceptance**
1. Given a host where no hardware encoder passes the probe, When any conversion or live stream runs, Then the job reports `libx264` and produces what the current implementation produces.
2. Given a host where ffmpeg lists an encoder but the probe fails, When the server starts and jobs run, Then the hardware backend stays marked unusable: no per-job re-probe, no added job latency, no client-visible warning.

### Scenario 3 - The hardware encoder fails mid-job (P2)
The probe passed but a real job fails (device busy, driver session limit, driver reset, source the encoder cannot consume). The user must still get their video, and cached-conversion failure semantics must not change.

**Acceptance**
1. Given a job whose hardware attempt exits non-zero, When the failure occurs, Then the server retries that job once with the software encoder inside the same job and delivers the normal result.
2. Given both attempts fail, When the job settles, Then the existing failure semantics apply unchanged (`Failed`/`Timeout` state, existing `X-Transcode-Warning`, existing retry cooldown).
3. Given a live stream whose hardware attempt fails, When the failure occurs, Then the internal fallback does not consume a step of the client-visible `remux → audio → transcode` ladder and does not change the mode the client asked for.

### Scenario 4 - Subtle encoder hint in the player (P2)
While a video is being converted server-side, the player shows a small, unobtrusive icon telling the user whether that conversion is running on the GPU or on the CPU. The hint describes the playback in front of them, so it never appears when nothing is being video-encoded.

**Acceptance**
1. Given a video the server is converting or streaming with a hardware encoder, When playback starts, Then a small icon with an accessible label naming the GPU encoder is visible in the player.
2. Given the same video converted by the software encoder, When playback starts, Then the icon is present with the software label instead — "not GPU" is always distinguishable from "no information".
3. Given a video played directly, remuxed, or served with its video track copied (audio-only conversion), When playback starts, Then no hint is shown, because no video encoding is happening.
4. Given a hint shown for one video, When the user moves to the next video or closes the viewer, Then the hint does not carry over to a playback it does not describe.

## Functional Requirements
- **FR-001**: The server MUST decide hardware-encoder usability from a real (small, cheap) encode attempt, not from ffmpeg's encoder listing alone.
- **FR-002**: The usability verdict MUST be process-wide and computed once; per-job re-probing is prohibited.
- **FR-003**: Candidate backends MUST cover NVENC, QSV, VAAPI, AMF and VideoToolbox, evaluated in a fixed, deterministic preference order.
- **FR-004**: When a usable backend exists, BOTH the cached whole-file conversion and the live streaming transcode rung MUST use it.
- **FR-005**: A failed hardware attempt MUST fall back to the software encoder within the same job; the client-visible outcome, mode and response headers stay unchanged.
- **FR-006**: The feature MUST NOT require new configuration: no new mandatory environment variable, no new config file key, and no change to existing ones.
- **FR-007**: Delivered media characteristics MUST be unchanged — H.264 (8-bit, `yuv420p`, Main/High profile) plus AAC audio in the same container, playable by the same client matrix. Hardware output MUST NOT require a newer client decoder than the software output.
- **FR-008**: All existing gating MUST still apply to hardware jobs: worker-pool permits, per-transcode timeout, the three cache namespaces, `claim_transcode` dedup, and the busy `503` + `Retry-After` behaviour.
- **FR-009**: When no hardware encoder is usable, behaviour MUST be indistinguishable from the current build: same ffmpeg invocations, same artifacts, same status transitions, same logs (no new warnings).
- **FR-010**: The encoder actually used and any hardware-to-software fallback MUST be observable: logged per job, reported through the existing transcode status surface for whole-file conversions, and reported per run on the live-stream response so a client can tell which encoder served the bytes it is playing.
- **FR-011**: The player MUST show a subtle, icon-only hint stating whether the video being played is currently being video-encoded by the GPU or by the CPU. A playback that is not video-encoded — direct play, remux, or a conversion that copies the video track — MUST show no hint, so "not GPU" is always distinguishable from "no information".
- **FR-012**: The hint MUST describe the bytes actually being played: a run that fell back to the software encoder MUST read as software. It MUST NOT require a new network round trip or add a new user-facing control, MUST be cleared when the viewer moves to another video or closes, and MUST carry an accessible text label translated in every supported language rather than relying on the glyph alone.

## Key Entities
- **Encoder backend**: a candidate hardware encoder (NVENC, QSV, VAAPI, AMF, VideoToolbox) plus the process-wide usability verdict established by the startup probe.
- **Transcode job**: the existing conversion or stream run, extended with the encoder actually used and whether a hardware fallback occurred.

## Edge Cases
- ffmpeg lists the encoder but the device is absent, busy, or permission-denied → backend unusable, no retry storm, CPU path used.
- ffmpeg build lacks hardware support entirely → same as above.
- Source the hardware encoder cannot consume (unsupported pixel format, 10-bit, odd dimensions) → job-level fallback to software, artifact still delivered.
- More concurrent jobs than consumer-driver encoder sessions allow (NVENC) → existing worker pool plus per-job fallback absorb it; no hang beyond the existing timeout.
- GPU removal or driver reset while jobs are in flight → those jobs fall back; the process neither crashes nor hangs past the existing timeout.
- Container deployment without GPU device passthrough → behaves exactly like Scenario 2.
- Empty, 0-byte or `.pending-*` sources → unchanged empty-output path; never reaches the encoder.
- A conversion deleted or rotated mid-job → existing `currentPhoto?.hash_sha256` staleness guards and cache cleanup semantics are unaffected by encoder choice.
- Cached artifact replayed after its conversion status was evicted from the status store → the hint stays hidden rather than guessing which encoder produced the file.
- A live run that falls back from hardware to software after the hint was already shown → the hint must follow the run's actual encoder, never the planned one.
- Hint shown while the viewer auto-advances to the next photo → cleared with the playback it described, never left behind for a video the hint does not describe.

## Research Notes
- https://jellyfin.org/docs/general/post-install/transcoding/hardware-acceleration/ — reference landscape for NVENC/QSV/VAAPI/AMF/VideoToolbox selection; confirms each vendor needs its own device/init handling and that availability must be actively probed.
- https://trac.ffmpeg.org/wiki/HWAccelIntro — authoritative mapping of vendor APIs to ffmpeg encoders and hardware-device initialization (`h264_nvenc`, `h264_qsv`, `h264_vaapi`, `h264_amf`, `h264_videotoolbox`).
- https://wiki.archlinux.org/title/Hardware_video_acceleration — Linux-side prerequisites (`/dev/dri/renderD*`, driver packages), i.e. why a listed encoder is not proof of usability.

## Assumptions
- Automatic with no override (confirmed by the requester): there is no switch to force or forbid GPU use.
- Parity over speed (confirmed): hardware output must match the software path's quality and decodability targets; correctness beats throughput.
- Silent fallback (confirmed): a missing or failing GPU is never surfaced to the end user as an error.
- Both transcoding paths are in scope (confirmed): cached whole-file conversion and the live stream rung.
- Single-host LAN deployment, no cross-host cache sharing — so the existing cache namespaces stay valid regardless of which encoder produced an artifact.
- The existing worker-pool size and per-transcode timeout remain the right valves for hardware jobs; they are not re-tuned by this feature.
- The player hint is informational only: no toggle, no preference, and no operator configuration — consistent with the hands-off decision.
- "Subtle" means icon-only, low-contrast, no layout shift: it may not become a second notice competing with the existing conversion notice.

## Success Criteria
- **SC-001**: On a host with a usable hardware encoder, a completed conversion reports a hardware encoder and its artifact plays back through the existing video E2E flows.
- **SC-002**: On a host without one, every conversion and stream reports `libx264` and the existing unit/E2E suites stay green with no behavioural diff.
- **SC-003**: A hardware failure injected at job level still yields a completed, playable artifact within the existing per-transcode timeout, with the fallback logged.
- **SC-004**: On a GPU host, a 1080p HEVC→H.264 conversion completes in at most 50% of the software baseline measured on the same host.
- **SC-005**: Live streaming on a GPU host passes the existing seek/buffering/ladder E2E coverage unchanged — no client-visible regression.
- **SC-006**: Opening a video the client cannot play shows exactly one encoder hint (hardware on a GPU host, software on a CPU-only host), opening a natively playable video shows none, and auto-advancing to the next video clears the previous hint.
