import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

/**
 * The viewer must PLAY THE ORIGINAL before it converts anything.
 *
 * Every test here keys on the URLs a playback can consist of, so the wire is
 * the evidence and no internal state needs to be read:
 *   - the plain `?client=` byte request IS the original attempt;
 *   - `/video/stream` and `?…transcode=true` are the only two URLs a
 *     conversion can be started by, so "a conversion ran" is nothing but one of
 *     them being requested.
 *
 * A server decision is what the viewer MUST NOT obey on faith, so the tests
 * that need a conversion planned while the original is playable rewrite the
 * server's answer (`underReportDecision`) instead of hoping for one.
 */

const PLAIN_VIDEO = /\/api\/photos\/[^/]+\/video\?client=/; // the attempt's own byte request
const STREAM_VIDEO = /\/api\/photos\/[^/]+\/video\/stream/;
const WHOLE_FILE = /\/api\/photos\/[^/]+\/video\?(?:[^/]*&)?transcode=true/;

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

/** Every request whose URL matches `pattern`, in order, with a client clock. */
function collectRequests(page, pattern) {
  const seen = [];
  page.on('request', (request) => {
    if (pattern.test(request.url())) seen.push({ url: request.url(), at: Date.now() });
  });
  return seen;
}

/** The viewer's video element, the only element a playback can happen in. */
function videoHandle(page) {
  return page.locator(TestHelpers.selectors.viewerVideo);
}

/** Wait until a request for `hash` was seen by the collector. */
async function waitForRequest(requests, hash, timeout = 5000) {
  await expect
    .poll(() => requests.some((request) => request.url.includes(hash)), { timeout })
    .toBe(true);
}

/** The element is playing something for `photo` — a decoded frame of its own delivery. */
async function waitForPlaybackOf(page, photo, timeout = 10000) {
  await page.waitForFunction(
    (hash) => {
      const el = document.querySelector('#viewer-video');
      return !!el && el.dataset.photoHash === hash && el.readyState >= 2 && el.currentTime > 0;
    },
    photo.hash_sha256,
    { timeout }
  );
}

/**
 * The element holds `photo`'s ORIGINAL bytes and has decoded a frame of them —
 * for THIS open.
 *
 * The clauses are load-bearing. The `photoHash` stamp the source assignment
 * writes (and `displayPhoto` clears) rules out the previous photo's playback,
 * which the element keeps in place until the new decision resolves; the plain
 * `?client=` src is the original itself, which a streamed conversion never
 * matches; and `readyState >= 2` with a decoded `videoWidth` is the frame —
 * exactly the proof `startOriginalAttempt` itself settles a playback on.
 *
 * The clock is deliberately NOT part of it: the app's `viewSettings.autoPlay`
 * defaults to false, so the original loads and decodes its first frame PAUSED
 * (`currentTime` stays 0) — requiring a moving clock would assert the viewer's
 * autoplay preference, not the codec. Measured on this checkout: the original
 * renders its frame (screenshot of a failed run shows it) with the native
 * controls at `0:00`, while the MSE path — which calls `play()` itself —
 * advances the clock.
 */
async function waitForOriginalPlayback(page, photo, timeout = 5000) {
  await page.waitForFunction(
    (hash) => {
      const el = document.querySelector('#viewer-video');
      if (!el || el.dataset.photoHash !== hash) return false;
      const src = el.getAttribute('src') || '';
      if (!src.includes(`/api/photos/${hash}/video?`) || !src.includes('client=')) return false;
      return el.readyState >= 2 && el.videoWidth > 0;
    },
    photo.hash_sha256,
    { timeout }
  );
}

/**
 * The codec tokens the page has recorded for proved playbacks — the store
 * `playbackVerification.js` writes through `markCodecVerified` (key
 * `turbopix_verified_video_codecs`).
 *
 * It is the only witness a verdict leaves behind: a token credited to the
 * wrong photo changes no request URL, so the wire the rest of this file reads
 * cannot see it.
 */
async function verifiedCodecTokens(page) {
  return page.evaluate(() => {
    const stored = localStorage.getItem('turbopix_verified_video_codecs');
    return stored === null ? [] : JSON.parse(stored);
  });
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

test.describe('Playable originals are never converted', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    // Every delivery the viewer starts must be VISIBLE, and Chromium's HTTP
    // cache would hide it: the byte response is cacheable for a year, and a
    // cache hit raises no request event — a source assigned twice (a reopen of
    // the same photo) would then look like "no request at all", and a
    // conversion could hide behind the same silence. Playwright's interception
    // bypasses the cache, so the wire stays the evidence. Tests that need a
    // specific delivery register their own handler later, which takes
    // precedence over this pass-through.
    await page.route(/\/api\/photos\/[^/]+\/video/, (route) => route.continue());
    // Every test here drives a real playback (and often a conversion) against
    // the shared server; under a fully loaded suite run the 30 s default is
    // not enough for the setup waits alone. The inner assertion bounds are
    // untouched, so a regression still fails — this is only the budget the
    // whole test may take (the sibling video-streaming spec sets the same).
    test.setTimeout(60_000);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('a playable original is never converted, even when the server plans a conversion', async ({
    page,
  }) => {
    // GIVEN the server plans a conversion for a file the browser can play
    const photo = await findVideoByFilename(page, 'test_video.mp4');
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
    await underReportDecision(page);
    const plainRequests = collectRequests(page, PLAIN_VIDEO);
    const streamRequests = collectRequests(page, STREAM_VIDEO);
    const wholeFileRequests = collectRequests(page, WHOLE_FILE);

    // WHEN the original is opened
    await openVideo(page, photo);
    await waitForRequest(plainRequests, photo.hash_sha256);
    await waitForOriginalPlayback(page, photo);

    // THEN it plays the original itself — the plain attempt URL is the source
    expect(await videoHandle(page).getAttribute('src')).toMatch(PLAIN_VIDEO);
    // AND no conversion was started: neither URL that can start one was asked
    expect(streamRequests).toHaveLength(0);
    expect(wholeFileRequests).toHaveLength(0);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);

    // AND the server's own answer is untouched, and no artifact was written
    // behind the playback. `page.request` bypasses `page.route`, so this reads
    // the real server, not the rewritten decision the page was served. The
    // decision's own `cached` flag cannot carry that claim: the Direct arm
    // hard-codes `cached: false` and never consults an artifact, so even a
    // viewer that ran a full conversion would be told `false` here — the cache
    // itself is the witness.
    const probe = await page.request.get(
      `/api/photos/${photo.hash_sha256}/video?decision&client=h264-8%2Caac`
    );
    expect(probe.ok()).toBeTruthy();
    const realDecision = await probe.json();
    expect(realDecision.action).toBe('direct');
    // The list is read UNFILTERED: `conversionCacheEntries` reports finished
    // artifacts and the temp file of a conversion still in flight (a whole-file
    // job's `…mp4.tmp`, a remux fill's `…{pid}.{seq}.tmp`). This is an INVARIANT
    // check — nothing converted behind this playback — not a witness for a
    // plan-obeying viewer: that viewer starts no attempt at all, so it fails the
    // plain-request wait and the request-count assertions above and never reaches
    // this read.
    const cacheEntries = await TestHelpers.conversionCacheEntries(photo.hash_sha256);
    expect(cacheEntries).toHaveLength(0);

    // AND reopening replays the original, still without a notice
    await TestHelpers.closeViewer(page);
    const reopenedPlainRequests = collectRequests(page, PLAIN_VIDEO);
    await openVideo(page, photo);
    // A NEW attempt request is what makes this the reopen's own playback: the
    // element already holds the previous one's state, and the URL is the same.
    await waitForRequest(reopenedPlainRequests, photo.hash_sha256);
    await waitForOriginalPlayback(page, photo);
    expect(await videoHandle(page).getAttribute('src')).toMatch(PLAIN_VIDEO);
    expect(streamRequests).toHaveLength(0);
    expect(wholeFileRequests).toHaveLength(0);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('a codec an actual playback proved is declared, and survives a reload', async ({ page }) => {
    // GIVEN the server names a codec for the file the attempt is about to play
    const photo = await findVideoByFilename(page, 'test_video.mp4');
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
    await underReportDecision(page, { codec: 'hevc' });

    // Every declaration the viewer sends, in order.
    const declarations = [];
    page.on('request', (request) => {
      const url = new URL(request.url());
      if (url.pathname.endsWith('/video') && url.searchParams.has('decision')) {
        declarations.push(url.searchParams.get('client') || '');
      }
    });

    // WHEN the original plays once
    await openVideo(page, photo);
    await waitForOriginalPlayback(page, photo);

    // THEN the first declaration did not (and could not) claim that codec…
    expect(declarations.length).toBeGreaterThan(0);
    expect(declarations[0]).not.toContain('hevc');

    // WHEN the page is reloaded and the video reopened
    await TestHelpers.closeViewer(page);
    await page.reload({ waitUntil: 'domcontentloaded' });
    await openVideo(page, photo);
    await expect.poll(() => declarations.length, { timeout: 5000 }).toBeGreaterThan(1);

    // THEN the declaration now carries the codec the playback proved.
    expect(declarations[declarations.length - 1]).toContain('hevc');
  });

  test('an unsupported codec converts only after the original attempt failed', async ({ page }) => {
    // GIVEN a cold hevc video the browser cannot decode
    const photo = await findVideoByFilename(page, 'test_video_hevc.mp4');
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
    const plainRequests = collectRequests(page, PLAIN_VIDEO);
    const streamRequests = collectRequests(page, STREAM_VIDEO);

    // WHEN the viewer opens it
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    const clickedAt = Date.now();
    await page.locator(TestHelpers.selectors.photoCard(photo.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);

    // THEN the original was attempted first…
    await expect.poll(() => plainRequests.length, { timeout: 3000 }).toBeGreaterThan(0);
    // …and the conversion follows it, almost at once: the element refuses the
    // codec in milliseconds, so a grace window before the rung would show up
    // here as thousands of milliseconds.
    await expect.poll(() => streamRequests.length, { timeout: 3000 }).toBeGreaterThan(0);
    expect(plainRequests[0].at).toBeLessThanOrEqual(streamRequests[0].at);
    expect(streamRequests[0].at - clickedAt).toBeLessThan(2000);
  });

  test('a video that failed this session starts its conversion on reopen without the attempt', async ({
    page,
  }) => {
    // GIVEN a cold hevc video whose original has just failed this session
    const photo = await findVideoByFilename(page, 'test_video_hevc.mp4');
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
    const firstStreamRequests = collectRequests(page, STREAM_VIDEO);
    await openVideo(page, photo);
    // The attempt fails and hands over to the plan — the fact the session
    // remembers.
    await expect.poll(() => firstStreamRequests.length, { timeout: 5000 }).toBeGreaterThan(0);
    await TestHelpers.closeViewer(page);
    // A stream run that completed fills the whole-file cache, which would turn
    // the reopen's planned rung into a cached FILE delivery instead of the
    // planned stream; the cold premise is restored here (which also waits out
    // whatever conversion is still running).
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);

    // WHEN the same video is opened again
    const plainRequests = collectRequests(page, PLAIN_VIDEO);
    const streamRequests = collectRequests(page, STREAM_VIDEO);
    const clickedAt = Date.now();
    await page.locator(TestHelpers.selectors.photoCard(photo.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);

    // THEN the planned rung starts right away — no grace window was repeated
    await expect.poll(() => streamRequests.length, { timeout: 2000 }).toBeGreaterThan(0);
    expect(streamRequests[0].at - clickedAt).toBeLessThan(2000);
    // …and the known-failing attempt was not repeated: the reopen is past the
    // decision by now, which is where an attempt would have been armed.
    expect(plainRequests).toHaveLength(0);
  });

  test('one failed file never disables direct play for its codec', async ({ page }) => {
    // GIVEN a video whose ORIGINAL attempt fails outright (a 404: the server
    // has nothing to serve for that file, and only that file)
    const ac3 = await findVideoByFilename(page, 'test_video_ac3.mp4');
    const h264 = await findVideoByFilename(page, 'test_video.mp4');
    await TestHelpers.failOriginalAttempt(page, ac3.hash_sha256);
    await TestHelpers.clearCachedConversions(page, ac3.hash_sha256);
    const ac3StreamRequests = collectRequests(page, STREAM_VIDEO);
    await openVideo(page, ac3);
    // Its audio rung still runs and plays: the failure handed over, it was not
    // swallowed.
    await expect.poll(() => ac3StreamRequests.length, { timeout: 10000 }).toBeGreaterThan(0);
    await waitForPlaybackOf(page, ac3);
    await TestHelpers.closeViewer(page);

    // WHEN a second, h264 video is opened
    const streamRequests = collectRequests(page, STREAM_VIDEO);
    const wholeFileRequests = collectRequests(page, WHOLE_FILE);
    await openVideo(page, h264);
    await waitForOriginalPlayback(page, h264);

    // THEN the first file's failure changed nothing for the codec: the second
    // plays from its original, with no conversion for it at all.
    expect(await videoHandle(page).getAttribute('src')).toMatch(PLAIN_VIDEO);
    expect(streamRequests.filter((request) => request.url.includes(h264.hash_sha256))).toHaveLength(
      0
    );
    expect(
      wholeFileRequests.filter((request) => request.url.includes(h264.hash_sha256))
    ).toHaveLength(0);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('containers the browser plays are not remuxed', async ({ page }) => {
    for (const filename of ['test_video_moov_end.mp4', 'test_video_long.mkv']) {
      // GIVEN a container/codec combination the browser plays, cold
      const photo = await findVideoByFilename(page, filename);
      await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
      const streamRequests = collectRequests(page, STREAM_VIDEO);
      const wholeFileRequests = collectRequests(page, WHOLE_FILE);

      // WHEN it is opened
      await openVideo(page, photo);
      await waitForOriginalPlayback(page, photo, 10000);

      // THEN it plays the original container: no remux, no conversion, no
      // notice.
      expect(await videoHandle(page).getAttribute('src')).toMatch(PLAIN_VIDEO);
      expect(streamRequests).toHaveLength(0);
      expect(wholeFileRequests).toHaveLength(0);
      await expect(page.locator('.transcode-toast')).toHaveCount(0);

      await TestHelpers.closeViewer(page);
    }
  });

  test('a slow-but-delivering original is not converted', async ({ page }) => {
    // GIVEN a delivery whose first byte takes LONGER than the grace window
    // itself. A delay inside the window (a 3 s one) could not tell this design
    // apart from a deadline armed once and never re-armed: the frame would land
    // inside either. At 6.5 s the frame only lands because the window is still
    // open — the watchdog re-arms the deadline on every tick while no stall is
    // pending, and the element's `stalled` (~4 s in) then freezes it a grace
    // window later (~9 s), past the 6.5 s resume. That freeze, not the resume's
    // own `progress`, is what covers the frame here (FR-004/SC-005).
    const photo = await findVideoByFilename(page, 'test_video.mp4');
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
    await page.route(PLAIN_VIDEO, async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 6500));
      await route.continue();
    });
    const streamRequests = collectRequests(page, STREAM_VIDEO);
    const wholeFileRequests = collectRequests(page, WHOLE_FILE);

    // WHEN the viewer opens it
    await openVideo(page, photo);
    await waitForOriginalPlayback(page, photo, 10000);

    // THEN the delivery is waited for and plays: slow is not unplayable.
    expect(await videoHandle(page).getAttribute('src')).toMatch(PLAIN_VIDEO);
    expect(streamRequests).toHaveLength(0);
    expect(wholeFileRequests).toHaveLength(0);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('a stalled delivery takes the planned rung after the window', async ({ page }) => {
    // GIVEN a delivery that is served but never delivers anything, and a
    // server that plans a conversion for the file
    const photo = await findVideoByFilename(page, 'test_video.mp4');
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
    await underReportDecision(page);
    await page.route(PLAIN_VIDEO, () => {});
    const streamRequests = collectRequests(page, STREAM_VIDEO);

    // WHEN the viewer opens it
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    const clickedAt = Date.now();
    await page.locator(TestHelpers.selectors.photoCard(photo.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);

    // THEN the grace window runs with no notice at all: there is no job to
    // report yet.
    await page.waitForTimeout(2500);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);

    // AND the planned rung starts only after that window — not before it, and
    // not never.
    //
    // The window alone would put this at grace + a moment, but the deadline is
    // frozen by the element's own `stalled` (that is what makes it a timeout
    // rather than an infinite wait), and inside the app Chromium reports that
    // stall LATE: measured ~4.1 s after the click here (9.1 s total), against
    // ~1.1 s after `loadstart` for an isolated element on this machine. The
    // lower bound is the contract (FR-003: never before the window); the upper
    // one only rules out a viewer that never gets there.
    await expect.poll(() => streamRequests.length, { timeout: 15000 }).toBeGreaterThan(0);
    const elapsed = streamRequests[0].at - clickedAt;
    expect(elapsed).toBeGreaterThanOrEqual(5000);
    expect(elapsed).toBeLessThanOrEqual(15000);
  });

  test('an empty file is never attempted', async ({ page }) => {
    // GIVEN a file the server reports as empty
    const photo = await findVideoByFilename(page, 'test_video.mp4');
    await page.route('**/video?decision*', (route) => route.fulfill({ json: { action: 'empty' } }));
    const plainRequests = collectRequests(page, PLAIN_VIDEO);
    const streamRequests = collectRequests(page, STREAM_VIDEO);
    const wholeFileRequests = collectRequests(page, WHOLE_FILE);

    // WHEN the viewer opens it
    await openVideo(page, photo);

    // THEN it says so, and fetches nothing at all: there are no bytes to
    // attempt and no bytes to convert.
    const toast = page.locator('.transcode-toast');
    await expect(toast).toBeVisible();
    await expect(toast).toContainText(/empty|synced/i);
    expect(plainRequests).toHaveLength(0);
    expect(streamRequests).toHaveLength(0);
    expect(wholeFileRequests).toHaveLength(0);
  });

  test('a direct plan that failed converts through the stream, not the bytes it refuted', async ({
    page,
  }) => {
    // GIVEN a file the server plans as DIRECT — its own bytes are what the plan
    // offers — and an original delivery that fails outright
    const photo = await findVideoByFilename(page, 'test_video.mp4');
    await TestHelpers.failOriginalAttempt(page, photo.hash_sha256);
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
    const streamRequests = collectRequests(page, STREAM_VIDEO);
    const wholeFileRequests = collectRequests(page, WHOLE_FILE);

    // WHEN the viewer opens it
    await openVideo(page, photo);

    // THEN the conversion runs through the STREAM endpoint: a Direct plan has no
    // rung of its own, and the byte endpoint ignores `transcode=true` for a
    // directly playable source — it would hand back the very bytes the attempt
    // just refuted, leaving the viewer to fail on the same media twice.
    await expect.poll(() => streamRequests.length, { timeout: 15000 }).toBeGreaterThan(0);
    expect(streamRequests[0].url).toContain('mode=transcode');
    // AND the file really plays from those converted bytes.
    await waitForPlaybackOf(page, photo);
    // AND the byte endpoint's trap was never walked into: no whole-file request
    // for this photo at all.
    expect(
      wholeFileRequests.filter((request) => request.url.includes(photo.hash_sha256))
    ).toHaveLength(0);
  });

  test('a switch away from a pending attempt leaves nothing behind', async ({ page }) => {
    // GIVEN an attempt that is still pending when the user moves on. The photo
    // is the 10-BIT one on purpose: this test's claim is attributed by a codec
    // token, and `test_video.mp4` shares its token (`h264-8`) with the photo
    // that follows it (`test_video_multitrack.mp4` is h264 8-bit too), so a
    // verdict credited to the abandoned photo would be indistinguishable from
    // the next photo's own — the assertion below could not fail. This photo
    // maps to `h264-10` and the one that replaces it to `h264-8`; both are read
    // from the decisions the viewer itself was served, below.
    const photo = await findVideoByFilename(page, 'test_video_10bit.mp4');
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
    // The verdict store, cleared before the open: every token in it afterwards
    // was recorded by a playback this test covers.
    await page.evaluate(() => localStorage.removeItem('turbopix_verified_video_codecs'));
    // The delivery is HELD, so the attempt can never settle from its own bytes
    // and is still pending when the switch comes — that pending attempt is what
    // the cancellation has to tear down, and what a regressed teardown leaves
    // listening. The hold is an order the test can wait on (the plain request
    // asserted below) instead of a sleep; the follower's own plain request is a
    // different hash, so its route is answered without delay.
    await page.route(PLAIN_VIDEO, async (route) => {
      if (!route.request().url().includes(photo.hash_sha256)) return route.continue();
      await new Promise((resolve) => setTimeout(resolve, 12000));
      await route.continue().catch(() => {});
    });
    const plainRequests = collectRequests(page, PLAIN_VIDEO);
    const streamRequests = collectRequests(page, STREAM_VIDEO);
    const wholeFileRequests = collectRequests(page, WHOLE_FILE);

    await openVideo(page, photo);
    // The attempt must be ARMED before the switch, and that is observed rather
    // than assumed: the pending `?client=` request for this photo is the
    // evidence, and its held route keeps it pending across the switch. A fixed
    // sleep could fire before the decision round-trip finished, leaving
    // `displayVideo` to arm nothing once the newer photo bails it — and the
    // "nothing left behind" assertion would then hold over a cancellation path
    // that never ran.
    await waitForRequest(plainRequests, photo.hash_sha256);

    // AND it is still PENDING at the moment of the switch — the second half of
    // that premise, and the one nothing else asserts: `waitForRequest` proves
    // only that the request was issued. A held delivery does not stay pending
    // forever (the element reports `stalled` ~4 s in, which freezes the
    // watchdog, and the grace window then expires the attempt ~9 s after
    // arming); an attempt that expired before the switch settles `unplayable`
    // and hands over to the plan, which for this 10-bit file is a conversion.
    // A conversion request for THIS photo before the switch is therefore the
    // witness that the attempt was already gone — and without this check the
    // test could pass with the cancellation path it exists for never exercised
    // (a settled attempt detaches its own listeners; only a pending one needs
    // `cancelOriginalAttempt`).
    expect(
      streamRequests.filter((request) => request.url.includes(photo.hash_sha256)),
      'the abandoned attempt must still be pending: a settled one hands over to a conversion'
    ).toHaveLength(0);
    expect(
      wholeFileRequests.filter((request) => request.url.includes(photo.hash_sha256))
    ).toHaveLength(0);

    // WHEN the viewer moves to the next photo
    await page.keyboard.press('ArrowRight');
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).not.toBe(photo.hash_sha256);
    const nextHash = await TestHelpers.getCurrentPhotoHash(page);

    // AND the abandoned attempt recorded no verdict behind. The element is
    // shared, and a still-armed attempt observes ITS events (`FRAME_EVENTS` in
    // `originalAttempt.js`): the photo now on screen decoding its own first
    // frame would settle the stale attempt `playable`, and that continuation
    // records the token captured BY VALUE when the stale attempt was armed —
    // the ABANDONED photo's — through `recordVerifiedCodec` (FR-007). The
    // cancellation is what detaches that listener set, so the store is the
    // witness, and it is read AFTER the next photo's own verdict landed, which
    // is what makes the absence below a claim about a live store rather than an
    // empty one: that frame event IS the settle trigger, and the stale attempt
    // is settled ahead of the new one (its listeners were attached first), so
    // its token would already be stored by the time the new photo's appears.
    //
    // The claim is about the OUTCOME (no verdict for the photo that was left
    // behind) rather than about a call site: the switch tears the attempt down
    // in `displayPhoto` for every photo change, and `displayVideo` re-states
    // the same idempotent cancellation at its own entry. The mutant this test
    // must fail is that teardown removed — `cancelOriginalAttempt()` deleted,
    // plus both guards dropped from the continuation armed in
    // `armOriginalAttempt` (`PhotoViewer.svelte`) — after which the stale
    // attempt settles on the next photo's frame event and credits `h264-10`
    // into the store read below.
    const abandonedDecision = await (
      await page.request.get(`/api/photos/${photo.hash_sha256}/video?decision&client=h264-8%2Caac`)
    ).json();
    const nextDecision = await (
      await page.request.get(`/api/photos/${nextHash}/video?decision&client=h264-8%2Caac`)
    ).json();
    // `codecTokenFor` maps these SOURCE facts — reported identically for every
    // delivery of a file — to the tokens the store can hold: h264 10-bit →
    // `h264-10`, h264 8-bit → `h264-8`. Asserted, so the two literals below can
    // never drift into a pair that shares one token, which is exactly the pair
    // `test_video.mp4` / `test_video_multitrack.mp4` is.
    expect(abandonedDecision.codec).toBe('h264');
    expect(abandonedDecision.bit_depth).toBe(10);
    expect(nextDecision.codec).toBe('h264');
    expect(nextDecision.bit_depth).toBe(8);
    // The order is load-bearing: the photo now on screen proves its own
    // playback FIRST — its token is the recorded evidence of the frame event
    // that is also the stale attempt's settle trigger — and only then is the
    // abandoned token's absence read, so that absence is a claim about a live
    // store rather than one that never saw the event at all.
    await expect.poll(() => verifiedCodecTokens(page), { timeout: 10000 }).toContain('h264-8');
    // THEN nothing the abandoned attempt observed is recorded: a verdict
    // credited to it would have been written by that same frame event.
    expect(await verifiedCodecTokens(page)).not.toContain('h264-10');

    // AND the photo now on screen is unaffected: it still owns the element and
    // holds a delivery of its own.
    await expect.poll(() => videoHandle(page).getAttribute('data-photo-hash')).toBe(nextHash);
    await expect
      .poll(async () => {
        const src = await videoHandle(page).getAttribute('src');
        if (!src) return false;
        return src.startsWith('blob:') || src.includes(`/api/photos/${nextHash}/video?`);
      })
      .toBe(true);
    await expect(page.locator(TestHelpers.selectors.viewer)).toHaveClass(/active/);
  });

  test('a granted but slow stream run is never labelled as queued', async ({ page }) => {
    // GIVEN a video the browser cannot decode (its attempt fails in
    // milliseconds, so the stream rung runs) and a conversion run that IS
    // granted but takes seconds to deliver its first bytes
    const photo = await findVideoByFilename(page, 'test_video_hevc.mp4');
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
    await underReportDecision(page);
    await page.route(STREAM_VIDEO, async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 4000));
      await route.continue();
    });

    // Every notice the toast ever shows, recorded as it is rendered. The toast
    // is created and replaced as the run progresses, so a snapshot taken after
    // playback would miss a notice that was only up while the bytes were
    // withheld — which is exactly the claim under test. The observer is
    // installed before the open, on the body, because the toast does not exist
    // yet and is mounted only once the viewer has something to say.
    await page.evaluate(() => {
      window.__toastTexts = [];
      const record = () => {
        const toast = document.querySelector('.transcode-toast');
        if (toast) window.__toastTexts.push(toast.textContent);
      };
      new MutationObserver(record).observe(document.body, {
        childList: true,
        subtree: true,
        characterData: true,
      });
    });

    // WHEN the viewer opens it and the granted run is slow to deliver
    await openVideo(page, photo);
    await waitForPlaybackOf(page, photo, 20000);

    // THEN the whole wait is reported as preparation, never as a queue wait:
    // the run holds its conversion slot, the pool was never busy, and only the
    // server's own 503 refusal may claim otherwise (spec FR-012, Scenario 4.2).
    const texts = await page.evaluate(() => window.__toastTexts);
    expect(texts.some((text) => /being prepared for playback/i.test(text))).toBe(true);
    expect(texts.some((text) => /free conversion slot/i.test(text))).toBe(false);
  });
});
