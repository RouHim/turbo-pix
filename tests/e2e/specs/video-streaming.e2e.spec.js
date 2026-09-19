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

/**
 * Answer the stream requests for the modes named in `failModes` with bytes the
 * browser cannot decode — the server answered, the delivery is unusable, which
 * is exactly what a remux the client cannot actually play looks like on the
 * wire — and record every mode the app asked for, in request order. The modes
 * not named keep hitting the real server, so a successful rung still plays.
 */
async function failStreamModes(page, failModes) {
  const requestedModes = [];
  await page.route('**/video/stream*', async (route) => {
    const mode = new URL(route.request().url()).searchParams.get('mode');
    requestedModes.push(mode);
    if (failModes.includes(mode)) {
      await route.fulfill({
        status: 200,
        headers: { 'content-type': 'video/mp4' },
        body: 'not-mp4',
      });
      return;
    }
    await route.continue();
  });
  return requestedModes;
}

test.describe('On-the-fly streaming playback', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForPhotosToLoad(page);
  });

  test('HEVC plays while converting: first frame within 5s, true duration', async ({ page }) => {
    const hevc = await findVideoByFilename(page, 'test_video_hevc.mp4');
    // This test is about the *streaming* path, so it starts from a cold cache:
    // a playthrough earlier in the run would otherwise make it a direct play.
    await TestHelpers.clearCachedConversions(hevc.hash_sha256);
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
    // A cold cache: a playthrough fills the cache (video copied, audio
    // converted), after which the same probe legitimately answers
    // `direct`/`cached`. Probe *before* playing so the answer is the first-play
    // one rather than a race against the background fill.
    await TestHelpers.clearCachedConversions(ac3.hash_sha256);
    const response = await page.request.get(
      `/api/photos/${ac3.hash_sha256}/video?decision&client=h264-8,aac`
    );
    const decision = await response.json();
    expect(decision.action).toBe('stream');
    expect(decision.mode).toBe('audio');

    await openVideo(page, ac3);
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

  test('saturated conversions wait visibly and then start', async ({ page }) => {
    test.setTimeout(60_000);
    const hevc = await findVideoByFilename(page, 'test_video_hevc.mp4');
    // Saturation is only observable while the decisions say "stream": a
    // playthrough earlier in the run may have cached the conversion.
    await TestHelpers.clearCachedConversions(hevc.hash_sha256);

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
    await expect(page.locator('.transcode-toast')).toContainText(
      'Waiting for a free conversion slot',
      {
        timeout: 10_000,
      }
    );
    // Waiting is not a dead end: the escape hatch stays reachable while the
    // pool is full (a permanently disabled pool would otherwise trap the user).
    await expect(page.locator('.transcode-toast [data-action="play-original"]')).toBeVisible();

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

  test('a previously converted video starts without a blocking conversion', async ({ page }) => {
    test.setTimeout(120_000);
    const hevc = await findVideoByFilename(page, 'test_video_hevc.mp4');
    // The first open must stream, so nothing may be cached for it yet.
    await TestHelpers.clearCachedConversions(hevc.hash_sha256);
    await openVideo(page, hevc);

    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );

    // A full playthrough fills the whole-file cache in the background (the
    // fixture is 2 s of video, so the conversion is quick); the same probe then
    // reports the conversion as a cached artifact.
    const decisionUrl = `/api/photos/${hevc.hash_sha256}/video?decision&client=h264-8,aac`;
    await expect
      .poll(
        async () => {
          const response = await page.request.get(decisionUrl);
          return (await response.json()).cached;
        },
        { timeout: 30_000, message: 'the playthrough must fill the conversion cache' }
      )
      .toBe(true);

    const decision = await (await page.request.get(decisionUrl)).json();
    expect(decision.action).toBe('direct');
    expect(decision.cached).toBe(true);

    // Reopen: playback starts natively from the cache, without a byte of
    // streaming and without a conversion notice. A `direct` source only
    // auto-plays when the viewer's autoPlay setting is on (the MSE path always
    // self-plays), so enable it before measuring.
    await page.evaluate(() =>
      localStorage.setItem('viewSettings', JSON.stringify({ autoPlay: true }))
    );
    const requests = [];
    page.on('request', (request) => requests.push(request.url()));
    await TestHelpers.closeViewer(page);
    const started = Date.now();
    await page.locator(TestHelpers.selectors.photoCard(hevc.hash_sha256)).click();
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        // Anchor on the cached artifact itself: closing the viewer only pauses
        // and revokes the media, so the previous playthrough's element state is
        // still observable until the reopen swaps the source.
        return (
          el && el.currentSrc.includes('transcode=true') && el.readyState >= 2 && el.currentTime > 0
        );
      },
      null,
      { timeout: 2000 }
    );
    expect(Date.now() - started).toBeLessThan(2000);
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
    expect(requests.filter((url) => url.includes('/video/stream'))).toEqual([]);
    expect(
      requests.some(
        (url) =>
          url.includes(`/api/photos/${hevc.hash_sha256}/video?`) && url.includes('transcode=true')
      ),
      'the cached conversion must be served as a file'
    ).toBe(true);
  });

  test('a failed remux stream escalates one step and recovers', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    // The ladder only runs on a stream delivery: a cached artifact would be
    // played as a file, with no stream request to fail.
    await TestHelpers.clearCachedConversions(mkv.hash_sha256);
    const requestedModes = await failStreamModes(page, ['remux']);

    await openVideo(page, mkv);
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );

    // Exactly one rung: the undecodable remux becomes audio, and the ladder
    // never falls back to the rung that just failed.
    expect(requestedModes[0]).toBe('remux');
    expect(requestedModes.filter((mode) => mode === 'remux')).toHaveLength(1);
    expect(new Set(requestedModes.slice(1))).toEqual(new Set(['audio']));
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });

  test('an exhausted ladder shows the error and keeps the original playable', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    await TestHelpers.clearCachedConversions(mkv.hash_sha256);
    const requestedModes = await failStreamModes(page, ['remux', 'audio', 'transcode']);

    await openVideo(page, mkv);
    await expect(page.locator('.transcode-toast')).toContainText('Video conversion failed', {
      timeout: 30_000,
    });

    // One rung per failure, in order, and the ladder stops at its end: three
    // attempts, never a fourth.
    expect(requestedModes).toEqual(['remux', 'audio', 'transcode']);
    // The escape hatch is offered instead of a dead end.
    await expect(page.locator('.transcode-toast [data-action="play-original"]')).toBeVisible();
  });

  test('a seek restart keeps the declared duration', async ({ page }) => {
    test.setTimeout(60_000);
    const mkv = await findVideoByFilename(page, 'test_video_long.mkv');
    await TestHelpers.clearCachedConversions(mkv.hash_sha256);

    // A seek only restarts the stream when its target is not buffered yet, and
    // this 20 s remux buffers in a few hundred milliseconds: throttle the
    // delivery so the seek below is a real restart, which is where the inflated
    // duration came from in the first place (a seek run is a stream copy whose
    // last packet lands past the source's own duration).
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Network.enable');
    await cdp.send('Network.emulateNetworkConditions', {
      offline: false,
      latency: 40,
      downloadThroughput: 150 * 1024,
      uploadThroughput: 150 * 1024,
      connectionType: 'cellular3g',
    });

    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator(TestHelpers.selectors.photoCard(mkv.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);

    const video = videoHandle(page);
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );

    // Seek to 15 s the moment playback starts, while the target is still
    // unbuffered: the player answers by restarting the stream there.
    const restarted = await video.evaluate(
      () =>
        new Promise((resolve) => {
          const el = document.querySelector('#viewer-video');
          const timer = setInterval(() => {
            if (!el || el.buffered.length === 0) return;
            if (el.buffered.end(el.buffered.length - 1) >= 15) return;
            if (el.currentTime <= 0) return;
            clearInterval(timer);
            el.currentTime = 15;
            resolve(true);
          }, 10);
          setTimeout(() => {
            clearInterval(timer);
            resolve(false);
          }, 25_000);
        })
    );
    expect(restarted, 'the seek must restart the stream at an unbuffered target').toBe(true);

    await cdp.send('Network.emulateNetworkConditions', {
      offline: false,
      latency: 0,
      downloadThroughput: -1,
      uploadThroughput: -1,
      connectionType: 'none',
    });
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.currentTime >= 15 && el.readyState >= 2;
      },
      null,
      { timeout: 20_000 }
    );

    // The element reports the source's own duration (20.02 s), not the end of
    // the fragment the restart delivered (21 s) — the seek bar must not grow
    // past the end of the video.
    const duration = await video.evaluate((el) => el.duration);
    expect(duration).toBeGreaterThan(19.5);
    expect(duration).toBeLessThan(20.5);
  });

  test("a streamed run is typed from its own MIME, not the decision's", async ({ page }) => {
    test.setTimeout(60_000);
    const ac3 = await findVideoByFilename(page, 'test_video_ac3.mp4');
    await TestHelpers.clearCachedConversions(ac3.hash_sha256);

    // Simulate the mismatch the ladder runs into: the decision advertises a
    // video-only type while the rung the server actually runs emits video +
    // AAC. Chromium rejects an append whose init segment does not match the
    // SourceBuffer's declared type, so a buffer typed from the decision would
    // fail a delivery that is perfectly playable — and the ladder would burn
    // its remaining rungs on it.
    await page.route('**/video?decision*', async (route) => {
      const response = await route.fetch();
      const decision = await response.json();
      await route.fulfill({
        response,
        json: { ...decision, mime: 'video/mp4; codecs="avc1.42E01E"' },
      });
    });

    await openVideo(page, ac3);
    await page.waitForFunction(
      () => {
        const el = document.querySelector('#viewer-video');
        return el && el.readyState >= 2 && el.currentTime > 0;
      },
      null,
      { timeout: 30_000 }
    );
    await expect(page.locator('.transcode-toast')).toHaveCount(0);
  });
});
