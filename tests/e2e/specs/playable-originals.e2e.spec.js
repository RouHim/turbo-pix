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

    // AND the server's own answer is untouched: no artifact was written behind
    // the playback. `page.request` bypasses `page.route`, so this reads the
    // real server, not the rewritten decision the page was served.
    const probe = await page.request.get(
      `/api/photos/${photo.hash_sha256}/video?decision&client=h264-8%2Caac`
    );
    expect(probe.ok()).toBeTruthy();
    const realDecision = await probe.json();
    expect(realDecision.action).toBe('direct');
    expect(realDecision.cached).toBe(false);

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
    // GIVEN a delivery that takes well over a second to produce its first byte
    const photo = await findVideoByFilename(page, 'test_video.mp4');
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
    await page.route(PLAIN_VIDEO, async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 3000));
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

  test('a switch away from a pending attempt leaves nothing behind', async ({ page }) => {
    // GIVEN an attempt that is still pending (its delivery takes 8 s) when the
    // user moves on
    const photo = await findVideoByFilename(page, 'test_video.mp4');
    await TestHelpers.clearCachedConversions(page, photo.hash_sha256);
    await page.route(PLAIN_VIDEO, async (route) => {
      if (!route.request().url().includes(photo.hash_sha256)) return route.continue();
      await new Promise((resolve) => setTimeout(resolve, 8000));
      await route.continue().catch(() => {});
    });
    const streamRequests = collectRequests(page, STREAM_VIDEO);
    const wholeFileRequests = collectRequests(page, WHOLE_FILE);

    await openVideo(page, photo);
    await page.waitForTimeout(500); // the attempt is armed and has not resolved

    // WHEN the viewer moves to the next photo
    await page.keyboard.press('ArrowRight');
    await expect.poll(() => TestHelpers.getCurrentPhotoHash(page)).not.toBe(photo.hash_sha256);
    await page.waitForTimeout(1000);

    // THEN the abandoned attempt left no notice and no conversion behind, and
    // the photo now on screen is unaffected.
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
    expect(
      streamRequests.filter((request) => request.url.includes(photo.hash_sha256))
    ).toHaveLength(0);
    expect(
      wholeFileRequests.filter((request) => request.url.includes(photo.hash_sha256))
    ).toHaveLength(0);
    await expect(page.locator(TestHelpers.selectors.viewer)).toHaveClass(/active/);
  });
});
