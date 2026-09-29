# Playable Originals Are Never Converted — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every video playback attempts the original file first and starts a conversion only after that attempt demonstrably failed — never because a client-side capability guess said so.

**Architecture:** The server keeps planning as it does today, but the plan degrades from verdict to hint: the viewer hands the original URL to the media element, watches for a first decoded frame (or a media error, or a stalled delivery that stays stalled for the grace window) and only then runs the rung the decision named. Two client-side memories make the hint converge on the truth: a persisted per-browser record of codecs an actual playback proved (folded into the declaration the server plans against) and a session-scoped set of videos whose attempt failed (reopened directly on the planned rung). The declaration loses its user-agent veto, and the conversion notices lose the timer that used to claim a saturated pool.

**Tech Stack:** Rust/warp/sqlx backend, Svelte 5 (runes) + Vite frontend, `node:test` unit tests, Playwright E2E against a real backend.

**Spec:** `.spec/unnecessary-video-conversion.md`

## Global Constraints

- Grace window default **5000 ms** (spec range 3–5 s), one constant, `ORIGINAL_ATTEMPT_GRACE_MS`.
- Server-side planning (`src/video_capability.rs::plan`), the capability record (`video_probe`) and the conversion implementations are **unchanged**; the only server change is additive fields on the `?decision` payload.
- No new server state. The playback verification lives in `localStorage` under `turbopix_verified_video_codecs`; the failed-video fact is session-scoped (module state), never persisted.
- The decision endpoint stays side-effect-free: it starts no conversion and never replaces the original attempt (FR-008).
- Breaking changes are allowed — no legacy shims, no dual paths, every caller migrates in the same task (AGENTS.md).
- User-visible strings live in **both** bundles (`frontend/src/i18n/en.json` and `de.json`); parity is enforced by `npm run test:i18n` in CI. `values` go inside the options object, never as a third argument. Feather icons only, no emojis.
- Frontend build order: `npm run build` → `cargo build --bin turbo-pix` (build.rs panics without `dist/`).
- Zero warnings: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `npm run lint`, `npm run format`, `npm run test:unit`, `npm run test:i18n`.
- E2E runs sequentially (`workers: 1`) against one server and one DB; fixtures are global state — use `TestHelpers`, never hard timeouts, and always `TestHelpers.clearCachedConversions(page, hash)` before asserting a first-play premise. A failed first run is infra (port race / stale server) — re-run once before suspecting a regression.
- Node-testable modules MUST NOT import `frontend/src/lib/utils.js` (it imports `en.json` without the JSON import attribute; `node --test` dies at link time).

## Review Focus

The spec is a vision document; these are the input classes it implies but does not spell out, most likely to bite a user first. Each is pinned by a named test in the task that owns the code.

1. **A delivery that never stalls and never decodes** (bytes are served, no frame ever appears): the attempt stays open by design — FR-004 forbids expiring while the delivery is being served, and the element shows its own loading state. Pinned by Task 4's `keeps the attempt open while the delivery is served`.
2. **A stalled delivery that resumes with no element signal** (pre-metadata resumption: Chromium fires neither `progress` nor metadata until enough bytes arrive): the attempt expires `graceMs` after the reported stall and converts a file that would have played. Documented limit in `originalAttempt.js`; the grace window is the mitigation.
3. **A verdict for a video the user has left** (switched photo, closed viewer): must not start a conversion or a notice over the new photo. Pinned by Task 5's `a switch away from a pending attempt leaves nothing behind` and Task 4's `cancel()` test.
4. **Frame decoded but audio undecodable** (AC-3/DTS MP4: Chromium plays the video silently and raises no error — measured): the attempt must not call that playable, or the user silently loses sound. Pinned by Task 4's `a frame with undecodable audio is not playable`, Task 5's AC-3 spec and Task 7's `mode=audio` assertion.
5. **A browser whose `stalled` fires while data still arrives**: the attempt expires early and converts. Mitigated by the grace window; pinned only as the premise of Task 4's timeout test, not observable in E2E.

---

### Task 1: The decision payload carries the facts the client keys on

**Files:**
- Modify: `src/handlers_video.rs` (the `?decision` response inside `get_video_file`)
- Test: `src/handlers_video.rs` (`mod tests`)

**Interfaces:**
- Consumes: `ResolvedCapabilities { codec: String, bit_depth: Option<u32>, audio_codec: Option<String>, … }`, already in scope in `get_video_file`.
- Produces: every non-`empty` `?decision` JSON carries `codec: string` (`""` when the source has no video stream), `bit_depth: number|null`, `audio_codec: string|null`. The `empty` arm is unchanged (the client skips the attempt there).

- [ ] **Step 1: Write the failing test**

Add next to `decision_endpoint_reports_direct_and_stream_actions` in `src/handlers_video.rs`:

```rust
#[tokio::test]
async fn decision_endpoint_reports_the_source_facts_the_client_keys_on() {
    let db_pool = create_in_memory_pool().await.expect("failed to create db");
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let hash = "1313131313131313131313131313131313131313131313131313131313131313";
    setup_test_video_with_content(&db_pool, &temp_dir, hash, b"fake-video-data").await;

    // A 10-bit HEVC source with an AAC track: the client keys its playback
    // verification on exactly this pair, and its audio gate on the codec below.
    // The record carries `capability_version`, so it is complete and no probe
    // runs — the facts below are the ones the endpoint must report verbatim.
    set_video_record(
        &db_pool,
        hash,
        json!({
            "codec": "hevc", "container": "mp4", "bit_depth": 10,
            "audio_codec": "aac", "moov_at_start": true, "capability_version": 1
        }),
    )
    .await;
    let decision = decision_for(&db_pool, hash, "h264-8,hevc,aac").await;
    assert_eq!(decision["action"], "direct");
    assert_eq!(decision["codec"], "hevc");
    assert_eq!(decision["bit_depth"], 10);
    assert_eq!(decision["audio_codec"], "aac");

    // A source with no audio stream reports that as null, never as a token the
    // client's audio gate would have to test.
    let silent = "1414141414141414141414141414141414141414141414141414141414141414";
    let silent_dir = TempDir::new().expect("failed to create temp dir");
    setup_test_video_with_content(&db_pool, &silent_dir, silent, b"fake-video-data").await;
    set_video_record(
        &db_pool,
        silent,
        json!({
            "codec": "h264", "container": "mp4", "bit_depth": 8,
            "moov_at_start": true, "capability_version": 1
        }),
    )
    .await;
    let decision = decision_for(&db_pool, silent, "h264-8,aac").await;
    assert_eq!(decision["codec"], "h264");
    assert_eq!(decision["audio_codec"], serde_json::Value::Null);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib handlers_video::tests::decision_endpoint_reports_the_source_facts_the_client_keys_on`
Expected: FAIL — `left: Null, right: String("hevc")`.

- [ ] **Step 3: Attach the facts once, to every arm**

In `get_video_file`'s `?decision` block, turn the arm match into a mutable value and attach the three facts in one place:

```rust
let mut response = match delivery { /* the four arms, unchanged */ };
// The source's own facts travel with the decision: the client keys its
// playback verification on `codec` + `bit_depth` and its audio gate on
// `audio_codec` (spec FR-006/FR-007). They describe the FILE, identically for
// every delivery, so they are attached once here rather than per arm.
let object = response.as_object_mut().expect("every decision arm is an object");
object.insert("codec".to_string(), json!(caps.codec));
object.insert("bit_depth".to_string(), json!(caps.bit_depth));
object.insert("audio_codec".to_string(), json!(caps.audio_codec));
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib handlers_video::tests`
Expected: PASS (the existing decision tests assert individual fields, never the whole object).

- [ ] **Step 5: Lint, format, commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/handlers_video.rs
git commit -m "feat(video): report source codec facts on the playback decision"
```

---

### Task 2: The per-browser playback verification store

**Files:**
- Create: `frontend/src/lib/video/playbackVerification.js`
- Test: `tests/playback-verification.test.js`

**Interfaces:**
- Produces:
  - `codecTokenFor(codec, bitDepth) → 'h264-8'|'h264-10'|'hevc'|'av1'|'vp9'|'vp8'|null` — the declaration token a codec + bit depth maps to (`h264` with `bitDepth > 8` → `h264-10`, a missing or ≤8 depth → `h264-8`; anything else → `null`).
  - `verifiedCodecs() → string[]` — the persisted tokens, deduplicated, in insertion order.
  - `markCodecVerified(token) → string[]` — appends `token` when it is one of the six declaration tokens and not already present, persists it, returns the resulting list. Unknown/`null` tokens are refused (the declaration cannot express them).
- Storage: a JSON array of strings under `VERIFIED_CODECS_STORAGE_KEY = 'turbopix_verified_video_codecs'`. Every `localStorage` access is guarded (`typeof localStorage === 'undefined'`, `try/catch`); unreadable or malformed storage reads as `[]` and never throws.

- [ ] **Step 1: Write the failing test**

```js
import { test, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';

import {
  VERIFIED_CODECS_STORAGE_KEY,
  codecTokenFor,
  markCodecVerified,
  verifiedCodecs,
} from '../frontend/src/lib/video/playbackVerification.js';

const realLocalStorage = globalThis.localStorage;

function fakeStorage(initial = {}) {
  const data = new Map(Object.entries(initial));
  return {
    getItem: (key) => (data.has(key) ? data.get(key) : null),
    setItem: (key, value) => data.set(key, String(value)),
    removeItem: (key) => data.delete(key),
  };
}

beforeEach(() => {
  globalThis.localStorage = fakeStorage();
});
afterEach(() => {
  globalThis.localStorage = realLocalStorage;
});

test('a codec and bit depth map to the token the declaration uses', () => {
  assert.equal(codecTokenFor('h264', 8), 'h264-8');
  assert.equal(codecTokenFor('h264', 10), 'h264-10');
  assert.equal(codecTokenFor('h264', null), 'h264-8');
  assert.equal(codecTokenFor('hevc', 10), 'hevc');
  assert.equal(codecTokenFor('av1', 8), 'av1');
  assert.equal(codecTokenFor('vp9', null), 'vp9');
  assert.equal(codecTokenFor('vp8', null), 'vp8');
  assert.equal(codecTokenFor('mpeg4', 8), null);
  assert.equal(codecTokenFor('', 8), null);
  assert.equal(codecTokenFor(null, 8), null);
  assert.equal(codecTokenFor(undefined, undefined), null);
});

test('a verification is written once and read back from storage', () => {
  assert.deepEqual(verifiedCodecs(), []);
  assert.deepEqual(markCodecVerified('hevc'), ['hevc']);
  assert.deepEqual(markCodecVerified('hevc'), ['hevc'], 'duplicates are not stored');
  assert.deepEqual(markCodecVerified('h264-10'), ['hevc', 'h264-10']);
  // What an earlier page load wrote is what the next one reads.
  assert.deepEqual(JSON.parse(globalThis.localStorage.getItem(VERIFIED_CODECS_STORAGE_KEY)), [
    'hevc',
    'h264-10',
  ]);
  assert.deepEqual(verifiedCodecs(), ['hevc', 'h264-10']);
});

test('tokens the declaration cannot express are refused', () => {
  assert.deepEqual(markCodecVerified(null), []);
  assert.deepEqual(markCodecVerified('mpeg4'), []);
  assert.deepEqual(markCodecVerified(''), []);
  assert.deepEqual(verifiedCodecs(), []);
});

test('malformed or unreadable storage reads as empty and never throws', () => {
  globalThis.localStorage = fakeStorage({ [VERIFIED_CODECS_STORAGE_KEY]: 'not json' });
  assert.deepEqual(verifiedCodecs(), []);
  globalThis.localStorage = fakeStorage({ [VERIFIED_CODECS_STORAGE_KEY]: '{"hevc":true}' });
  assert.deepEqual(verifiedCodecs(), []);

  globalThis.localStorage = {
    getItem: () => {
      throw new Error('denied');
    },
    setItem: () => {
      throw new Error('denied');
    },
  };
  assert.deepEqual(verifiedCodecs(), []);
  assert.deepEqual(markCodecVerified('hevc'), ['hevc'], 'the answer survives a denied write');
});
```

Run: `node --test tests/playback-verification.test.js`
Expected: FAIL — `Cannot find module …/playbackVerification.js`.

- [ ] **Step 2: Implement the module**

Hold the token vocabulary (`['h264-8','h264-10','hevc','av1','vp9','vp8']`), read the stored array through a `try/catch` helper that accepts only an array of strings, and append inside a second `try/catch` so a denied write still returns the intended list. It MUST NOT import `utils.js`.

- [ ] **Step 3: Run the test to verify it passes**

Run: `node --test tests/playback-verification.test.js`
Expected: PASS.

- [ ] **Step 4: Lint, format, commit**

```bash
npm run lint && npm run format && npm run test:unit
git add frontend/src/lib/video/playbackVerification.js tests/playback-verification.test.js
git commit -m "feat(video): remember codecs an actual playback proved"
```

---

### Task 3: The capability declaration stops guessing

**Files:**
- Create: `frontend/src/lib/video/capabilities.js` (moves `videoCodecSupport` out of `utils.js`)
- Modify: `frontend/src/lib/utils.js` (delete the `// ── Video codec detection ──` block, roughly lines 300–425)
- Modify: `frontend/src/components/PhotoViewer.svelte` (import path only: `videoCodecSupport` comes from `../lib/video/capabilities.js`)
- Test: `tests/video-capabilities.test.js`

**Interfaces:**
- Consumes: `verifiedCodecs()`, `markCodecVerified()` (Task 2).
- Produces: the same `videoCodecSupport` surface as today (`canPlayType`, `canPlayH264`, `canPlayH264High10`, `canPlayHEVC`, `canPlayAV1`, `canPlayVP9`, `canPlayVP8`, `audioProbeTokens`, `getClientCodecsString`, `clearCache`) plus:
  - `canPlayAudioCodec(token) → boolean` — `true` for `null`/`''` (no audio track), otherwise `audioProbeTokens().includes(token)`.
  - `recordVerifiedCodec(token) → void` — `markCodecVerified(token)` then `clearCache()`, so the next declaration carries the newly proven token.
- `getClientCodecsString()` emits the probed tokens **followed by** the verified tokens, each added only when not already present. `canPlayHEVC()` has **no** user-agent branch.

- [ ] **Step 1: Write the failing test**

```js
import { test, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';

import { videoCodecSupport } from '../frontend/src/lib/video/capabilities.js';

const realDocument = globalThis.document;
const realNavigator = globalThis.navigator;
const realLocalStorage = globalThis.localStorage;

/** A stand-in element whose canPlayType answers from `answers` (default: "no"). */
function fakeDocument(answers = {}) {
  return { createElement: () => ({ canPlayType: (mime) => answers[mime] ?? '' }) };
}

beforeEach(() => {
  globalThis.document = fakeDocument();
  globalThis.localStorage = { getItem: () => null, setItem: () => {}, removeItem: () => {} };
  videoCodecSupport.clearCache();
});
afterEach(() => {
  globalThis.document = realDocument;
  globalThis.navigator = realNavigator;
  globalThis.localStorage = realLocalStorage;
  videoCodecSupport.clearCache();
});

test('HEVC is declared from the browser answer, never from the user agent', () => {
  globalThis.navigator = { userAgent: 'Mozilla/5.0 Firefox/141.0' };
  globalThis.document = fakeDocument({
    'video/mp4; codecs="hvc1.1.6.L93.B0"': 'maybe',
    'video/mp4; codecs="avc1.42E01E, mp4a.40.2"': 'probably',
  });
  const declared = videoCodecSupport.getClientCodecsString().split(',');
  assert.ok(declared.includes('hevc'), `a Firefox answering "maybe" must declare HEVC: ${declared}`);
  assert.ok(declared.includes('h264-8'));
});

test('a playback-proven codec is declared even when the probe says no', () => {
  globalThis.document = fakeDocument({});
  assert.equal(videoCodecSupport.getClientCodecsString(), '', 'nothing is guessed');
  videoCodecSupport.recordVerifiedCodec('hevc');
  assert.equal(videoCodecSupport.getClientCodecsString(), 'hevc');
  videoCodecSupport.clearCache();
  assert.equal(videoCodecSupport.getClientCodecsString(), 'hevc', 'storage backs the claim');
});

test('audio capability answers from the same probes the declaration uses', () => {
  globalThis.document = fakeDocument({
    'audio/mp4; codecs="mp4a.40.2"': 'probably',
    'audio/mp4; codecs="ac-3"': '',
  });
  assert.equal(videoCodecSupport.canPlayAudioCodec('aac'), true);
  assert.equal(videoCodecSupport.canPlayAudioCodec('ac3'), false);
  assert.equal(videoCodecSupport.canPlayAudioCodec(null), true);
  assert.equal(videoCodecSupport.canPlayAudioCodec(''), true);
  assert.equal(videoCodecSupport.canPlayAudioCodec('truehd'), false);
});
```

Run: `node --test tests/video-capabilities.test.js`
Expected: FAIL — `Cannot find module …/capabilities.js`; with the file present but unmerged, the HEVC and proven-codec tests fail.

- [ ] **Step 2: Move and change the code**

Move the block to `frontend/src/lib/video/capabilities.js`, then:

- delete the `isFirefox` early return from `canPlayHEVC()` (with its comment), keeping the three `hvc1`/`hev1` probes;
- append the verified tokens in `getClientCodecsString()`, skipping any already in `parts`;
- add `canPlayAudioCodec(token)` and `recordVerifiedCodec(token)`;
- rewrite the header comment: the set is *the browser's real capability answers plus what an actual playback proved* — never a user-agent verdict.

Remove the block from `utils.js` and update `PhotoViewer.svelte`'s import (`utils.js` keeps supplying `getPhotoUrl`, `getVideoUrl`, `isCollagePhoto`, `isRawFile`, `isVideoFile`, `showToast`).

- [ ] **Step 3: Run the tests to verify they pass**

Run: `node --test tests/video-capabilities.test.js && npm run test:unit && npm run lint`
Expected: PASS; `grep -rn "videoCodecSupport" frontend/src` lists only `capabilities.js` and `PhotoViewer.svelte`.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/video/capabilities.js frontend/src/lib/utils.js frontend/src/components/PhotoViewer.svelte tests/video-capabilities.test.js
git commit -m "feat(video): derive the capability declaration from real answers"
```

---

### Task 4: The original attempt — verdict, grace window, session memory

**Files:**
- Create: `frontend/src/lib/video/originalAttempt.js`
- Test: `tests/original-attempt.test.js`

**Interfaces:**
- Produces:
  - `ORIGINAL_ATTEMPT_GRACE_MS = 5000`
  - `startOriginalAttempt(videoEl, { graceMs = ORIGINAL_ATTEMPT_GRACE_MS, audioPlayable = () => true } = {}) → { promise, cancel }`; `promise` settles exactly once with `{ verdict: 'playable'|'unplayable'|'cancelled', reason: null|'error'|'timeout'|'audio'|'cancelled', frameObserved: boolean }`.
  - `createOriginalFailureRegistry() → { has(hash), record(hash), clear(hash) }` — session-scoped, per hash.
- The module owns **only** observation: it never assigns `videoEl.src`, never calls `play()`, never touches the UI. It MUST NOT import `utils.js`.

**The algorithm — the tests check it, they do not determine it:**

- Frame signals: `loadeddata` and `playing` — a first frame was decoded. Also checked at arm time (`videoEl.readyState >= 2 && videoEl.videoWidth > 0`), so an element that already holds a frame settles immediately.
- Failure signal: the element's `error` event.
- Delivery signals: `loadstart`, `loadedmetadata`, `durationchange`, `progress`, `suspend`, `canplay`, `seeked`, `timeupdate` — the delivery is being served; each clears a pending stall.
- `stalled` is the element saying *fetching, but no data is forthcoming*; it sets a pending stall. The deadline (`now + graceMs`) is armed at arming time and re-armed on every 250 ms watchdog tick **while no stall is pending**. A pending stall freezes it, so the verdict lands between `graceMs - 250 ms` and `graceMs` after the last arming — at most `graceMs` after the last progress signal. That is the whole timeout rule: the window never expires while the delivery is still being served (FR-004), and it always expires once a reported stall has gone `graceMs` without a frame.
- A frame with `audioPlayable()` false settles `{ verdict: 'unplayable', reason: 'audio', frameObserved: true }` — the video track played, the audio cannot (measured: Chromium plays an AC-3/DTS MP4 silently, with no error and no play rejection).
- The first verdict wins; every listener and timer is removed on any verdict and on `cancel()`; later signals are ignored.
- Pauses and seeks are neither progress nor failure: they leave the verdict pending.

- [ ] **Step 1: Write the failing test**

Build a `fakeVideo()` in the style of `tests/mse-player.test.js` (listener map, `dispatch`, `listenerCount`, plus `readyState`, `videoWidth`, `error`, `src`), and enable both timer APIs per test: `t.mock.timers.enable({ apis: ['setTimeout', 'setInterval'] })`. One test each:

1. `a loaded frame settles the attempt as playable` — `dispatch('loadeddata')` → `{ verdict: 'playable', frameObserved: true }`; afterwards `listenerCount(type) === 0` for every event the module used.
2. `playing settles the attempt as playable`.
3. `an element error settles the attempt as unplayable` — `dispatch('error')` → `{ verdict: 'unplayable', reason: 'error' }`.
4. `an element that already holds a frame is playable at arming` — `readyState = 4`, `videoWidth = 320` → settles with no event.
5. `a stall that produces no frame times out` — `dispatch('stalled')` (immediately after arming, so the deadline armed at arm time is the one in flight); `tick(graceMs - 1)` → pending; `tick(1)` → `{ verdict: 'unplayable', reason: 'timeout' }`.
6. `progress after a stall keeps the attempt alive` — `stalled`; `tick(4000)`; `progress`; `tick(4000)` → pending; `loadeddata` → playable (FR-004).
7. `keeps the attempt open while the delivery is served` — no events; `tick(graceMs * 10)` → pending.
8. `a frame with undecodable audio is not playable` — `audioPlayable: () => false`; `loadeddata` → `{ verdict: 'unplayable', reason: 'audio', frameObserved: true }`.
9. `seeking and pausing while the attempt is pending settle nothing` — `dispatch('pause')`, `dispatch('seeking')`, `dispatch('seeked')`, `dispatch('waiting')`; `tick(graceMs * 2)` → pending; `loadeddata` → playable.
10. `the first verdict wins` — `error` then `loadeddata` → error; a fresh attempt with `loadeddata` then `error` → playable.
11. `cancel() settles the attempt as cancelled and detaches` — `cancel()` → `{ verdict: 'cancelled' }`; a later `error` changes nothing; listener counts are 0.
12. `the failure registry is per hash and starts empty` — `record('a')` → `has('a')` true, `has('b')` false; `clear('a')` → false; a second `createOriginalFailureRegistry()` is empty.

Run: `node --test tests/original-attempt.test.js`
Expected: FAIL — `Cannot find module …/originalAttempt.js`.

- [ ] **Step 2: Implement the module**

Write the state machine exactly as described; put the two limits (Review Focus 2 and 5) in the module header, naming the grace window as the mitigation.

- [ ] **Step 3: Run the tests to verify they pass**

Run: `node --test tests/original-attempt.test.js && npm run test:unit`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/video/originalAttempt.js tests/original-attempt.test.js
git commit -m "feat(video): attempt the original before any conversion rung"
```

---

### Task 5: The viewer attempts the original first

This is the only task whose behaviour is observable end-to-end, so its test is the E2E spec — written first.

**Files:**
- Create: `tests/e2e/specs/playable-originals.e2e.spec.js`
- Modify: `tests/e2e/setup/test-helpers.js` (add `failOriginalAttempt`)
- Modify: `frontend/src/components/PhotoViewer.svelte`
- Modify: `frontend/src/i18n/en.json`, `frontend/src/i18n/de.json` (new key `video.playback_failed`)

**Interfaces:**
- Consumes: `startOriginalAttempt`, `createOriginalFailureRegistry`, `ORIGINAL_ATTEMPT_GRACE_MS` (Task 4); `codecTokenFor` (Task 2); `videoCodecSupport` from `capabilities.js` (Task 3); `decision.codec`, `decision.bit_depth`, `decision.audio_codec` (Task 1); `TestHelpers` (`goto`, `waitForPhotosToLoad`, `navigateToView`, `selectors.photoCard`, `selectors.viewerVideo`, `verifyViewerOpen`, `closeViewer`, `clearCachedConversions`).
- Produces: `TestHelpers.failOriginalAttempt(page, hash)`; in `PhotoViewer.svelte`: `showVideoSource(photo, videoUrl)`, `setVideoSource(photo, videoUrl)`, `cancelOriginalAttempt()`, `startPlannedDelivery(photo, decision)`. **Deleted:** `retryOnFailure` and `hasUserChosenOriginal` — after this task no caller passes `retryOnFailure: true`.

- [ ] **Step 1: Write the failing E2E spec**

Helpers at the top of `tests/e2e/specs/playable-originals.e2e.spec.js`:

```js
const PLAIN_VIDEO = /\/api\/photos\/[^/]+\/video\?client=/; // the attempt's own byte request
const STREAM_VIDEO = /\/api\/photos\/[^/]+\/video\/stream/;
const WHOLE_FILE = /\/api\/photos\/[^/]+\/video\?(?:[^/]*&)?transcode=true/;

async function findVideoByFilename(page, filename) { /* as in video-streaming.e2e.spec.js */ }
async function openVideo(page, photo) { /* card click + verifyViewerOpen, as in video-streaming.e2e.spec.js */ }
function collectRequests(page, pattern) {
  const seen = [];
  page.on('request', (request) => {
    if (pattern.test(request.url())) seen.push({ url: request.url(), at: Date.now() });
  });
  return seen;
}
/** Rewrite the decision into the conversion the client must NOT obey on faith. */
async function underReportDecision(page, { codec } = {}) {
  await page.route('**/video?decision*', async (route) => {
    const response = await route.fetch();
    const decision = await response.json();
    await route.fulfill({
      response,
      json: {
        ...decision,
        action: 'stream',
        mode: 'transcode',
        mime: 'video/mp4; codecs="avc1.42E01E, mp4a.40.2"',
        url: decision.url.replace('/video?', '/video/stream?'),
        ...(codec ? { codec } : {}),
      },
    });
  });
}
```

Tests:

1. `a playable original is never converted, even when the server plans a conversion` — `underReportDecision(page)`; `clearCachedConversions`; open `test_video.mp4`; `page.waitForFunction(el => el.readyState >= 2 && el.currentTime > 0)`; `src` matches `PLAIN_VIDEO`; `collectRequests` for `STREAM_VIDEO` and `WHOLE_FILE` are empty (a conversion can only be started by one of those two URLs); no `.transcode-toast`; the decision probe (`page.request`, which bypasses `page.route`) still answers `action: 'direct'` and `cached: false` — no artifact was written by the playback. Then close and reopen: the original plays again, still no notice (Scenario 1.3).
2. `a codec an actual playback proved is declared, and survives a reload` — `underReportDecision(page, { codec: 'hevc' })`; capture every decision request's `client` param (`new URL(request.url()).searchParams.get('client')`); open and play `test_video.mp4` → the first declaration does **not** contain `hevc`; close, `page.reload()`, open again → the second declaration contains `hevc` (FR-006/FR-007/SC-004).
3. `an unsupported codec converts only after the original attempt failed` — no mocks; `clearCachedConversions(hevc)`; record the click time; open `test_video_hevc.mp4`; the first `PLAIN_VIDEO` request precedes the first `STREAM_VIDEO` request, and the stream request arrives within 2000 ms of the click (a grace window would be ≥5 s) (FR-002/FR-003/SC-003).
4. `a video that failed this session starts its conversion on reopen without the attempt` — `clearCachedConversions(hevc)`; open `test_video_hevc.mp4`, wait for playback, close; start a fresh `collectRequests(page, PLAIN_VIDEO)`; reopen → zero new plain requests and a `STREAM_VIDEO` request within 2000 ms (FR-009).
5. `one failed file never disables direct play for its codec` — `TestHelpers.failOriginalAttempt(page, ac3.hash_sha256)`; open `test_video_ac3.mp4` (its attempt fails by 404, the audio rung runs); then open `test_video.mp4` — the h264 file still plays from the original with no conversion request (FR-010).
6. `containers the browser plays are not remuxed` — for `test_video_moov_end.mp4` and `test_video_long.mkv` (cache cleared, fresh request collection each): no `STREAM_VIDEO`, no `WHOLE_FILE`, playback from `PLAIN_VIDEO`, no toast (SC-001/FR-013).
7. `a slow-but-delivering original is not converted` — `page.route(PLAIN_VIDEO, async (route) => { await new Promise((r) => setTimeout(r, 3000)); await route.continue(); })`; open `test_video.mp4`; plays from the original; no conversion requests (FR-004/SC-005).
8. `a stalled delivery takes the planned rung after the window` — `underReportDecision(page)`; `page.route(PLAIN_VIDEO, (route) => {})` (never answered, so the delivery stalls); record the click time; open `test_video.mp4`; while the window runs (before any rung starts) there is **no** `.transcode-toast` at all (no notice without a job — FR-011); the `STREAM_VIDEO` request arrives no earlier than 5000 ms and no later than 8000 ms after the click (the grace window, then the plan) (FR-003 timeout arm, the moov-class stall).
9. `an empty file is never attempted` — `page.route('**/video?decision*', … fulfil with `{ action: 'empty' }` …)`; open `test_video.mp4`; the empty-file message shows; zero `PLAIN_VIDEO`, `STREAM_VIDEO` and `WHOLE_FILE` requests (FR-001 exemption, SC-007).
10. `a switch away from a pending attempt leaves nothing behind` — delay the plain request for `test_video.mp4` by 8000 ms; open it; `ArrowRight` to the next photo; no `.transcode-toast`, no conversion request for the old hash, and the newly shown photo is unaffected (spec edge case "Stale playback").

`TestHelpers.failOriginalAttempt(page, hash)` in `tests/e2e/setup/test-helpers.js`:

```js
  /**
   * Make the ORIGINAL attempt fail, so the planned rung is what runs. The
   * predicate is disjoint from the decision and stream URLs on purpose: those
   * keep reaching their own route mocks.
   */
  static async failOriginalAttempt(page, hash) {
    await page.route(
      (url) => url.pathname === `/api/photos/${hash}/video` && url.searchParams.has('client'),
      (route) => route.fulfill({ status: 404, contentType: 'text/plain', body: 'no original' })
    );
  }
```

Run: `npx playwright test tests/e2e/specs/playable-originals.e2e.spec.js`
Expected: FAIL — the viewer converts on the server's word (test 1) etc.

- [ ] **Step 2: Wire the new state and helpers in `PhotoViewer.svelte`**

```js
let originalAttempt = null;                                // plain field, like streamPlayer
const originalFailures = createOriginalFailureRegistry();  // videos whose original failed this session
let currentVideoToken = null;                              // declaration token for the current video's codec
```

```js
function cancelOriginalAttempt() {
  if (!originalAttempt) return;
  originalAttempt.cancel();
  originalAttempt = null;
}
```

Call `cancelOriginalAttempt()` next to every `destroyStreamPlayer()` in `displayPhoto`, in `close()`, and at the top of `displayVideo`.

Extract from `setVideoSource` and drop the retry path:

```js
/** Point the element at `videoUrl` and show it (display, photoHash, autoplay). */
function showVideoSource(photo, videoUrl) {
  // Every path states its own failures (the stream path's rule): a leftover
  // property handler from an earlier conversion delivery would toast
  // "conversion failed" next to this playback's own verdict.
  videoEl.onerror = null;
  /* today's body minus the onerror handler */
}

function setVideoSource(photo, videoUrl) {
  showVideoSource(photo, videoUrl);
  videoEl.onerror = () => {
    if (currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
    showToast(
      get(t)('notifications.error', { default: 'Error' }),
      get(t)('video.transcoding.failed', { default: 'Video conversion failed' }),
      'error'
    );
  };
}
```

Also delete `hasUserChosenOriginal`: the field, its comment block, and every write to it (`displayVideo`, `playStream`) — nothing reads it once `retryOnFailure` is gone.

- [ ] **Step 3: Restructure `displayVideo`**

```js
async function displayVideo(photo, forceTranscode = false) {
  if (!videoEl) return;
  destroyStreamPlayer();
  cancelOriginalAttempt();
  hideTranscodeToast();
  activeEncoder = null;

  if (forceTranscode) { /* unchanged legacy whole-file block */ }

  const decision = await api.getVideoDecision(
    photo.hash_sha256,
    videoCodecSupport.getClientCodecsString()
  );
  if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return;

  currentVideoToken = codecTokenFor(decision.codec, decision.bit_depth);
  if (decision.action === 'empty') {
    showTranscodeToast(get(t)('video.file_empty', { default: 'This video file is empty or still being synced.' }), true);
    return;
  }
  if (originalFailures.has(photo.hash_sha256)) {
    startPlannedDelivery(photo, decision);
    return;
  }
  armOriginalAttempt(photo, decision);
}
```

```js
function armOriginalAttempt(photo, decision) {
  const url = getVideoUrl(photo.hash_sha256, {
    clientCodecs: videoCodecSupport.getClientCodecsString(),
  });
  showVideoSource(photo, url);
  const attempt = startOriginalAttempt(videoEl, {
    graceMs: ORIGINAL_ATTEMPT_GRACE_MS,
    audioPlayable: () => videoCodecSupport.canPlayAudioCodec(decision.audio_codec),
  });
  originalAttempt = attempt;
  attempt.promise.then(({ verdict, frameObserved }) => {
    if (originalAttempt !== attempt) return; // superseded: a newer open owns the element
    originalAttempt = null;
    if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
    // The frame is proof even when the audio keeps the file off this path:
    // FR-007 records an actual playback, not the plan's verdict.
    if (frameObserved) videoCodecSupport.recordVerifiedCodec(currentVideoToken);
    if (verdict === 'playable') {
      originalFailures.clear(photo.hash_sha256);
      return;
    }
    if (verdict === 'cancelled') return;
    originalFailures.record(photo.hash_sha256);
    startPlannedDelivery(photo, decision);
  });
}
```

`startPlannedDelivery(photo, decision)` — the existing branch bodies, now reachable only after a failed attempt or a remembered failure:

```js
function startPlannedDelivery(photo, decision) {
  if (decision.action === 'stream') {
    if (!decision.mime || !mseSupported(decision.mime)) {
      /* today's legacy whole-file block (tryStartTranscode + setVideoSource) */
      return;
    }
    playStream(photo, decision);
    return;
  }
  if (decision.action === 'direct') {
    // A cached conversion is a file: serve it as one, no job needed. A plain
    // direct URL means the server expected the original to work and it did
    // not — the whole-file conversion is the fallback it always was.
    if (decision.cached && decision.url.includes('transcode=true')) {
      setVideoSource(photo, decision.url);
      return;
    }
    displayVideo(photo, true);
    return;
  }
  showTranscodeToast(
    get(t)('video.conversion_reason', {
      values: { reason: decision.reason || '' },
      default: 'Could not convert this video: {reason}',
    }),
    true
  );
}
```

`playOriginalAnyway(photo)` keeps its teardown (`destroyStreamPlayer`, `hideTranscodeToast`, `activeEncoder = null`) and then routes through the same attempt, so a proved playback is recorded and a failure is reported — never answered with the loop the user just left:

```js
  const url = getVideoUrl(photo.hash_sha256, {
    clientCodecs: videoCodecSupport.getClientCodecsString(),
  });
  showVideoSource(photo, url);
  const attempt = startOriginalAttempt(videoEl, {
    graceMs: ORIGINAL_ATTEMPT_GRACE_MS,
    audioPlayable: () => true,
  });
  originalAttempt = attempt;
  attempt.promise.then(({ verdict, frameObserved }) => {
    if (originalAttempt !== attempt) return;
    originalAttempt = null;
    if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
    if (frameObserved) {
      videoCodecSupport.recordVerifiedCodec(currentVideoToken);
      originalFailures.clear(photo.hash_sha256);
    }
    if (verdict === 'playable' || verdict === 'cancelled') return;
    showToast(
      get(t)('notifications.error', { default: 'Error' }),
      get(t)('video.playback_failed', { default: 'This video could not be played' }),
      'error'
    );
  });
```

- [ ] **Step 4: Add the i18n key to both bundles**

`frontend/src/i18n/en.json` → `video.playback_failed`: `"This video could not be played"`.
`frontend/src/i18n/de.json` → `video.playback_failed`: `"Dieses Video konnte nicht abgespielt werden"`.

- [ ] **Step 5: Run the spec and the build**

```bash
npm run test:i18n && npm run lint && npm run build && cargo build --bin turbo-pix
npx playwright test tests/e2e/specs/playable-originals.e2e.spec.js
```
Expected: PASS (pre-seeded model cache required; a failed first run is infra — re-run once).

- [ ] **Step 6: Commit**

```bash
git add frontend/src/components/PhotoViewer.svelte frontend/src/i18n/en.json frontend/src/i18n/de.json tests/e2e/setup/test-helpers.js tests/e2e/specs/playable-originals.e2e.spec.js
git commit -m "feat(viewer): play the original first and convert only after a failure"
```

---

### Task 6: The queued notice stops being a timer's guess

**Files:**
- Modify: `frontend/src/lib/video/msePlayer.js` (delete the `slotTimer` and its `state('waiting')` arm; keep the first-bytes `state('buffering')`)
- Modify: `frontend/src/components/PhotoViewer.svelte` (delete the `state === 'waiting'` branch of `playStream`'s `onState`; the queued notice now comes only from `handleStreamFailure`'s 503 path)
- Test: `tests/mse-player.test.js`, and one test appended to `tests/e2e/specs/playable-originals.e2e.spec.js`

**Interfaces:**
- `createStreamPlayer`'s `onState` only ever emits `'buffering' | 'playing' | 'ended'` (plus whatever `destroy()` semantics already implied). A request the server holds — before any response exists — emits nothing: the viewer states "preparing" for the whole hold (FR-012). `streamWaiting` stays, written only by `handleStreamFailure` and cleared by `playStream`/`hideTranscodeToast`.

- [ ] **Step 1: Update the failing unit test**

Replace the 1.5 s arm in `tests/mse-player.test.js` (the test around line 1955, currently `a slow first fragment is never announced as a slot wait`) with:

```js
  const run = player.start(0);
  await settle(2);
  assert.equal(requested.length, 1, 'the run issued its request');
  assert.equal(calls.length, 0, 'the server has not answered yet');

  // The hold may be a saturated pool — or a run that already holds its slot and
  // is slow. The player cannot tell, so it announces neither: the viewer's
  // "preparing" notice covers the whole wait, and the queued wording is
  // reserved for the server's own refusal (spec FR-012).
  t.mock.timers.tick(5000);
  assert.deepEqual(states, [], 'a held request is never announced as a slot wait');

  release();
  await settle(10);
  assert.deepEqual(states, ['buffering', 'ended'], 'the first bytes announce the buffering run');
```

Also re-comment `a refused run leaves no slot-wait notice behind` (around line 1996): there is no timer to fire any more — the test now pins that a refusal emits no state at all.

Run: `node --test tests/mse-player.test.js`
Expected: FAIL — `states` is `['waiting']` after the tick.

- [ ] **Step 2: Delete the timer**

In `msePlayer.js`: remove `const slotTimer = setTimeout(() => state('waiting'), 1500);`, both `clearTimeout(slotTimer)` calls, and the comments describing the armed-ahead-of-response timer. In `PhotoViewer.svelte`'s `playStream` `onState`, remove the `state === 'waiting'` branch together with the `streamWaiting = true` write that belongs to it.

Run: `npm run test:unit && npm run lint`
Expected: PASS.

- [ ] **Step 3: Pin it end-to-end**

Append to `tests/e2e/specs/playable-originals.e2e.spec.js`:

`a granted but slow stream run is never labelled as queued` — `underReportDecision(page)` on `test_video_hevc.mp4`; `page.route(STREAM_VIDEO, async (route) => { await new Promise((r) => setTimeout(r, 4000)); await route.continue(); })`; before opening, install a `MutationObserver` on the toast that pushes `textContent` into `window.__toastTexts`; open; wait for playback; assert the preparing text appears and the queued text (`video.stream.waiting`'s copy) **never** does (FR-012/Scenario 4.2).

Run: `npx playwright test tests/e2e/specs/playable-originals.e2e.spec.js`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/video/msePlayer.js frontend/src/components/PhotoViewer.svelte tests/mse-player.test.js tests/e2e/specs/playable-originals.e2e.spec.js
git commit -m "fix(viewer): only a real slot refusal claims the conversion pool is busy"
```

---

### Task 7: The existing E2E specs move onto the attempt-first flow

**Files:**
- Modify: `tests/e2e/specs/video-streaming.e2e.spec.js`

Every spec that mocks the stream endpoint to make the ladder run needs its original attempt to fail first (`TestHelpers.failOriginalAttempt`), and the Matroska specs must now expect the original to play.

- [ ] **Step 1: Fail the original where the ladder is the subject**

Use `TestHelpers.failOriginalAttempt(page, hash)` in:

- `a failed remux stream escalates one step and recovers` (line ~426)
- `a rung that fails before attaching resumes at the position the viewer holds` (line ~456)
- `an exhausted ladder shows the error and keeps the original playable` (line ~568)
- `a stream that keeps being lost still ends on the ladder` (line ~601)
- `a seek restart keeps the declared duration` (line ~662)
- `a permanently disabled conversion pool is named, not waited for` (line ~321)

- [ ] **Step 2: Update the expectations the new flow changes**

- `Matroska h264 remuxes losslessly and seeks within 3s` → rename to `a Matroska the browser can play is served from the original`: keep the decision probe (`stream`/`remux` — the server is unchanged), then assert the element plays the original (`src` matches the plain URL, no `blob:`), no stream request was made, and a seek to 15 s completes. Keep the seek bound as the wait's own timeout; if the original's seek measures slower than 3 s, raise the bound and leave the timing claim out of the name.
- `AC-3 audio converts without re-encoding video` → assert the first `/video/stream*` request carries `mode=audio` (the attempt's audio gate is what sends the file there), in addition to the existing playback assertion.
- `an exhausted ladder shows the error and keeps the original playable` → after the escape hatch is clicked and the original plays, assert no further `/video/stream*` request is made (the hatch must not re-enter the conversion loop, FR-015).
- `a previously converted video starts without a blocking conversion` → expectations unchanged; verify it passes (the first open's attempt fails instantly; the reopen is served from the session-remembered failure).

- [ ] **Step 3: Run the video specs**

Run: `npx playwright test tests/e2e/specs/video-streaming.e2e.spec.js tests/e2e/specs/transcoding.e2e.spec.js tests/e2e/specs/video-playback.e2e.spec.js tests/e2e/specs/video-moov.e2e.spec.js`
Expected: PASS.

- [ ] **Step 4: Full verification and commit**

```bash
npm run test:unit && npm run test:i18n && npm run lint && npm run format
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
npm run test:e2e
git add tests/e2e/specs/video-streaming.e2e.spec.js
git commit -m "test(e2e): move the streaming specs onto the attempt-first flow"
```
